use std::{borrow::Cow, io};
use std::path::PathBuf;
use std::process::ExitCode;

use futures::future;
use samsunghvac_client::transport;
use samsunghvac_protocol::packet::Address;
use serde::{Deserialize, Deserializer};
use structopt::StructOpt;
use thiserror::Error;
use tokio::task::LocalSet;

mod util;
mod control;
mod mqtt;

#[derive(StructOpt)]
struct Opt {

}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), ExitCode> {
    let opt = Opt::from_args();

    env_logger::builder()
        .format_timestamp_millis()
        .filter_level(default_log_level())
        .parse_default_env()
        .init();

    let local = LocalSet::new();
    let result = local.run_until(run(opt)).await;

    result.map_err(|err| {
        log::error!("{err}");
        ExitCode::FAILURE
    })
}

#[derive(Error, Debug)]
enum RunError {
    #[error("reading config: {0}")]
    Config(#[from] ConfigError),
    #[error(transparent)]
    OpenBus(#[from] transport::OpenError),
    #[error(transparent)]
    Client(#[from] samsunghvac_client::Error),
}

async fn run(_: Opt) -> Result<(), RunError> {
    let config = load_config()?;
    let hvac = control::SamsungHvac::new(&config.device).await?;
    mqtt::start(&config.mqtt, &config.discovery, hvac).await;
    // we're started, now run forever:
    future::pending().await
}

#[derive(Error, Debug)]
enum ConfigError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
}

fn default_log_level() -> log::LevelFilter {
    if cfg!(debug_assertions) {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    }
}

fn load_config() -> Result<Config, ConfigError> {
    let path = config_path();
    log::info!("reading config from: {}", path.display());
    let text = std::fs::read_to_string(&path)?;
    let config = toml::from_str(&text)?;
    Ok(config)
}

fn config_path() -> PathBuf {
    let current_dir = std::env::current_dir().unwrap();

    if let Some(path) = std::env::var_os("CONFIG_PATH") {
        return PathBuf::from(path);
    }

    let cwd_config = current_dir.join("mqtt.toml");
    if cwd_config.exists() {
        return cwd_config;
    }

    return PathBuf::from("/etc/samsunghvac/mqtt.toml");
}

#[derive(Deserialize)]
struct Config {
    mqtt: MqttConfig,
    discovery: DiscoveryConfig,
    device: DeviceConfig,
}

#[derive(Deserialize, Clone)]
struct MqttConfig {
    host: String,
    port: Option<u16>,
    #[serde(flatten)]
    credentials: Option<MqttCredentials>,
    client_id: String,
}

#[derive(Deserialize, Clone)]
struct MqttCredentials {
    username: String,
    password: String,
}

#[derive(Deserialize, Clone)]
struct DiscoveryConfig {
    prefix: String,
    object_id: String,
    unique_id: String,
}

#[derive(Deserialize)]
struct DeviceConfig {
    bus: PathBuf,
    #[serde(deserialize_with = "deserialize_address")]
    address: Address,
}

fn deserialize_address<'de, D>(de: D) -> Result<Address, D::Error> where D: Deserializer<'de> {
    let addr = Cow::<str>::deserialize(de)?;
    let addr = addr.parse().map_err(serde::de::Error::custom)?;
    Ok(addr)
}
