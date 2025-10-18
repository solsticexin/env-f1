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
mod actuators;
mod dh11;
mod lcd;
mod light_sensor;
mod protocol;
mod soil_sensor;

// Cortex-M 运行时入口点
use cortex_m_rt::entry;
// 嵌入式图形库
use actuators::Actuators;
use core::fmt::Write;
use defmt::{info, warn};
use dh11::{Dht11, Error as DhtError};
use embedded_graphics::{
    draw_target::DrawTarget,
    mono_font::{
        MonoTextStyleBuilder,
        ascii::{FONT_6X12, FONT_6X13_BOLD},
    },
    pixelcolor::{Rgb565, raw::RawU16},
    prelude::*,
    primitives::{PrimitiveStyleBuilder, Rectangle},
    text::Text,
};
use embedded_hal::delay::DelayNs;
use heapless::String;
use light_sensor::Bh1750;
use nb::Error as NbError;
use soil_sensor::SoilSensor;
// STM32F1xx HAL 库
use protocol::{ProtocolError, ProtocolLink, TelemetryFrame};
use stm32f1xx_hal::{
    adc::AdcExt,
    afio::AfioExt,
    gpio::PinState,
    i2c::{BlockingI2c, Mode as I2cMode},
    pac,
    prelude::*,
    rcc,
    serial::{Config as SerialConfig, Error as SerialError, Instance, Rx, SerialExt},
    spi::{Mode as SpiMode, Phase, Polarity},
};
// SPI 模式配置：空闲时钟低电平，第一个时钟边沿捕获
pub(crate) const SPI_MODE: SpiMode = SpiMode {
    polarity: Polarity::IdleLow,
    phase: Phase::CaptureOnFirstTransition,
};

const BACKGROUND_WIDTH: u32 = 120;
const BACKGROUND_HEIGHT: u32 = 96;
const BACKGROUND_SCALE: u32 = 1;
const ESP_IP: &str = "192.168.4.1";

fn draw_scaled_rgb565_image<D>(
    display: &mut D,
    origin: Point,
    raw: &[u8],
    src_width: u32,
    src_height: u32,
    scale: u32,
) -> Result<(), D::Error>
where
    D: DrawTarget<Color = Rgb565>,
{
    if scale == 0 {
        return Ok(());
    }

    let scaled_width = src_width / scale;
    let scaled_height = src_height / scale;

    for y in 0..scaled_height {
        let src_y = y * scale;
        for x in 0..scaled_width {
            let src_x = x * scale;
            let index = ((src_y * src_width + src_x) * 2) as usize;
            if index + 1 >= raw.len() {
                continue;
            }
            let pixel = u16::from_le_bytes([raw[index], raw[index + 1]]);
            let color = Rgb565::from(RawU16::new(pixel));
            Pixel(origin + Point::new(x as i32, y as i32), color).draw(display)?;
        }
    }

    Ok(())
}

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
        rcc::Config::hse(8.MHz())
            .sysclk(72.MHz())
            .pclk1(36.MHz())
            .pclk2(72.MHz()),
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

    // 配置继电器与蜂鸣器输出引脚
    let water_pin = gpiob.pb0.into_push_pull_output(&mut gpiob.crl);
    let light_pin = gpiob.pb1.into_push_pull_output(&mut gpiob.crl);
    let fan_pin = gpiob.pb10.into_push_pull_output(&mut gpiob.crh);
    let buzzer_pin = gpioa.pa1.into_push_pull_output(&mut gpioa.crl);

    // 配置 USART1：PA9 (TX), PA10 (RX)
    let tx_pin = gpioa.pa9.into_alternate_push_pull(&mut gpioa.crh);
    let rx_pin = gpioa.pa10;
    let serial = dp.USART1.serial(
        (tx_pin, rx_pin),
        SerialConfig::default().baudrate(115_200.bps()),
        &mut rcc,
    );
    let (tx, mut rx) = serial.split();
    let mut protocol_link = ProtocolLink::new(tx);
    let mut actuators = Actuators::new(water_pin, light_pin, fan_pin, buzzer_pin);

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

    //DHT11 初始化后延迟 2 秒
    DelayNs::delay_ms(&mut delay, 2_000_u32);

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
        .font(&FONT_6X12)
        .text_color(Rgb565::WHITE)
        .build();
    let value_text_style = MonoTextStyleBuilder::new()
        .font(&FONT_6X13_BOLD)
        .text_color(Rgb565::YELLOW)
        .build();
    let ip_text_style = MonoTextStyleBuilder::new()
        .font(&FONT_6X13_BOLD)
        .text_color(Rgb565::CYAN)
        .build();
    let background_bytes = include_bytes!("../assets/images/status_background.rgb565");
    let background_origin = Point::new(8, 28);
    let value_text_origin = Point::new(92, 55);
    let ip_text_origin = Point::new(8, 18);
    let scaled_background_height = BACKGROUND_HEIGHT / BACKGROUND_SCALE;

    const TICK_MS: u32 = 1;
    const SAMPLE_PERIOD_MS: u32 = 5_000;
    let mut command_buffer: String<256> = String::new();
    let mut elapsed_ms: u32 = SAMPLE_PERIOD_MS;

    // 主循环：定时采集数据，并在空闲时处理控制命令
    loop {
        // 处理来自 ESP 的命令
        drain_uart(&mut rx, &mut command_buffer);

        if elapsed_ms >= SAMPLE_PERIOD_MS {
            elapsed_ms = 0;
            let mut telemetry = TelemetryFrame::default();
            let mut soil_value: String<16> = String::new();
            let mut light_value: String<16> = String::new();

            match soil_sensor.read_raw() {
                Ok(raw_value) => {
                    let percent = SoilSensor::raw_to_percent(raw_value);
                    let _ = write!(soil_value, "{}%", percent);
                    telemetry.sensors.soil_pct = Some(percent);
                    // info!("Soil moisture: raw {} counts (~{}%)", raw_value, percent);
                }
                Err(_) => {
                    let _ = soil_value.push_str("--");
                    warn!("Soil sensor read error");
                }
            }
            drain_uart(&mut rx, &mut command_buffer);

            match light_sensor.read_lux_tenths(&mut delay) {
                Ok(lux_tenths) => {
                    let lux_int = lux_tenths / 10;
                    let lux_dec = lux_tenths % 10;
                    let _ = write!(light_value, "{}.{}lx", lux_int, lux_dec);
                    telemetry.sensors.lux_tenths = Some(lux_tenths);
                    // info!("BH1750: {}.{} lux", lux_int, lux_dec);
                }
                Err(_) => {
                    let _ = light_value.push_str("--");
                    warn!("BH1750 read error");
                }
            }
            drain_uart(&mut rx, &mut command_buffer);

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
                    telemetry.sensors.temperature_tenths_c =
                        Some(reading.temperature_tenths as i16);
                    telemetry.sensors.humidity_tenths_pct = Some(reading.humidity_tenths);
                    // info!(
                    //     "DHT11: temp {}.{} C, humidity {}.{} %",
                    //     temp_int, temp_dec, hum_int, hum_dec
                    // );
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
            drain_uart(&mut rx, &mut command_buffer);

            telemetry.actuators = actuators.snapshot();
            test_print_telemetry_json(&telemetry);

            let clear_top = (background_origin.y - 8).max(0);
            let clear_height = (scaled_background_height + 16).min(128);
            Rectangle::new(Point::new(0, clear_top), Size::new(160, clear_height))
                .into_styled(
                    PrimitiveStyleBuilder::new()
                        .fill_color(Rgb565::BLACK)
                        .build(),
                )
                .draw(&mut display)
                .unwrap();

            draw_scaled_rgb565_image(
                &mut display,
                background_origin,
                background_bytes,
                BACKGROUND_WIDTH,
                BACKGROUND_HEIGHT,
                BACKGROUND_SCALE,
            )
            .unwrap();
            drain_uart(&mut rx, &mut command_buffer);

            Text::new(ESP_IP, ip_text_origin, ip_text_style)
                .draw(&mut display)
                .unwrap();
            drain_uart(&mut rx, &mut command_buffer);

            let value_texts = [
                temp_value.as_str(),
                hum_value.as_str(),
                light_value.as_str(),
                soil_value.as_str(),
            ];
            for (index, text) in value_texts.iter().enumerate() {
                let offset = Point::new(0, (index as i32) * 21);
                Text::new(*text, value_text_origin + offset, value_text_style)
                    .draw(&mut display)
                    .unwrap();
                drain_uart(&mut rx, &mut command_buffer);
            }

            if let Some(status) = status_line {
                Text::new(
                    status,
                    background_origin + Point::new(0, -10),
                    label_text_style,
                )
                .draw(&mut display)
                .unwrap();
                drain_uart(&mut rx, &mut command_buffer);
            }

            if let Err(error) = protocol_link.send_data(&telemetry) {
                match error {
                    ProtocolError::BufferOverflow => warn!("Telemetry buffer overflow"),
                    ProtocolError::Serial(serial_err) => {
                        let _ = serial_err;
                        warn!("Telemetry UART error")
                    }
                }
            }
        }

        DelayNs::delay_ms(&mut delay, TICK_MS);
        drain_uart(&mut rx, &mut command_buffer);
        elapsed_ms = elapsed_ms.saturating_add(TICK_MS);
    }
}

// fn process_command_line<USART, D>(
//     line: &[u8],
//     actuators: &mut Actuators,
//     protocol_link: &mut ProtocolLink<USART>,
//     delay: &mut D,
// ) where
//     USART: Instance,
//     D: DelayNs,
// {
//     match protocol::parse_command_frame(line) {
//         Ok(command) => {
//             let apply_result = actuators.apply_command(&command, delay);
//             let ack_result = if apply_result.is_ok() {
//                 AckResult::Ok
//             } else {
//                 AckResult::Error
//             };
//
//             if let Err(error) = &apply_result {
//                 match error {
//                     ActuatorError::MissingPulseDuration => {
//                         warn!("Pulse duration missing for command")
//                     }
//                 }
//             }
//
//             if let Err(error) = protocol_link.send_ack(AckFrame {
//                 target: command.target,
//                 action: command.action,
//                 result: ack_result,
//             }) {
//                 match error {
//                     ProtocolError::BufferOverflow => warn!("Ack buffer overflow"),
//                     ProtocolError::Serial(_serial_err) => warn!("Ack UART error"),
//                 }
//             }
//         }
//         Err(CommandParseError::Json) => warn!("Invalid JSON received"),
//         Err(CommandParseError::UnexpectedType) => warn!("Ignoring non-command frame"),
//         Err(CommandParseError::UnknownTarget) => warn!("Unknown command target"),
//         Err(CommandParseError::UnknownAction) => warn!("Unknown command action"),
//         Err(CommandParseError::MissingPulseDuration) => warn!("Pulse command missing time field"),
//     }
// }

fn test_print_telemetry_json(frame: &TelemetryFrame) {
    match protocol::build_data_payload(frame) {
        Ok(payload) => {
            let json = payload.trim_end_matches('\n');
            info!("Telemetry JSON: {}", json);
        }
        Err(ProtocolError::BufferOverflow) => warn!("Telemetry JSON buffer overflow"),
        Err(ProtocolError::Serial(_)) => warn!("Telemetry JSON serialize error"),
    }
}

fn print_received_command_json(line: &[u8]) {
    match core::str::from_utf8(line) {
        Ok(text) => info!("Received command JSON: {}", text),
        Err(_) => warn!("Received command JSON is not valid UTF-8"),
    }
}

fn drain_uart<USART>(rx: &mut Rx<USART>, command_buffer: &mut String<256>)
where
    USART: Instance,
{
    loop {
        match rx.read() {
            Ok(byte) => {
                if byte == b'\n' {
                    if let Some(b'\r') = command_buffer.as_bytes().last() {
                        let _ = command_buffer.pop();
                    }
                    if !command_buffer.is_empty() {
                        print_received_command_json(command_buffer.as_bytes());
                    }
                    command_buffer.clear();
                } else if byte == b'\r' {
                    // 忽略 CR
                } else if command_buffer.push(byte as char).is_err() {
                    warn!("Command line overflow, clearing buffer");
                    command_buffer.clear();
                }
            }
            Err(NbError::WouldBlock) => break,
            Err(NbError::Other(error)) => {
                match error {
                    SerialError::Overrun => warn!("UART RX overrun, clearing buffer"),
                    SerialError::FrameFormat => warn!("UART RX frame format error"),
                    SerialError::Parity => warn!("UART RX parity error"),
                    SerialError::Noise => warn!("UART RX noise error"),
                    SerialError::Other => warn!("UART RX unknown error"),
                    #[allow(unreachable_patterns)]
                    _ => warn!("UART RX unexpected error"),
                }
                command_buffer.clear();
                // Clear hardware overrun flag by reading SR and DR implicitly via HAL
            }
        }
    }
}
