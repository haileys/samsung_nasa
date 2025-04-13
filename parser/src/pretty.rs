use crate::packet::{u1, u2, u3, Data, DataType, Packet, PacketType, Value};

pub fn pretty_print(
    out: &mut dyn core::fmt::Write,
    packet: &Packet,
    use_color: bool,
) -> core::fmt::Result {
    let typ_color = color(use_color, match packet.data_type {
        DataType::Undefined => "",
        DataType::Read => "\x1b[1;32m",
        DataType::Write => "\x1b[1;33m",
        DataType::Request => "\x1b[1;95m",
        DataType::Notification => "\x1b[2m",
        DataType::Response => "\x1b[1;36m",
        DataType::Ack => "\x1b[1;34m",
        DataType::Nack => "\x1b[1;31m",
    });
    let typ_reset = color(use_color, "\x1b[0m");

    let num_color = color(use_color, "\x1b[90m");
    let num_reset = color(use_color, "\x1b[0m");

    writeln!(out, "{typ_color}{typ:?}{typ_reset} {num_color}#{num}{num_reset}: {src} => {dst}",
        typ = packet.data_type,
        src = packet.source,
        dst = packet.destination,
        num = packet.packet_number,
    )?;

    if packet.packet_info.info != u1::new(1) {
        writeln!(out, "  * packet_info: INFO BIT NOT SET")?;
    }
    if packet.packet_info.reserved != u3::new(0) {
        writeln!(out, "  * packet_info: RESERVED BITS NOT CLEAR")?;
    }
    if packet.packet_info.protocol_version != u2::new(2) {
        writeln!(out, "  * protocol_version: NOT 2, is: {}", packet.packet_info.protocol_version)?;
    }
    if packet.packet_info.retry_count != u2::new(0) {
        writeln!(out, "  * retry_count: {}", packet.packet_info.retry_count)?;
    }
    if packet.packet_type != PacketType::Normal {
        writeln!(out, "  * packet_type: {:?}", packet.packet_type)?;
    }
    match &packet.data {
        Data::Messages(msgs) => {
            if msgs.is_empty() {
                writeln!(out, "  (empty)")?;
            } else {
                for msg in msgs {
                    write!(out, "  {} => ", msg.number)?;
                    match msg.value {
                        Value::Enum(value) => writeln!(out, "0x{value:02x} ({value})")?,
                        Value::Variable(value) => writeln!(out, "0x{value:04x} ({value})")?,
                        Value::LongVariable(value) => writeln!(out, "0x{value:08x} ({value})")?,
                    }
                }
            }
        }
        Data::Structure(structure) => {
            writeln!(out, "  {} => {:x?}", structure.number, structure.data)?;
        }
    }
    writeln!(out)?;
    Ok(())
}

fn color(use_color: bool, s: &str) -> &str {
    if use_color {
        s
    } else {
        ""
    }
}
