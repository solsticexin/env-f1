#![deny(unsafe_code)]
#![no_main]
#![no_std]


// Print panic message to probe console
use panic_probe as _;


use cortex_m_rt::entry;
use stm32f1xx_hal::{
    pac,
    prelude::*, rcc::Config,
};

#[allow(clippy::empty_loop)]
#[entry]
fn main() -> ! {
    let dp=pac::Peripherals::take().unwrap();
    let cp = cortex_m::Peripherals::take().unwrap();
    //配置时钟
    // let mut flash=dp.FLASH.constrain();
    // let mut rcc=dp.RCC.freeze(Config::hse(8.MHz()), &mut flash.acr);
    let mut rcc = dp.RCC.constrain();
    let mut gpioc = dp.GPIOC.split(&mut rcc);
    let mut delay = cp.SYST.delay(&rcc.clocks);
    let mut led=gpioc.pc13.into_push_pull_output(&mut gpioc.crh);
    led.set_low();
    loop {
        led.set_high();
        // Use `embedded_hal_02::DelayMs` trait
        delay.delay_ms(1_000_u16);
        led.set_low();
        // or use `fugit` duration units
        delay.delay(1.secs());
    }
}
