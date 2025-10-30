use defmt::{info, warn};
use embassy_rp::gpio::{Input, Output};
use embassy_rp::pwm::Pwm;
use embassy_time::{Duration, Instant, Timer};

use crate::{
    safety::current_fault,
    state::{ControlMode, CONTROL_SETTINGS, CONTROL_STATUS, MEASUREMENTS, POWER_LIMIT_KW},
    utils::{pwm_disable, pwm_enable},
};

const DEADTIME_NS: u32 = 512;
const BASE_FREQUENCY_HZ: f32 = 29_700.0;
const MIN_FREQUENCY_HZ: f32 = 26_000.0;
const MAX_FREQUENCY_HZ: f32 = 32_000.0;
const CONTROL_PERIOD: Duration = Duration::from_millis(10);
const CONTROL_DT_S: f32 = 0.010;
const RUN_DEBOUNCE: Duration = Duration::from_millis(80);
const TARGET_TOLERANCE_C: f32 = 2.0;

#[embassy_executor::task]
pub async fn control_task(
    pwm: &'static mut Pwm<'static>,
    hs_enable: &'static mut Output<'static>,
    ls_enable: &'static mut Output<'static>,
    solenoid: &'static mut Output<'static>,
    run_button: &'static mut Input<'static>,
) {
    let mut power_ctrl = PowerController::new(BASE_FREQUENCY_HZ);
    let mut temp_ctrl = TemperatureController::new();
    let mut run_active = false;
    let mut last_button_low = false;
    let mut last_toggle = Instant::now() - RUN_DEBOUNCE;
    let mut pwm_running = false;
    let mut last_mode = ControlMode::Idle;

    ls_enable.set_low();
    hs_enable.set_low();
    solenoid.set_low();
    pwm_disable(pwm);

    loop {
        let settings = *CONTROL_SETTINGS.lock().await;
        let mode = settings.mode;
        let fault = current_fault().await;

        if mode != last_mode {
            power_ctrl.reset(BASE_FREQUENCY_HZ);
            temp_ctrl.reset();
            run_active = false;
            pwm_running = false;
            pwm_disable(pwm);
            last_mode = mode;
        }

        let button_low = run_button.is_low();
        if button_low != last_button_low {
            if button_low && Instant::now().saturating_duration_since(last_toggle) >= RUN_DEBOUNCE {
                if matches!(mode, ControlMode::ManualPower | ControlMode::Temperature) {
                    run_active = !run_active;
                    info!("Run button toggled -> {}", run_active);
                }
                last_toggle = Instant::now();
            }
            last_button_low = button_low;
        }

        if fault != crate::state::FaultCode::None
            || !matches!(mode, ControlMode::ManualPower | ControlMode::Temperature)
        {
            if run_active {
                warn!("Run cancelled due to fault or mode change");
            }
            run_active = false;
        }

        let mut power_setpoint = 0.0f32;
        let mut heating = false;
        let mut switching_freq = 0.0f32;
        let mut target_reached = false;

        match mode {
            ControlMode::Cooldown => {
                solenoid.set_high();
                pwm_running = false;
                pwm_disable(pwm);
                run_active = false;
                ls_enable.set_low();
                hs_enable.set_low();
            }
            ControlMode::ManualPower | ControlMode::Temperature => {
                solenoid.set_low();

                let meas = MEASUREMENTS.lock().await;
                let measured_power = meas.coil_power_kw;
                let object_temp = meas.object_temp_c;
                drop(meas);

                if run_active && fault == crate::state::FaultCode::None {
                    heating = true;
                } else {
                    heating = false;
                }

                if mode == ControlMode::ManualPower {
                    power_setpoint = settings.manual_power_kw.clamp(0.0, POWER_LIMIT_KW);
                } else {
                    target_reached = object_temp >= settings.target_temp_c - TARGET_TOLERANCE_C;
                    power_setpoint = temp_ctrl
                        .update(settings.target_temp_c, object_temp, CONTROL_DT_S)
                        .clamp(0.0, POWER_LIMIT_KW);
                }

                if heating {
                    switching_freq =
                        power_ctrl.update(power_setpoint, measured_power, CONTROL_DT_S);
                    pwm_enable(pwm, DEADTIME_NS, switching_freq as u32);
                    pwm_running = true;
                    ls_enable.set_high();
                    hs_enable.set_high();
                } else {
                    if pwm_running {
                        pwm_disable(pwm);
                        pwm_running = false;
                    }
                    ls_enable.set_low();
                    hs_enable.set_low();
                }
                switching_freq = power_ctrl.freq_hz;
            }
            ControlMode::Idle => {
                solenoid.set_low();
                pwm_running = false;
                pwm_disable(pwm);
                run_active = false;
                ls_enable.set_low();
                hs_enable.set_low();
            }
        }

        {
            let mut status = CONTROL_STATUS.lock().await;
            status.mode = mode;
            status.heating_enabled = heating && pwm_running;
            status.run_active = run_active;
            status.target_reached = target_reached;
            status.cooldown_active = mode == ControlMode::Cooldown;
            status.power_setpoint_kw = power_setpoint;
            status.switching_freq_hz = switching_freq;
            status.fault = fault;
        }

        Timer::after(CONTROL_PERIOD).await;
    }
}

struct PowerController {
    freq_hz: f32,
    integrator: f32,
}

impl PowerController {
    fn new(initial_freq: f32) -> Self {
        Self {
            freq_hz: initial_freq,
            integrator: 0.0,
        }
    }

    fn reset(&mut self, initial_freq: f32) {
        self.freq_hz = initial_freq;
        self.integrator = 0.0;
    }

    fn update(&mut self, setpoint_kw: f32, measured_kw: f32, dt: f32) -> f32 {
        const KP: f32 = 60.0;
        const KI: f32 = 8.0;
        let error = setpoint_kw - measured_kw;
        self.integrator = (self.integrator + error * KI * dt).clamp(-2000.0, 2000.0);
        self.freq_hz =
            (self.freq_hz + KP * error + self.integrator).clamp(MIN_FREQUENCY_HZ, MAX_FREQUENCY_HZ);
        self.freq_hz
    }
}

struct TemperatureController {
    integrator: f32,
}

impl TemperatureController {
    fn new() -> Self {
        Self { integrator: 0.0 }
    }

    fn reset(&mut self) {
        self.integrator = 0.0;
    }

    fn update(&mut self, target_c: f32, measured_c: f32, dt: f32) -> f32 {
        const KP: f32 = 0.08;
        const KI: f32 = 0.03;
        let error = (target_c - measured_c).max(-20.0);
        self.integrator = (self.integrator + error * KI * dt).clamp(0.0, POWER_LIMIT_KW);
        (KP * error + self.integrator).clamp(0.0, POWER_LIMIT_KW)
    }
}
