use defmt::{info, warn};
use embassy_rp::gpio::Input;
use embassy_time::{Duration, Instant, Timer};

use crate::state::{
    FaultCode, Measurements, COIL_TEMP_LIMIT_C, CURRENT_LIMIT_A, FAULT_STATE, MEASUREMENTS,
    MODULE_TEMP_LIMIT_C, PCB_TEMP_LIMIT_C, POWER_LIMIT_KW,
};

const POWER_OVERSHOOT_MARGIN: f32 = 1.05;
const EARLY_WARNING_MARGIN_C: f32 = 5.0;
const WATCHDOG_LOG_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
struct SafetyReport {
    code: FaultCode,
    snapshot: Measurements,
}

#[embassy_executor::task]
pub async fn safety_task(
    interlock: &'static mut Input<'static>,
    gate_fault: &'static mut Input<'static>,
    gate_ready: &'static mut Input<'static>,
) {
    let mut next_watchdog_log = Instant::now();

    loop {
        let report = evaluate_fault(interlock, gate_fault, gate_ready).await;
        let code = report.code;

        let mut fault = FAULT_STATE.lock().await;
        if fault.code != code {
            if code == FaultCode::None {
                if fault.code != FaultCode::None {
                    info!(
                        "Fault cleared: {} (coil={}C{} module={}C pcb={}C power={}kW)",
                        fault.code.message(),
                        report.snapshot.coil_temp_c,
                        if report.snapshot.coil_temp_disconnected {
                            " disc"
                        } else {
                            ""
                        },
                        report.snapshot.module_temp_c,
                        report.snapshot.pcb_temp_c,
                        report.snapshot.coil_power_kw,
                    );
                } else {
                    info!("Fault state reset");
                }
            } else {
                warn!(
                    "Fault detected: {} (coil={}C{} module={}C pcb={}C power={}kW current={}A)",
                    code.message(),
                    report.snapshot.coil_temp_c,
                    if report.snapshot.coil_temp_disconnected {
                        " disc"
                    } else {
                        ""
                    },
                    report.snapshot.module_temp_c,
                    report.snapshot.pcb_temp_c,
                    report.snapshot.coil_power_kw,
                    report.snapshot.coil_current_rms_a,
                );
            }
            fault.code = code;
        }

        if Instant::now() >= next_watchdog_log && should_log_watchdog(&report.snapshot, code) {
            info!(
                "Safety watch: fault={} coil={}C{} module={}C pcb={}C power={}kW current={}A",
                code.message(),
                report.snapshot.coil_temp_c,
                if report.snapshot.coil_temp_disconnected {
                    " disc"
                } else {
                    ""
                },
                report.snapshot.module_temp_c,
                report.snapshot.pcb_temp_c,
                report.snapshot.coil_power_kw,
                report.snapshot.coil_current_rms_a,
            );
            next_watchdog_log = Instant::now() + WATCHDOG_LOG_INTERVAL;
        }

        Timer::after(Duration::from_millis(25)).await;
    }
}

pub async fn clear_fault() {
    let mut fault = FAULT_STATE.lock().await;
    fault.code = FaultCode::None;
}

pub async fn current_fault() -> FaultCode {
    FAULT_STATE.lock().await.code
}

async fn evaluate_fault(
    interlock: &Input<'static>,
    gate_fault: &Input<'static>,
    gate_ready: &Input<'static>,
) -> SafetyReport {
    let mut code = check_gpio_faults(interlock, gate_fault, gate_ready);
    let meas = *MEASUREMENTS.lock().await;

    if code == FaultCode::None {
        code = detect_measurement_fault(&meas);
    }

    SafetyReport {
        code,
        snapshot: meas,
    }
}

fn check_gpio_faults(
    interlock: &Input<'static>,
    gate_fault: &Input<'static>,
    gate_ready: &Input<'static>,
) -> FaultCode {
    if interlock.is_low() {
        return FaultCode::InterlockOpen;
    }
    if gate_fault.is_low() {
        return FaultCode::GateDriverFault;
    }
    if gate_ready.is_low() {
        return FaultCode::GateDriverNotReady;
    }
    FaultCode::None
}

fn detect_measurement_fault(meas: &Measurements) -> FaultCode {
    if meas.coil_temp_disconnected {
        return FaultCode::SensorFault;
    }

    if meas.coil_temp_c > COIL_TEMP_LIMIT_C {
        return FaultCode::CoilOverTemp;
    }
    if meas.module_temp_c > MODULE_TEMP_LIMIT_C {
        return FaultCode::ModuleOverTemp;
    }
    if meas.pcb_temp_c > PCB_TEMP_LIMIT_C {
        return FaultCode::PcbOverTemp;
    }

    if meas.valid {
        if meas.coil_power_kw > POWER_LIMIT_KW * POWER_OVERSHOOT_MARGIN {
            return FaultCode::PowerLimit;
        }
        if meas.coil_current_rms_a > CURRENT_LIMIT_A {
            return FaultCode::CurrentLimit;
        }
    }

    FaultCode::None
}

fn should_log_watchdog(meas: &Measurements, code: FaultCode) -> bool {
    if code != FaultCode::None {
        return true;
    }

    meas.coil_temp_disconnected
        || meas.coil_temp_c >= COIL_TEMP_LIMIT_C - EARLY_WARNING_MARGIN_C
        || meas.module_temp_c >= MODULE_TEMP_LIMIT_C - EARLY_WARNING_MARGIN_C
        || meas.pcb_temp_c >= PCB_TEMP_LIMIT_C - EARLY_WARNING_MARGIN_C
        || (meas.valid && meas.coil_power_kw >= POWER_LIMIT_KW * 0.9)
}
