#![no_std]
#![no_main]


use defmt::*;
use embassy_executor::Spawner;
use cortex_m::singleton;
use embassy_rp::{
    gpio::{AnyPin, Input, Level, Output, Pull},
    pwm::{Config, Pwm, SetDutyCycle},
    Peripherals,
};
use embassy_rp::i2c::{I2c, Config as I2cConfig};


use embassy_time::{Duration, Timer};

use {defmt_rtt as _, panic_probe as _};

mod utils;
mod ads7828;
mod channel_buffers;
mod tasks;

use ads7828::Ads7828;
use utils::*;
use channel_buffers::{ChannelBuffers, SafeChannelBuffers};
use tasks::*;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p: Peripherals = embassy_rp::init(Default::default());

    // ------------------------------------------------------------------------------------------
    // GPIO setups
    // ------------------------------------------------------------------------------------------
    let mut io_interlock_loop = Output::new(p.PIN_15, Level::Low);
    let mut io_hs_enable = Output::new(p.PIN_5, Level::Low);
    let mut io_ls_enable = Output::new(p.PIN_9, Level::Low);
    let input_gate_driver_fault = Input::new(p.PIN_6, Pull::Up);
    let input_gate_driver_ready = Input::new(p.PIN_7, Pull::Up);
    let input_dummy_pwm1a = Input::new(p.PIN_2, Pull::None);
    let input_dummy_pwm1b = Input::new(p.PIN_3, Pull::None);
    let input_sic_rtd = Input::new(p.PIN_4, Pull::None);

    io_interlock_loop.set_high();
    info!("Interlock loop overwritten on");

    io_ls_enable.set_high();
    info!("LS enabled");

    io_hs_enable.set_high();
    info!("HS enabled");

    info!("Gate driver fault: {}", input_gate_driver_fault.is_low()); //open drain pulled-up. Low is fault
    info!("Gate driver ready: {}", input_gate_driver_ready.is_high()); //open drain pulled-up. High is ready (VDD and VCC are ok)

    // ------------------------------------------------------------------------------------------
    // PWM setup for SiC MOSFET
    // ------------------------------------------------------------------------------------------
    let mut c = Config::default();

    let desired_freq_hz = 1_000;
    let clock_freq_hz = embassy_rp::clocks::clk_sys_freq();
    let divider = 2u8;
    let period = ((clock_freq_hz / (desired_freq_hz * divider as u32))/2) as u16 - 1;
    let dt_ns: u32= 10000;

    c.top = period;
    c.divider = divider.into();
    c.phase_correct = true;
    c.invert_b = true; // Invert B output
    c.enable = false; //start disabled

    let mut pwm_sic: Pwm<'static> = Pwm::new_output_ab(
        p.PWM_SLICE0,   // the underlying hardware PWM channel
        p.PIN_0,     // A output -> GPIO0
        p.PIN_1,     // B output -> GPIO1
        c.clone()
    );

    // ------------------------------------------------------------------------------------------
    // I2C ADC Setup
    // ------------------------------------------------------------------------------------------
    let mut i2c_cfg = I2cConfig::default();
    i2c_cfg.frequency = 100_000;
    let i2c1 = I2c::new_blocking(p.I2C1, p.PIN_19, p.PIN_18, i2c_cfg);

    // ------------------------------------------------------------------------------------------
    // Prepare and spawn tasks
    // ------------------------------------------------------------------------------------------
    let ads: &mut Ads7828<'_> = singleton!(: Ads7828<'static> = {
        Ads7828::new(i2c1, 0x48)
    }).unwrap();

    let buffers = singleton!(: SafeChannelBuffers = {
        SafeChannelBuffers::new(ChannelBuffers::new())
    }).unwrap();

    spawner.spawn(gather_channels_task(ads, buffers)).unwrap();
    spawner.spawn(log_channels(buffers)).unwrap();

    // ------------------------------------------------------------------------------------------
    // PWM test
    // ------------------------------------------------------------------------------------------

    Timer::after(Duration::from_secs(2)).await;
    pwm_enable(&mut pwm_sic, 512, 50000);
    Timer::after(Duration::from_secs(50)).await;
    pwm_disable(&mut pwm_sic);

    // sleep main forever
    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}
