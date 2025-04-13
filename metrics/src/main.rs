use std::collections::HashMap;
use std::{fmt, io};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::fmt::{Display, Write};

use axum::extract::State;
use axum::Router;
use futures::future;
use samsung_nasa_parser::frame::{FrameBuffer, FrameParser};
use samsung_nasa_parser::message::{self, FromMessage, FromValue};
use samsung_nasa_parser::packet::{Address, Data, DataType, MessageNumber, Packet, PacketType, Value};
use structopt::StructOpt;
use thiserror::Error;

use samsung_nasa_busd::DEFAULT_SOCKET;
use tokio::io::AsyncReadExt;
use tokio::net::UnixStream;

#[derive(StructOpt)]
struct Opt {
    #[structopt(short = "b", long = "bus", default_value_os = DEFAULT_SOCKET.as_os_str())]
    pub bus: PathBuf,
    #[structopt(short = "l", long = "listen", default_value_os = DEFAULT_SOCKET.as_os_str())]
    pub listen: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), ExitCode> {
    let opt = Opt::from_args();

    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();

    run(opt).await.map_err(|err| {
        log::error!("{err}");
        ExitCode::FAILURE
    })
}

#[derive(Error, Debug)]
enum RunError {
    #[error("listening on {1}: {0}")]
    Bind(#[source] io::Error, String),
    #[error("opening bus: {bus}: {0}", bus = .1.display())]
    OpenBus(#[source] io::Error, PathBuf),
    #[error("bus i/o: {0}")]
    RunBus(#[source] io::Error),
    #[error("serving metrics: {0}")]
    RunHttp(#[source] io::Error)
}

#[derive(Default)]
struct AppState {
    metrics: Mutex<HashMap<Address, AttrMap>>,
}

type AttrMap = HashMap<MessageNumber, Value>;

async fn run(opt: Opt) -> Result<(), RunError> {
    let state = Arc::new(AppState::default());

    let bus = UnixStream::connect(&opt.bus).await
        .map_err(|e| RunError::OpenBus(e, opt.bus))?;

    let bus_task = tokio::task::spawn({
        let state = state.clone();
        async move {
            run_bus(bus, state).await.map_err(RunError::RunBus)
        }
    });

    let app = Router::new()
        .route("/metrics", axum::routing::get(metrics))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&opt.listen).await
        .map_err(|e| RunError::Bind(e, opt.listen))?;

    let http_task = tokio::task::spawn(async move {
        axum::serve(listener, app).await.map_err(RunError::RunHttp)
    });

    let (result, _) = future::select(bus_task, http_task).await.factor_first();
    result.unwrap()
}

async fn run_bus(mut stream: UnixStream, state: Arc<AppState>) -> Result<(), io::Error> {
    let mut buff = [0u8; 128];
    let mut frame_parser = FrameParser::new();

    loop {
        let data = match stream.read(&mut buff).await {
            Ok(0) => { break }
            Ok(n) => &buff[..n],
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                // TODO we should just make this poll or something
                continue;
            }
            Err(e) => { return Err(e) }
        };

        for byte in data {
            match frame_parser.feed(*byte) {
                Ok(None) => {}
                Ok(Some(frame)) => {
                    on_frame(frame, &state);
                }
                Err(err) => {
                    log::warn!("frame error: {err}");
                }
            }
        }
    }

    Ok(())
}

fn on_frame(frame: &FrameBuffer, state: &AppState) {
    match Packet::parse(frame) {
        Ok(packet) => on_packet(&packet, state),
        Err(err) => {
            log::warn!("packet error: {err}");
        }
    }
}

fn on_packet(packet: &Packet, state: &AppState) {
    if packet.packet_type != PacketType::Normal {
        return;
    }

    if packet.data_type != DataType::Notification {
        return;
    }

    let Data::Messages(msgs) = &packet.data else {
        return;
    };

    let mut metrics = state.metrics.lock().unwrap();

    for msg in msgs {
        metrics.entry(packet.source)
            .or_default()
            .insert(msg.number, msg.value);
    }
}

async fn metrics(state: State<Arc<AppState>>) -> Result<String, ()> {
    render_metrics(&state).map_err(|_| ())
}

fn render_metrics(state: &AppState) -> Result<String, fmt::Error> {
    let mut out = String::new();

    let metrics = state.metrics.lock().unwrap();

    for (address, attrs) in metrics.iter() {
        let m = AddressMetrics { out: &mut out, address: *address };
        render_attributes(m, attrs)?;
    }

    Ok(out)
}

fn render_attributes(mut m: AddressMetrics, attrs: &AttrMap) -> fmt::Result {
    if let Some(temp) = get_message::<message::SetTemp>(&attrs) {
        m.gauge("set_temperature_celsius", temp.as_float())?;
    }

    if let Some(temp) = get_message::<message::CurrentTemp>(&attrs) {
        m.gauge("current_temperature_celsius", temp.as_float())?;
    }

    if let Some(temp) = get_message::<message::EvaInTemp>(&attrs) {
        m.gauge("coil_inlet_temperature_celsius", temp.as_float())?;
    }

    if let Some(temp) = get_message::<message::EvaOutTemp>(&attrs) {
        m.gauge("coil_outlet_temperature_celsius", temp.as_float())?;
    }

    if let Some(temp) = get_message::<message::OutdoorTemp>(&attrs) {
        m.gauge("outdoor_temperature_celsius", temp.as_float())?;
    }

    if let Some(temp) = get_message::<message::OutdoorDischargeTemp>(&attrs) {
        m.gauge("outdoor_discharge_temperature_celsius", temp.as_float())?;
    }

    if let Some(temp) = get_message::<message::OutdoorExchangerTemp>(&attrs) {
        m.gauge("outdoor_exchanger_temperature_celsius", temp.as_float())?;
    }

    // render raw notification values
    for (message, value) in attrs.iter() {
        let int = match *value {
            Value::Enum(i) => u32::from(i),
            Value::Variable(i) => u32::from(i),
            Value::LongVariable(i) => i,
        };

        writeln!(&mut m.out,
            "samsung_hvac_notification_value{{address=\"{address}\",message=\"{message}\"}} {int}",
            address = m.address,
        )?;
    }

    Ok(())
}

fn get_message<M: FromMessage>(attrs: &AttrMap) -> Option<M::Output> {
    let value = attrs.get(&M::NUMBER)?;
    M::Output::try_from_value(*value)
}

struct AddressMetrics<'a> {
    out: &'a mut String,
    address: Address,
}

impl<'a> AddressMetrics<'a> {
    pub fn gauge(&mut self, name: &str, value: impl Display) -> fmt::Result {
        self.gauge_kv(name, value, &[])
    }

    pub fn gauge_kv(&mut self, name: &str, value: impl Display, kvs: &[(&str, &str)]) -> fmt::Result {
        write!(self.out, "samsung_hvac_{name}{{address=\"{address}\"", address = self.address)?;
        for (k, v) in kvs {
            write!(self.out, ",{k}=\"{v}\"")?;
        }
        writeln!(self.out, "}} {value}")?;
        Ok(())
    }
}
