use core::convert::Infallible;

use defmt::info;
use embedded_hal::{
    delay::DelayNs,
    digital::{OutputPin, StatefulOutputPin},
};
use stm32f1xx_hal::gpio::{
    Output, PushPull,
    gpioa::PA1,
    gpiob::{PB0, PB1, PB10},
};

use crate::protocol::{ActuatorSnapshot, CommandAction, CommandFrame, CommandTarget};

/// Maximum pulse duration accepted by the actuators module (in milliseconds).
#[allow(dead_code)]
const MAX_PULSE_MS: u32 = 10_000;

/// Control interface that owns all actuator pins and tracks their runtime state.
pub struct Actuators {
    water: PB0<Output<PushPull>>,
    light: PB1<Output<PushPull>>,
    fan: PB10<Output<PushPull>>,
    buzzer: PA1<Output<PushPull>>,
}

/// Errors that can occur while executing a command on the actuators.
#[allow(dead_code)]
pub enum ActuatorError {
    MissingPulseDuration,
}

impl Actuators {
    pub fn new(
        mut water: PB0<Output<PushPull>>,
        mut light: PB1<Output<PushPull>>,
        mut fan: PB10<Output<PushPull>>,
        mut buzzer: PA1<Output<PushPull>>,
    ) -> Self {
        // Ensure all outputs default to the "off" state.
        set_output(&mut water, false);
        set_output(&mut light, false);
        set_output(&mut fan, false);
        set_output(&mut buzzer, false);

        Self {
            water,
            light,
            fan,
            buzzer,
        }
    }

    /// Execute a parsed command.
    #[allow(dead_code)]
    pub fn apply_command<D: DelayNs>(
        &mut self,
        command: &CommandFrame,
        delay: &mut D,
    ) -> Result<(), ActuatorError> {
        info!(
            "Apply command target={} action={}",
            command.target.as_str(),
            command.action.as_str()
        );

        match command.action {
            CommandAction::On => self.set_target(command.target, true),
            CommandAction::Off => self.set_target(command.target, false),
            CommandAction::Pulse => {
                let duration = command
                    .pulse_ms
                    .filter(|&value| value <= MAX_PULSE_MS)
                    .ok_or(ActuatorError::MissingPulseDuration)?;
                self.set_target(command.target, true);
                delay.delay_ms(duration);
                self.set_target(command.target, false);
            }
        }
        Ok(())
    }

    /// Snapshot of the current actuator states by sampling the GPIO output levels.
    pub fn snapshot(&mut self) -> ActuatorSnapshot {
        ActuatorSnapshot {
            water_on: read_output(&mut self.water),
            light_on: read_output(&mut self.light),
            fan_on: read_output(&mut self.fan),
            buzzer_on: read_output(&mut self.buzzer),
        }
    }

    #[allow(dead_code)]
    fn set_target(&mut self, target: CommandTarget, on: bool) {
        match target {
            CommandTarget::Water => {
                set_output(&mut self.water, on);
            }
            CommandTarget::Light => {
                set_output(&mut self.light, on);
            }
            CommandTarget::Fan => {
                set_output(&mut self.fan, on);
            }
            CommandTarget::Buzzer => {
                set_output(&mut self.buzzer, on);
            }
        }
        info!(
            "target={} now {}",
            target.as_str(),
            if on { "on" } else { "off" }
        );
    }
}

fn set_output(pin: &mut impl OutputPin<Error = Infallible>, on: bool) {
    if on {
        let _ = pin.set_high();
    } else {
        let _ = pin.set_low();
    }
}

fn read_output(pin: &mut impl StatefulOutputPin<Error = Infallible>) -> bool {
    pin.is_set_high().unwrap_or(false)
}
