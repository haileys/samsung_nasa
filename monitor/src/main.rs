use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use samsung_nasa_monitor::pretty_print;
use samsung_nasa_parser::frame::FrameParser;
use samsung_nasa_parser::packet::{Address, Packet};

use serialport::{DataBits, FlowControl, Parity, StopBits};
use structopt::StructOpt;

/// Monitors traffic on Samsung NASA bus.
/// Reads from stdin by default if path to serial port not specified.
#[derive(StructOpt)]
struct Args {
    #[structopt(short = "i", long = "ignore", help = "ignore traffic to/from an address")]
    ignore: Vec<Address>,
    #[structopt(short = "S", long = "save-raw", help = "save raw bus traffic to file")]
    save: Option<PathBuf>,
    path: PathBuf,
}

fn main() -> Result<(), ExitCode> {
    let args = Args::from_args();

    let save_raw = args.save.map(|path| {
        File::create(&path).map_err(|e| {
            eprintln!("error opening file {}: {}", path.display(), e);
            ExitCode::FAILURE
        })
    }).transpose()?;

    let mut io = match open_path(&args.path) {
        Ok(io) => io,
        Err(e) => {
            eprintln!("error opening port {}: {}", args.path.display(), e);
            return Err(ExitCode::FAILURE);
        }
    };

    monitor(&mut io, save_raw, &args.ignore).map_err(|e| {
        eprintln!("{e}");
        ExitCode::FAILURE
    })
}

fn open_path(path: &Path) -> Result<Box<dyn Read>, io::Error> {
    let port = serialport::new(path.to_string_lossy(), 9600)
        .data_bits(DataBits::Eight)
        .flow_control(FlowControl::None)
        .parity(Parity::Even)
        .stop_bits(StopBits::One)
        // i do not like this bit!
        .timeout(Duration::from_secs(1))
        .open()?;

    Ok(port as Box<dyn Read>)
}

fn monitor(io: &mut dyn Read, mut save_raw: Option<File>, ignore: &[Address]) -> Result<(), io::Error> {
    let mut buff = [0u8; 128];
    let mut frame_parser = FrameParser::new();

    loop {
        let data = match io.read(&mut buff) {
            Ok(0) => { break }
            Ok(n) => &buff[..n],
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                // TODO we should just make this poll or something
                continue;
            }
            Err(e) => { return Err(e) }
        };

        if let Some(raw) = save_raw.as_mut() {
            if let Err(e) = raw.write_all(data) {
                eprintln!("error writing to raw save file: {e}");
            }
        }

        for byte in data {
            match frame_parser.feed(*byte) {
                Ok(None) => {}
                Ok(Some(frame)) => {
                    dump_frame(frame, ignore);
                }
                Err(e) => {
                    eprintln!("frame error: {e}");
                }
            }
        }
    }

    Ok(())
}

fn dump_frame(frame: &[u8], ignore: &[Address]) {
    let packet = match Packet::parse(frame) {
        Ok(packet) => packet,
        Err(e) => {
            eprintln!("packet error: {e:?}");
            return;
        }
    };

    if ignore.contains(&packet.source) || ignore.contains(&packet.destination) {
        return;
    }

    pretty_print(&packet);
}
