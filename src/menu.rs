use core::fmt::Write;
use core::pin::pin;

use embassy_futures::select::{select3, Either3};
use embassy_rp::gpio::Input;
use embassy_time::{Duration, Timer};
use heapless::String;

use crate::{
    lcd::Lcd,
    safety::current_fault,
    state::{
        ControlMode, FaultCode, Measurements, COIL_TEMP_LIMIT_C, CONTROL_SETTINGS, CONTROL_STATUS,
        CURRENT_LIMIT_A, MEASUREMENTS, MODULE_TEMP_LIMIT_C, PCB_TEMP_LIMIT_C, POWER_LIMIT_KW,
    },
};

const MANUAL_STEP_KW: f32 = 0.5;
const TEMP_STEP_C: f32 = 10.0;
const TEMP_MIN_C: f32 = 40.0;
const TEMP_MAX_C: f32 = 350.0;
const STATUS_REFRESH_MS: u64 = 50;

#[embassy_executor::task]
pub async fn menu_task(
    mut lcd: Lcd<'static>,
    mut up: Input<'static>,
    mut down: Input<'static>,
    mut enter: Input<'static>,
) {
    lcd.backlight(true);
    lcd.clear().await;
    lcd.home().await;

    let mut screen = Screen::ModeSelect;
    let mut selected_mode = ControlMode::ManualPower;

    loop {
        if let FaultCode::None = current_fault().await {
        } else {
            screen = fault_screen(&mut lcd).await;
            continue;
        }

        screen = match screen {
            Screen::ModeSelect => {
                set_mode(ControlMode::Idle).await;
                mode_select_screen(&mut lcd, &mut up, &mut down, &mut enter, selected_mode).await
            }
            Screen::ManualConfig => {
                selected_mode = ControlMode::ManualPower;
                set_mode(ControlMode::ManualPower).await;
                manual_config_screen(&mut lcd, &mut up, &mut down, &mut enter).await
            }
            Screen::ManualStatus => {
                selected_mode = ControlMode::ManualPower;
                set_mode(ControlMode::ManualPower).await;
                manual_status_screen(&mut lcd, &mut up, &mut down, &mut enter).await
            }
            Screen::TemperatureConfig => {
                selected_mode = ControlMode::Temperature;
                set_mode(ControlMode::Temperature).await;
                temperature_config_screen(&mut lcd, &mut up, &mut down, &mut enter).await
            }
            Screen::TemperatureStatus => {
                selected_mode = ControlMode::Temperature;
                set_mode(ControlMode::Temperature).await;
                temperature_status_screen(&mut lcd, &mut up, &mut down, &mut enter).await
            }
            Screen::Cooldown => {
                set_mode(ControlMode::Cooldown).await;
                cooldown_screen(&mut lcd, &mut up, &mut down, &mut enter).await
            }
        };
    }
}

#[derive(Clone, Copy)]
enum Screen {
    ModeSelect,
    ManualConfig,
    ManualStatus,
    TemperatureConfig,
    TemperatureStatus,
    Cooldown,
}

async fn mode_select_screen(
    lcd: &mut Lcd<'static>,
    up: &mut Input<'static>,
    down: &mut Input<'static>,
    enter: &mut Input<'static>,
    current_mode: ControlMode,
) -> Screen {
    let mut index = if current_mode == ControlMode::Temperature {
        1
    } else {
        0
    };
    loop {
        lcd.clear().await;
        display_line(
            lcd,
            0,
            if index == 0 {
                "> Manual Power"
            } else {
                "  Manual Power"
            },
        )
        .await;
        display_line(
            lcd,
            1,
            if index == 1 {
                "> Temperature "
            } else {
                "  Temperature "
            },
        )
        .await;

        match wait_for_press(up, down, enter).await {
            ButtonPressed::Up => {
                index = (index + 1) % 2;
            }
            ButtonPressed::Down => {
                index = (index + 1) % 2;
            }
            ButtonPressed::Enter => {
                return if index == 0 {
                    Screen::ManualConfig
                } else {
                    Screen::TemperatureConfig
                };
            }
        }
    }
}

async fn manual_config_screen(
    lcd: &mut Lcd<'static>,
    up: &mut Input<'static>,
    down: &mut Input<'static>,
    enter: &mut Input<'static>,
) -> Screen {
    lcd.clear().await;
    display_line(lcd, 0, "Manual power set").await;

    loop {
        let value = {
            let settings = CONTROL_SETTINGS.lock().await;
            settings.manual_power_kw
        };

        let mut line = String::<16>::new();
        write!(&mut line, "Target: {:>4.1}kW", value).ok();
        display_line(lcd, 1, line.as_str()).await;

        match wait_for_press(up, down, enter).await {
            ButtonPressed::Up => {
                let next = (value + MANUAL_STEP_KW).clamp(0.0, POWER_LIMIT_KW);
                set_manual_power(next).await;
            }
            ButtonPressed::Down => {
                let next = (value - MANUAL_STEP_KW).clamp(0.0, POWER_LIMIT_KW);
                set_manual_power(next).await;
            }
            ButtonPressed::Enter => {
                return Screen::ManualStatus;
            }
        }
    }
}

async fn manual_status_screen(
    lcd: &mut Lcd<'static>,
    up: &mut Input<'static>,
    down: &mut Input<'static>,
    enter: &mut Input<'static>,
) -> Screen {
    lcd.clear().await;
    loop {
        let status = CONTROL_STATUS.lock().await.clone();
        let meas = MEASUREMENTS.lock().await.clone();
        let v_display = meas.dc_voltage_v.clamp(0.0, 999.0);
        let i_display = meas.coil_current_rms_a.clamp(0.0, 999.0);

        let mut line1 = String::<16>::new();
        write!(
            &mut line1,
            "P {:>4.1}k T {:>4.1}k",
            meas.coil_power_kw, status.power_setpoint_kw
        )
        .ok();
        display_line(lcd, 0, line1.as_str()).await;

        let mut line2 = String::<16>::new();
        write!(
            &mut line2,
            "{} V{:>3.0} I{:>3.0}",
            if status.run_active { "R:ON" } else { "R:OFF" },
            v_display,
            i_display
        )
        .ok();
        display_line(lcd, 1, line2.as_str()).await;

        if enter.is_low() {
            wait_for_release(enter).await;
            return Screen::ModeSelect;
        }
        if up.is_low() {
            wait_for_release(up).await;
            return Screen::ManualConfig;
        }
        if down.is_low() {
            wait_for_release(down).await;
            return Screen::ModeSelect;
        }

        Timer::after(Duration::from_millis(STATUS_REFRESH_MS)).await;
    }
}

async fn temperature_config_screen(
    lcd: &mut Lcd<'static>,
    up: &mut Input<'static>,
    down: &mut Input<'static>,
    enter: &mut Input<'static>,
) -> Screen {
    lcd.clear().await;
    display_line(lcd, 0, "Target temperature").await;

    loop {
        let value = {
            let settings = CONTROL_SETTINGS.lock().await;
            settings.target_temp_c
        };

        let mut line = String::<16>::new();
        write!(&mut line, "Target: {:>4.0}C", value).ok();
        display_line(lcd, 1, line.as_str()).await;

        match wait_for_press(up, down, enter).await {
            ButtonPressed::Up => {
                let next = (value + TEMP_STEP_C).clamp(TEMP_MIN_C, TEMP_MAX_C);
                set_temperature_target(next).await;
            }
            ButtonPressed::Down => {
                let next = (value - TEMP_STEP_C).clamp(TEMP_MIN_C, TEMP_MAX_C);
                set_temperature_target(next).await;
            }
            ButtonPressed::Enter => {
                return Screen::TemperatureStatus;
            }
        }
    }
}

async fn temperature_status_screen(
    lcd: &mut Lcd<'static>,
    up: &mut Input<'static>,
    down: &mut Input<'static>,
    enter: &mut Input<'static>,
) -> Screen {
    lcd.clear().await;
    loop {
        let status = CONTROL_STATUS.lock().await.clone();
        let meas = MEASUREMENTS.lock().await.clone();
        let target_temp = CONTROL_SETTINGS.lock().await.target_temp_c;

        let mut line1 = String::<16>::new();
        write!(
            &mut line1,
            "Obj {:>4.0}C T {:>4.0}C",
            meas.object_temp_c, target_temp
        )
        .ok();
        display_line(lcd, 0, line1.as_str()).await;

        if status.target_reached {
            display_line(lcd, 1, "Press Enter Cool").await;
        } else {
            let mut line2 = String::<16>::new();
            write!(
                &mut line2,
                "Coil{:>3.0}C Mod{:>3.0}",
                meas.coil_temp_c, meas.module_temp_c
            )
            .ok();
            display_line(lcd, 1, line2.as_str()).await;
        }

        if enter.is_low() {
            wait_for_release(enter).await;

            if status.target_reached {
                return Screen::Cooldown;
            } else {
                return Screen::TemperatureConfig;
            }
        }
        if up.is_low() {
            wait_for_release(up).await;
            return Screen::TemperatureConfig;
        }
        if down.is_low() {
            wait_for_release(down).await;
            return Screen::ModeSelect;
        }

        Timer::after(Duration::from_millis(STATUS_REFRESH_MS)).await;
    }
}

async fn cooldown_screen(
    lcd: &mut Lcd<'static>,
    up: &mut Input<'static>,
    down: &mut Input<'static>,
    enter: &mut Input<'static>,
) -> Screen {
    lcd.clear().await;
    display_line(lcd, 0, "Cooling active").await;
    display_line(lcd, 1, "Enter to exit").await;

    loop {
        if enter.is_low() || up.is_low() || down.is_low() {
            wait_for_release(enter).await;
            wait_for_release(up).await;
            wait_for_release(down).await;
            set_mode(ControlMode::Idle).await;
            return Screen::ModeSelect;
        }

        Timer::after(Duration::from_millis(STATUS_REFRESH_MS)).await;
    }
}

async fn fault_screen(lcd: &mut Lcd<'static>) -> Screen {
    let mut last_code = FaultCode::None;
    let mut last_header = String::<16>::new();
    let mut last_detail = String::<16>::new();

    loop {
        let code = current_fault().await;
        if code == FaultCode::None {
            lcd.clear().await;
            display_line(lcd, 0, "Fault cleared").await;
            Timer::after(Duration::from_millis(400)).await;
            return Screen::ModeSelect;
        }

        let meas = MEASUREMENTS.lock().await.clone();
        let header = fault_header_line(code);
        let detail = fault_detail_line(code, &meas);

        if code != last_code {
            lcd.clear().await;
            last_code = code;
            last_header.clear();
            last_detail.clear();
        }

        if header != last_header {
            display_line(lcd, 0, header.as_str()).await;
            last_header = header;
        }

        if detail != last_detail {
            display_line(lcd, 1, detail.as_str()).await;
            last_detail = detail;
        }

        Timer::after(Duration::from_millis(200)).await;
    }
}

async fn set_manual_power(value: f32) {
    let mut settings = CONTROL_SETTINGS.lock().await;
    settings.manual_power_kw = value;
}

async fn set_temperature_target(value: f32) {
    let mut settings = CONTROL_SETTINGS.lock().await;
    settings.target_temp_c = value;
}

async fn set_mode(mode: ControlMode) {
    let mut settings = CONTROL_SETTINGS.lock().await;
    settings.mode = mode;
}

async fn display_line(lcd: &mut Lcd<'static>, row: u8, text: &str) {
    let formatted = fit_to_line(text);
    lcd.set_cursor(0, row).await;
    lcd.message(formatted.as_str()).await;
}

fn fit_to_line(text: &str) -> String<16> {
    let mut buf = String::<16>::new();
    for ch in text.chars().take(16) {
        buf.push(ch).ok();
    }
    while buf.len() < 16 {
        buf.push(' ').ok();
    }
    buf
}

fn fault_header_line(code: FaultCode) -> String<16> {
    fit_to_line(code.lcd_label())
}

fn fault_detail_line(code: FaultCode, meas: &Measurements) -> String<16> {
    match code {
        FaultCode::PowerLimit => power_detail_line(meas.coil_power_kw),
        FaultCode::CoilOverTemp => temp_detail_line("Coil ", meas.coil_temp_c, COIL_TEMP_LIMIT_C),
        FaultCode::ModuleOverTemp => {
            temp_detail_line("Mod ", meas.module_temp_c, MODULE_TEMP_LIMIT_C)
        }
        FaultCode::PcbOverTemp => temp_detail_line("PCB ", meas.pcb_temp_c, PCB_TEMP_LIMIT_C),
        FaultCode::CurrentLimit => current_detail_line(meas.coil_current_rms_a),
        FaultCode::InterlockOpen => fit_to_line("Check E-STOP"),
        FaultCode::GateDriverFault => fit_to_line("Gate drv fault"),
        FaultCode::GateDriverNotReady => fit_to_line("Gate drv wait"),
        FaultCode::SensorFault => fit_to_line("Coil NTC open"),
        FaultCode::None => fit_to_line("All clear"),
    }
}

fn temp_detail_line(label: &str, value: f32, limit: f32) -> String<16> {
    let mut buf = String::<16>::new();
    let _ = write!(buf, "{}{:>3.0}>{:.0}C", label, value, limit);
    fit_to_line(buf.as_str())
}

fn power_detail_line(power_kw: f32) -> String<16> {
    let mut buf = String::<16>::new();
    let _ = write!(buf, "P {:>4.1}>{:.0}kW", power_kw, POWER_LIMIT_KW);
    fit_to_line(buf.as_str())
}

fn current_detail_line(current_a: f32) -> String<16> {
    let mut buf = String::<16>::new();
    let _ = write!(buf, "I {:>3.0}>{:.0}A", current_a, CURRENT_LIMIT_A);
    fit_to_line(buf.as_str())
}

async fn wait_for_release(button: &mut Input<'static>) {
    while button.is_low() {
        Timer::after(Duration::from_millis(10)).await;
    }
}

#[derive(Debug)]
enum ButtonPressed {
    Up,
    Down,
    Enter,
}

async fn wait_for_press(
    up: &mut Input<'static>,
    down: &mut Input<'static>,
    enter: &mut Input<'static>,
) -> ButtonPressed {
    let pressed = {
        let mut up_wait = pin!(up.wait_for_falling_edge());
        let mut down_wait = pin!(down.wait_for_falling_edge());
        let mut enter_wait = pin!(enter.wait_for_falling_edge());

        select3(up_wait.as_mut(), down_wait.as_mut(), enter_wait.as_mut()).await
    };

    match pressed {
        Either3::First(_) => {
            debounce_and_release(up).await;
            ButtonPressed::Up
        }
        Either3::Second(_) => {
            debounce_and_release(down).await;
            ButtonPressed::Down
        }
        Either3::Third(_) => {
            debounce_and_release(enter).await;
            ButtonPressed::Enter
        }
    }
}

async fn debounce_and_release(button: &mut Input<'static>) {
    Timer::after(Duration::from_millis(20)).await;
    wait_for_release(button).await;
}
