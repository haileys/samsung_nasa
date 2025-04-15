use std::str::FromStr;

use derive_more::Display;
use samsunghvac_protocol::message::types::FanSetting;
use thiserror::Error;

#[derive(Error, Debug)]
#[error("invalid value for {0}: {1}")]
pub struct InvalidEnumString(&'static str, String);

#[derive(Debug, PartialEq, Display)]
pub enum HvacMode {
    #[display("off")]
    Off,
    #[display("auto")]
    Auto,
    #[display("cool")]
    Cool,
    #[display("heat")]
    Heat,
    #[display("dry")]
    Dry,
    #[display("fan_only")]
    FanOnly,
    #[display("None")]
    Unknown,
}

impl FromStr for HvacMode {
    type Err = InvalidEnumString;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "off" => Ok(HvacMode::Off),
            "auto" => Ok(HvacMode::Auto),
            "cool" => Ok(HvacMode::Cool),
            "heat" => Ok(HvacMode::Heat),
            "dry" => Ok(HvacMode::Dry),
            "fan_only" => Ok(HvacMode::FanOnly),
            _ => Err(InvalidEnumString("HvacMode", s.to_string())),
        }
    }
}

#[derive(Debug, PartialEq, Display)]
pub enum FanMode {
    #[display("auto")]
    Auto,
    #[display("low")]
    Low,
    #[display("medium")]
    Medium,
    #[display("high")]
    High,
}

impl FromStr for FanMode {
    type Err = InvalidEnumString;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "auto" => Ok(FanMode::Auto),
            "low" => Ok(FanMode::Low),
            "medium" => Ok(FanMode::Medium),
            "high" => Ok(FanMode::High),
            _ => Err(InvalidEnumString("FanMode", s.to_string())),
        }
    }
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
