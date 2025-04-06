use core::str::FromStr;

use derive_more::{Debug, Display};

use thiserror::Error;
pub use ux::{u1, u2, u3, u4};

use crate::frame::{crc16, FRAME_END, FRAME_START};

pub const MAX_MESSAGE_COUNT: usize = u8::MAX as usize;
pub const MAX_STRUCTURE_SIZE: usize = 256;

pub type MessagesVec = heapless::Vec<Message, MAX_MESSAGE_COUNT>;
pub type StructureData = heapless::Vec<u8, MAX_STRUCTURE_SIZE>;

#[derive(Debug)]
pub struct Packet {
    pub source: Address,
    pub destination: Address,
    pub packet_info: PacketInfo,
    pub packet_type: PacketType,
    pub data_type: DataType,
    pub packet_number: u8,
    pub data: Data,
}

#[derive(Debug)]
pub enum Data {
    Messages(MessagesVec),
    Structure(Structure),
}

#[derive(Debug, Error)]
pub enum PacketError {
    /// reached end of packet while reading data
    #[error("unexpected end of packet")]
    TooShort,
    /// unknown packet type
    #[error("unknown packet type: {0}")]
    UnknownPacketType(u4),
    /// encounted a structure unexpectedly - structures may only appear
    /// as the sole message in a packet
    #[error("invalid structure in packet payload")]
    UnexpectedStructure,
    #[error("structure too large: {size}")]
    StructureTooLong { size: usize },
}

#[derive(Debug, Error)]
pub enum SerializePacketError {
    /// reached end of buffer while writing packet
    #[error("buffer too short")]
    BufferTooShort,
    /// an invalid value type was supplied for a message
    #[error("packet message contains value of wrong type")]
    InvalidMessageValue,
    /// written packet would be too large to transmit
    #[error("packet too long for wire")]
    PacketTooLong,
}

impl Packet {
    pub fn parse(data: &[u8]) -> Result<Self, PacketError> {
        let mut reader = PacketReader::new(data);

        let source = Address::from_bytes(reader.read_array()?);
        let destination = Address::from_bytes(reader.read_array()?);

        let packet_info = PacketInfo::from_byte(reader.read_u8()?);

        let byte = reader.read_u8()?;
        let packet_type = PacketType::from_u4(u4::new(byte >> 4))?;
        let data_type = DataType::from_u4(u4::new(byte & 0xf));

        let packet_number = reader.read_u8()?;
        let message_count = reader.read_u8()?;

        let data = read_payload(message_count, &mut reader)?;

        Ok(Packet {
            source,
            destination,
            packet_info,
            packet_type,
            data_type,
            packet_number,
            data,
        })
    }

    pub fn serialize_frame(&self, out: &mut [u8]) -> Result<usize, SerializePacketError> {
        // start frame
        let mut writer = PacketWriter::new(out);

        writer.write_u8(0xfd)?;
        writer.write_u8(0xf8)?;
        writer.write_u8(0xef)?;
        writer.write_u8(0x7c)?;

        writer.write_u8(FRAME_START)?;

        // write zero for length, we'll fill it in later
        let len_pos = writer.pos;
        writer.write_u16(0)?;

        // serialize frame data
        let data_pos = writer.pos;
        let data_len = self.serialize(&mut writer)?;

        // calculate and write crc16
        let crc = crc16(&writer.buff[data_pos..][..data_len]);
        writer.write_u16(crc)?;

        // frame length on the wire includes length field and crc
        let wire_len = u16::try_from(writer.pos - len_pos)
            .map_err(|_| SerializePacketError::PacketTooLong)?;

        // fix up frame length
        writer.buff[len_pos..][..2].copy_from_slice(&u16::to_be_bytes(wire_len));

        // finish frame
        writer.write_u8(FRAME_END)?;

        Ok(writer.pos)
    }

    fn serialize(&self, writer: &mut PacketWriter) -> Result<usize, SerializePacketError> {
        writer.write_array(self.source.to_bytes())?;
        writer.write_array(self.destination.to_bytes())?;
        writer.write_u8(self.packet_info.to_byte())?;

        let mut byte = 0u8;
        byte |= u8::from(self.packet_type.to_u4()) << 4;
        byte |= u8::from(self.data_type.to_u4());
        writer.write_u8(byte)?;

        writer.write_u8(self.packet_number)?;

        match &self.data {
            Data::Messages(messages) => {
                writer.write_u8(u8::try_from(messages.len()).unwrap())?;

                for message in messages {
                    writer.write_u16(message.number.0)?;
                    match (message.number.kind(), message.value) {
                        (MessageKind::Enum, Value::Enum(value)) => {
                            writer.write_u8(value)?;
                        }
                        (MessageKind::Variable, Value::Variable(value)) => {
                            writer.write_u16(value)?;
                        }
                        (MessageKind::LongVariable, Value::LongVariable(value)) => {
                            writer.write_u32(value)?;
                        }
                        _ => { return Err(SerializePacketError::InvalidMessageValue); }
                    }
                }
            }
            Data::Structure(structure) => {
                // message count:
                writer.write_u8(1)?;

                if structure.number.kind() != MessageKind::Structure {
                    return Err(SerializePacketError::InvalidMessageValue);
                }

                writer.write_u16(structure.number.0)?;
                writer.write_bytes(&structure.data)?;
            }
        }

        Ok(writer.pos)
    }
}

fn read_payload(message_count: u8, reader: &mut PacketReader) -> Result<Data, PacketError> {
    let mut messages = MessagesVec::new();

    for i in 0..message_count {
        let number = MessageNumber(reader.read_u16()?);
        let value = match number.kind() {
            MessageKind::Enum => Value::Enum(reader.read_u8()?),
            MessageKind::Variable => Value::Variable(reader.read_u16()?),
            MessageKind::LongVariable => Value::LongVariable(reader.read_u32()?),
            MessageKind::Structure => {
                if i != 0 || message_count != 1 {
                    return Err(PacketError::UnexpectedStructure);
                }

                let data = StructureData::from_slice(reader.data)
                    .map_err(|_| PacketError::StructureTooLong { size: reader.data.len() })?;

                return Ok(Data::Structure(Structure { number, data }));
            }
        };

        messages.push(Message { number, value })
            .map_err(|_| ())
            .expect("exceeded messages capacity, should never happen");
    }

    Ok(Data::Messages(messages))
}

#[derive(Debug, Display, PartialEq, Eq)]
#[debug("{class:02x}.{channel:02x}.{address:02x}")]
#[display("{:?}", self)]
pub struct Address {
    pub class: u8,
    pub channel: u8,
    pub address: u8,
}

impl Address {
    pub fn to_bytes(&self) -> [u8; 3] {
        [self.class, self.channel, self.address]
    }

    pub fn from_bytes(bytes: [u8; 3]) -> Self {
        Address {
            class: bytes[0],
            channel: bytes[1],
            address: bytes[2],
        }
    }
}

#[derive(Display, Debug)]
#[display("invalid address")]
pub struct InvalidAddress;

impl FromStr for Address {
    type Err = InvalidAddress;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.is_ascii() {
            return Err(InvalidAddress);
        }

        if s.len() != 8 {
            return Err(InvalidAddress);
        }

        if &s[2..3] != "." || &s[5..6] != "." {
            return Err(InvalidAddress);
        }

        return Ok(Address {
            class: parse_hex(&s[0..2])?,
            channel: parse_hex(&s[3..5])?,
            address: parse_hex(&s[6..8])?,
        });

        fn parse_hex(s: &str) -> Result<u8, InvalidAddress> {
            u8::from_str_radix(s, 16).map_err(|_| InvalidAddress)
        }
    }
}

#[derive(Debug)]
pub struct PacketInfo {
    /// dunno what this is?
    pub info: u1,
    pub protocol_version: u2,
    pub retry_count: u2,
    pub reserved: u3,
}

impl PacketInfo {
    pub fn new() -> Self {
        Self::with_retry_count(u2::new(0))
    }

    pub fn with_retry_count(retry_count: u2) -> Self {
        PacketInfo {
            info: u1::new(1),
            protocol_version: u2::new(2),
            retry_count,
            reserved: u3::new(0),
        }
    }

    pub fn from_byte(byte: u8) -> Self {
        PacketInfo {
            info: u1::new(byte >> 7),
            protocol_version: u2::new((byte & 0x60) >> 5),
            retry_count: u2::new((byte & 0x18) >> 3),
            reserved: u3::new(byte & 0x07),
        }
    }

    pub fn to_byte(&self) -> u8 {
        let mut byte = 0;
        byte |= 1 << 7;
        byte |= u8::from(self.protocol_version) << 5;
        byte |= u8::from(self.retry_count) << 3;
        byte
    }
}

impl Default for PacketInfo {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum PacketType {
    StandBy = 0,
    Normal = 1,
    Gathering = 2,
    Install = 3,
    Download = 4,
}

impl PacketType {
    pub fn from_u4(u: u4) -> Result<Self, PacketError> {
        match u8::from(u) {
            0 => Ok(PacketType::StandBy),
            1 => Ok(PacketType::Normal),
            2 => Ok(PacketType::Gathering),
            3 => Ok(PacketType::Install),
            4 => Ok(PacketType::Download),
            _ => Err(PacketError::UnknownPacketType(u))
        }
    }

    pub fn to_u4(self) -> u4 {
        u4::new(self as u8)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(u8)]
pub enum DataType {
    Undefined = 0,
    Read = 1,
    Write = 2,
    Request = 3,
    Notification = 4,
    Response = 5,
    Ack = 6,
    Nack = 7,
}

impl DataType {
    pub fn from_u4(u: u4) -> Self {
        match u8::from(u) {
            0 => DataType::Undefined,
            1 => DataType::Read,
            2 => DataType::Write,
            3 => DataType::Request,
            4 => DataType::Notification,
            5 => DataType::Response,
            6 => DataType::Ack,
            7 => DataType::Nack,
            _ => unreachable!(),
        }
    }

    pub fn to_u4(self) -> u4 {
        u4::new(self as u8)
    }
}

#[derive(Debug, Clone)]
pub struct Message {
    pub number: MessageNumber,
    pub value: Value,
}

#[derive(Debug, Display, Clone, Copy)]
#[debug("MessageNumber({:04x?})", self.0)]
#[display("{:04x?}", self.0)]
pub struct MessageNumber(pub u16);

impl MessageNumber {
    pub fn kind(&self) -> MessageKind {
        match (self.0 & 0x0600) >> 9 {
            0 => MessageKind::Enum,
            1 => MessageKind::Variable,
            2 => MessageKind::LongVariable,
            3 => MessageKind::Structure,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageKind {
    Enum = 0,
    Variable = 1,
    LongVariable = 2,
    Structure = 3,
}

#[derive(Debug, Clone, Copy)]
pub enum Value {
    Enum(u8),
    Variable(u16),
    LongVariable(u32),
}

#[derive(Debug)]
pub struct Structure {
    pub number: MessageNumber,
    pub data: StructureData,
}

struct PacketReader<'a> {
    data: &'a [u8],
}

impl<'a> PacketReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        PacketReader { data }
    }

    pub fn read_array<const N: usize>(&mut self) -> Result<[u8; N], PacketError> {
        let (head, tail) = self.data.split_at_checked(N)
            .ok_or(PacketError::TooShort)?;

        let bytes = head.try_into().unwrap();
        self.data = tail;

        Ok(bytes)
    }

    pub fn read_u8(&mut self) -> Result<u8, PacketError> {
        let [byte] = self.read_array()?;
        Ok(byte)
    }

    pub fn read_u16(&mut self) -> Result<u16, PacketError> {
        Ok(u16::from_be_bytes(self.read_array()?))
    }

    pub fn read_u32(&mut self) -> Result<u32, PacketError> {
        Ok(u32::from_be_bytes(self.read_array()?))
    }
}

#[derive(Debug)]
pub struct BufferTooSmall;

struct PacketWriter<'a> {
    buff: &'a mut [u8],
    pos: usize,
}

impl<'a> PacketWriter<'a> {
    pub fn new(buff: &'a mut [u8]) -> Self {
        PacketWriter { buff, pos: 0 }
    }

    pub fn remaining_mut(&mut self) -> &mut [u8] {
        &mut self.buff[self.pos..]
    }

    pub fn write_bytes(&mut self, slice: &[u8]) -> Result<(), SerializePacketError> {
        let (head, _) = self.remaining_mut()
            .split_at_mut_checked(slice.len())
            .ok_or(SerializePacketError::BufferTooShort)?;

        head.copy_from_slice(&slice);
        self.pos += slice.len();

        Ok(())
    }

    pub fn write_array<const N: usize>(&mut self, array: [u8; N]) -> Result<(), SerializePacketError> {
        self.write_bytes(&array)
    }

    pub fn write_u8(&mut self, u: u8) -> Result<(), SerializePacketError> {
        self.write_array([u])
    }

    pub fn write_u16(&mut self, u: u16) -> Result<(), SerializePacketError> {
        self.write_array(u16::to_be_bytes(u))
    }

    pub fn write_u32(&mut self, u: u32) -> Result<(), SerializePacketError> {
        self.write_array(u32::to_be_bytes(u))
    }
}
