#![no_std]
#![no_main]
#![deny(unsafe_code)]

use defmt_rtt as _;
use panic_probe as _;

use cortex_m_rt::entry;
use defmt::{info, warn};
use embedded_hal::delay::DelayNs;
use stm32f1xx_hal::{gpio::PinState, pac, prelude::*, rcc};

#[path = "../dh11.rs"]
mod dh11;

use dh11::{Dht11, Error as DhtError};

#[entry]
fn main() -> ! {
    let dp = pac::Peripherals::take().unwrap();
    let mut cp = cortex_m::Peripherals::take().unwrap();

    let mut flash = dp.FLASH.constrain();
    let mut rcc = dp.RCC.constrain();
    let _afio = dp.AFIO.constrain(&mut rcc);
    rcc = rcc.freeze(
        rcc::Config::hse(8.MHz()).sysclk(72.MHz()).pclk1(36.MHz()),
        &mut flash.acr,
    );

    cp.DCB.enable_trace();
    cp.DWT.enable_cycle_counter();

    let mut delay = cp.SYST.delay(&rcc.clocks);
    let mut gpiob = dp.GPIOB.split(&mut rcc);

    let dht_pin = gpiob
        .pb5
        .into_open_drain_output_with_state(&mut gpiob.crl, PinState::High);
    let mut dht11 = Dht11::new(dht_pin).expect("Failed to init DHT11");

    info!("DHT11 test started");
    DelayNs::delay_ms(&mut delay, 2_000_u32);

    loop {
        match dht11.read(&mut delay) {
            Ok(reading) => {
                let temp_int = reading.temperature_tenths / 10;
                let temp_dec = reading.temperature_tenths % 10;
                let hum_int = reading.humidity_tenths / 10;
                let hum_dec = reading.humidity_tenths % 10;
                info!(
                    "Temperature {}.{} C, humidity {}.{} %",
                    temp_int, temp_dec, hum_int, hum_dec
                );
            }
            Err(error) => match error {
                DhtError::Timeout => warn!("DHT11 timeout"),
                DhtError::Checksum => warn!("DHT11 checksum mismatch"),
                DhtError::Pin(_) => warn!("DHT11 pin error"),
            },
        }

        DelayNs::delay_ms(&mut delay, 2_000_u32);
    }
}
