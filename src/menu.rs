use defmt::Str;
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_time::{Duration, Instant, Timer};
use heapless::{String, Vec};
use core::sync::atomic::{AtomicU16, Ordering};
use core::fmt::Write;

use {defmt_rtt as _, panic_probe as _};

use crate::lcd as lcd_driver;
// ---------------------------------------------------------------------
// Menu states
// ---------------------------------------------------------------------
#[derive(Debug, Clone, Copy)]
enum MenuState {
    Welcome,
    ManualFreqTime,
    ManualPowerTime,
    PresetSelect,
    Running,
    Cooldown,
}

struct MenuSelections {
    time_ms: u32,       // e.g., in increments of 100ms
    freq_khz: u32,      // e.g., in kHz
    power_w: u32,       // e.g., in 100W increments
    tool_diameter_inch: f32,
}

#[embassy_executor::task]
pub async fn menu_task(
    mut lcd: lcd_driver::Lcd<'static>,
    up: Input<'static>,
    down: Input<'static>,
    enter: Input<'static>,
    run_pin: Input<'static>,
    estop: Input<'static>,
){
    // Initialize the LCD display in some manner
    lcd.backlight(true);
    lcd.clear().await;
    lcd.home().await;

    // Our current state
    let mut state = MenuState::Welcome;

    // The user’s chosen parameters
    let mut selections = MenuSelections {
        time_ms: 1000,       // default 1 second
        freq_khz: 50,        // default 50 kHz
        power_w: 1000,       // default 1kW
        tool_diameter_inch: 0.5,
    };

    // Main loop
    loop {
        match state {
            MenuState::Welcome => {
                display_welcome(&mut lcd).await;
                // Wait for Enter to proceed (just an example)
                wait_for_button_press(&enter).await;
                state = MenuState::ManualFreqTime;
            }

            MenuState::ManualFreqTime => {
                manual_freq_time_menu(&mut lcd, &up, &down, &enter, &mut selections).await;
                // Next step
                state = MenuState::ManualPowerTime;
            }

            MenuState::ManualPowerTime => {
                manual_power_time_menu(&mut lcd, &up, &down, &enter, &mut selections).await;
                // Next step
                state = MenuState::PresetSelect;
            }

            MenuState::PresetSelect => {
                preset_select_menu(&mut lcd, &up, &down, &enter, &mut selections).await;
                // Next step
                state = MenuState::Running;
            }

            MenuState::Running => {
                // If E-stop is active, skip running
                if estop.is_low() {
                    // Show E-stop message
                    lcd.clear().await;
                    lcd.set_cursor(0, 0).await;
                    lcd.message("E-STOP ACTIVE!").await;
                    Timer::after(Duration::from_secs(2)).await;
                } else {
                    run_coil_sequence(&mut lcd, &run_pin, &selections).await;
                }
                // Next step
                state = MenuState::Cooldown;
            }

            MenuState::Cooldown => {
                cooldown_sequence(&mut lcd).await;
                // Once cooldown is done, go back to welcome or end?
                state = MenuState::Welcome;
            }
        }
    }
}

// ---------------------------------------------------------------------
// Sample submenus
// ---------------------------------------------------------------------
async fn display_welcome(lcd: &mut lcd_driver::Lcd<'static>) {
    lcd.clear().await;
    lcd.set_cursor(0, 0).await;
    lcd.message("Shrink Fit v1.0").await;
    lcd.set_cursor(0, 1).await;
    lcd.message("Press Enter...").await;
}

async fn manual_freq_time_menu(
    lcd: &mut lcd_driver::Lcd<'_>,
    up: &Input<'_>,
    down: &Input<'_>,
    enter: &Input<'_>,
    selections: &mut MenuSelections,
) {
    // Adjust time (increments of 100ms) and freq (kHz)
    loop {
        // Show current settings
        lcd.clear().await;
        lcd.set_cursor(0, 0).await;
        let mut msg: String<16> = String::new();
        write!(msg, "Time: {}ms", selections.time_ms);
        lcd.message(&msg).await;

        lcd.set_cursor(0, 1).await;
        let mut msg2: String<16> = String::new();
        write!(&mut msg2, "Freq: {}kHz", selections.freq_khz);
        lcd.message(&msg2).await;

        // Wait for a button press
        match wait_for_any_button(up, down, enter).await {
            ButtonPressed::Up => {
                // example: increment freq
                selections.freq_khz += 1;
            }
            ButtonPressed::Down => {
                // example: decrement freq (not below 1kHz)
                if selections.freq_khz > 1 {
                    selections.freq_khz -= 1;
                }
            }
            ButtonPressed::Enter => {
                // Move to adjusting time
                adjust_time(lcd, selections, up, down, enter).await;
                // After adjusting time, exit the loop
                break;
            }
            _ => {}
        }
    }
}

async fn manual_power_time_menu(
    lcd: &mut lcd_driver::Lcd<'_>,
    up: &Input<'_>,
    down: &Input<'_>,
    enter: &Input<'_>,
    selections: &mut MenuSelections,
) {
    // Adjust time (increments of 100ms) and power (in 100W increments)
    loop {
        lcd.clear().await;
        lcd.set_cursor(0, 0).await;
        let mut msg: String<16> = String::new();
        write!(msg, "Time: {}ms", selections.time_ms);
        lcd.message(&msg).await;

        lcd.set_cursor(0, 1).await;
        let mut msg2: String<16> = String::new();
        write!(msg2, "Power: {}W", selections.power_w);
        lcd.message(&msg2).await;

        match wait_for_any_button(up, down, enter).await {
            ButtonPressed::Up => {
                // increment power
                selections.power_w += 100;
            }
            ButtonPressed::Down => {
                // decrement power (not below 100W)
                if selections.power_w >= 100 {
                    selections.power_w -= 100;
                }
            }
            ButtonPressed::Enter => {
                // Move to adjusting time
                adjust_time(lcd, selections, up, down, enter).await;
                break;
            }
            _ => {}
        }
    }
}

// Submenu for selecting a tool preset
async fn preset_select_menu(
    lcd: &mut lcd_driver::Lcd<'_>,
    up: &Input<'_>,
    down: &Input<'_>,
    enter: &Input<'_>,
    selections: &mut MenuSelections,
) {
    // Just 2 presets: 0.5in or 0.75in
    let mut idx = 0; // 0 => 0.5in, 1 => 0.75in

    loop {
        lcd.clear().await;
        lcd.set_cursor(0, 0).await;
        lcd.message("Select Tool").await;

        lcd.set_cursor(0, 1).await;
        if idx == 0 {
            lcd.message("0.5in").await;
        } else {
            lcd.message("0.75in").await;
        }

        match wait_for_any_button(up, down, enter).await {
            ButtonPressed::Up => {
                idx = 0;
            }
            ButtonPressed::Down => {
                idx = 1;
            }
            ButtonPressed::Enter => {
                selections.tool_diameter_inch = if idx == 0 { 0.5 } else { 0.75 };
                break;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------
// Running coil subroutine
// ---------------------------------------------------------------------
async fn run_coil_sequence(lcd: &mut lcd_driver::Lcd<'_>, run_pin: &Input<'_>, sel: &MenuSelections) {
    // Wait for user to press RUN
    lcd.clear().await;
    lcd.set_cursor(0, 0).await;
    lcd.message("Press RUN...").await;
    lcd.set_cursor(0, 1).await;
    lcd.message("to start coil").await;

    wait_for_button_press(run_pin).await;

    // Show "Running" screen
    lcd.clear().await;
    lcd.set_cursor(0, 0).await;
    lcd.message("Coil Active!").await;

    // Example usage: user might pick either frequency-based or power-based run
    // or we run both in some logic. For demonstration:
    // run_coil_freq(sel.time_ms, sel.freq_khz);
    // or: run_coil_power(sel.time_ms, sel.power_w);
    // or: run_coil_preset(sel.tool_diameter_inch);

    // While coil runs, we show the time left
    // We'll do a simple countdown from `time_ms`. In a real design,
    // your coil run might be non-blocking. For demonstration, we just do a naive approach:
    let start = Instant::now();
    let total = Duration::from_millis(sel.time_ms as u64);

    loop {
        let elapsed = Instant::now() - start;
        if elapsed >= total {
            break;
        }
        let remain = total - elapsed;
        let remain_ms = remain.as_millis() as u32;

        lcd.set_cursor(0, 1).await;
        let mut msg: String<16> = String::new();
        write!(msg, "Time left: {}ms", remain_ms);
        lcd.message(&msg).await;

        Timer::after(Duration::from_millis(100)).await;
    }

    // Once done, coil is off. Show "Done!"
    lcd.clear().await;
    lcd.set_cursor(0, 0).await;
    lcd.message("Coil run done.").await;
    Timer::after(Duration::from_secs(2)).await;
}

// ---------------------------------------------------------------------
// Cooldown state
// ---------------------------------------------------------------------
async fn cooldown_sequence(lcd: &mut lcd_driver::Lcd<'_>) {
    // Wait until temperature is below some threshold
    lcd.clear().await;
    lcd.set_cursor(0, 0).await;
    lcd.message("Cooling down...").await;

    loop {
        // let temp = TEMPERATURE_C.load(Ordering::Relaxed);
        let temp = 25;
        lcd.set_cursor(0, 1).await;
        let mut msg: String<16> = String::new();
        write!(msg, "Temp: {}C", temp);
        lcd.message(&msg).await;

        if temp <= 40 {
            // Suppose "safe to touch" = 40°C
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    lcd.clear().await;
    lcd.set_cursor(0, 0).await;
    lcd.message("Safe to remove").await;
    Timer::after(Duration::from_secs(2)).await;
}

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

// Example: adjust time in 100ms increments
async fn adjust_time(
    lcd: &mut lcd_driver::Lcd<'_>,
    selections: &mut MenuSelections,
    up: &Input<'_>,
    down: &Input<'_>,
    enter: &Input<'_>,
) {
    loop {
        lcd.clear().await;
        lcd.set_cursor(0, 0).await;
        lcd.message("Adjust Time").await;

        lcd.set_cursor(0, 1).await;
        let mut msg: String<16> = String::new();
        write!(msg, "{}ms (+/-)", selections.time_ms);
        lcd.message(&msg).await;

        match wait_for_any_button(up, down, enter).await {
            ButtonPressed::Up => {
                selections.time_ms += 100;
            }
            ButtonPressed::Down => {
                if selections.time_ms >= 100 {
                    selections.time_ms -= 100;
                }
            }
            ButtonPressed::Enter => {
                break;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------
// Reading button presses
// ---------------------------------------------------------------------
#[derive(Debug)]
enum ButtonPressed {
    Up,
    Down,
    Enter,
    Run,
    None,
}

/// Blocks until one of the 3 menu buttons is pressed.
async fn wait_for_any_button(
    up: &Input<'_>,
    down: &Input<'_>,
    enter: &Input<'_>,
) -> ButtonPressed {
    loop {
        if up.is_low() {
            // naive debouncing
            Timer::after(Duration::from_millis(20)).await;
            if up.is_low() {
                return ButtonPressed::Up;
            }
        }
        if down.is_low() {
            Timer::after(Duration::from_millis(20)).await;
            if down.is_low() {
                return ButtonPressed::Down;
            }
        }
        if enter.is_low() {
            Timer::after(Duration::from_millis(20)).await;
            if enter.is_low() {
                return ButtonPressed::Enter;
            }
        }
        Timer::after(Duration::from_millis(10)).await;
    }
}

/// Blocks until a specific button is pressed.
async fn wait_for_button_press(pin: &Input<'_>) {
    loop {
        if pin.is_low() {
            Timer::after(Duration::from_millis(20)).await;
            if pin.is_low() {
                // wait until release
                while pin.is_low() {
                    Timer::after(Duration::from_millis(10)).await;
                }
                break;
            }
        }
        Timer::after(Duration::from_millis(10)).await;
    }
}
