#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_hal_internal::Peripheral;
use embassy_rp::{
    adc::{Adc, Async, Channel, Config as AdcConfig, InterruptHandler},
    bind_interrupts,
    gpio::{Drive, Input, Level, Output, Pull},
    i2c::{Config as I2cConfig, I2c},
    pwm::{Config as PwmConfig, Pwm},
    Peripherals,
};
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;

use {defmt_rtt as _, panic_probe as _};

mod ads7828;
mod control;
mod lcd;
mod menu;
mod mlx90614;
mod safety;
mod sensors;
mod state;
mod utils;

use ads7828::Ads7828;
use control::control_task;
use lcd::Lcd;
use menu::menu_task;
use mlx90614::Mlx90614;
use safety::safety_task;
use sensors::{adc_task, ads_task, mlx_task, sic_temp_task};
use utils::pwm_disable;

static PWM_DRIVE_CELL: StaticCell<Pwm<'static>> = StaticCell::new();
static HS_ENABLE_CELL: StaticCell<Output<'static>> = StaticCell::new();
static LS_ENABLE_CELL: StaticCell<Output<'static>> = StaticCell::new();
static SOLENOID_CELL: StaticCell<Output<'static>> = StaticCell::new();
static RUN_BUTTON_CELL: StaticCell<Input<'static>> = StaticCell::new();
static INTERLOCK_CELL: StaticCell<Input<'static>> = StaticCell::new();
static GATE_FAULT_CELL: StaticCell<Input<'static>> = StaticCell::new();
static GATE_READY_CELL: StaticCell<Input<'static>> = StaticCell::new();
static ADC_CELL: StaticCell<Adc<'static, Async>> = StaticCell::new();
static ADC_CHANNELS_CELL: StaticCell<[Channel<'static>; 2]> = StaticCell::new();
static ADS_CELL: StaticCell<Ads7828<'static>> = StaticCell::new();

bind_interrupts!(struct AdcIrqs {
    ADC_IRQ_FIFO => InterruptHandler;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p: Peripherals = embassy_rp::init(Default::default());

    // ------------------------------------------------------------------------------------------
    // GPIO setups
    // ------------------------------------------------------------------------------------------
    let hs_enable = HS_ENABLE_CELL.init(Output::new(p.PIN_5, Level::Low));
    let ls_enable = LS_ENABLE_CELL.init(Output::new(p.PIN_9, Level::Low));
    let solenoid = SOLENOID_CELL.init(Output::new(p.PIN_11, Level::Low));
    let run_button = RUN_BUTTON_CELL.init(Input::new(p.PIN_14, Pull::Up));
    let interlock = INTERLOCK_CELL.init(Input::new(p.PIN_15, Pull::Down));
    let gate_fault = GATE_FAULT_CELL.init(Input::new(p.PIN_6, Pull::Up));
    let gate_ready = GATE_READY_CELL.init(Input::new(p.PIN_7, Pull::Up));

    let down_pin = Input::new(p.PIN_12, Pull::Up);
    let up_pin = Input::new(p.PIN_13, Pull::Up);
    let enter_pin = Input::new(p.PIN_27, Pull::Up);
    let sic_temp_pin = Input::new(p.PIN_4, Pull::Down);

    // ------------------------------------------------------------------------------------------
    // PWM setup for SiC MOSFET
    // ------------------------------------------------------------------------------------------
    let mut drive_cfg = PwmConfig::default();
    drive_cfg.phase_correct = true;
    drive_cfg.invert_b = true;
    drive_cfg.enable = false;
    let pwm_drive = PWM_DRIVE_CELL.init(Pwm::new_output_ab(
        p.PWM_SLICE0,
        p.PIN_0,
        p.PIN_1,
        drive_cfg,
    ));
    pwm_disable(pwm_drive);

    // ------------------------------------------------------------------------------------------
    // I2C ADC Setup
    // ------------------------------------------------------------------------------------------
    let mut ads_i2c_cfg = I2cConfig::default();
    ads_i2c_cfg.frequency = 100_000;
    let ads_i2c = I2c::new_blocking(p.I2C1, p.PIN_19, p.PIN_18, ads_i2c_cfg);

    // ------------------------------------------------------------------------------------------
    // LCD Config
    // ------------------------------------------------------------------------------------------
    let mut rs_pin: Output<'_> = Output::new(p.PIN_25, Level::Low);
    rs_pin.set_drive_strength(Drive::_12mA);
    let mut en_pin = Output::new(p.PIN_24, Level::Low);
    en_pin.set_drive_strength(Drive::_12mA);
    let mut d4_pin = Output::new(p.PIN_23, Level::Low);
    d4_pin.set_drive_strength(Drive::_12mA);
    let mut d5_pin = Output::new(p.PIN_22, Level::Low);
    d5_pin.set_drive_strength(Drive::_12mA);
    let mut d6_pin = Output::new(p.PIN_21, Level::Low);
    d6_pin.set_drive_strength(Drive::_12mA);
    let mut d7_pin = Output::new(p.PIN_20, Level::Low);
    d7_pin.set_drive_strength(Drive::_12mA);

    let backlight_pin = None;

    let mut lcd = Lcd::new(
        rs_pin,
        en_pin,
        backlight_pin,
        d4_pin,
        d5_pin,
        d6_pin,
        d7_pin,
        16,
        2,
    );

    lcd.init().await;
    lcd.backlight(true);
    lcd.set_cursor(0, 0).await;
    lcd.message("Induction Shrink").await;
    lcd.set_cursor(0, 1).await;
    lcd.message("System init...").await;
    lcd.show_blink(false).await;

    // ------------------------------------------------------------------------------------------
    // Menu
    // ------------------------------------------------------------------------------------------
    spawner
        .spawn(menu_task(lcd, up_pin, down_pin, enter_pin))
        .unwrap();

    // ------------------------------------------------------------------------------------------
    // MLX90614 setup
    // ------------------------------------------------------------------------------------------
    let mut mlx_i2c_cfg = I2cConfig::default();
    mlx_i2c_cfg.frequency = 100_000;
    let mlx_i2c = I2c::new_blocking(p.I2C0, p.PIN_17, p.PIN_16, mlx_i2c_cfg);
    let mlx = Mlx90614::new(mlx_i2c);
    spawner.spawn(mlx_task(mlx)).unwrap();

    // ------------------------------------------------------------------------------------------
    // ADS7828 task
    // ------------------------------------------------------------------------------------------
    let ads = ADS_CELL.init(Ads7828::new(ads_i2c, 0x48));
    spawner.spawn(ads_task(ads)).unwrap();

    // ------------------------------------------------------------------------------------------
    // On-chip ADC sampling task
    // ------------------------------------------------------------------------------------------
    let adc = ADC_CELL.init(Adc::new(p.ADC, AdcIrqs, AdcConfig::default()));
    let channels = ADC_CHANNELS_CELL.init([
        Channel::new_pin(p.PIN_26, Pull::None),
        Channel::new_pin(p.PIN_29, Pull::None),
    ]);
    spawner
        .spawn(adc_task(adc, channels, p.DMA_CH0.into_ref()))
        .unwrap();

    // ------------------------------------------------------------------------------------------
    // SiC module temperature duty monitor
    // ------------------------------------------------------------------------------------------
    spawner.spawn(sic_temp_task(sic_temp_pin)).unwrap();

    // ------------------------------------------------------------------------------------------
    // Safety monitor
    // ------------------------------------------------------------------------------------------
    spawner
        .spawn(safety_task(interlock, gate_fault, gate_ready))
        .unwrap();

    // ------------------------------------------------------------------------------------------
    // Control loop
    // ------------------------------------------------------------------------------------------
    spawner
        .spawn(control_task(
            pwm_drive, hs_enable, ls_enable, solenoid, run_button,
        ))
        .unwrap();

    // ------------------------------------------------------------------------------------------
    // Idle loop
    // ------------------------------------------------------------------------------------------
    loop {
        Timer::after(Duration::from_secs(1)).await;
        // info!("alive");
    }
}
