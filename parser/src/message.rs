use crate::packet::{Message, MessageNumber};

pub mod convert;
pub mod types;

use convert::{IsMessage, TypedMessage};
use types::{Celsius, OperationMode, PowerSetting};

pub type SetTemp = TypedMessage<0x4201, Celsius>;
pub type CurrentTemp = TypedMessage<0x4203, Celsius>;
pub type EvaInTemp = TypedMessage<0x4201, Celsius>;
pub type EvaOutTemp = TypedMessage<0x4203, Celsius>;
pub type OutdoorTemp = TypedMessage<0x8204, Celsius>;
pub type OutdoorDischargeTemp = TypedMessage<0x820a, Celsius>;
pub type OutdoorExchangerTemp = TypedMessage<0x8218, Celsius>;

pub type Power = TypedMessage<0x4000, PowerSetting>;
pub type Mode = TypedMessage<0x4001, OperationMode>;
pub type ModeReal = TypedMessage<0x4001, OperationMode>;

pub fn new<M: IsMessage>(value: M::Value) -> Message {
    M::new(value).to_message()
}

pub const FAN_SPEED: MessageNumber = MessageNumber(0x4006);
pub const FAN_MODE_REAL: MessageNumber = MessageNumber(0x4007);
pub const THERMO: MessageNumber = MessageNumber(0x4028);
pub const DEFROST: MessageNumber = MessageNumber(0x402e);
pub const USE_SILENCE: MessageNumber = MessageNumber(0x4045);
pub const CONTROL_SILENCE: MessageNumber = MessageNumber(0x4046);
pub const OUTDOOR_SERVICE_MODE: MessageNumber = MessageNumber(0x8000);
pub const OUTDOOR_DRIVE_MODE: MessageNumber = MessageNumber(0x8001);
pub const OUTDOOR_MODE: MessageNumber = MessageNumber(0x8003);
pub const OUTDOOR_COMP1_STATUS: MessageNumber = MessageNumber(0x8010);
pub const OUTDOOR_4WAY_STATUS: MessageNumber = MessageNumber(0x801a);
pub const INDOOR_DEFROST_STAGE: MessageNumber = MessageNumber(0x8061);
