use crate::packet::{Message, MessageId};

pub mod convert;
pub mod types;

pub use convert::IsMessage;

use convert::TypedMessage;
use types::{Celsius, CelsiusLvar, FanSetting, OperationMode, PowerSetting};

pub type SetTemp = TypedMessage<0x4201, Celsius>;
pub type CurrentTemp = TypedMessage<0x4203, Celsius>;
pub type ModifiedCurrentTemp = TypedMessage<0x4204, Celsius>;
pub type EvaInTemp = TypedMessage<0x4205, Celsius>;
pub type EvaOutTemp = TypedMessage<0x4206, Celsius>;
pub type OutdoorTemp = TypedMessage<0x8204, Celsius>;
pub type OutdoorDischargeTemp = TypedMessage<0x820a, Celsius>;
pub type OutdoorExchangerTemp = TypedMessage<0x8218, Celsius>;

pub type CoolHighTempLimit = TypedMessage<0x0411, CelsiusLvar>;
pub type CoolLowTempLimit = TypedMessage<0x0412, CelsiusLvar>;
pub type HeatHighTempLimit = TypedMessage<0x0413, CelsiusLvar>;
pub type HeatLowTempLimit = TypedMessage<0x0414, CelsiusLvar>;

pub type Power = TypedMessage<0x4000, PowerSetting>;
pub type Mode = TypedMessage<0x4001, OperationMode>;
pub type ModeReal = TypedMessage<0x4002, OperationMode>;
pub type FanMode = TypedMessage<0x4006, FanSetting>;

pub fn new<M: IsMessage>(value: M::Value) -> Message {
    M::new(value).to_message()
}

pub type UnknownTemp4202 = TypedMessage<0x4202, Celsius>;
pub type UnknownTemp42df = TypedMessage<0x42df, Celsius>;
pub type UnknownTemp42e0 = TypedMessage<0x42e0, Celsius>;
pub type UnknownTemp42e1 = TypedMessage<0x42e1, Celsius>;
pub type UnknownTemp42e2 = TypedMessage<0x42e2, Celsius>;
pub type UnknownTemp42e3 = TypedMessage<0x42e3, Celsius>;
pub type UnknownTemp42e4 = TypedMessage<0x42e4, Celsius>;
pub type UnknownTemp8254 = TypedMessage<0x8254, Celsius>;
pub type UnknownTemp8280 = TypedMessage<0x8280, Celsius>;
pub type UnknownTemp82a1 = TypedMessage<0x82a1, Celsius>;
pub type UnknownTemp82e3 = TypedMessage<0x82e3, Celsius>;
pub type UnknownTemp82e5 = TypedMessage<0x82e5, Celsius>;
pub type UnknownTemp82e6 = TypedMessage<0x82e6, Celsius>;
pub type UnknownTemp82eb = TypedMessage<0x82eb, Celsius>;
pub type UnknownTemp82ec = TypedMessage<0x82ec, Celsius>;

pub const FAN_SPEED: MessageId = MessageId(0x4006);
pub const FAN_MODE_REAL: MessageId = MessageId(0x4007);
pub const THERMO: MessageId = MessageId(0x4028);
pub const DEFROST: MessageId = MessageId(0x402e);
pub const USE_SILENCE: MessageId = MessageId(0x4045);
pub const CONTROL_SILENCE: MessageId = MessageId(0x4046);
pub const OUTDOOR_SERVICE_MODE: MessageId = MessageId(0x8000);
pub const OUTDOOR_DRIVE_MODE: MessageId = MessageId(0x8001);
pub const OUTDOOR_MODE: MessageId = MessageId(0x8003);
pub const OUTDOOR_COMP1_STATUS: MessageId = MessageId(0x8010);
pub const OUTDOOR_4WAY_STATUS: MessageId = MessageId(0x801a);
pub const INDOOR_DEFROST_STAGE: MessageId = MessageId(0x8061);
