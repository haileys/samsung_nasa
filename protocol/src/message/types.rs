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
#[derive(Display, PartialEq, PartialOrd, Eq, Ord, Clone, Copy)]
#[display("{:.1} °C", self.as_float())]
pub struct Celsius(u16);

impl Celsius {
    pub fn from_float(temp: f32) -> Self {
        Celsius(decis_from_float(temp))
    }

    pub fn as_float(&self) -> f32 {
        float_from_decis(self.0)
    }
}

impl From<CelsiusLvar> for Celsius {
    fn from(value: CelsiusLvar) -> Self {
        Celsius(value.0)
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

#[derive(Display, PartialEq, PartialOrd, Eq, Ord, Clone, Copy)]
#[display("{:.1} °C", self.as_float())]
/// This is a celsius value, but represented in the high 16 bits of a
/// 32 bit long variable for some reason
pub struct CelsiusLvar(u16);

impl CelsiusLvar {
    pub fn from_float(temp: f32) -> Self {
        CelsiusLvar(decis_from_float(temp))
    }

    pub fn as_float(&self) -> f32 {
        float_from_decis(self.0)
    }
}

impl ValueType for CelsiusLvar {
    type Err = Infallible;
    type Repr = u32;

    fn try_from_repr(value: u32) -> Result<Self, Infallible> {
        let value = (value >> 16) as u16;
        Ok(CelsiusLvar(value))
    }

    fn to_repr(&self) -> u32 {
        (self.0 as u32) << 16
    }
}

fn decis_from_float(value: f32) -> u16 {
    f32::round(value * 10.0) as u16
}

fn float_from_decis(decis: u16) -> f32 {
    decis as f32 / 10.0
}

macro_rules! define_enum {
    { enum $name:ident { $( $variant:ident = $value:expr, )+ } } => {
        #[repr(u8)]
        #[derive(Debug, Display, Clone, Copy, PartialEq, Eq)]
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

define_enum! {
    enum FanSetting {
        Auto = 0,
        Low = 1,
        Medium = 2,
        High = 3,
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
