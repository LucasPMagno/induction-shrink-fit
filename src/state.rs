use core::fmt;

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlMode {
    Idle,
    ManualPower,
    Temperature,
    Cooldown,
}

#[derive(Debug, Clone, Copy)]
pub struct ControlSettings {
    pub mode: ControlMode,
    pub manual_power_kw: f32,
    pub target_temp_c: f32,
}

impl ControlSettings {
    pub const fn new() -> Self {
        Self {
            mode: ControlMode::ManualPower,
            manual_power_kw: 5.0,
            target_temp_c: 120.0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ControlStatus {
    pub mode: ControlMode,
    pub heating_enabled: bool,
    pub run_active: bool,
    pub target_reached: bool,
    pub cooldown_active: bool,
    pub power_setpoint_kw: f32,
    pub switching_freq_hz: f32,
    pub fault: FaultCode,
}

impl ControlStatus {
    pub const fn new() -> Self {
        Self {
            mode: ControlMode::Idle,
            heating_enabled: false,
            run_active: false,
            target_reached: false,
            cooldown_active: false,
            power_setpoint_kw: 0.0,
            switching_freq_hz: 0.0,
            fault: FaultCode::None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Measurements {
    pub dc_voltage_v: f32,
    pub coil_current_rms_a: f32,
    pub coil_power_kw: f32,
    pub coil_temp_c: f32,
    pub pcb_temp_c: f32,
    pub module_temp_c: f32,
    pub object_temp_c: f32,
    pub valid: bool,
}

impl Measurements {
    pub const fn new() -> Self {
        Self {
            dc_voltage_v: 0.0,
            coil_current_rms_a: 0.0,
            coil_power_kw: 0.0,
            coil_temp_c: 0.0,
            pcb_temp_c: 0.0,
            module_temp_c: 0.0,
            object_temp_c: 0.0,
            valid: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultCode {
    None,
    PowerLimit,
    CoilOverTemp,
    ModuleOverTemp,
    PcbOverTemp,
    InterlockOpen,
    GateDriverFault,
    GateDriverNotReady,
    SensorFault,
    CurrentLimit,
}

impl FaultCode {
    pub const fn message(self) -> &'static str {
        match self {
            FaultCode::None => "OK",
            FaultCode::PowerLimit => "Power limit exceeded",
            FaultCode::CoilOverTemp => "Coil over-temp",
            FaultCode::ModuleOverTemp => "SiC module over-temp",
            FaultCode::PcbOverTemp => "PCB over-temp",
            FaultCode::InterlockOpen => "Interlock open",
            FaultCode::GateDriverFault => "Gate driver fault",
            FaultCode::GateDriverNotReady => "Gate driver not ready",
            FaultCode::SensorFault => "Sensor fault",
            FaultCode::CurrentLimit => "Current limit exceeded",
        }
    }
}

impl fmt::Display for FaultCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FaultState {
    pub code: FaultCode,
}

impl FaultState {
    pub const fn new() -> Self {
        Self {
            code: FaultCode::None,
        }
    }
}

pub const POWER_LIMIT_KW: f32 = 10.0;
pub const CURRENT_LIMIT_A: f32 = 150.0;
pub const COIL_TEMP_LIMIT_C: f32 = 80.0;
pub const MODULE_TEMP_LIMIT_C: f32 = 35.0;
pub const PCB_TEMP_LIMIT_C: f32 = 85.0;

pub static MEASUREMENTS: Mutex<CriticalSectionRawMutex, Measurements> =
    Mutex::new(Measurements::new());
pub static CONTROL_SETTINGS: Mutex<CriticalSectionRawMutex, ControlSettings> =
    Mutex::new(ControlSettings::new());
pub static CONTROL_STATUS: Mutex<CriticalSectionRawMutex, ControlStatus> =
    Mutex::new(ControlStatus::new());
pub static FAULT_STATE: Mutex<CriticalSectionRawMutex, FaultState> = Mutex::new(FaultState::new());
