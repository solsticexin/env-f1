// 禁止使用不安全代码
#![deny(unsafe_code)]
// 不使用标准库主函数
#![no_main]
// 不链接标准库
#![no_std]

// 打印恐慌消息到探针控制台
use panic_probe as _;
use defmt_rtt as _;

// LCD 模块
mod dh11;
mod lcd;

// Cortex-M 运行时入口点
use cortex_m_rt::entry;
// 嵌入式图形库
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyleBuilder},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::Text,
};
use heapless::String;
use core::fmt::Write;
use defmt::{info, warn};
use dh11::{Dht11, Error as DhtError};
use embedded_hal::delay::DelayNs;
// STM32F1xx HAL 库
use stm32f1xx_hal::{
    afio::AfioExt,
    pac,
    prelude::*,
    rcc,
    spi::{Mode, Phase, Polarity},
};
// SPI 模式配置：空闲时钟低电平，第一个时钟边沿捕获
pub(crate) const SPI_MODE: Mode = Mode {
    polarity: Polarity::IdleLow,
    phase: Phase::CaptureOnFirstTransition,
};

// 程序入口点
#[entry]
fn main() -> ! {
    // 获取设备外设所有权
    let dp = pac::Peripherals::take().unwrap();
    // 获取核心外设所有权
    let cp = cortex_m::Peripherals::take().unwrap();

    // 配置时钟：外部高速时钟 8MHz，系统时钟 48MHz，PCLK1 24MHz
    let mut flash = dp.FLASH.constrain();
    let mut rcc = dp.RCC.constrain();
    let mut afio = dp.AFIO.constrain(&mut rcc);
    rcc = rcc.freeze(
        rcc::Config::hse(8.MHz())
            .sysclk(48.MHz())
            .pclk1(24.MHz()),
        &mut flash.acr,
    );
    // 创建延迟提供者
    let mut delay = cp.SYST.delay(&rcc.clocks);
    // 配置 GPIOA / GPIOB
    let mut gpioa = dp.GPIOA.split(&mut rcc);
    let mut gpiob = dp.GPIOB.split(&mut rcc);

    // 预先配置 LCD 所需引脚
    let sck = gpioa.pa5.into_alternate_push_pull(&mut gpioa.crl);
    let mosi = gpioa.pa7.into_alternate_push_pull(&mut gpioa.crl);
    let cs = gpioa.pa4.into_push_pull_output(&mut gpioa.crl);
    let dc = gpioa.pa2.into_push_pull_output(&mut gpioa.crl);
    let rst = gpioa.pa3.into_push_pull_output(&mut gpioa.crl);

    // 释放 JTAG 对 PA15/PB3/PB4 的占用
    let (_pa15, pb3, _pb4) = afio.mapr.disable_jtag(gpioa.pa15, gpiob.pb3, gpiob.pb4);

    // 配置 DHT11 数据引脚 PB3 为开漏输出，初始拉高
    let dht_pin = pb3.into_open_drain_output(&mut gpiob.crl);
    let mut dht11 = Dht11::new(dht_pin).expect("DHT11 初始化失败");

    // 背光引脚 PB9
    let backlight_pin = gpiob.pb9.into_push_pull_output(&mut gpiob.crh);

    // 初始化 LCD 显示器
    let (mut display, mut backlight) =
        lcd::init(dp.SPI1, sck, mosi, cs, dc, rst, backlight_pin, &mut rcc, &mut delay)
            .expect("LCD 初始化失败");
    // 确保背光开启
    let _ = backlight.set_high();
    // 清空显示器为黑色
    display.clear(Rgb565::BLACK).unwrap();

    let text_style = MonoTextStyleBuilder::new()
        .font(&FONT_6X10)
        .text_color(Rgb565::WHITE)
        .build();

    // 主循环：每隔 5 秒读取一次温湿度并刷新显示
    loop {
        match dht11.read(&mut delay) {
            Ok(reading) => {
                let temp_int = reading.temperature_tenths / 10;
                let temp_dec = reading.temperature_tenths % 10;
                let hum_int = reading.humidity_tenths / 10;
                let hum_dec = reading.humidity_tenths % 10;

                let mut line1: String<32> = String::new();
                let mut line2: String<32> = String::new();

                let _ = write!(line1, "Temp: {}.{} C", temp_int, temp_dec);
                let _ = write!(line2, "RH:   {}.{} %", hum_int, hum_dec);

                Rectangle::new(Point::new(0, 0), Size::new(128, 40))
                    .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
                    .draw(&mut display)
                    .unwrap();

                Text::new(line1.as_str(), Point::new(10, 20), text_style)
                    .draw(&mut display)
                    .unwrap();
                Text::new(line2.as_str(), Point::new(10, 35), text_style)
                    .draw(&mut display)
                    .unwrap();

                info!(
                    "DHT11: temp {}.{} C, humidity {}.{} %",
                    temp_int,
                    temp_dec,
                    hum_int,
                    hum_dec
                );
            }
            Err(error) => {
                Rectangle::new(Point::new(0, 0), Size::new(128, 40))
                    .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
                    .draw(&mut display)
                    .unwrap();
                Text::new("DHT11 Error", Point::new(10, 20), text_style)
                    .draw(&mut display)
                    .unwrap();

                match error {
                    DhtError::Timeout => warn!("DHT11 timeout"),
                    DhtError::Checksum => warn!("DHT11 checksum mismatch"),
                    DhtError::Pin(_) => warn!("DHT11 pin error"),
                }
            }
        }

        DelayNs::delay_ms(&mut delay, 5_000_u32);
    }
}
