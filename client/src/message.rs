use samsunghvac_protocol::{message::convert::IsMessage, packet::Message};

use crate::Error;

#[derive(Default)]
pub struct MessageSet {
    messages: Vec<Message>,
}

impl MessageSet {
    pub fn new(messages: Vec<Message>) -> Self {
        MessageSet { messages }
    }

    pub fn get<M: IsMessage>(&self) -> Result<M::Value, Error> {
        for message in &self.messages {
            if let Some(value) = M::get(&message) {
                return Ok(value);
            }
        }

        Err(Error::MissingMessage(M::ID))
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }
}

impl FromIterator<Message> for MessageSet {
    fn from_iter<T: IntoIterator<Item = Message>>(iter: T) -> Self {
        MessageSet { messages: Vec::from_iter(iter) }
    }
}
