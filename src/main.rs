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
mod lcd;

// Cortex-M 运行时入口点
use cortex_m_rt::entry;
// 嵌入式图形库
use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{Circle, PrimitiveStyle},
};
// STM32F1xx HAL 库
use stm32f1xx_hal::{
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

// 允许空的循环（因为这是嵌入式程序的主循环）
#[allow(clippy::empty_loop)]
// 程序入口点
#[entry]
fn main() -> ! {
    // 获取设备外设所有权
    let dp = pac::Peripherals::take().unwrap();
    // 获取核心外设所有权
    let cp = cortex_m::Peripherals::take().unwrap();

    // 配置时钟：外部高速时钟 8MHz，系统时钟 48MHz，PCLK1 24MHz
    let mut flash = dp.FLASH.constrain();
    let mut rcc = dp.RCC.freeze(
        rcc::Config::hse(8.MHz())
        .sysclk(48.MHz())
        .pclk1(24.MHz()),
        &mut flash.acr,
    );
    // 创建延迟提供者
    let mut delay = cp.SYST.delay(&rcc.clocks);
    // 配置 GPIOA
    let gpioa= dp.GPIOA.split(&mut rcc);

    // 初始化 LCD 显示器
    let mut display = lcd::init(dp.SPI1, gpioa, &mut rcc, &mut delay)
        .expect("LCD 初始化失败");
    // 清空显示器为黑色
    display.clear(Rgb565::BLACK).unwrap();

    // 绘制一个白色圆圈，中心点 (64, 80)，半径 60，线宽 2
    Circle::with_center(Point::new(64, 80), 60)
        .into_styled(PrimitiveStyle::with_stroke(Rgb565::WHITE, 2))
        .draw(&mut display)
        .unwrap();

    // 主循环：等待中断
    loop {
        cortex_m::asm::wfi();
    }
}
