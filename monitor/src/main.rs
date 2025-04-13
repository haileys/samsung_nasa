use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;

use samsunghvac_client::transport::{self, TransportOpt, TransportReceiver};
use samsunghvac_parser::packet::Address;

use structopt::StructOpt;
use thiserror::Error;

/// Monitors traffic on Samsung NASA bus.
/// Reads from stdin by default if path to serial port not specified.
#[derive(StructOpt)]
struct Opt {
    #[structopt(short = "i", long = "ignore", help = "ignore traffic to/from an address")]
    ignore: Vec<Address>,
    #[structopt(flatten)]
    transport: TransportOpt,
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

#[derive(Debug, Error)]
#[error(transparent)]
enum RunError {
    OpenBus(#[from] transport::OpenError),
    Monitor(#[from] io::Error),
}

async fn run(opt: Opt) -> Result<(), RunError> {
    let (mut rd, _wr) = transport::open(&opt.transport).await?;
    monitor(&mut rd, &opt.ignore).await?;
    Ok(())
}

async fn monitor(rd: &mut TransportReceiver, ignore: &[Address]) -> Result<(), io::Error> {
    loop {
        let packet = rd.read().await?;

        if ignore.contains(&packet.source) || ignore.contains(&packet.destination) {
            continue;
        }

        let mut rendered = String::new();
        samsunghvac_parser::pretty::pretty_print(&mut rendered, &packet, use_color()).unwrap();
        std::io::stdout().write_all(rendered.as_bytes()).unwrap();
    }
}

fn use_color() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}
