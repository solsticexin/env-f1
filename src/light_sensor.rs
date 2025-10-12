//! BH1750 光照传感器驱动模块。
//!
//! 该模块封装了对 BH1750（I²C 光强度传感器）的最小阻塞式访问逻辑，方便在主循环中
//! 定期读取当前环境光照强度。ADDR 引脚接地时，设备地址固定为 0x23。

use embedded_hal::{delay::DelayNs, i2c::I2c};

/// BH1750 传感器的简单驱动。
///
/// 持有 I²C 外设实例并提供高分辨率模式下的读数接口；读数结果以十分之一勒克斯返回，
/// 这样既保持了一位小数的分辨率，又避免在微控制器上使用浮点运算。
pub struct Bh1750<I2C> {
    i2c: I2C,
}

impl<I2C> Bh1750<I2C>
where
    I2C: I2c,
{
    const ADDRESS: u8 = 0x23;
    const CMD_POWER_ON: u8 = 0x01;
    const CMD_RESET: u8 = 0x07;
    const CMD_CONT_HIGH_RES: u8 = 0x10;
    // 官方手册建议高分辨率模式约 120ms，这里取 180ms 留足余量。
    const MEASUREMENT_DELAY_MS: u32 = 180;

    /// 创建传感器驱动并完成基本上电/复位命令。
    ///
    /// 该函数会向传感器发送 POWER_ON 与 RESET 指令，若通信失败则返回对应的 I²C 错误。
    pub fn new(mut i2c: I2C) -> Result<Self, I2C::Error> {
        i2c.write(Self::ADDRESS, &[Self::CMD_POWER_ON])?;
        i2c.write(Self::ADDRESS, &[Self::CMD_RESET])?;
        Ok(Self { i2c })
    }

    /// 读取一次光照强度，返回单位为 0.1 lx 的整数。
    ///
    /// 流程为：触发连续高分辨率测量模式 -> 等待转换完成 -> 读取两字节测量结果并换算。
    pub fn read_lux_tenths<DELAY>(&mut self, delay: &mut DELAY) -> Result<u32, I2C::Error>
    where
        DELAY: DelayNs,
    {
        self.i2c.write(Self::ADDRESS, &[Self::CMD_CONT_HIGH_RES])?;
        delay.delay_ms(Self::MEASUREMENT_DELAY_MS);

        let mut buf = [0u8; 2];
        self.i2c.read(Self::ADDRESS, &mut buf)?;

        let raw = u16::from_be_bytes(buf) as u32;
        // 数据手册给出的换算系数为 1 lx ≈ raw / 1.2
        let lux_tenths = (raw * 10) / 12;
        Ok(lux_tenths)
    }
}
