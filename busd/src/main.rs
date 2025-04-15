use std::io;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::ExitCode;
use std::task::{Context, Poll, ready};
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use derive_more::Display;
use futures::{future, Stream, StreamExt};
use async_stream::stream;
use samsunghvac_client::transport::{TransportReceiver, DEFAULT_SOCKET};
use samsunghvac_protocol::frame::{FrameError, MAX_FRAME_SIZE};
use samsunghvac_protocol::packet::{Packet, PacketError, SerializePacketError};
use structopt::StructOpt;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc;
use tokio_serial::{SerialPortBuilderExt, SerialStream};

const BAUD_RATE: u32 = 9600;

#[derive(StructOpt)]
struct Opt {
    #[structopt(short = "l", long = "listen", default_value_os = DEFAULT_SOCKET.as_os_str())]
    pub socket: PathBuf,
    pub port: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), ExitCode> {
    let opt = Opt::from_args();

    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();

    run(opt).await.map_err(|err| {
        log::error!("{err}");
        ExitCode::FAILURE
    })
}

async fn run(opt: Opt) -> Result<(), RunError> {
    let listen = UnixListener::bind(&opt.socket)
        .map_err(|err| RunError::Bind(err, opt.socket.clone()))?;

    let port = open_serial_port(&opt.port)
        .map_err(|err| RunError::OpenPort(err, opt.port.clone()))?;

    let accept = start_accept(listen);
    let bus = Peer::new(PeerLabel::Bus, port);
    multiplex(accept, bus).await;
    Ok(())
}

fn multiplex(mut accept: mpsc::Receiver<Peer>, bus: Peer) -> impl Future<Output = ()> {
    let mut peers = vec![bus];

    future::poll_fn(move |cx| {
        // handle accepting new clients first:
        match accept.poll_recv(cx) {
            Poll::Pending => {}
            Poll::Ready(None) => { return Poll::Ready(()); }
            Poll::Ready(Some(peer)) => { peers.push(peer); }
        }

        // handle peer activity
        loop {
            let (rx_idx, packet) = ready!(poll_peers(&mut peers, cx));

            let bytes = match serialize_frame(&packet) {
                Ok(bytes) => bytes,
                Err(err) => {
                    log::warn!("serializing frame: {err}");
                    continue;
                }
            };

            let mut dead = vec![];

            for (idx, peer) in peers.iter_mut().enumerate() {
                // don't reflect packets back where they came:
                if idx == rx_idx {
                    continue;
                }

                match peer.tx.try_send(bytes.clone()) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {}
                    Err(TrySendError::Closed(_)) => {
                        dead.push(idx);
                    }
                }
            }

            while let Some(idx) = dead.pop() {
                peers.swap_remove(idx);
            }
        }
    })
}

fn poll_peers(peers: &mut Vec<Peer>, cx: &mut Context<'_>) -> Poll<(usize, Box<Packet>)> {
    'again: loop {
        for (idx, peer) in peers.iter_mut().enumerate() {
            match peer.rx.poll_next_unpin(cx) {
                Poll::Pending => continue,
                Poll::Ready(None) => {
                    peers.swap_remove(idx);
                    continue 'again;
                }
                Poll::Ready(Some(packet)) => {
                    return Poll::Ready((idx, packet));
                }
            }
        }

        return Poll::Pending;
    }
}

struct Peer {
    rx: Pin<Box<dyn Stream<Item = Box<Packet>> + Send>>,
    tx: mpsc::Sender<Bytes>,
}

#[derive(Display, Clone)]
enum PeerLabel {
    #[display("bus")]
    Bus,
    #[display("client")]
    Client,
}

impl Peer {
    fn new<Io>(label: PeerLabel, io: Io) -> Self
        where Io: AsyncRead + AsyncWrite + Send + 'static
    {
        let (rx, tx) = tokio::io::split(io);
        let rx = TransportReceiver::new(rx);
        let rx = Box::pin(recv_stream(rx, label.clone())) as Pin<_>;

        // spawn sender task, so that we can post messages without blocking
        let (send_tx, send_rx) = mpsc::channel(8);
        let tx = Box::pin(tx) as Pin<Box<_>>;
        tokio::spawn(send_task(tx, send_rx, label));

        Peer { rx, tx: send_tx }
    }
}

fn recv_stream(mut rx: TransportReceiver, label: PeerLabel) -> impl Stream<Item = Box<Packet>> {
    stream! {
        loop {
            match rx.try_read().await {
                Ok(Ok(packet)) => { yield packet; }
                Ok(Err(err)) => { log::warn!("{label} recv: {err}"); }
                Err(err) => {
                    log::warn!("{label} recv: {err}");
                    break;
                }
            }
        }
    }
}

async fn send_task(
    mut tx: Pin<Box<dyn AsyncWrite + Send>>,
    mut rx: mpsc::Receiver<Bytes>,
    label: PeerLabel,
) {
    while let Some(bytes) = rx.recv().await {
        if let Err(err) = tx.write_all(&bytes).await {
            log::warn!("{label} send: {err}");
            break;
        }
    }
}

#[derive(Error, Debug)]
enum RunError {
    #[error("binding {path}: {0}", path = .1.display())]
    Bind(#[source] io::Error, PathBuf),
    #[error("opening bus port {1}: {0}")]
    OpenPort(#[source] serialport::Error, String),
}

fn start_accept(listen: UnixListener) -> mpsc::Receiver<Peer> {
    let (tx, rx) = mpsc::channel(8);
    tokio::task::spawn(accept_task(listen, tx));
    rx
}

async fn accept_task(listen: UnixListener, tx: mpsc::Sender<Peer>) {
    loop {
        let (client, _) = match listen.accept().await {
            Ok(result) => result,
            Err(err) => {
                log::error!("accept: {err}");
                break;
            }
        };

        let label = PeerLabel::Client;
        let peer = Peer::new(label, client);
        if let Err(_) = tx.send(peer).await {
            break;
        }
    }
}

fn serialize_frame(packet: &Packet) -> Result<Bytes, SerializePacketError> {
    let mut bytes = BytesMut::zeroed(MAX_FRAME_SIZE);
    let n = packet.serialize_frame(&mut bytes)?;
    bytes.truncate(n);
    Ok(bytes.into())
}

#[derive(Error, Debug)]
enum ReadPacketError {
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error(transparent)]
    Packet(#[from] PacketError),
}

fn open_serial_port(path: &str) -> Result<SerialStream, tokio_serial::Error> {
    tokio_serial::new(path, BAUD_RATE)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::Even)
        .stop_bits(serialport::StopBits::One)
        .timeout(Duration::from_secs(1))
        .open_native_async()
}
