use std::io::{self, IsTerminal, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;

use samsunghvac_parser::frame::FrameParser;
use samsunghvac_parser::message;
use samsunghvac_parser::message::types::{OperationMode, PowerSetting};
use samsunghvac_parser::{frame::MAX_FRAME_SIZE, packet::{Address, Data, DataType, Packet, PacketInfo, PacketType}};
use structopt::StructOpt;

#[derive(StructOpt)]
struct Args {
    port: PathBuf,
    #[structopt(short = "A", long = "address")]
    addr: Address,
    #[structopt(subcommand)]
    cmd: Cmd,
}

#[derive(StructOpt)]
enum Cmd {
    On,
    Off,
}

const SRC_ADDR: Address = Address { class: 0x80, channel: 0x00, address: 0x10 };
const IGNORED: Address = Address { class: 0x10, channel: 0, address: 0 };

fn main() -> ExitCode {
    let args = Args::from_args();

    // let mut port = serialport::new(args.port.to_string_lossy(), 9600)
    //     .data_bits(serialport::DataBits::Eight)
    //     .flow_control(serialport::FlowControl::Hardware)
    //     .parity(serialport::Parity::Even)
    //     .stop_bits(serialport::StopBits::One)
    //     // i do not like this bit!
    //     .timeout(Duration::from_secs(1))
    //     .open_native()
    //     .unwrap();

    let mut port = UnixStream::connect(args.port).unwrap();

    let packet = Packet {
        destination: args.addr,
        source: SRC_ADDR,
        packet_info: PacketInfo::default(),
        packet_type: PacketType::Normal,
        data_type: DataType::Request,
        packet_number: 123,
        data: Data::Messages(heapless::Vec::from_slice(&[
            message::new::<message::Power>(match args.cmd {
                Cmd::On => PowerSetting::On,
                Cmd::Off => PowerSetting::Off,
            }),
            message::new::<message::Mode>(OperationMode::Fan),
        ]).unwrap())
    };

    let mut buff = [0u8; MAX_FRAME_SIZE];

    // write our packet
    let len = packet.serialize_frame(&mut buff).unwrap();
    let frame = &buff[..len];
    port.write_all(&frame).unwrap();
    port.flush().unwrap();

    // pretty print it
    pretty_print(&packet);

    // read responses
    let mut frame_parser = FrameParser::new();

    loop {
        let data = match port.read(&mut buff) {
            Ok(0) => { break; }
            Ok(n) => &buff[..n],
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                // TODO we should just make this poll or something
                continue;
            }
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };

        for byte in data {
            match frame_parser.feed(*byte) {
                Ok(None) => {}
                Ok(Some(frame)) => {
                    match Packet::parse(&frame) {
                        Ok(packet) => {
                            if packet.destination != IGNORED && packet.source != IGNORED {
                                pretty_print(&packet);
                            }
                        }
                        Err(e) => { eprintln!("{e:?}"); }
                    }
                }
                Err(e) => {
                    eprintln!("{e}");
                }
            }
        }
    }

    ExitCode::SUCCESS
}

fn pretty_print(packet: &Packet) {
    let mut rendered = String::new();
    samsunghvac_parser::pretty::pretty_print(&mut rendered, &packet, use_color()).unwrap();

    std::io::stdout().write_all(rendered.as_bytes()).unwrap();
}

fn use_color() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}
