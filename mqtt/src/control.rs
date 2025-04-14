use std::sync::Arc;

use samsunghvac_client::{Client, Error, Watch};
use samsunghvac_client::transport::{TransportOpt};
use samsunghvac_parser::message::types::CelsiusLvar;
use samsunghvac_parser::message::{self, IsMessage};
use samsunghvac_parser::packet::{Address, Message};

use crate::DeviceConfig;

#[derive(Clone)]
pub struct SamsungHvac {
    shared: Arc<Shared>,
}

struct Shared {
    client: Client,
    address: Address,
    params: Params,
}

pub struct Params {
    pub cooling_limit: TempLimits,
    pub heating_limit: TempLimits,
}

pub struct TempLimits {
    pub low: CelsiusLvar,
    pub high: CelsiusLvar,
}

impl SamsungHvac {
    pub async fn new(config: &DeviceConfig) -> Result<Self, Error> {
        let transport = TransportOpt { bus: config.bus.clone() };
        let client = Client::connect(&transport).await?;
        let params = read_params(&client, config.address).await?;
        Ok(SamsungHvac {
            shared: Arc::new(Shared {
                client,
                address: config.address,
                params,
            })
        })
    }

    pub fn params(&self) -> &Params {
        &self.shared.params
    }

    pub fn watch<M: IsMessage>(&self) -> Watch<M> {
        self.shared.client.watch::<M>(self.shared.address)
    }

    pub async fn request(&self, messages: &[Message]) -> Result<(), Error> {
        self.shared.client.request(self.shared.address, messages).await
    }

    pub async fn reload_watches(&self) -> Result<(), Error> {
        self.shared.client.reload_watches().await
    }
}

async fn read_params(client: &Client, address: Address) -> Result<Params, Error> {
    log::info!("reading initial params from {}", address);

    let reply = client.read(address, &[
        message::CoolLowTempLimit::ID,
        message::HeatLowTempLimit::ID,
        message::CoolHighTempLimit::ID,
        message::HeatHighTempLimit::ID,
    ]).await?;

    Ok(Params {
        cooling_limit: TempLimits {
            low: reply.get::<message::CoolLowTempLimit>()?,
            high: reply.get::<message::CoolHighTempLimit>()?,
        },
        heating_limit: TempLimits {
            low: reply.get::<message::HeatLowTempLimit>()?,
            high: reply.get::<message::HeatHighTempLimit>()?,
        },
    })
}
