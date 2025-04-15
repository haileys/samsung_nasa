use std::collections::HashMap;
use std::rc::Rc;
use std::str::{self, FromStr};
use std::cmp;
use std::time::Duration;

use rumqttc::{AsyncClient, ConnectionError, EventLoop, MqttOptions, QoS};
use serde::Serialize;
use tokio::sync::watch;
use tokio::{task, time};

use samsunghvac_client::Error;
use samsunghvac_protocol::message::types::{Celsius, OperationMode, PowerSetting};
use samsunghvac_protocol::message;

use crate::control::{self, SamsungHvac};
use crate::types::{FanMode, HvacMode};
use crate::{DiscoveryConfig, MqttConfig};

const REFUSED_BACKOFF: Duration = Duration::from_secs(1);
const LIVENESS_TIMEOUT: Duration = Duration::from_secs(60);

struct MqttCtx {
    mqtt: AsyncClient,
    hvac: SamsungHvac,
    discovery: DiscoveryConfig,
    topics: Topics,
}

pub async fn start(
    mqtt: &MqttConfig,
    discovery: &DiscoveryConfig,
    hvac: SamsungHvac,
) {
    let options = mqtt_options(mqtt);
    let (mqtt, eventloop) = AsyncClient::new(options, 8);

    let ctx = Rc::new(MqttCtx {
        mqtt,
        hvac: hvac.clone(),
        discovery: discovery.clone(),
        topics: Topics::new(discovery),
    });

    // receiver task
    task::spawn_local(run_mqtt(ctx.clone(), eventloop));

    // subscriptions
    subscribe_topics(&ctx).await;

    // state updates
    let (liveness, liveness_rx) = watch::channel(());
    task::spawn_local(availability_task(ctx.clone(), liveness_rx));
    task::spawn_local(update_state_task(ctx.clone(), liveness.clone()));

    // broadcast device config on boot
    announce_device(&ctx).await;
}

async fn update_state_task(ctx: Rc<MqttCtx>, liveness: watch::Sender<()>) {
    let topics = &ctx.topics.climate;
    let mut updated = ctx.hvac.state_updated();

    while updated.changed().await.is_ok() {
        let state = ctx.hvac.state();

        // push updates to state topics
        if let Some(mode) = hvac_mode(&state) {
            publish(&ctx, &topics.mode_state, mode).await;
        }

        if let Some(fan) = &state.fan {
            publish(&ctx, &topics.fan_mode_state, FanMode::from(*fan)).await;
        }

        if let Some(temp) = &state.set_temp {
            let temp = temp.as_float();
            publish(&ctx, &topics.temperature_state, temp).await;
        }

        if let Some(temp) = &state.current_temp {
            let temp = temp.as_float();
            publish(&ctx, &topics.current_temperature, temp).await;
        }

        // notify the availability task of liveness
        liveness.send_replace(());
    }
}

async fn availability_task(ctx: Rc<MqttCtx>, mut liveness: watch::Receiver<()>) {
    loop {
        let result = time::timeout(LIVENESS_TIMEOUT, liveness.changed()).await;

        let availability = match result {
            Ok(Ok(())) => "online",
            Err(_) => "offline",
            Ok(Err(_)) => { break }
        };

        publish(&ctx, &ctx.topics.climate.availability, availability).await;
    }
}

async fn publish(ctx: &MqttCtx, topic: &str, payload: impl ToString) {
    let payload = payload.to_string();
    let result = ctx.mqtt.publish(topic, QoS::AtLeastOnce, false, payload).await;
    // only returns err if can't post an event to the send task.
    // this should never happen, so unwrap
    result.unwrap()
}

async fn run_mqtt(ctx: Rc<MqttCtx>, mut eventloop: EventLoop) {
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

async fn subscribe_topics(ctx: &MqttCtx) {
    for topic in &[
        &ctx.topics.homeassistant_status,
        &ctx.topics.climate.fan_mode_command,
        &ctx.topics.climate.mode_command,
        &ctx.topics.climate.power_command,
        &ctx.topics.climate.temperature_command,
    ] {
        // ClientError is only returned if there's an error pushing to the
        // request_tx channel, so just unwrap.
        ctx.mqtt.subscribe(topic.as_str(), QoS::AtLeastOnce).await.unwrap();
    }
}

// announces device config for homeassistant discovery
async fn announce_device(ctx: &MqttCtx) {
    let device = device_config(ctx);
    let payload = serde_json::to_string(&device).unwrap();
    log::debug!("publish {payload}: {payload}");

    ctx.mqtt.publish(
        &ctx.topics.device_config,
        QoS::AtLeastOnce,
        false,
        payload,
    ).await.unwrap();
}

fn hvac_mode(state: &control::State) -> Option<HvacMode> {
    let mode = match (state.power?, state.mode?) {
        (PowerSetting::Off, _) => HvacMode::Off,
        (_, OperationMode::Auto) => HvacMode::Auto,
        (_, OperationMode::Cool) => HvacMode::Cool,
        (_, OperationMode::Heat) => HvacMode::Heat,
        (_, OperationMode::Dry) => HvacMode::Dry,
        (_, OperationMode::Fan) => HvacMode::FanOnly,
        _ => HvacMode::Unknown,
    };

    Some(mode)
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

    if ctx.topics.homeassistant_status == topic {
        announce_device(ctx).await;
    }

    if ctx.topics.climate.power_command == topic {
        let power = match message {
            "OFF" => Some(PowerSetting::Off),
            "ON" => Some(PowerSetting::On),
            _ => None
        };

        if let Some(power) = power {
            messages.push(message::new::<message::Power>(power));
        }
    }

    if ctx.topics.climate.mode_command == topic {
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
    }

    if ctx.topics.climate.temperature_command == topic {
        let temp = f32::from_str(message).ok().map(Celsius::from_float);

        if let Some(temp) = temp {
            messages.push(message::new::<message::SetTemp>(temp));
        }
    }

    if ctx.topics.climate.fan_mode_command == topic {
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

fn device_config(ctx: &MqttCtx) -> DeviceConfig {
    let params = ctx.hvac.params();

    let component = ClimateComponent {
        platform: "climate",
        name: "Samsung HVAC",
        object_id: &ctx.discovery.object_id,
        unique_id: &ctx.discovery.unique_id,
        topics: &ctx.topics.climate,
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
            name: "Samsung HVAC",
            ids: &ctx.discovery.unique_id,
        },
        origin: OriginMapping {
            name: "samsunghvac-mqtt",
        },
        components: HashMap::from([
            (ctx.discovery.object_id.as_str(), component),
        ]),
        qos: 1,
    };

    device
}

struct Topics {
    homeassistant_status: String,
    climate: ClimateComponentTopics,
    device_config: String,
}

impl Topics {
    pub fn new(config: &DiscoveryConfig) -> Self {
        let prefix = &config.prefix;
        let object_id = &config.object_id;

        let component = format!("{prefix}/climate/{object_id}");
        let climate = ClimateComponentTopics::new(&component);

        Topics {
            homeassistant_status: format!("{prefix}/status"),
            device_config: format!("{prefix}/device/{object_id}/config"),
            climate,
        }
    }
}

#[derive(Serialize)]
struct ClimateComponentTopics {
    // #[serde(rename = "action_topic")]
    // action: String,
    // #[serde(rename = "json_attributes_topic")]
    // attributes: String,
    #[serde(rename = "availability_topic")]
    availability: String,
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
            availability: format!("{base}/availability"),
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
    name: &'static str,
    object_id: &'a str,
    unique_id: &'a str,
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
    device: DeviceMapping<'a>,
    #[serde(rename = "o")]
    origin: OriginMapping<'a>,
    #[serde(rename = "cmps")]
    components: HashMap<&'a str, ClimateComponent<'a>>,
    qos: usize,
}

#[derive(Serialize)]
struct DeviceMapping<'a> {
    name: &'a str,
    ids: &'a str,
}

#[derive(Serialize)]
struct OriginMapping<'a> {
    name: &'a str,
}

#[derive(Serialize, Clone)]
#[serde(into = "[(); 0]")]
struct EmptyList;

impl From<EmptyList> for [(); 0] {
    fn from(_: EmptyList) -> Self {
        []
    }
}
