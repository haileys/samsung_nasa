use std::cell::Ref;
use std::rc::Rc;

use samsunghvac_client::message::MessageSet;
use samsunghvac_client::{Client, Error};
use samsunghvac_client::transport::TransportOpt;
use samsunghvac_protocol::message::types::{Celsius, FanSetting, OperationMode, PowerSetting};
use samsunghvac_protocol::message::{self, IsMessage};
use samsunghvac_protocol::packet::{Address, Message};
use tokio::sync::watch;
use tokio::task;

use crate::util::NotifyCell;
use crate::DeviceConfig;

#[derive(Clone)]
pub struct SamsungHvac {
    inner: Rc<Inner>,
}

struct Inner {
    client: Client,
    params: Params,
    shared: Rc<Shared>,
}

struct Shared {
    address: Address,
    state: NotifyCell<State>,
}

#[derive(Default)]
pub struct State {
    pub power: Option<PowerSetting>,
    pub mode: Option<OperationMode>,
    pub fan: Option<FanSetting>,
    pub set_temp: Option<Celsius>,
    pub current_temp: Option<Celsius>,
}

pub struct Params {
    pub cooling_limit: TempLimits,
    pub heating_limit: TempLimits,
}

pub struct TempLimits {
    pub low: Celsius,
    pub high: Celsius,
}

impl SamsungHvac {
    pub async fn new(config: &DeviceConfig) -> Result<Self, Error> {
        let transport = TransportOpt { bus: config.bus.clone() };

        let shared = Rc::new(Shared {
            address: config.address,
            state: NotifyCell::default(),
        });

        let client = Client::connect(&transport, Callbacks {
            shared: shared.clone()
        }).await?;

        // read essential initial params first:
        let params = read_params(&client, config.address).await?;

        let inner = Rc::new(Inner {
            client,
            params,
            shared,
        });

        // read initial hvac state asynchronously to constructor:
        task::spawn_local(read_state(inner.clone()));

        Ok(SamsungHvac { inner })
    }

    pub fn state(&self) -> Ref<'_, State> {
        self.inner.shared.state.borrow()
    }

    pub fn state_updated(&self) -> watch::Receiver<()> {
        self.inner.shared.state.subscribe()
    }

    pub fn params(&self) -> &Params {
        &self.inner.params
    }

    pub async fn request(&self, messages: &[Message]) -> Result<(), Error> {
        self.inner.client.request(self.inner.shared.address, messages).await
    }
}

struct Callbacks {
    shared: Rc<Shared>,
}

impl samsunghvac_client::Callbacks for Callbacks {
    fn on_notification(&self, sender: Address, data: &MessageSet) {
        if sender == self.shared.address {
            let mut state = self.shared.state.borrow_mut();
            update_state(&mut state, data);
        }
    }
}

fn update_state(state: &mut State, data: &MessageSet) {
    state.power = data.get::<message::Power>();
    state.mode = data.get::<message::Mode>();
    state.fan = data.get::<message::FanMode>();
    state.set_temp = data.get::<message::SetTemp>()
        .filter(|_| has_temperature(state));
    state.current_temp = data.get::<message::CurrentTemp>();
}

// some modes don't have an associated set temperature, and for these
// modes the hvac reports a temperature of 24 C. we want to ignore that
fn has_temperature(state: &State) -> bool {
    match state.mode {
        Some(OperationMode::Fan) => false,
        _ => true,
    }
}

async fn read_state(inner: Rc<Inner>) {
    let result = inner.client.read(inner.shared.address, &[
        message::Power::ID,
        message::Mode::ID,
        message::FanMode::ID,
        message::SetTemp::ID,
        message::CurrentTemp::ID,
    ]).await;

    match result {
        Ok(data) => {
            let mut state = inner.shared.state.borrow_mut();
            update_state(&mut state, &data);
        }
        Err(err) => {
            log::warn!("reading initial hvac state: {err}");
        }
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
            low: reply.try_get::<message::CoolLowTempLimit>()?.into(),
            high: reply.try_get::<message::CoolHighTempLimit>()?.into(),
        },
        heating_limit: TempLimits {
            low: reply.try_get::<message::HeatLowTempLimit>()?.into(),
            high: reply.try_get::<message::HeatHighTempLimit>()?.into(),
        },
    })
}
