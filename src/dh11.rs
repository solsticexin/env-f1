//! DHT11 温湿度传感器驱动。
//!
//! 本模块提供一个最小的阻塞式驱动，用于通过配置为开漏的 GPIO 从单线 DHT11 传感器读取测量值。
//! 时序由实现了 [`DelayNs`] 的延时提供者完成，通常使用 HAL 中基于 SysTick 的延时实现。

use core::convert::Infallible;

use embedded_hal::{
    delay::DelayNs,
    digital::{InputPin, OutputPin},
};

/// 查询 DHT11 传感器时可能返回的错误类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error<PinError = Infallible> {
    /// 传感器在期望的时序窗口内没有响应（超时）。
    Timeout,
    /// 接收到的数据校验和验证失败。
    Checksum,
    /// 对 GPIO 的访问失败（引脚操作错误）。
    Pin(PinError),
}

/// 单次 DHT11 测量值，单位为十分之一。
///
/// 使用整数的十分之一表示法以避免浮点运算，同时保留传感器报出的个位小数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    /// 相对湿度，单位为百分比的十分之一（tenths）。
    pub humidity_tenths: u16,
    /// 温度，单位为摄氏度的十分之一（tenths）。
    pub temperature_tenths: u16,
}

/// 阻塞式 DHT11 驱动。
pub struct Dht11<P> {
    pin: P,
}

impl<P, PinError> Dht11<P>
where
    P: OutputPin<Error = PinError> + InputPin<Error = PinError>,
{
    /// 使用开漏 GPIO 引脚创建新的驱动实例。
    pub fn new(mut pin: P) -> Result<Self, PinError> {
        // 在第一次请求之前确保数据线处于空闲高电平。
        pin.set_high()?;
        Ok(Self { pin })
    }

    /// 从传感器读取一次测量值（阻塞）。
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
                value <<= 1; //左移一位

                // 每一位以传感器将数据线拉低开始（约 50 µs）。
                self.wait_for_level(delay, false, 70)?;
                // 随后是高电平：约 26 µs 表示 '0'，约 70 µs 表示 '1'。
                self.wait_for_level(delay, true, 70)?;

                let mut high_time = 0u32;
                while self.pin.is_high().map_err(Error::Pin)? {
                    delay.delay_us(1_u32);
                    high_time += 1;
                    if high_time > 100 {
                        return Err(Error::Timeout);
                    }
                }

                // 根据高电平持续时间阈值判断位值（阈值在此处使用 40 µs）。
                if high_time > 50 {
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

        // 添加调试输出（方便在 defmt 日志中查看原始字节）
        defmt::info!(
            "DHT11 raw data: {} {} {} {} {}",
            data[0],
            data[1],
            data[2],
            data[3],
            data[4]
        );

        let humidity_tenths = u16::from(data[0]) * 10 + u16::from(data[1]);
        let temperature_tenths = u16::from(data[2]) * 10 + u16::from(data[3]);

        Ok(Reading {
            humidity_tenths,
            temperature_tenths,
        })
    }

    fn start_signal<DELAY>(&mut self, delay: &mut DELAY) -> Result<(), Error<PinError>>
    where
        DELAY: DelayNs,
    {
        // 主机拉低数据线至少 18ms（此处使用 20ms），作为启动信号，然后拉高并等待传感器准备。
        self.pin.set_low().map_err(Error::Pin)?;
        delay.delay_ms(20_u32);
        self.pin.set_high().map_err(Error::Pin)?;
        delay.delay_us(30_u32);
        Ok(())
    }

    fn await_sensor_response<DELAY>(&mut self, delay: &mut DELAY) -> Result<(), Error<PinError>>
    where
        DELAY: DelayNs,
    {
        // 传感器响应阶段：先拉低（约 80 µs），再拉高（约 80 µs），然后再次拉低，随后开始发送数据位。
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
            // 每次循环等待 1 µs，总共最多等待 timeout_us 微秒。
            delay.delay_us(1_u32);
        }
        // 超时未达到期望电平
        Err(Error::Timeout)
    }
}
