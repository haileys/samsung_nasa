use std::collections::HashMap;
use std::rc::Rc;
use std::str::FromStr;
use std::{cmp, fmt::Display};
use std::time::Duration;

use async_stream::stream;
use futures::future::Either;
use futures::{stream, Stream, StreamExt};
use rumqttc::{AsyncClient, ConnectionError, MqttOptions, QoS};
use samsunghvac_client::Error;
use samsunghvac_protocol::message::types::{Celsius, FanSetting, OperationMode, PowerSetting};
use samsunghvac_protocol::message::{self, CurrentTemp, SetTemp};
use serde::Serialize;
use strum::{Display, EnumString};
use tokio::task::LocalSet;

use crate::{control::SamsungHvac, DiscoveryConfig, MqttConfig};

const REFUSED_BACKOFF: Duration = Duration::from_secs(1);

pub async fn run(
    mqtt: &MqttConfig,
    discovery: &DiscoveryConfig,
    hvac: SamsungHvac,
) -> Result<(), Error> {
    let options = mqtt_options(mqtt);
    let topics = Topics::new(discovery);

    let local = LocalSet::new();

    let (client, mut eventloop) = AsyncClient::new(options, 8);
    let client = Rc::new(client);

    // receiver task
    local.spawn_local({
        let ctx = MqttCtx {
            hvac: hvac.clone(),
            topics: topics.clone(),
        };

        async move {
            loop {
                match eventloop.poll().await {
                    Ok(event) => { on_event(&ctx, event).await; }
                    // don't immediately try to reconnect if the server
                    // sent us a connection refused, back off for some delay:
                    Err(ConnectionError::ConnectionRefused(code)) => {
                        log::error!("connection refused: {code:?}");
                        tokio::time::sleep(REFUSED_BACKOFF).await;
                    }
                    Err(error) => { log::error!("error: {error}"); }
                }
            }
        }
    });

    // subscriptions
    for topic in &[
        &topics.homeassistant_status,
        &topics.climate.fan_mode_command,
        &topics.climate.mode_command,
        &topics.climate.power_command,
        &topics.climate.temperature_command,
    ] {
        // ClientError is only returned if there's an error pushing to the
        // request_tx channel, so just unwrap.
        client.subscribe(topic.as_str(), QoS::AtLeastOnce).await.unwrap();
    }

    // broadcast device config on boot
    let device = device_config(&hvac, discovery, &topics);
    let payload = serde_json::to_string(&device).unwrap();
    log::info!("publish {payload}: {payload}");
    client.publish(&topics.device_config, QoS::AtLeastOnce, false, payload).await.unwrap();

    // wire up state updates
    let publish = Publish::new(&local, client.clone());

    publish.bind(
        &topics.climate.current_temperature,
        hvac.watch::<CurrentTemp>().map(|temp| temp.as_float()),
    );

    publish.bind(
        &topics.climate.mode_state,
        hvac_mode_stream(hvac.clone()),
    );

    publish.bind(
        &topics.climate.temperature_state,
        hvac.watch::<SetTemp>().map(|temp| temp.as_float()),
    );

    publish.bind(
        &topics.climate.fan_mode_state,
        hvac.watch::<message::FanMode>().map(FanMode::from)
    );

    hvac.reload_watches().await?;

    local.await;

    Ok(())
}

fn hvac_mode_stream(hvac: SamsungHvac) -> impl Stream<Item = HvacMode> {
    combine(hvac.watch::<message::Power>(), hvac.watch::<message::Mode>())
        .map(|(power, mode)| {
            match (power, mode) {
                (PowerSetting::Off, _) => HvacMode::Off,
                (_, OperationMode::Auto) => HvacMode::Auto,
                (_, OperationMode::Cool) => HvacMode::Cool,
                (_, OperationMode::Heat) => HvacMode::Heat,
                (_, OperationMode::Dry) => HvacMode::Dry,
                (_, OperationMode::Fan) => HvacMode::FanOnly,
                _ => HvacMode::Unknown,
            }
        })
}

fn combine<T: Clone, U: Clone>(
    left_stream: impl Stream<Item = T>,
    right_stream: impl Stream<Item = U>,
) -> impl Stream<Item = (T, U)> {
    stream! {
        let mut left = None;
        let mut right = None;

        let stream = stream::select(
            left_stream.map(Either::Left),
            right_stream.map(Either::Right),
        );

        for await item in stream {
            match item {
                Either::Left(l) => left = Some(l),
                Either::Right(r) => right = Some(r),
            }

            if let (Some(l), Some(r)) = (&left, &right) {
                yield (l.clone(), r.clone());
            }
        }
    }
}

struct Publish<'a> {
    local: &'a LocalSet,
    client: Rc<AsyncClient>,
}

impl<'a> Publish<'a> {
    pub fn new(local: &'a LocalSet, client: Rc<AsyncClient>) -> Self {
        Publish { local, client }
    }

    pub fn bind(&self, topic: &str, stream: impl Stream<Item = impl Display> + 'static) {
        let task = stream.for_each({
            let client = self.client.clone();
            let topic = topic.to_owned();
            move |value| {
                let payload = value.to_string();
                let topic = topic.clone();
                let client = client.clone();
                async move {
                    log::info!("publish {topic}: {payload}");
                    let result = client.publish(&topic, QoS::AtLeastOnce, false, payload).await;
                    if let Err(err) = result {
                        log::warn!("publishing {topic}: {err}");
                    }
                }
            }
        });

        self.local.spawn_local(task);
    }
}

struct MqttCtx {
    topics: Rc<Topics>,
    hvac: SamsungHvac,
}

async fn on_event(ctx: &MqttCtx, event: rumqttc::Event) {
    use rumqttc::{Packet, Event};
    match event {
        Event::Incoming(Packet::Publish(packet)) => {
            let topic = packet.topic;
            if let Some(payload) = str::from_utf8(&packet.payload).ok() {
                log::debug!("received {topic}: {payload}");
                if let Err(err) = on_message(ctx, &topic, payload).await {
                    log::warn!("error dispatching command on {topic}: {err}");
                }
            }
        }
        _ => {}
    }
}

async fn on_message(ctx: &MqttCtx, topic: &str, message: &str) -> Result<(), Error> {
    let mut messages = Vec::new();

    if ctx.topics.climate.power_command == topic {
        let power = match message {
            "OFF" => Some(PowerSetting::Off),
            "ON" => Some(PowerSetting::On),
            _ => None
        };

        if let Some(power) = power {
            messages.push(message::new::<message::Power>(power));
        }
    } else if ctx.topics.climate.mode_command == topic {
        let mode = HvacMode::from_str(message).ok()
            .and_then(|mode| match mode {
                HvacMode::Off => None,
                HvacMode::Auto => Some(OperationMode::Auto),
                HvacMode::Cool => Some(OperationMode::Cool),
                HvacMode::Heat => Some(OperationMode::Heat),
                HvacMode::Dry => Some(OperationMode::Dry),
                HvacMode::FanOnly => Some(OperationMode::Fan),
                HvacMode::Unknown => None,
            });

        if let Some(mode) = mode {
            messages.push(message::new::<message::Power>(PowerSetting::On));
            messages.push(message::new::<message::Mode>(mode));
        } else {
            messages.push(message::new::<message::Power>(PowerSetting::Off));
        }
    } else if ctx.topics.climate.temperature_command == topic {
        let temp = f32::from_str(message).ok().map(Celsius::from_float);

        if let Some(temp) = temp {
            messages.push(message::new::<message::SetTemp>(temp));
        }
    } else if ctx.topics.climate.fan_mode_command == topic {
        let mode = FanMode::from_str(message).ok().map(Into::into);

        if let Some(mode) = mode {
            messages.push(message::new::<message::FanMode>(mode));
        }
    }

    ctx.hvac.request(&messages).await
}

fn mqtt_options(mqtt: &MqttConfig) -> MqttOptions {
    let mut options = MqttOptions::new(&mqtt.client_id, &mqtt.host, mqtt.port.unwrap_or(1883));
    options.set_keep_alive(Duration::from_secs(5));

    if let Some(creds) = &mqtt.credentials {
        options.set_credentials(&creds.username, &creds.password);
    }

    options
}

fn device_config<'a>(
    hvac: &SamsungHvac,
    discovery: &DiscoveryConfig,
    topics: &'a Topics,
) -> DeviceConfig<'a> {
    let params = hvac.params();

    let component = ClimateComponent {
        platform: "climate",
        name: "Samsung HVAC".to_owned(),
        object_id: discovery.object_id.clone(),
        unique_id: discovery.unique_id.clone(),
        topics: &topics.climate,
        // home assistant doesn't support different limits by hvac mode,
        // so set limits according to greatest bounds and then clamp down
        // when handling temperature commands
        min_temp: cmp::min(params.heating_limit.low, params.cooling_limit.low).as_float(),
        max_temp: cmp::max(params.heating_limit.high, params.cooling_limit.high).as_float(),
        precision: 0.1,
        temp_step: 0.1,
        // swing_modes: EmptyList,
        temperature_unit: 'C',
    };

    let device = DeviceConfig {
        device: DeviceMapping {
            name: "Samsung HVAC".to_string(),
            ids: discovery.unique_id.clone(),
        },
        origin: OriginMapping {
            name: "samsunghvac-mqtt".to_string(),
        },
        components: HashMap::from([
            (discovery.object_id.to_string(), component),
        ]),
        qos: 1,
    };

    device
}

#[derive(Debug, PartialEq, EnumString, Display)]
#[strum(serialize_all = "snake_case")]
pub enum HvacMode {
    Off,
    Auto,
    Cool,
    Heat,
    Dry,
    FanOnly,
    #[strum(serialize = "None")]
    Unknown,
}

#[derive(Debug, PartialEq, EnumString, Display)]
#[strum(serialize_all = "snake_case")]
pub enum FanMode {
    Auto,
    Low,
    Medium,
    High,
}

impl From<FanSetting> for FanMode {
    fn from(value: FanSetting) -> Self {
        match value {
            FanSetting::Auto => FanMode::Auto,
            FanSetting::Low => FanMode::Low,
            FanSetting::Medium => FanMode::Medium,
            FanSetting::High => FanMode::High,
        }
    }
}

impl From<FanMode> for FanSetting {
    fn from(value: FanMode) -> Self {
        match value {
            FanMode::Auto => FanSetting::Auto,
            FanMode::Low => FanSetting::Low,
            FanMode::Medium => FanSetting::Medium,
            FanMode::High => FanSetting::High,
        }
    }
}

struct Topics {
    homeassistant_status: String,
    climate: ClimateComponentTopics,
    device_config: String,
}

impl Topics {
    pub fn new(config: &DiscoveryConfig) -> Rc<Self> {
        let prefix = &config.prefix;
        let object_id = &config.object_id;

        let component = format!("{prefix}/climate/{object_id}");
        let climate = ClimateComponentTopics::new(&component);

        Rc::new(Topics {
            homeassistant_status: format!("{prefix}/status"),
            device_config: format!("{prefix}/device/{object_id}/config"),
            climate,
        })
    }
}

#[derive(Serialize)]
struct ClimateComponentTopics {
    // #[serde(rename = "action_topic")]
    // action: String,
    // #[serde(rename = "json_attributes_topic")]
    // attributes: String,
    // #[serde(rename = "availability_topic")]
    // availability: String,
    #[serde(rename = "current_temperature_topic")]
    current_temperature: String,
    #[serde(rename = "fan_mode_command_topic")]
    fan_mode_command: String,
    #[serde(rename = "fan_mode_state_topic")]
    fan_mode_state: String,
    #[serde(rename = "mode_command_topic")]
    mode_command: String,
    #[serde(rename = "mode_state_topic")]
    mode_state: String,
    #[serde(rename = "power_command_topic")]
    power_command: String,
    #[serde(rename = "temperature_command_topic")]
    temperature_command: String,
    #[serde(rename = "temperature_state_topic")]
    temperature_state: String,
}

impl ClimateComponentTopics {
    pub fn new(base: &str) -> Self {
        ClimateComponentTopics {
            // action: format!("{base}/action"),
            // attributes: format!("{base}/attributes"),
            // availability: format!("{base}/availability"),
            current_temperature: format!("{base}/current_temperature"),
            fan_mode_command: format!("{base}/fan/set"),
            fan_mode_state: format!("{base}/fan/state"),
            mode_command: format!("{base}/mode/set"),
            mode_state: format!("{base}/mode/state"),
            power_command: format!("{base}/power/set"),
            temperature_command: format!("{base}/temperature/set"),
            temperature_state: format!("{base}/temperature/state"),
        }
    }
}

#[derive(Serialize)]
struct ClimateComponent<'a> {
    #[serde(rename="p")]
    platform: &'static str,
    name: String,
    object_id: String,
    unique_id: String,
    max_temp: f32,
    min_temp: f32,
    precision: f32,
    // swing_modes: EmptyList,
    temp_step: f32,
    temperature_unit: char,
    #[serde(flatten)]
    topics: &'a ClimateComponentTopics
}

#[derive(Serialize)]
struct DeviceConfig<'a> {
    device: DeviceMapping,
    #[serde(rename = "o")]
    origin: OriginMapping,
    #[serde(rename = "cmps")]
    components: HashMap<String, ClimateComponent<'a>>,
    qos: usize,
}

#[derive(Serialize)]
struct DeviceMapping {
    name: String,
    ids: String,
}

#[derive(Serialize)]
struct OriginMapping {
    name: String,
}

#[derive(Serialize, Clone)]
#[serde(into = "[(); 0]")]
struct EmptyList;

impl From<EmptyList> for [(); 0] {
    fn from(_: EmptyList) -> Self {
        []
    }
}
