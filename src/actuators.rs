use core::convert::Infallible;

use defmt::info;
use embedded_hal::{delay::DelayNs, digital::OutputPin};
use stm32f1xx_hal::gpio::{
    gpioa::PA1,
    gpiob::{PB0, PB1, PB10},
    Output, PushPull,
};

use crate::protocol::{ActuatorSnapshot, CommandAction, CommandFrame, CommandTarget};

/// Maximum pulse duration accepted by the actuators module (in milliseconds).
const MAX_PULSE_MS: u32 = 10_000;

/// Control interface that owns all actuator pins and tracks their runtime state.
pub struct Actuators {
    water: PB0<Output<PushPull>>,
    light: PB1<Output<PushPull>>,
    fan: PB10<Output<PushPull>>,
    buzzer: PA1<Output<PushPull>>,
    snapshot: ActuatorSnapshot,
}

/// Errors that can occur while executing a command on the actuators.
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
            snapshot: ActuatorSnapshot::default(),
        }
    }

    /// Execute a parsed command and update the cached actuator state.
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

    /// Snapshot of the current actuator states.
    pub fn snapshot(&self) -> ActuatorSnapshot {
        self.snapshot
    }

    fn set_target(&mut self, target: CommandTarget, on: bool) {
        match target {
            CommandTarget::Water => {
                set_output(&mut self.water, on);
                self.snapshot.water_on = on;
            }
            CommandTarget::Light => {
                set_output(&mut self.light, on);
                self.snapshot.light_on = on;
            }
            CommandTarget::Fan => {
                set_output(&mut self.fan, on);
                self.snapshot.fan_on = on;
            }
            CommandTarget::Buzzer => {
                set_output(&mut self.buzzer, on);
                self.snapshot.buzzer_on = on;
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
