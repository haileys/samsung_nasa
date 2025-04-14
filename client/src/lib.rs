use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use message::MessageSet;
use samsunghvac_parser::message::convert::IsMessage;
use samsunghvac_parser::packet::{u2, Address, Data, DataType, Message, MessageKind, MessageId, Packet, PacketInfo, PacketType, Value};
use thiserror::Error;
use tokio::sync::{oneshot, Mutex as AsyncMutex};
use tokio::task;
use transport::{SendPacketError, TransportReceiver, TransportSender};
use watch::WatchRegistry;

pub mod transport;
pub mod message;
mod watch;

pub use watch::Watch;

const LOCAL_ADDRESS: Address = Address { class: 0x80, channel: 0x10, address: 0x10 };
const RETRY_DELAY: Duration = Duration::from_secs(1);

pub struct Client {
    shared: Arc<Shared>,
    task: task::JoinHandle<()>,
    packet_number: AtomicU8,
}

struct Shared {
    address: Address,
    writer: AsyncMutex<TransportSender>,
    waiting: Mutex<HashMap<u8, oneshot::Sender<Box<Packet>>>>,
    watches: WatchRegistry,
}

impl Client {
    pub async fn connect(opt: &transport::TransportOpt) -> Result<Self, transport::OpenError> {
        let (reader, writer) = transport::open(opt).await?;

        let shared = Arc::new(Shared {
            address: LOCAL_ADDRESS,
            writer: AsyncMutex::new(writer),
            waiting: Default::default(),
            watches: Default::default(),
        });

        let task = tokio::task::spawn({
            let shared = shared.clone();
            async move {
                if let Err(err) = task(shared, reader).await {
                    log::error!("client task: {err}");
                }
            }
        });

        Ok(Client {
            shared,
            task,
            packet_number: AtomicU8::default(),
        })
    }

    pub fn watch<M: IsMessage>(&self, sender: Address) -> Watch<M> {
        self.shared.watches.subscribe::<M>(sender)
    }

    fn next_packet_number(&self) -> u8 {
        self.packet_number.fetch_add(1, Ordering::SeqCst)
    }

    pub async fn read(&self, address: Address, attrs: &[MessageId]) -> Result<MessageSet, Error> {
        let queries = attrs.iter()
            .filter_map(|attr| query(*attr))
            .collect::<Vec<_>>();

        let reply = self.send(address, DataType::Read, &queries).await?;
        let reply = expect_reply(reply, DataType::Response)?;

        let messages = match reply.data {
            Data::Messages(msgs) => MessageSet::from_iter(msgs),
            Data::Structure(_) => MessageSet::default(),
        };

        return Ok(messages);

        fn query(number: MessageId) -> Option<Message> {
            Some(Message { id: number, value: null_value(number)? })
        }

        fn null_value(number: MessageId) -> Option<Value> {
            match number.kind() {
                MessageKind::Enum => Some(Value::Enum(u8::MAX)),
                MessageKind::Variable => Some(Value::Variable(u16::MAX)),
                MessageKind::LongVariable => Some(Value::LongVariable(u32::MAX)),
                MessageKind::Structure => None,
            }
        }
    }

    pub async fn reload_watches(&self) -> Result<(), Error> {
        log::debug!("refreshing watches...");
        for (address, attrs) in self.shared.watches.all_watches() {
            log::debug!("reading attrs from {address}: {:?}", attrs);

            let set = match self.read(address, &attrs).await {
                Ok(set) => set,
                Err(err) => {
                    log::warn!("reading attrs from {address}: {err}");
                    continue;
                }
            };

            self.shared.watches.notify(address, set.messages());
        }
        Ok(())
    }

    pub async fn request(&self, address: Address, messages: &[Message]) -> Result<(), Error> {
        let reply = self.send(address, DataType::Request, messages).await?;
        expect_reply(reply, DataType::Ack)?;
        Ok(())
    }

    async fn send(&self, destination: Address, data_type: DataType, messages: &[Message])
        -> Result<Box<Packet>, Error>
    {
        let messages = heapless::Vec::from_slice(messages).unwrap();

        // acquire packet number
        let packet_number = self.next_packet_number();

        // build packet
        let packet = Box::new(Packet {
            source: self.shared.address,
            destination,
            packet_info: PacketInfo::default(),
            packet_type: PacketType::Normal,
            packet_number,
            data_type,
            data: Data::Messages(messages),
        });

        // send in a new task for cancel safety
        let send_fut = send_with_retry(self.shared.clone(), packet);
        let reply = tokio::task::spawn(send_fut).await.unwrap()?;

        Ok(reply)
    }

}

fn expect_reply(reply: Box<Packet>, data_type: DataType) -> Result<Box<Packet>, Error> {
    if reply.data_type == DataType::Nack {
        return Err(Error::Nack(reply));
    }

    if reply.data_type != data_type {
        return Err(Error::UnexpectedReply { actual: reply.data_type, expected: data_type });
    }

    return Ok(reply);
}

async fn send_with_retry(shared: Arc<Shared>, mut packet: Box<Packet>) -> Result<Box<Packet>, Error> {
    let (reply_tx, mut reply_rx) = oneshot::channel();

    // lock waiting map and insert our reply oneshot
    {
        let mut waiting = shared.waiting.lock().unwrap();
        waiting.insert(packet.packet_number, reply_tx);
    }

    // TODO remove waiting oneshot on drop

    loop {
        // lock writer to send packet:
        {
            let mut writer = shared.writer.lock().await;
            writer.send(&packet).await?;
        }

        // wait for reply:
        match tokio::time::timeout(RETRY_DELAY, &mut reply_rx).await {
            Ok(Ok(reply)) => { return Ok(reply); }
            Ok(Err(_)) => { return Err(Error::LostTransport); }
            Err(_) => {
                // timeout waiting on reply
                // check if we've already exhausted max retries:
                let retry_count = packet.packet_info.retry_count;
                if retry_count == u2::MAX {
                    return Err(Error::MaxRetriesExceeded);
                }

                // otherwise loop around and try sending it again
                packet.packet_info.retry_count = retry_count.wrapping_add(u2::new(1));
            }
        }
    }
}

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    OpenTransport(#[from] transport::OpenError),
    #[error(transparent)]
    Send(#[from] SendPacketError),
    #[error("max retries exceeded")]
    MaxRetriesExceeded,
    #[error("lost transport")]
    LostTransport,
    #[error("received negative acknowledgement")]
    Nack(Box<Packet>),
    #[error("unexpected reply {actual:?}, expected {expected:?}")]
    UnexpectedReply { actual: DataType, expected: DataType },
    #[error("missing message: {0}")]
    MissingMessage(MessageId),
}

impl Drop for Client {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn task(shared: Arc<Shared>, mut rx: TransportReceiver) -> io::Result<()> {
    loop {
        let packet = rx.read().await?;

        if packet.packet_type != PacketType::Normal {
            continue;
        }

        let Data::Messages(messages) = &packet.data else {
            continue;
        };

        match packet.data_type {
            DataType::Notification => {
                shared.watches.notify(packet.source, messages);
            }
            | DataType::Ack
            | DataType::Nack
            | DataType::Response => {
                on_reply(&shared, packet);
            }
            _ => {}
        }
    }
}

fn on_reply(shared: &Shared, packet: Box<Packet>) {
    // ignore reply-type packets if not addressed directly to us
    if packet.destination != shared.address {
        return;
    }

    // look up waiting task (if any) by packet number
    let reply_tx = {
        let mut waiting = shared.waiting.lock().unwrap();
        waiting.remove(&packet.packet_number)
    };

    // send it to the waiting task
    if let Some(reply_tx) = reply_tx {
        let _: Result<_, _> = reply_tx.send(packet);
    }
}
