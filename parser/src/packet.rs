use derive_more::{Debug, Display};

pub use ux::{u1, u2, u3, u4};

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

#[derive(Debug)]
pub enum PacketError {
    /// reached end of packet while reading data
    TooShort,
    /// unknown packet type
    UnknownPacketType(u4),
    /// encounted a structure unexpectedly - structures may only appear
    /// as the sole message in a packet
    UnexpectedStructure,
    StructureTooLong { size: usize },
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

#[derive(Debug, Display)]
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
}

impl Default for PacketInfo {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, PartialEq, Eq)]
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
}

#[derive(Debug)]
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
}

#[derive(Debug)]
pub struct Message {
    pub number: MessageNumber,
    pub value: Value,
}

#[derive(Debug, Display)]
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

#[derive(Debug)]
#[repr(u8)]
pub enum MessageKind {
    Enum = 0,
    Variable = 1,
    LongVariable = 2,
    Structure = 3,
}

#[derive(Debug)]
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
