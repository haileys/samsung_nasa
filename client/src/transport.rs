use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::LazyLock;
use std::time::Duration;

use async_stream::try_stream;
use bytes::{Bytes, BytesMut};
use futures::{Stream, StreamExt};
use samsunghvac_protocol::frame::{FrameError, FrameParser, MAX_FRAME_SIZE};
use samsunghvac_protocol::packet::{Packet, PacketError, SerializePacketError};
use samsunghvac_protocol::pretty::pretty_print;
use structopt::StructOpt;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio_serial::SerialPortBuilderExt;

const BAUD_RATE: u32 = 9600;

#[derive(StructOpt)]
pub struct TransportOpt {
    #[structopt(long = "bus", env = "SAMSUNGHVAC_BUS", default_value_os = DEFAULT_SOCKET.as_os_str())]
    pub bus: PathBuf,
}

pub type AsyncTransport = (TransportReceiver, TransportSender);

pub async fn open(opt: &TransportOpt) -> Result<AsyncTransport, OpenError> {
    match open_unix_socket(&opt.bus).await {
        Ok(Some(io)) => { return Ok(io); }
        Ok(None) => {}
        Err(error) => {
            return Err(OpenError { path: opt.bus.to_owned(), error });
        }
    }

    match open_serial_port(&opt.bus).await {
        Ok(io) => { return Ok(io); }
        Err(error) => {
            return Err(OpenError { path: opt.bus.to_owned(), error: error.into() });
        }
    }
}

#[derive(Debug, Error)]
#[error("opening bus {path}: {error}")]
pub struct OpenError {
    path: PathBuf,
    #[source]
    error: io::Error,
}

pub struct TransportReceiver {
    rd: Pin<Box<dyn Stream<Item = PacketStreamResult> + Send>>,
}

#[derive(Error, Debug)]
pub enum ReadPacketError {
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error(transparent)]
    Packet(#[from] PacketError),
}

impl TransportReceiver {
    pub fn new(rd: impl AsyncRead + Send + 'static) -> Self {
        // monomorphise before calling packet_stream:
        let rd = Box::pin(rd) as Pin<Box<dyn AsyncRead + Send + 'static>>;
        let rd = Box::pin(packet_stream(rd)) as Pin<Box<_>>;
        TransportReceiver { rd }
    }

    pub async fn read(&mut self) -> Result<Box<Packet>, io::Error> {
        while let Some(result) = self.rd.next().await {
            match result? {
                Ok(packet) => {
                    if packet.source.class != 0x10 {
                        let mut pretty = String::new();
                        pretty_print(&mut pretty, &packet, true).unwrap();
                        log::debug!("recv packet: {pretty}");
                    }
                    return Ok(packet);
                }
                Err(err) => { log::warn!("receive: {err}"); }
            }
        }

        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "transport receiver stream ended",
        ));
    }
}

pub struct TransportSender {
    wr: Pin<Box<dyn AsyncWrite + Send>>,
}

#[derive(Error, Debug)]
pub enum SendPacketError {
    #[error(transparent)]
    Serialize(#[from] SerializePacketError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl TransportSender {
    pub fn new(wr: impl AsyncWrite + Send + 'static) -> Self {
        let wr = Box::pin(wr) as Pin<Box<_>>;
        TransportSender { wr }
    }

    pub async fn send(&mut self, packet: &Packet) -> Result<(), SendPacketError> {
        let mut pretty = String::new();
        pretty_print(&mut pretty, &packet, true).unwrap();
        log::debug!("send packet: {pretty}");

        let bytes = serialize_frame(packet)?;
        self.wr.write_all(&bytes).await?;
        Ok(())
    }
}

fn serialize_frame(packet: &Packet) -> Result<Bytes, SerializePacketError> {
    let mut bytes = BytesMut::zeroed(MAX_FRAME_SIZE);
    let n = packet.serialize_frame(&mut bytes)?;
    bytes.truncate(n);
    Ok(bytes.into())
}

type PacketStreamResult = io::Result<Result<Box<Packet>, ReadPacketError>>;

fn packet_stream(mut io: Pin<Box<dyn AsyncRead + Send>>)
    -> impl Stream<Item = PacketStreamResult>
{
    try_stream! {
        let mut parser = FrameParser::new();
        let mut buffer = [0u8; 1024];

        loop {
            let data = match io.read(&mut buffer).await? {
                0 => break,
                n => &buffer[..n],
            };

            for byte in data {
                let frame = match parser.feed(*byte) {
                    Ok(None) => continue,
                    Ok(Some(frame)) => frame,
                    Err(err) => {
                        yield Err(err.into());
                        continue;
                    }
                };

                let packet = match Packet::parse(frame) {
                    Ok(packet) => packet,
                    Err(err) => {
                        yield Err(err.into());
                        continue;
                    }
                };

                yield Ok(Box::new(packet));
            }
        }
    }
}

async fn open_unix_socket(path: &Path) -> Result<Option<AsyncTransport>, io::Error> {
    let stream = match UnixStream::connect(path).await {
        Ok(stream) => stream,
        Err(err) if err.kind() == io::ErrorKind::ConnectionRefused => {
            // not a unix socket
            return Ok(None);
        }
        Err(err) => return Err(err)
    };

    let (rd, wr) = tokio::io::split(stream);
    let rd = TransportReceiver::new(rd);
    let wr = TransportSender::new(wr);

    Ok(Some((rd, wr)))
}

async fn open_serial_port(path: &Path) -> Result<AsyncTransport, tokio_serial::Error> {
    let path = path.to_string_lossy();

    let serial = tokio_serial::new(path, BAUD_RATE)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::Even)
        .stop_bits(serialport::StopBits::One)
        .timeout(Duration::from_secs(1))
        .open_native_async()?;

    let (rd, wr) = tokio::io::split(serial);
    let rd = TransportReceiver::new(rd);
    let wr = TransportSender::new(wr);

    Ok((rd, wr))
}

pub static DEFAULT_SOCKET: LazyLock<PathBuf> = LazyLock::new(|| {
    runtime_dir().join("bus")
});

pub fn runtime_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("RUNTIME_DIRECTORY") {
        PathBuf::from(dir)
    } else {
        PathBuf::from("/var/run/samsunghvac")
    }
}
