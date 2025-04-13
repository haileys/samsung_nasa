use tokio::io::{AsyncRead, AsyncWrite};

pub mod transport;

pub struct Client {
    rd: Box<dyn AsyncRead>,
    wr: Box<dyn AsyncWrite>,
}

impl Client {
    pub async fn connect(opt: &transport::TransportOpt) -> Result<Self, transport::OpenError> {
        let (_rd, _wr) = transport::open(opt).await?;
        todo!();
    }
}
