use std::borrow::Cow;

use samsunghvac_protocol::{message::convert::IsMessage, packet::Message};

use crate::Error;

#[derive(Default)]
pub struct MessageSet<'a> {
    messages: Cow<'a, [Message]>,
}

impl<'a> MessageSet<'a> {
    pub fn new(messages: &'a [Message]) -> Self {
        MessageSet { messages: Cow::Borrowed(messages) }
    }

    pub fn from_vec(messages: Vec<Message>) -> Self {
        MessageSet { messages: Cow::Owned(messages) }
    }

    pub fn get<M: IsMessage>(&self) -> Option<M::Value> {
        for message in self.messages.as_ref() {
            if let Some(value) = M::get(&message) {
                return Some(value);
            }
        }

        None
    }

    pub fn try_get<M: IsMessage>(&self) -> Result<M::Value, Error> {
        self.get::<M>().ok_or(Error::MissingMessage(M::ID))
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }
}
