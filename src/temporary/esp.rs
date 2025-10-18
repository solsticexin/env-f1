#![no_std]
#![no_main]
#![deny(unsafe_code)]

use core::{cell::RefCell, mem::take};

use defmt_rtt as _;
use panic_probe as _;

use cortex_m::{
    asm::wfi,
    interrupt::{Mutex, free},
};
use cortex_m_rt::entry;
use defmt::{Debug2Format, info, warn};
use heapless::String;
use nb::Error as NbError;
use serde::Deserialize;
use stm32f1xx_hal::{
    pac::{self, interrupt},
    prelude::*,
    rcc,
    serial::{Config as SerialConfig, Error as SerialError, Rx1, SerialExt},
};

const LINE_BUF_CAPACITY: usize = 128;
type LineBuf = String<LINE_BUF_CAPACITY>;

static RX: Mutex<RefCell<Option<Rx1>>> = Mutex::new(RefCell::new(None));
static LINE_BUFFER: Mutex<RefCell<LineBuf>> = Mutex::new(RefCell::new(LineBuf::new()));
static PENDING_LINE: Mutex<RefCell<Option<LineBuf>>> = Mutex::new(RefCell::new(None));

#[derive(Deserialize)]
struct EspMessage<'a> {
    #[serde(rename = "type")]
    #[serde(default)]
    kind: Option<&'a str>,
}

fn log_esp_json(raw: &str) {
    match serde_json_core::from_str::<EspMessage>(raw) {
        Ok((msg, _)) => {
            let kind = msg.kind.unwrap_or("unknown");
            info!("ESP->STM JSON[{}]: {}", kind, raw);
        }
        Err(err) => warn!("ESP JSON parse error: {} raw: {}", Debug2Format(&err), raw),
    }
}

fn take_pending_line() -> Option<LineBuf> {
    free(|cs| PENDING_LINE.borrow(cs).borrow_mut().take())
}

#[allow(unsafe_code)]
fn enable_usart1_interrupt() {
    unsafe {
        cortex_m::peripheral::NVIC::unmask(pac::Interrupt::USART1);
    }
}

#[entry]
fn main() -> ! {
    let dp = pac::Peripherals::take().expect("Failed to take device peripherals");

    let mut flash = dp.FLASH.constrain();
    let mut rcc = dp.RCC.constrain();
    let _afio = dp.AFIO.constrain(&mut rcc);
    rcc = rcc.freeze(
        rcc::Config::hse(8.MHz())
            .sysclk(72.MHz())
            .pclk1(36.MHz())
            .pclk2(72.MHz()),
        &mut flash.acr,
    );

    let mut gpioa = dp.GPIOA.split(&mut rcc);
    let tx_pin = gpioa.pa9.into_alternate_push_pull(&mut gpioa.crh);
    let rx_pin = gpioa.pa10;

    let serial = dp.USART1.serial(
        (tx_pin, rx_pin),
        SerialConfig::default().baudrate(115_200.bps()),
        &mut rcc,
    );

    let (_tx, mut rx) = serial.split();
    rx.listen();

    free(|cs| {
        RX.borrow(cs).replace(Some(rx));
        LINE_BUFFER.borrow(cs).borrow_mut().clear();
    });

    enable_usart1_interrupt();

    info!("ESP UART monitor started");

    loop {
        if let Some(line) = take_pending_line() {
            log_esp_json(line.as_str());
            continue;
        }

        wfi();
    }
}

#[interrupt]
fn USART1() {
    free(|cs| {
        let mut rx_ref = RX.borrow(cs).borrow_mut();
        let Some(rx) = rx_ref.as_mut() else {
            warn!("USART1 interrupt without RX handle");
            return;
        };

        let mut line = LINE_BUFFER.borrow(cs).borrow_mut();

        loop {
            match rx.read() {
                Ok(byte) => match byte {
                    b'\r' => {}
                    b'\n' => {
                        if !line.is_empty() {
                            let dropped = {
                                let mut pending = PENDING_LINE.borrow(cs).borrow_mut();
                                if pending.is_some() {
                                    true
                                } else {
                                    *pending = Some(take(&mut *line));
                                    false
                                }
                            };

                            if dropped {
                                warn!("ESP line dropped: pending message not processed yet");
                                line.clear();
                            }
                        }
                    }
                    byte if byte.is_ascii() => {
                        if line.push(byte as char).is_err() {
                            warn!("ESP line overflow, clearing buffer");
                            line.clear();
                        }
                    }
                    other => {
                        warn!("ESP non-ASCII byte: {=u8:02X}", other);
                    }
                },
                Err(NbError::WouldBlock) => break,
                Err(NbError::Other(err)) => {
                    match err {
                        SerialError::Overrun => warn!("ESP UART overrun, clearing buffer"),
                        SerialError::FrameFormat => warn!("ESP UART frame format error"),
                        SerialError::Parity => warn!("ESP UART parity error"),
                        SerialError::Noise => warn!("ESP UART noise error"),
                        SerialError::Other => warn!("ESP UART unknown error"),
                        #[allow(unreachable_patterns)]
                        _ => warn!("ESP UART unexpected error: {}", Debug2Format(&err)),
                    }
                    line.clear();
                }
            }
        }
    });
}
