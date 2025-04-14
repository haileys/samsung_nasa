use std::collections::HashMap;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Mutex;
use std::task::{Context, Poll};

use futures::{ready, Stream, StreamExt};
use samsunghvac_protocol::message::convert::IsMessage;
use samsunghvac_protocol::packet::{Address, Message, MessageId, Value};
use tokio::sync::watch;
use tokio_stream::wrappers::WatchStream;

#[derive(Default)]
pub struct WatchRegistry {
    watches: Mutex<HashMap<Address, HashMap<MessageId, WatchRegistration>>>,
}

impl WatchRegistry {
    pub fn notify(&self, sender: Address, messages: &[Message]) {
        let watches = self.watches.lock().unwrap();
        for message in messages {
            let Some(messages) = watches.get(&sender) else { continue };
            let Some(register) = messages.get(&message.id) else { continue };
            register.notify(message);
        }
    }

    pub fn subscribe<M: IsMessage>(&self, sender: Address) -> Watch<M> {
        let rx = {
            let mut watches = self.watches.lock().unwrap();
            let messages = watches.entry(sender).or_default();
            let register = messages.entry(M::ID).or_default();
            register.tx.subscribe()
        };

        Watch { rx: WatchStream::new(rx), _phantom: PhantomData }
    }

    pub fn all_watches(&self) -> Vec<(Address, Vec<MessageId>)> {
        let watches = self.watches.lock().unwrap();

        watches.iter().map(|(address, messages)| {
            (*address, messages.keys().copied().collect())
        }).collect()
    }
}

#[pin_project::pin_project]
pub struct Watch<M: IsMessage> {
    #[pin]
    rx: WatchStream<Option<Value>>,
    _phantom: PhantomData<M>,
}

impl<M: IsMessage> Stream for Watch<M> {
    type Item = M::Value;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<M::Value>> {
        let mut rx = self.project().rx;
        loop {
            match ready!(rx.poll_next_unpin(cx)) {
                // end of watch stream:
                None => { return Poll::Ready(None); }
                // start of watch stream, no value yet:
                Some(None) => { continue; }
                Some(Some(value)) => {
                    let message = Message { id: M::ID, value };
                    match M::get(&message) {
                        Some(value) => { return Poll::Ready(Some(value)); }
                        // couldn't deserialize this one, wait for next:
                        None => { return Poll::Pending; }
                    }
                }
            }
        }
    }
}

struct WatchRegistration {
    tx: watch::Sender<Option<Value>>,
}

impl WatchRegistration {
    fn notify(&self, msg: &Message) {
        self.tx.send_replace(Some(msg.value));
    }
}

impl Default for WatchRegistration {
    fn default() -> Self {
        let (tx, _) = watch::channel(None);
        WatchRegistration { tx }
    }
}
