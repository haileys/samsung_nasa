use std::io;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::LazyLock;

use bytes::{Bytes, BytesMut};
use futures::{future, pin_mut, Stream, StreamExt};
use async_stream::try_stream;
use samsung_nasa_parser::frame::{FrameBuffer, FrameError, FrameParser, MAX_FRAME_SIZE};
use samsung_nasa_parser::packet::{Packet, PacketError, SerializePacketError};
use structopt::StructOpt;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, broadcast};
use tokio_serial::{SerialPortBuilderExt, SerialStream};

static DEFAULT_SOCKET: LazyLock<PathBuf> = LazyLock::new(|| {
    runtime_dir().join("bus")
});

const BAUD_RATE: u32 = 9600;

#[derive(StructOpt)]
struct Opt {
    #[structopt(short = "l", long = "listen", default_value_os = DEFAULT_SOCKET.as_os_str())]
    pub socket: PathBuf,
    pub port: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), ExitCode> {
    let opt = Opt::from_args();

    env_logger::init();

    run(opt).await.map_err(|err| {
        log::error!("{err}");
        ExitCode::FAILURE
    })
}

async fn run(opt: Opt) -> Result<(), RunError> {
    let listen = UnixListener::bind(&opt.socket)
        .map_err(|err| RunError::Bind(err, opt.socket.clone()))?;

    let port = open_serial_port(&opt.port)
        .map_err(|err| RunError::OpenPort(err, opt.port.clone()))?;

    let (recv_tx, _) = tokio::sync::broadcast::channel(16);
    let (send_tx, send_rx) = tokio::sync::mpsc::channel(16);

    let bus = {
        let recv_tx = recv_tx.clone();
        async move {
            tokio::spawn(run_bus(port, recv_tx, send_rx))
                .await
                .unwrap()
                .map_err(RunError::Bus)
        }
    };

    let accept = async move {
        let recv_tx = recv_tx.clone();
        let send_tx = send_tx.clone();
        tokio::spawn(run_accept(listen, recv_tx, send_tx))
            .await
            .unwrap()
            .map_err(RunError::Accept)
    };

    race(bus, accept).await
}

#[derive(Error, Debug)]
enum RunError {
    #[error("binding {path}: {0}", path = .1.display())]
    Bind(#[source] io::Error, PathBuf),
    #[error("opening bus port {1}: {0}")]
    OpenPort(#[source] serialport::Error, String),
    #[error("bus i/o: {0}")]
    Bus(#[source] io::Error),
    #[error("accepting client: {0}")]
    Accept(#[source] io::Error),
}

async fn run_accept(
    listen: UnixListener,
    recv_tx: broadcast::Sender<Bytes>,
    send_tx: mpsc::Sender<Box<Packet>>,
) -> io::Result<()> {
    loop {
        let (client, _) = listen.accept().await?;

        let recv_rx = recv_tx.subscribe();
        let send_tx = send_tx.clone();
        tokio::spawn(run_client(client, recv_rx, send_tx));
    }
}

async fn run_bus(
    port: SerialStream,
    recv_tx: broadcast::Sender<Bytes>,
    mut send_rx: mpsc::Receiver<Box<Packet>>,
) -> io::Result<()> {
    let (port_rx, mut port_tx) = tokio::io::split(port);

    let send = async move {
        let mut buffer = FrameBuffer::new();

        while let Some(packet) = send_rx.recv().await {
            let frame = match packet.serialize_frame(&mut buffer) {
                Ok(n) => &buffer[..n],
                Err(err) => {
                    log::warn!("serializing frame sending to bus: {err:?}");
                    continue;
                }
            };

            port_tx.write_all(&frame).await?;
        }

        Ok(())
    };

    let recv = async move {
        let packets = packet_stream(port_rx);
        pin_mut!(packets);

        while let Some(packet) = packets.next().await {
            let packet = match packet? {
                Ok(packet) => packet,
                Err(err) => {
                    log::warn!("reading bus packet: {err}");
                    continue;
                }
            };

            // we've validated that the packet looks ok!
            // re-serialize it to send to clients.
            // this ensures it roundtrips correctly
            match serialize_frame(&packet) {
                Ok(bytes) => {
                    let _: Result<_, _> = recv_tx.send(bytes.into());
                }
                Err(err) => {
                    log::warn!("serializing frame received from bus: {err}");
                }
            }
        }

        log::info!("bus hung up, exiting");
        Ok(())
    };

    race(send, recv).await
}

async fn run_client(
    client: UnixStream,
    mut recv_rx: broadcast::Receiver<Bytes>,
    send_tx: mpsc::Sender<Box<Packet>>,
) {
    let (client_rx, mut client_tx) = tokio::io::split(client);

    let recv = async move {
        let packets = packet_stream(client_rx);
        pin_mut!(packets);

        while let Some(packet) = packets.next().await {
            let packet = match packet? {
                Ok(packet) => packet,
                Err(err) => {
                    log::warn!("reading client packet: {err}");
                    continue;
                }
            };

            if let Err(_) = send_tx.send(packet).await {
                // bus task has quit, we should quit too
                break;
            }
        }

        io::Result::Ok(())
    };

    let send = async move {
        loop {
            let bytes = match recv_rx.recv().await {
                Ok(bytes) => bytes,
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            };

            client_tx.write_all(&bytes).await?;
        }

        Ok(())
    };

    if let Err(err) = race(recv, send).await {
        log::warn!("client i/o error: {err}");
    }
}

async fn race<T>(a: impl Future<Output = T>, b: impl Future<Output = T>) -> T {
    pin_mut!(a);
    pin_mut!(b);
    future::select(a, b).await.factor_first().0
}

fn serialize_frame(packet: &Packet) -> Result<Bytes, SerializePacketError> {
    let mut bytes = BytesMut::zeroed(MAX_FRAME_SIZE);
    let n = packet.serialize_frame(&mut bytes)?;
    bytes.truncate(n);
    Ok(bytes.into())
}
fn packet_stream(io: impl AsyncRead)
    -> impl Stream<Item = io::Result<Result<Box<Packet>, ReadPacketError>>>
{
    try_stream! {
        pin_mut!(io);

        let mut parser = FrameParser::new();
        let mut buffer = FrameBuffer::new();

        loop {
            let data = match io.read(&mut buffer).await? {
                0 => break,
                n => &buffer[..n],
            };

            for byte in data {
                let frame = match parser.feed(*byte) {
                    Ok(None) => continue,
                    Ok(Some(frame)) => frame,
                    Err(err) => {
                        yield Err(err.into());
                        continue;
                    }
                };

                let packet = match Packet::parse(frame) {
                    Ok(packet) => packet,
                    Err(err) => {
                        yield Err(err.into());
                        continue;
                    }
                };

                yield Ok(Box::new(packet));
            }
        }
    }
}

#[derive(Error, Debug)]
enum ReadPacketError {
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error(transparent)]
    Packet(#[from] PacketError),
}

fn open_serial_port(path: &str) -> Result<SerialStream, tokio_serial::Error> {
    tokio_serial::new(path, BAUD_RATE)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::Even)
        .stop_bits(serialport::StopBits::One)
        .open_native_async()
}

fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("RUNTIME_DIRECTORY") {
        PathBuf::from(dir)
    } else {
        PathBuf::from("/var/run/samsunghvac")
    }
}
