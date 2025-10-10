//! Soil moisture sensing utilities.
//!
//! The soil moisture probe is connected to the ADC as an analog voltage source
//! on pin `PA0`. Higher voltages are interpreted as dryer soil. Readings are
//! converted into a coarse percentage (0–100%) to simplify display logic.

use nb::block;
use stm32f1xx_hal::{
    adc::{Adc, SampleTime},
    gpio::{gpioa::PA0, Analog},
    hal_02::adc::OneShot,
    pac,
};

/// 12-bit ADC maximum reading.
const ADC_MAX_COUNT: u16 = 4095;

/// Blocking soil moisture reader backed by `ADC1` and channel `PA0`.
pub struct SoilSensor {
    adc: Adc<pac::ADC1>,
    pin: PA0<Analog>,
}

impl SoilSensor {
    /// Create a new soil sensor instance from an initialized ADC and analog pin.
    ///
    /// The constructor configures a long sampling time to improve stability when
    /// reading high-impedance probes.
    pub fn new(mut adc: Adc<pac::ADC1>, pin: PA0<Analog>) -> Self {
        adc.set_sample_time(SampleTime::T_239);
        Self { adc, pin }
    }

    /// Return the raw 12-bit ADC sample.
    pub fn read_raw(&mut self) -> Result<u16, ()> {
        block!(self.adc.read(&mut self.pin))
    }

    /// Helper that maps the raw ADC count (0–4095) to a 0–100% moisture level.
    ///
    /// The conversion assumes that lower voltages correspond to wetter soil and
    /// performs a simple linear mapping with clamping.
    pub fn raw_to_percent(raw: u16) -> u8 {
        let clamped = raw.min(ADC_MAX_COUNT);
        let dry_percentage = (u32::from(clamped) * 100) / u32::from(ADC_MAX_COUNT);
        let wet_percentage = 100u32.saturating_sub(dry_percentage);
        wet_percentage as u8
    }
}
