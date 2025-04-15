use core::future::Future;
use embassy_executor::Spawner;
use embassy_rp::gpio::{Level, Output, Pin, Pull};
use embassy_rp::Peripherals;
use embassy_time::{Duration, Timer};
use {defmt_rtt as _, panic_probe as _}; // Example panicking/logging; adjust to your project.

///////////////////////////////////////////////////////////////////////////////
// LCD CONSTANTS & FLAGS (same as your C code)
///////////////////////////////////////////////////////////////////////////////

// Mode flags
const LCD_CHR: u8 = 1;  // Character mode
const LCD_CMD: u8 = 0;  // Command mode

// Command flags
const LCD_CLEAR: u8           = 0x01;
const LCD_CURSORSHIFT: u8     = 0x10;
const LCD_DISPLAYCONTROL: u8  = 0x08;
const LCD_HOME: u8            = 0x02;
const LCD_SETDDRAMADDR: u8    = 0x80;
const LCD_SETCGRAMADDR: u8    = 0x40;

// Control flags
const LCD_DISPLAYON: u8  = 0x04;
const LCD_DISPLAYOFF: u8 = 0x00;
const LCD_CURSORON: u8   = 0x02;
const LCD_CURSOROFF: u8  = 0x00;
const LCD_BLINKON: u8    = 0x01;
const LCD_BLINKOFF: u8   = 0x00;

// Move flags
const LCD_DISPLAYMOVE: u8 = 0x08;
const LCD_MOVELEFT: u8    = 0x00;
const LCD_MOVERIGHT: u8   = 0x04;

// Timing constants
// Adjust as needed for your particular LCD or microsecond constraints.
const E_PULSE_US: u32 = 500;  // 500us
const E_DELAY_US: u32 = 500;  // 500us
const HOMEDELAY_MS: u64 = 50; // 50ms

///////////////////////////////////////////////////////////////////////////////
// LCD Driver
///////////////////////////////////////////////////////////////////////////////
pub struct Lcd<'a> {
    rs: Output<'a>,
    en: Output<'a>,
    bl: Option<Output<'a>>,
    d4: Output<'a>,
    d5: Output<'a>,
    d6: Output<'a>,
    d7: Output<'a>,

    rows: u8,
    cols: u8,

    // Holds the current display-control flags: display on/off, cursor on/off, blink on/off.
    display_control: u8,
}

impl<'a> Lcd<'a> {
    /// Creates a new `Lcd` struct with uninitialized pins.
    ///
    /// * `rs_pin` – Register Select pin
    /// * `en_pin` – Enable pin
    /// * `backlight_pin` – Optional backlight pin
    /// * `d4_pin`, `d5_pin`, `d6_pin`, `d7_pin` – 4 data pins
    /// * `cols` – Number of columns
    /// * `rows` – Number of rows
    pub fn new(
        rs_pin: Output<'a>,
        en_pin: Output<'a>,
        backlight_pin: Option<Output<'a>>,
        d4_pin: Output<'a>,
        d5_pin: Output<'a>,
        d6_pin: Output<'a>,
        d7_pin: Output<'a>,
        cols: u8,
        rows: u8,
    ) -> Self {
        Self {
            rs: rs_pin,
            en: en_pin,
            bl: backlight_pin,
            d4: d4_pin,
            d5: d5_pin,
            d6: d6_pin,
            d7: d7_pin,
            rows,
            cols,
            display_control: LCD_DISPLAYON | LCD_CURSOROFF | LCD_BLINKOFF,
        }
    }

    /// Initializes the LCD in 4-bit mode and clears it.
    pub async fn init(&mut self) {
        // Following the standard HD44780 4-bit init procedure:
        self.write_byte(0x33, LCD_CMD).await; // Initialize
        self.write_byte(0x32, LCD_CMD).await; // Set to 4-bit mode
        self.write_byte(0x28, LCD_CMD).await; // 2 line, 5x8 font
        self.write_byte(0x0C, LCD_CMD).await; // Turn on display, cursor off, no blink
        self.write_byte(0x06, LCD_CMD).await; // Left to right entry
        self.clear().await;
        // Store initial display_control flags
        self.display_control = LCD_DISPLAYON | LCD_CURSOROFF | LCD_BLINKOFF;
    }

    /// Clears display and moves cursor to home position.
    pub async fn clear(&mut self) {
        self.write_byte(LCD_CLEAR, LCD_CMD).await;
        Timer::after(Duration::from_millis(HOMEDELAY_MS)).await;
    }

    /// Returns cursor to home position (without clearing).
    pub async fn home(&mut self) {
        self.write_byte(LCD_HOME, LCD_CMD).await;
        Timer::after(Duration::from_millis(HOMEDELAY_MS)).await;
    }

    /// Write a string to the LCD.
    pub async fn message(&mut self, text: &str) {
        for byte in text.as_bytes() {
            self.write_byte(*byte, LCD_CHR).await;
        }
    }

    /// Move display left by one position.
    pub async fn move_left(&mut self) {
        self.write_byte(LCD_CURSORSHIFT | LCD_DISPLAYMOVE | LCD_MOVELEFT, LCD_CMD)
            .await;
    }

    /// Move display right by one position.
    pub async fn move_right(&mut self) {
        self.write_byte(LCD_CURSORSHIFT | LCD_DISPLAYMOVE | LCD_MOVERIGHT, LCD_CMD)
            .await;
    }

    /// Sets the cursor to an explicit (x,y) position, zero-based.
    pub async fn set_cursor(&mut self, x: u8, y: u8) {
        // Ensure row is clamped to number of rows
        let row = if y >= self.rows { self.rows - 1 } else { y };

        // Determine row offset
        let row_offset = match row {
            0 => 0x00, // For 16x2 or 20x4, etc. The standard "Line 1" offset
            1 => 0x40, // "Line 2"
            2 => 0x14, // "Line 3"
            3 => 0x54, // "Line 4"
            _ => 0x00,
        };

        self.write_byte(LCD_SETDDRAMADDR | (x + row_offset), LCD_CMD).await;
    }

    /// Enables or disables the backlight (if present).
    pub fn backlight(&mut self, enable: bool) {
        if let Some(ref mut bl_pin) = self.bl {
            bl_pin.set_level(if enable { Level::High } else { Level::Low });
        }
    }

    /// Enables or disables the LCD display (but doesn’t power it off).
    pub async fn display_enable(&mut self, on: bool) {
        if on {
            self.display_control |= LCD_DISPLAYON;
        } else {
            self.display_control &= !LCD_DISPLAYON;
        }
        self.write_byte(LCD_DISPLAYCONTROL | self.display_control, LCD_CMD).await;
    }

    /// Enable or disable underline cursor.
    pub async fn show_underline(&mut self, show: bool) {
        if show {
            self.display_control |= LCD_CURSORON;
        } else {
            self.display_control &= !LCD_CURSORON;
        }
        self.write_byte(LCD_DISPLAYCONTROL | self.display_control, LCD_CMD).await;
    }

    /// Enable or disable blinking cursor.
    pub async fn show_blink(&mut self, show: bool) {
        if show {
            self.display_control |= LCD_BLINKON;
        } else {
            self.display_control &= !LCD_BLINKON;
        }
        self.write_byte(LCD_DISPLAYCONTROL | self.display_control, LCD_CMD).await;
    }

    /// Create a custom character (stored in CGRAM) at `location` (0-7).
    /// `pattern` must be 8 bytes (5x8 pixels, but each row in a single byte).
    pub async fn create_char(&mut self, location: u8, pattern: &[u8]) {
        if location > 7 {
            return; // Only 8 custom chars allowed
        }
        self.write_byte(LCD_SETCGRAMADDR | (location << 3), LCD_CMD).await;
        for row in pattern {
            self.write_byte(*row, LCD_CHR).await;
        }
    }

    /// Write a single byte (command or data) to the LCD in 4-bit mode.
    async fn write_byte(&mut self, bits: u8, mode: u8) {
        // Set RS line for command or data
        self.rs.set_level(if mode == LCD_CHR {
            Level::High
        } else {
            Level::Low
        });

        // A short delay after RS changes
        Timer::after(Duration::from_micros(E_DELAY_US.into())).await;

        // High nibble
        let high_nibble = (bits & 0xF0) >> 4;
        self.set_data_pins(high_nibble);
        self.toggle_enable().await;

        // Low nibble
        let low_nibble = bits & 0x0F;
        self.set_data_pins(low_nibble);
        self.toggle_enable().await;
    }

    /// Set D4..D7 pins according to the nibble (lower 4 bits).
    fn set_data_pins(&mut self, nibble: u8) {
        self.d4.set_level(if (nibble & 0x01) != 0 {
            Level::High
        } else {
            Level::Low
        });
        self.d5.set_level(if (nibble & 0x02) != 0 {
            Level::High
        } else {
            Level::Low
        });
        self.d6.set_level(if (nibble & 0x04) != 0 {
            Level::High
        } else {
            Level::Low
        });
        self.d7.set_level(if (nibble & 0x08) != 0 {
            Level::High
        } else {
            Level::Low
        });
    }

    /// Toggle the EN (enable) pin to latch command/data.
    async fn toggle_enable(&mut self) {
        // Pulse EN pin high
        self.en.set_high();
        Timer::after(Duration::from_micros(E_PULSE_US.into())).await;
        self.en.set_low();
        Timer::after(Duration::from_micros(E_DELAY_US.into())).await;
    }
}
