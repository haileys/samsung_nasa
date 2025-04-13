use core::convert::Infallible;

use derive_more::Display;
use thiserror::Error;

use super::convert::ValueType;

#[derive(Debug, Error)]
#[error("enum value out of range: {enum_name}: {value}")]
pub struct EnumOutOfRange {
    pub enum_name: &'static str,
    pub value: u8,
}

// Celcius
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

impl ValueType for Celsius {
    type Err = Infallible;
    type Repr = u16;

    fn try_from_repr(value: u16) -> Result<Self, Infallible> {
        Ok(Celsius(value))
    }

    fn to_repr(&self) -> u16 {
        self.0
    }
}

macro_rules! define_enum {
    { enum $name:ident { $( $variant:ident = $value:expr, )+ } } => {
        #[repr(u8)]
        #[derive(Debug, Display, Clone, Copy)]
        #[display("{:?}", self)]
        pub enum $name {
            $(
                $variant = $value,
            )+
        }

        impl ValueType for $name {
            type Err = EnumOutOfRange;
            type Repr = u8;

            fn try_from_repr(repr: u8) -> Result<Self, Self::Err> {
                match repr {
                    $( $value => Ok($name::$variant), )+
                    _ => Err(EnumOutOfRange { enum_name: stringify!($name), value: repr })
                }
            }

            fn to_repr(&self) -> u8 {
                *self as u8
            }
        }
    };
}

define_enum! {
    enum PowerSetting {
        Off = 0,
        On = 1,
        On2 = 2,
    }
}

define_enum! {
    enum OperationMode {
        Auto = 0,
        Cool = 1,
        Dry = 2,
        Fan = 3,
        Heat = 4,
        AutoCool = 11,
        AutoDry = 12,
        AutoFan = 13,
        AutoHeat = 14,
    }
}

impl ValueType for bool {
    type Err = EnumOutOfRange;
    type Repr = u8;

    fn try_from_repr(repr: u8) -> Result<Self, Self::Err> {
        match repr {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(EnumOutOfRange { enum_name: "bool", value: repr }),
        }
    }

    fn to_repr(&self) -> u8 {
        *self as u8
    }
}
