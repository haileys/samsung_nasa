use core::fmt::Display;

use crate::packet::{Message, MessageId, Value, WrongValueKind};

pub struct TypedMessage<const N: u16, T>(pub T);

impl<const N: u16, T: ValueType> IsMessage for TypedMessage<N, T> {
    type Value = T;
    const ID: MessageId = MessageId(N);

    fn get(msg: &Message) -> Option<Self::Value> {
        if msg.id == Self::ID {
            let repr = T::Repr::try_from_value(msg.value).ok()?;

            match T::try_from_repr(repr) {
                Ok(value) => Some(value),
                Err(err) => {
                    log::warn!("deserializing message {}: {err} (repr = {repr})", msg.id);
                    None
                }
            }
        } else {
            None
        }
    }

    fn new(value: T) -> Self {
        Self(value)
    }

    fn to_message(&self) -> Message {
        Message {
            id: Self::ID,
            value: self.0.to_value(),
        }
    }
}

pub trait IsMessage {
    type Value: Sized + ValueType;
    const ID: MessageId;
    fn get(msg: &Message) -> Option<Self::Value>;
    fn new(value: Self::Value) -> Self;
    fn to_message(&self) -> Message;
}

pub trait ValueType: Sized {
    type Err: Display + Sized;
    type Repr: ValueRepr;

    fn try_from_repr(repr: Self::Repr) -> Result<Self, Self::Err>;
    fn to_repr(&self) -> Self::Repr;

    fn try_from_value(value: Value) -> Option<Self> {
        let repr = Self::Repr::try_from_value(value).ok()?;
        Self::try_from_repr(repr).ok()
    }

    fn to_value(&self) -> Value {
        self.to_repr().to_value()
    }
}

pub trait ValueRepr: Display + Sized + Copy {
    fn try_from_value(value: Value) -> Result<Self, WrongValueKind>;
    fn to_value(&self) -> Value;
}

impl ValueRepr for u8 {
    fn try_from_value(value: Value) -> Result<Self, WrongValueKind> {
        value.expect_u8()
    }
    fn to_value(&self) -> Value {
        Value::Enum(*self)
    }
}

impl ValueRepr for u16 {
    fn try_from_value(value: Value) -> Result<Self, WrongValueKind> {
        value.expect_u16()
    }
    fn to_value(&self) -> Value {
        Value::Variable(*self)
    }
}

impl ValueRepr for u32 {
    fn try_from_value(value: Value) -> Result<Self, WrongValueKind> {
        value.expect_u32()
    }
    fn to_value(&self) -> Value {
        Value::LongVariable(*self)
    }
}
