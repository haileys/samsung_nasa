use core::{convert::Infallible, marker::PhantomData};

use derive_more::Display;

use crate::packet::{Message, MessageNumber, Value, WrongValueKind};

pub const POWER: MessageNumber = MessageNumber(0x4000);
pub const MODE: MessageNumber = MessageNumber(0x4001);
pub const MODE_REAL: MessageNumber = MessageNumber(0x4002);
pub const FAN_SPEED: MessageNumber = MessageNumber(0x4006);
pub const FAN_MODE_REAL: MessageNumber = MessageNumber(0x4007);
pub const THERMO: MessageNumber = MessageNumber(0x4028);
pub const DEFROST: MessageNumber = MessageNumber(0x402e);
pub const USE_SILENCE: MessageNumber = MessageNumber(0x4045);
pub const CONTROL_SILENCE: MessageNumber = MessageNumber(0x4046);
pub const OUTDOOR_SERVICE_MODE: MessageNumber = MessageNumber(0x8000);
pub const OUTDOOR_DRIVE_MODE: MessageNumber = MessageNumber(0x8001);
pub const OUTDOOR_MODE: MessageNumber = MessageNumber(0x8003);
pub const OUTDOOR_COMP1_STATUS: MessageNumber = MessageNumber(0x8010);
pub const OUTDOOR_4WAY_STATUS: MessageNumber = MessageNumber(0x801a);
pub const INDOOR_DEFROST_STAGE: MessageNumber = MessageNumber(0x8061);

pub type SetTemp = TypedMessage<0x4201, Celsius>;
pub type CurrentTemp = TypedMessage<0x4203, Celsius>;
pub type EvaInTemp = TypedMessage<0x4201, Celsius>;
pub type EvaOutTemp = TypedMessage<0x4203, Celsius>;
pub type OutdoorTemp = TypedMessage<0x8204, Celsius>;
pub type OutdoorDischargeTemp = TypedMessage<0x820a, Celsius>;
pub type OutdoorExchangerTemp = TypedMessage<0x8218, Celsius>;

pub struct TypedMessage<const N: u16, T> {
    _phantom: PhantomData<T>,
}

impl<const N: u16, T: FromValue> FromMessage for TypedMessage<N, T> {
    type Output = T;
    const NUMBER: MessageNumber = MessageNumber(N);

    fn get(msg: &Message) -> Option<Self::Output> {
        if msg.number == Self::NUMBER {
            let repr = T::Repr::try_from_value(msg.value).ok()?;
            T::try_from_repr(repr).ok()
        } else {
            None
        }
    }
}

pub trait FromMessage {
    type Output: Sized + FromValue;
    const NUMBER: MessageNumber;
    fn get(msg: &Message) -> Option<Self::Output>;
}

#[derive(Display)]
#[display("{:.1} °c", self.0)]
pub struct Celsius(u16);

impl Celsius {
    pub fn from_float(f: f32) -> Self {
        Celsius(f32::round(f * 10.0) as u16)
    }

    pub fn as_float(&self) -> f32 {
        self.0 as f32 / 10.0
    }
}

impl FromValue for Celsius {
    type Err = Infallible;
    type Repr = u16;
    fn try_from_repr(value: u16) -> Result<Self, Infallible> {
        Ok(Celsius(value))
    }
}

pub trait FromValue: Sized {
    type Err: Sized;
    type Repr: ValueType;
    fn try_from_repr(repr: Self::Repr) -> Result<Self, Self::Err>;
    fn try_from_value(value: Value) -> Option<Self> {
        let repr = Self::Repr::try_from_value(value).ok()?;
        Self::try_from_repr(repr).ok()
    }
}

pub trait ValueType: Sized {
    fn try_from_value(value: Value) -> Result<Self, WrongValueKind>;
}

impl ValueType for u8 {
    fn try_from_value(value: Value) -> Result<Self, WrongValueKind> {
        value.expect_u8()
    }
}
impl ValueType for u16 {
    fn try_from_value(value: Value) -> Result<Self, WrongValueKind> {
        value.expect_u16()
    }
}
impl ValueType for u32 {
    fn try_from_value(value: Value) -> Result<Self, WrongValueKind> {
        value.expect_u32()
    }
}
