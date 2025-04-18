use core::fmt::Debug;
use defmt::*;
use embassy_rp::i2c::{self, I2c};
use embassy_time::{Duration, Timer};

/// Default 7‑bit SMBus address
pub const MLX90614_ADDR: u8 = 0x5A;

/// RAM / EEPROM locations we care about
const REG_TOBJ1: u8          = 0x07;  // object temperature 1, read‑only RAM
const EEPROM_EMISSIVITY: u8  = 0x04;  // EEPROM emissivity
const EEPROM_UNLOCK: u8      = 0x0F;  // xCx devices only

/// Value for ε = 0.82 → round(0.82 × 65535) = 0xD1EB
const EMISSIVITY_WORD: u16   = 0xD1EB;

/// MLX90614 object – owns the I²C peripheral
pub struct Mlx90614<'d, T: i2c::Instance, M: i2c::Mode> {
    i2c: I2c<'d, T, M>,
}

impl<'d, T: i2c::Instance, M: i2c::Mode> Mlx90614<'d, T, M> {
    /// Create a new driver from an already‑configured Embassy I²C bus
    pub fn new(i2c: I2c<'d, T, M>) -> Self {
        Self { i2c }
    }

    // ───────────────────────────────── temperature read ─────────────────────────────────
    /// Read object temperature 1 and return it in °C
    pub async fn read_object_temp(&mut self) -> Result<f32, i2c::Error> {
        let raw: u16 = self.read_word(REG_TOBJ1).await?;
        // data sheet: Temp[°C] = (RAW * 0.02) – 273.15
        Ok(raw as f32 * 0.02 - 273.15)
    }

    // ─────────────────────────────── emissivity programming ────────────────────────────
    /// Program ε = 0.82 permanently (writes cells 0x04 & 0x0F).
    /// *⚠ A power‑cycle is required for the new value to take effect.*
    pub async fn program_emissivity_082(&mut self) -> Result<(), i2c::Error> {
        // 1) unlock cell 0x0F (device expects the “key” command 0x60).
        self.simple_command(0x60).await?;
        Timer::after(Duration::from_millis(10)).await;

        // 2) erase 0x04, then write new value
        self.write_word(EEPROM_EMISSIVITY, 0x0000).await?;
        Timer::after(Duration::from_millis(10)).await;
        self.write_word(EEPROM_EMISSIVITY, EMISSIVITY_WORD).await?;
        Timer::after(Duration::from_millis(10)).await;

        // 3) erase 0x0F, then write new shadow copy
        self.write_word(EEPROM_UNLOCK, 0x0000).await?;
        Timer::after(Duration::from_millis(10)).await;
        self.write_word(EEPROM_UNLOCK, !EMISSIVITY_WORD).await?; // see App‑note
        Timer::after(Duration::from_millis(10)).await;

        Ok(())
    }

    // ───────────────────────────── SMBus helpers (no PEC) ──────────────────────────────
    async fn read_word(&mut self, cmd: u8) -> Result<u16, i2c::Error> {
        // write command byte, then repeated‑START + read 2 bytes
        let mut buf = [0u8; 3];
        self.i2c.blocking_write_read(MLX90614_ADDR, &[cmd], &mut buf)?;
        Ok(u16::from_le_bytes([buf[0], buf[1]]))
        }

    async fn write_word(&mut self, cmd: u8, data: u16) -> Result<(), i2c::Error> {
        let mut pkt = [0u8; 3];
        pkt[0] = cmd;
        pkt[1..].copy_from_slice(&data.to_le_bytes());
        self.i2c.blocking_write(MLX90614_ADDR, &pkt)
    }

    async fn simple_command(&mut self, cmd: u8) -> Result<(), i2c::Error> {
        self.i2c.blocking_write(MLX90614_ADDR, &[cmd])
    }
}
