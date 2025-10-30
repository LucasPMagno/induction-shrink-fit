use defmt::warn;
use embassy_rp::gpio::Input;
use embassy_time::{Duration, Timer};

use crate::state::{
    FaultCode, COIL_TEMP_LIMIT_C, FAULT_STATE, MEASUREMENTS, MODULE_TEMP_LIMIT_C, PCB_TEMP_LIMIT_C,
    POWER_LIMIT_KW,
};

const POWER_OVERSHOOT_MARGIN: f32 = 1.05;

#[embassy_executor::task]
pub async fn safety_task(
    interlock: &'static mut Input<'static>,
    gate_fault: &'static mut Input<'static>,
    gate_ready: &'static mut Input<'static>,
) {
    loop {
        let code = evaluate_fault(interlock, gate_fault, gate_ready).await;
        if code != FaultCode::None {
            let mut fault = FAULT_STATE.lock().await;
            if fault.code == FaultCode::None {
                warn!("Fault detected: {}", code.message());
                fault.code = code;
            }
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

    let meas = MEASUREMENTS.lock().await;

    if !meas.valid {
        return FaultCode::None;
    }

    if meas.coil_power_kw > POWER_LIMIT_KW * POWER_OVERSHOOT_MARGIN {
        return FaultCode::PowerLimit;
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

    FaultCode::None
}
