use std::io::Read;
use std::process::ExitCode;

use samsung_nasa_parser::frame::FrameParser;
use samsung_nasa_parser::packet::{u1, u2, u3, Data, Packet, PacketType, Value};

fn main() -> Result<(), ExitCode> {
    let mut buff = [0u8; 128];
    let mut stdin = std::io::stdin().lock();
    let mut frame_parser = FrameParser::new();

    loop {
        let n = stdin.read(&mut buff).map_err(|e| {
            eprintln!("{e}");
            ExitCode::FAILURE
        })?;

        if n == 0 {
            break;
        }

        for byte in &buff[..n] {
            match frame_parser.feed(*byte) {
                Ok(None) => {}
                Ok(Some(frame)) => {
                    dump_frame(frame);
                }
                Err(e) => {
                    eprintln!("frame error: {e}");
                }
            }
        }
    }

    Ok(())
}

fn dump_frame(frame: &[u8]) {
    let packet = match Packet::parse(frame) {
        Ok(packet) => packet,
        Err(e) => {
            eprintln!("packet error: {e:?}");
            return;
        }
    };

    println!("PACKET: {:?}; {} => {}; #{}", packet.data_type, packet.source, packet.destination, packet.packet_number);
    if packet.packet_info.info != u1::new(1) {
        println!("  * packet_info: INFO BIT NOT SET");
    }
    if packet.packet_info.reserved != u3::new(0) {
        println!("  * packet_info: RESERVED BITS NOT CLEAR");
    }
    if packet.packet_info.protocol_version != u2::new(2) {
        println!("  * protocol_version: NOT 2, is: {}", packet.packet_info.protocol_version);
    }
    if packet.packet_info.retry_count != u2::new(0) {
        println!("  * retry_count: {}", packet.packet_info.retry_count);
    }
    if packet.packet_type != PacketType::Normal {
        println!("  * packet_type: {:?}", packet.packet_type);
    }
    match &packet.data {
        Data::Messages(msgs) => {
            if msgs.is_empty() {
                println!("  (empty)");
            } else {
                for msg in msgs {
                    print!("  {} => ", msg.number);
                    match msg.value {
                        Value::Enum(value) => println!("0x{value:02x} ({value})"),
                        Value::Variable(value) => println!("0x{value:04x} ({value})"),
                        Value::LongVariable(value) => println!("0x{value:08x} ({value})"),
                    }
                }
            }
        }
        Data::Structure(structure) => {
            println!("  {} => {:x?}", structure.number, structure.data);
        }
    }
    println!();
}
