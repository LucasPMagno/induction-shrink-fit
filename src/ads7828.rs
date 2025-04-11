use embassy_rp::pac::Interrupt::I2C1_IRQ;
use embassy_time::Duration;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_rp::i2c::{I2c, Error as I2cError, Blocking};
use embassy_rp::peripherals::I2C1; // or I2C1 if that’s your hardware
use core::future::Future;


// Map from your original code
const ADS7828_CHANNEL_MAP: [u8; 8] = [
    0b00000000,
    0b01000000,
    0b00010000,
    0b01010000,
    0b00100000,
    0b01100000,
    0b00110000,
    0b01110000,
];

/// ADS7828 driver on a shared I2C bus (blocking mode).
///
/// `'d`: The Embassy "lifetime" for device usage
/// `I2C1` is the peripheral instance
/// `Blocking` is the embassy-rp "Mode" for blocking I2C
pub struct Ads7828<'d> {
    i2c: Mutex<CriticalSectionRawMutex, I2c<'d, I2C1, Blocking>>,
    address: u8,
}

impl<'d> Ads7828<'d> {
    /// Create a new `Ads7828`.
    /// `i2c` must be `I2c<'d, I2C1, Blocking>` (or similar),
    /// `address` is the 7-bit address of the ADS7828.
    pub fn new(i2c: I2c<'d, I2C1, Blocking>, address: u8) -> Self {
        Self {
            i2c: Mutex::new(i2c),
            address,
        }
    }

    /// Generate the command byte. 
    fn generate_command_byte(channel: u8, ref_on: bool, converter_on: bool) -> u8 {
        let mut byte = 0b1000_0000; // single ended mode
        if channel > 7 {
            return 0; // clamp or handle error
        }
        byte |= ADS7828_CHANNEL_MAP[channel as usize];

        if ref_on {
            byte |= 0b0000_1000;
        }
        if converter_on {
            byte |= 0b0000_0100;
        }
        byte
    }

    /// Get a single 12-bit reading from `channel` (0..7).
    /// 
    /// `nostop` typically implies a repeated-start. In Embassy’s blocking
    /// I2C, `write_then_read` does a repeated start, not a “no stop” cycle.
    pub async fn get_channel(&self, channel: u8, _nostop: bool) -> Result<u16, I2cError> {
        let cmd = Self::generate_command_byte(channel, false, true);

        let mut i2c_guard = self.i2c.lock().await;
        // Write command:
        i2c_guard.blocking_write(self.address, &[cmd])?;

        // Read 2 bytes:
        let mut buf = [0; 2];
        i2c_guard.blocking_read(self.address, &mut buf)?;

        // Extract the 12-bit sample:
        let sample = (((buf[0] & 0x0F) as u16) << 8) | (buf[1] as u16);
        Ok(sample)
    }

    /// Read all 8 channels (0..7).
    pub async fn get_channels(&self, _nostop: bool) -> Result<[u16; 8], I2cError> {
        let mut out = [0; 8];
        for c in 0..8 {
            out[c] = self.get_channel(c as u8, true).await?;
        }
        Ok(out)
    }
}
