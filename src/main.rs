// 禁止使用不安全代码
#![deny(unsafe_code)]
// 不使用标准库主函数
#![no_main]
// 不链接标准库
#![no_std]

// 打印恐慌消息到探针控制台
use defmt_rtt as _;
use panic_probe as _;

// LCD 模块
mod dh11;
mod lcd;
mod light_sensor;
mod soil_sensor;

// Cortex-M 运行时入口点
use cortex_m_rt::entry;
// 嵌入式图形库
use core::fmt::Write;
use defmt::{info, warn};
use dh11::{Dht11, Error as DhtError};
use embedded_graphics::{
    image::{Image, ImageRawLE},
    mono_font::{
        MonoTextStyleBuilder,
        ascii::{FONT_9X15, FONT_9X15_BOLD},
    },
    pixelcolor::Rgb565,
    prelude::*,
    text::Text,
};
use embedded_hal::delay::DelayNs;
use heapless::String;
use light_sensor::Bh1750;
use soil_sensor::SoilSensor;
// STM32F1xx HAL 库
use stm32f1xx_hal::{
    adc::AdcExt,
    afio::AfioExt,
    gpio::PinState,
    i2c::{BlockingI2c, Mode as I2cMode},
    pac,
    prelude::*,
    rcc,
    spi::{Mode as SpiMode, Phase, Polarity},
};
// SPI 模式配置：空闲时钟低电平，第一个时钟边沿捕获
pub(crate) const SPI_MODE: SpiMode = SpiMode {
    polarity: Polarity::IdleLow,
    phase: Phase::CaptureOnFirstTransition,
};

// 程序入口点
#[entry]
fn main() -> ! {
    // 获取设备外设所有权
    let dp = pac::Peripherals::take().unwrap();
    // 获取核心外设所有权
    let mut cp = cortex_m::Peripherals::take().unwrap();

    // 配置时钟：外部高速时钟 8MHz，系统时钟 48MHz，PCLK1 24MHz
    let mut flash = dp.FLASH.constrain();
    let mut rcc = dp.RCC.constrain();
    let _afio = dp.AFIO.constrain(&mut rcc);
    rcc = rcc.freeze(
        rcc::Config::hse(8.MHz()).sysclk(48.MHz()).pclk1(24.MHz()),
        &mut flash.acr,
    );
    // 开启 DWT 周期计数器以支持 I2C 阻塞实现的超时
    cp.DCB.enable_trace();
    cp.DWT.enable_cycle_counter();

    // 创建延迟提供者
    let mut delay = cp.SYST.delay(&rcc.clocks);
    // 配置 GPIOA / GPIOB
    let mut gpioa = dp.GPIOA.split(&mut rcc);
    let mut gpiob = dp.GPIOB.split(&mut rcc);

    // 配置 I2C1：PB6 (SCL), PB7 (SDA)
    let scl = gpiob.pb6.into_alternate_open_drain(&mut gpiob.crl);
    let sda = gpiob.pb7.into_alternate_open_drain(&mut gpiob.crl);
    let i2c = BlockingI2c::new(
        dp.I2C1,
        (scl, sda),
        I2cMode::Standard {
            frequency: 100.kHz(),
        },
        &mut rcc,
        1_000,
        10,
        1_000,
        1_000,
    );

    // 预先配置 LCD 所需引脚
    let sck = gpioa.pa5.into_alternate_push_pull(&mut gpioa.crl);
    let mosi = gpioa.pa7.into_alternate_push_pull(&mut gpioa.crl);
    let cs = gpioa.pa4.into_push_pull_output(&mut gpioa.crl);
    let dc = gpioa.pa2.into_push_pull_output(&mut gpioa.crl);
    let rst = gpioa.pa3.into_push_pull_output(&mut gpioa.crl);

    // 释放 JTAG 对 PA15/PB3/PB4 的占用
    // let (_pa15, pb3, _pb4) = _afio.mapr.disable_jtag(gpioa.pa15, gpiob.pb3, gpiob.pb4);

    // 配置 DHT11 数据引脚 PB5 为开漏输出，初始拉高
    let dht_pin = gpiob
        .pb5
        .into_open_drain_output_with_state(&mut gpiob.crl, PinState::High);
    // dht_pin.set_high();
    let mut dht11 = Dht11::new(dht_pin).expect("DHT11 初始化失败");

    // 配置土壤湿度传感器：PA0 模拟输入 + ADC1
    let soil_pin = gpioa.pa0.into_analog(&mut gpioa.crl);
    let soil_adc = dp.ADC1.adc(&mut rcc);
    let mut soil_sensor = SoilSensor::new(soil_adc, soil_pin);
    let mut light_sensor = Bh1750::new(i2c).expect("BH1750 初始化失败");

    // //配置蜂鸣器引脚
    // let mut buzzer=gpioa.pa1.into_push_pull_output(&mut gpioa.crl);
    // buzzer.set_low();

    // DHT11 初始化后延迟 2 秒
    DelayNs::delay_ms(&mut delay, 3_500_u32);

    // 背光引脚 PB9
    let backlight_pin = gpiob.pb9.into_push_pull_output(&mut gpiob.crh);

    // 初始化 LCD 显示器
    let (mut display, mut backlight) = lcd::init(
        dp.SPI1,
        sck,
        mosi,
        cs,
        dc,
        rst,
        backlight_pin,
        &mut rcc,
        &mut delay,
    )
    .expect("LCD 初始化失败");
    // 确保背光开启
    let _ = backlight.set_high();
    // 清空显示器为黑色
    display.clear(Rgb565::BLACK).unwrap();

    let label_text_style = MonoTextStyleBuilder::new()
        .font(&FONT_9X15)
        .text_color(Rgb565::WHITE)
        .build();
    let value_text_style = MonoTextStyleBuilder::new()
        .font(&FONT_9X15_BOLD)
        .text_color(Rgb565::YELLOW)
        .build();
    let background_raw = ImageRawLE::new(
        include_bytes!("../assets/images/status_background.rgb565"),
        120,
    );
    let background_origin = Point::new(0, 16);
    let background_image = Image::new(&background_raw, background_origin);

    // 主循环：每隔 5 秒读取一次温湿度并刷新显示
    loop {
        // 测试用的 1 秒节拍，累计 5 秒刷新一次
        // for _ in 0..5 {
        //     defmt::info!("Waiting 1s...");
        //     DelayNs::delay_ms(&mut delay, 1_000_u32);
        // }
        // 翻转蜂鸣器
        // buzzer.toggle();
        // DelayNs::delay_ms(&mut delay, 5_000_u32);
        let mut soil_value: String<16> = String::new();
        let mut light_value: String<16> = String::new();

        match soil_sensor.read_raw() {
            Ok(raw_value) => {
                let percent = SoilSensor::raw_to_percent(raw_value);
                let _ = write!(soil_value, "{}%", percent);
                info!("Soil moisture: raw {} counts (~{}%)", raw_value, percent);
            }
            Err(_) => {
                let _ = soil_value.push_str("--");
                warn!("Soil sensor read error");
            }
        }

        match light_sensor.read_lux_tenths(&mut delay) {
            Ok(lux_tenths) => {
                let lux_int = lux_tenths / 10;
                let lux_dec = lux_tenths % 10;
                let _ = write!(light_value, "{}.{}lx", lux_int, lux_dec);
                info!("BH1750: {}.{} lux", lux_int, lux_dec);
            }
            Err(_) => {
                let _ = light_value.push_str("--");
                warn!("BH1750 read error");
            }
        }

        let mut temp_value: String<16> = String::new();
        let mut hum_value: String<16> = String::new();
        let mut status_line: Option<&'static str> = None;

        match dht11.read(&mut delay) {
            Ok(reading) => {
                let temp_int = reading.temperature_tenths / 10;
                let temp_dec = reading.temperature_tenths % 10;
                let hum_int = reading.humidity_tenths / 10;
                let hum_dec = reading.humidity_tenths % 10;

                let _ = write!(temp_value, "{}.{}℃", temp_int, temp_dec);
                let _ = write!(hum_value, "{}.{}%", hum_int, hum_dec);
                info!(
                    "DHT11: temp {}.{} C, humidity {}.{} %",
                    temp_int, temp_dec, hum_int, hum_dec
                );
            }
            Err(error) => {
                let _ = temp_value.push_str("--");
                let _ = hum_value.push_str("--");
                status_line = Some("DHT11 ERR");
                match error {
                    DhtError::Timeout => warn!("DHT11 timeout"),
                    DhtError::Checksum => warn!("DHT11 checksum mismatch"),
                    DhtError::Pin(_) => warn!("DHT11 pin error"),
                }
            }
        }

        background_image.draw(&mut display).unwrap();

        let value_texts = [
            temp_value.as_str(),
            hum_value.as_str(),
            light_value.as_str(),
            soil_value.as_str(),
        ];
        for (index, text) in value_texts.iter().enumerate() {
            let offset = Point::new(80, 30 + (index as i32) * 20);
            Text::new(*text, background_origin + offset, value_text_style)
                .draw(&mut display)
                .unwrap();
        }

        if let Some(status) = status_line {
            Text::new(
                status,
                background_origin + Point::new(0, -16),
                label_text_style,
            )
            .draw(&mut display)
            .unwrap();
        }

        DelayNs::delay_ms(&mut delay, 5_000_u32);
    }
}
