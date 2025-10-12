// LCD 显示模块
//
// 原理说明：
// 本模块用于控制ST7735 LCD显示屏，通过SPI接口与MCU通信。
// MCU（STM32F1）作为主设备，LCD作为从设备，通过SPI总线发送命令和数据。
// 显示原理：ST7735是TFT LCD控制器，通过接收命令和像素数据来控制屏幕显示。
// 通信方式：使用SPI协议，MCU发送初始化命令、设置方向、写入像素数据等。
// 显示过程：先初始化显示屏，设置方向和偏移，然后通过SPI发送像素数据到显示缓冲区，最终在屏幕上渲染。

use core::convert::Infallible;

use cortex_m::asm::nop;
use embedded_hal::{
    delay::DelayNs,
    digital::OutputPin,
    spi::{Operation, SpiBus, SpiDevice},
};
use st7735_lcd::{Orientation, ST7735};
use stm32f1xx_hal::{
    gpio::{
        Alternate, Floating, Input, Output, PushPull,
        gpioa::{PA2, PA3, PA4, PA5, PA7},
        gpiob::PB9,
    },
    pac::SPI1,
    prelude::*,
    rcc::Rcc,
    spi::Spi,
};

// SPI总线类型定义，使用SPI1，8位数据，浮空输入
type SpiBusType = Spi<SPI1, u8, Floating>;
// 片选引脚类型，PA4作为输出推挽
type CsPin = PA4<Output<PushPull>>;
// 数据/命令选择引脚类型，PA2作为输出推挽
type DcPin = PA2<Output<PushPull>>;
// 复位引脚类型，PA3作为输出推挽
type RstPin = PA3<Output<PushPull>>;
// 背光引脚类型，PB9作为输出推挽
type BacklightPin = PB9<Output<PushPull>>;

/// 配置好的ST7735显示屏类型别名
pub type Display = ST7735<SpiDeviceWithCs<SpiBusType, CsPin>, DcPin, RstPin>;

/// 初始化连接到GPIOA的SPI1的LCD显示屏
pub fn init<DELAY>(
    spi_peripheral: SPI1,
    sck: PA5<Alternate<PushPull>>,
    mosi: PA7<Alternate<PushPull>>,
    mut cs: CsPin,
    dc: DcPin,
    rst: RstPin,
    mut backlight: BacklightPin,
    rcc: &mut Rcc,
    delay: &mut DELAY,
) -> Result<(Display, BacklightPin), ()>
where
    DELAY: DelayNs,
{
    // 在交给SPI设备包装器之前，将片选引脚置高空闲
    let _ = cs.set_high();
    // 初始化阶段先关闭背光，避免亮屏闪烁
    let _ = backlight.set_low();

    // 初始化SPI总线，配置引脚、模式、频率等
    let spi = spi_peripheral.spi(
        (
            Some(sck),
            None::<stm32f1xx_hal::gpio::gpioa::PA6<Input<Floating>>>,
            Some(mosi),
        ),
        crate::SPI_MODE,
        16.MHz(),
        rcc,
    );

    // 创建带CS管理的SPI设备
    let spi_device = SpiDeviceWithCs::new(spi, cs);

    // 创建ST7735显示屏实例，参数：SPI设备、DC、RST、RGB顺序、反转行、宽度、高度
    let mut display = ST7735::new(spi_device, dc, rst, true, false, 128, 160);
    // 初始化显示屏硬件
    display.init(delay)?;
    // 设置显示方向为横向，长边水平显示
    let _ = display.set_orientation(&Orientation::Landscape);
    // 设置显示偏移
    display.set_offset(0, 0);
    // 初始化完成后打开背光
    let _ = backlight.set_high();

    Ok((display, backlight))
}

/// 简化的SpiDevice实现，手动管理CS片选引脚
pub struct SpiDeviceWithCs<SPI, CS> {
    spi: SPI,
    cs: CS,
}

impl<SPI, CS> SpiDeviceWithCs<SPI, CS>
where
    SPI: SpiBus<u8>,
    CS: OutputPin,
{
    // 创建新的SpiDeviceWithCs实例，初始化CS引脚为高电平
    pub fn new(spi: SPI, mut cs: CS) -> Self {
        let _ = cs.set_high();
        Self { spi, cs }
    }
}

// 实现Deref trait，允许直接访问内部SPI总线
impl<SPI, CS> core::ops::Deref for SpiDeviceWithCs<SPI, CS> {
    type Target = SPI;

    fn deref(&self) -> &Self::Target {
        &self.spi
    }
}

// 实现DerefMut trait，允许直接修改内部SPI总线
impl<SPI, CS> core::ops::DerefMut for SpiDeviceWithCs<SPI, CS> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.spi
    }
}

// 实现ErrorType trait，定义SPI设备的错误类型
impl<SPI, CS> embedded_hal::spi::ErrorType for SpiDeviceWithCs<SPI, CS>
where
    SPI: SpiBus<u8>,
{
    type Error = SPI::Error;
}

// 实现SpiDevice trait，处理SPI事务并管理CS引脚
impl<SPI, CS> SpiDevice for SpiDeviceWithCs<SPI, CS>
where
    SPI: SpiBus<u8>,
    CS: OutputPin<Error = Infallible>,
{
    // 执行SPI事务：拉低CS，开始通信，执行操作，拉高CS结束
    fn transaction(&mut self, operations: &mut [Operation<'_, u8>]) -> Result<(), SPI::Error> {
        // 拉低CS引脚，开始SPI通信
        let _ = self.cs.set_low();
        // 执行所有SPI操作
        let result = (|| -> Result<(), SPI::Error> {
            for operation in operations.iter_mut() {
                match operation {
                    Operation::Read(buffer) => self.spi.read(buffer)?, // 读取数据到缓冲区
                    Operation::Write(buffer) => self.spi.write(buffer)?, // 写入缓冲区数据
                    Operation::Transfer(read, write) => self.spi.transfer(read, write)?, // 同时读写
                    Operation::TransferInPlace(buffer) => self.spi.transfer_in_place(buffer)?, // 原地传输
                    Operation::DelayNs(delay_ns) => busy_wait_ns(*delay_ns), // 软件延时
                }
            }
            // 刷新SPI总线，确保所有数据发送完成
            self.spi.flush()
        })();
        // 拉高CS引脚，结束SPI通信
        let _ = self.cs.set_high();
        result
    }
}

// 软件忙等待延时函数，用于可选的DelayNs操作
fn busy_wait_ns(ns: u32) {
    // 通过循环执行nop指令实现粗略的纳秒级延时
    for _ in 0..ns {
        nop();
    }
}
