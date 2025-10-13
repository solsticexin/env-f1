//! JSON payload helpers for the Bonsai care monitoring system communication
//! protocol.
//!
//! According to `智能盆栽系统通信协议与控制规范.md`, the STM32 firmware must emit
//! newline-delimited JSON (NDJSON) messages with the following schemas:
//! * `type: "data"`  — telemetry upload towards the ESP-01S / web UI
//! * `type: "ack"`   — execution acknowledgement after handling a command
//! The helpers in this module build those frames using fixed-size buffers and
//! stream them over the provided UART transmitter.

use core::fmt::Write as _;

use heapless::String;
use nb::block;
use stm32f1xx_hal::serial::{Error as SerialError, Instance, Tx};

/// Maximum length of a single telemetry (`type: data`) JSON line, including the
/// trailing newline.
const DATA_BUF_CAPACITY: usize = 160;
/// Maximum length of an acknowledgement (`type: ack`) JSON line, including the
/// trailing newline.
#[allow(dead_code)]
const ACK_BUF_CAPACITY: usize = 96;

macro_rules! pushf {
    ($buffer:expr, $($arg:tt)*) => {
        if write!($buffer, $($arg)*).is_err() {
            return Err(Error::BufferOverflow);
        }
    };
}

/// Error type returned when building or transmitting a JSON payload fails.
pub enum Error {
    /// Formatting the JSON string overflowed the fixed buffer.
    BufferOverflow,
    /// Underlying UART write operation failed.
    Serial(SerialError),
}

/// Snapshot of sensor readings to be embedded in the `type: data` payload.
#[derive(Clone, Copy)]
pub struct SensorSnapshot {
    /// Temperature in 0.1 °C units.
    pub temperature_tenths_c: Option<i16>,
    /// Relative humidity in 0.1 % units.
    pub humidity_tenths_pct: Option<u16>,
    /// Soil moisture percentage (0–100).
    pub soil_pct: Option<u8>,
    /// Ambient light level in 0.1 lx units.
    pub lux_tenths: Option<u32>,
}

impl Default for SensorSnapshot {
    fn default() -> Self {
        Self {
            temperature_tenths_c: None,
            humidity_tenths_pct: None,
            soil_pct: None,
            lux_tenths: None,
        }
    }
}

/// Actuator state report included in telemetry frames.
#[derive(Clone, Copy)]
pub struct ActuatorSnapshot {
    pub water_on: bool,
    pub light_on: bool,
    pub fan_on: bool,
}

impl Default for ActuatorSnapshot {
    fn default() -> Self {
        Self {
            water_on: false,
            light_on: false,
            fan_on: false,
        }
    }
}

/// Aggregated telemetry frame that groups sensor and actuator snapshots.
#[derive(Default, Clone, Copy)]
pub struct TelemetryFrame {
    pub sensors: SensorSnapshot,
    pub actuators: ActuatorSnapshot,
}

/// Supported relay targets referenced by incoming commands and acknowledgement
/// payloads.
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub enum CommandTarget {
    Water,
    Light,
    Fan,
}

impl CommandTarget {
    const fn as_str(self) -> &'static str {
        match self {
            CommandTarget::Water => "water",
            CommandTarget::Light => "light",
            CommandTarget::Fan => "fan",
        }
    }
}

/// Supported actions for command and acknowledgement payloads.
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub enum CommandAction {
    On,
    Off,
    Pulse,
}

impl CommandAction {
    const fn as_str(self) -> &'static str {
        match self {
            CommandAction::On => "on",
            CommandAction::Off => "off",
            CommandAction::Pulse => "pulse",
        }
    }
}

/// Result flag propagated back to the web UI via `type: ack` messages.
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub enum AckResult {
    Ok,
    Error,
}

impl AckResult {
    const fn as_str(self) -> &'static str {
        match self {
            AckResult::Ok => "ok",
            AckResult::Error => "error",
        }
    }
}

/// Convenient container for an acknowledgement frame.
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct AckFrame {
    pub target: CommandTarget,
    pub action: CommandAction,
    pub result: AckResult,
}

/// Stateful JSON payload emitter that streams frames to the provided UART TX.
pub struct JsonPayload<USART: Instance> {
    tx: Tx<USART>,
}

impl<USART: Instance> JsonPayload<USART> {
    /// Create a new payload emitter with the provided UART transmitter.
    pub fn new(tx: Tx<USART>) -> Self {
        Self { tx }
    }

    /// Emit a `type: "data"` JSON line that follows the communication protocol.
    pub fn send_data(&mut self, frame: &TelemetryFrame) -> Result<(), Error> {
        let mut payload: String<DATA_BUF_CAPACITY> = String::new();

        pushf!(payload, "{{\"type\":\"data\"");

        // Temperature (Optional, formatted with one decimal place)
        pushf!(payload, ",\"temp\":");
        match frame.sensors.temperature_tenths_c {
            Some(value) => {
                let (sign, abs) = if value < 0 {
                    ("-", (-value) as u16)
                } else {
                    ("", value as u16)
                };
                pushf!(payload, "{}{}.{}", sign, abs / 10, abs % 10);
            }
            None => pushf!(payload, "null"),
        };

        // Humidity (Optional, formatted with one decimal place)
        pushf!(payload, ",\"humi\":");
        match frame.sensors.humidity_tenths_pct {
            Some(value) => pushf!(payload, "{}.{}", value / 10, value % 10),
            None => pushf!(payload, "null"),
        };

        // Soil moisture (Optional, integer percentage)
        pushf!(payload, ",\"soil\":");
        match frame.sensors.soil_pct {
            Some(value) => pushf!(payload, "{}", value),
            None => pushf!(payload, "null"),
        };

        // Ambient light (Optional, formatted with one decimal place)
        pushf!(payload, ",\"lux\":");
        match frame.sensors.lux_tenths {
            Some(value) => pushf!(payload, "{}.{}", value / 10, value % 10),
            None => pushf!(payload, "null"),
        };

        // Actuator states (boolean as 0/1)
        pushf!(
            payload,
            ",\"water\":{},\"light\":{},\"fan\":{}",
            bool_to_bit(frame.actuators.water_on),
            bool_to_bit(frame.actuators.light_on),
            bool_to_bit(frame.actuators.fan_on)
        );

        // Close the object and append newline for NDJSON framing.
        pushf!(payload, "}}\n");

        self.flush_bytes(payload.as_bytes())
    }

    /// Emit a `type: "ack"` JSON line to report command execution result.
    #[allow(dead_code)]
    pub fn send_ack(&mut self, ack: AckFrame) -> Result<(), Error> {
        let mut payload: String<ACK_BUF_CAPACITY> = String::new();
        pushf!(
            payload,
            "{{\"type\":\"ack\",\"target\":\"{}\",\"action\":\"{}\",\"result\":\"{}\"}}\n",
            ack.target.as_str(),
            ack.action.as_str(),
            ack.result.as_str()
        );
        self.flush_bytes(payload.as_bytes())
    }

    fn flush_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        for &byte in bytes {
            block!(self.tx.write_u8(byte)).map_err(Error::Serial)?;
        }
        block!(self.tx.flush()).map_err(Error::Serial)?;
        Ok(())
    }
}

#[inline(always)]
const fn bool_to_bit(value: bool) -> u8 {
    if value {
        1
    } else {
        0
    }
}
