//! DHT11 temperature and humidity sensor driver.
//!
//! This module provides a minimal blocking driver for reading measurements
//! from a single-wire DHT11 sensor using a GPIO configured as open-drain.
//! Timing is implemented with a [`DelayNs`] provider, which is typically the
//! SysTick-based delay from the HAL.

use core::convert::Infallible;

use embedded_hal::{
    delay::DelayNs,
    digital::{InputPin, OutputPin},
};

/// Possible errors returned when querying the DHT11 sensor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error<PinError = Infallible> {
    /// The sensor did not respond within the expected timing window.
    Timeout,
    /// The received data failed the checksum validation step.
    Checksum,
    /// GPIO access failed.
    Pin(PinError),
}

/// A single DHT11 measurement expressed in tenths.
///
/// The integer value is stored in tenths to avoid using floating point while
/// still exposing the decimal digit reported by the sensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    /// Relative humidity in tenths of a percent.
    pub humidity_tenths: u16,
    /// Temperature in tenths of a °C.
    pub temperature_tenths: u16,
}

/// Blocking DHT11 driver.
pub struct Dht11<P> {
    pin: P,
}

impl<P, PinError> Dht11<P>
where
    P: OutputPin<Error = PinError> + InputPin<Error = PinError>,
{
    /// Create a new driver from an open-drain GPIO pin.
    pub fn new(mut pin: P) -> Result<Self, PinError> {
        // Ensure the line is idle-high before the first request.
        pin.set_high()?;
        Ok(Self { pin })
    }

    /// Read a single measurement from the sensor.
    pub fn read<DELAY>(&mut self, delay: &mut DELAY) -> Result<Reading, Error<PinError>>
    where
        DELAY: DelayNs,
    {
        let mut data = [0u8; 5];

        self.start_signal(delay)?;
        self.await_sensor_response(delay)?;

        for byte in data.iter_mut() {
            let mut value = 0u8;
            for _ in 0..8 {
                value <<= 1;

                // Each bit starts with the sensor pulling the line low (~50 µs).
                self.wait_for_level(delay, false, 70)?;
                // Followed by a high level: ~26 µs for '0', ~70 µs for '1'.
                self.wait_for_level(delay, true, 70)?;

                let mut high_time = 0u32;
                while self.pin.is_high().map_err(Error::Pin)? {
                    delay.delay_us(1_u32);
                    high_time += 1;
                    if high_time > 100 {
                        return Err(Error::Timeout);
                    }
                }

                if high_time > 40 {
                    value |= 1;
                }
            }
            *byte = value;
        }

        let checksum = data[0]
            .wrapping_add(data[1])
            .wrapping_add(data[2])
            .wrapping_add(data[3]);
        if checksum != data[4] {
            return Err(Error::Checksum);
        }

        // 添加调试输出
        defmt::info!("DHT11 raw data: {} {} {} {} {}", data[0], data[1], data[2], data[3], data[4]);

        let humidity_tenths =
            u16::from(data[0]) * 10 + u16::from(data[1]);
        let temperature_tenths =
            u16::from(data[2]) * 10 + u16::from(data[3]);

        Ok(Reading {
            humidity_tenths,
            temperature_tenths,
        })
    }

    fn start_signal<DELAY>(&mut self, delay: &mut DELAY) -> Result<(), Error<PinError>>
    where
        DELAY: DelayNs,
    {
        self.pin.set_low().map_err(Error::Pin)?;
        delay.delay_ms(20_u32);
        self.pin.set_high().map_err(Error::Pin)?;
        delay.delay_us(30_u32);
        Ok(())
    }

    fn await_sensor_response<DELAY>(
        &mut self,
        delay: &mut DELAY,
    ) -> Result<(), Error<PinError>>
    where
        DELAY: DelayNs,
    {
        // Sensor pulls the line low (~80 µs), high (~80 µs), then low
        self.wait_for_level(delay, false, 120)?;
        self.wait_for_level(delay, true, 120)?;
        self.wait_for_level(delay, false, 120)?;
        Ok(())
    }

    fn wait_for_level<DELAY>(
        &mut self,
        delay: &mut DELAY,
        level_high: bool,
        timeout_us: u32,
    ) -> Result<(), Error<PinError>>
    where
        DELAY: DelayNs,
    {
        for _ in 0..timeout_us {
            let is_high = self.pin.is_high().map_err(Error::Pin)?;
            if is_high == level_high {
                return Ok(());
            }
            delay.delay_us(1_u32);
        }
        Err(Error::Timeout)
    }
}
