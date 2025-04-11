#![no_std]
#![no_main]


use defmt::*;
use embassy_executor::Spawner;
use cortex_m::singleton;
use embassy_rp::{
    gpio::{Drive, Level, Output}, pwm::{Config, Pwm, PwmOutput, SetDutyCycle}, Peripherals
};
use embassy_rp::i2c::{I2c, Config as I2cConfig};


use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Timer};

use {defmt_rtt as _, panic_probe as _};

mod hardware;
mod ads7828;
mod channel_buffers;
mod tasks; // gather_channels_task, etc.

use ads7828::Ads7828;
use hardware::set_dead_time;
use channel_buffers::{ChannelBuffers, SafeChannelBuffers};
use tasks::{gather_channels_task, log_channels};


// #[embassy_executor::task]
// async fn pwm_sweep_task(mut pwm_ch0: Pwm<'static>) {
//     loop {
//         // Ramp duty cycle from 0% to 100%
//         for duty in 0..=100 {
//             pwm_ch0.set_duty(duty).await;
//             Timer::after(Duration::from_millis(10)).await;
//         }
//         // And back down from 100% to 0%
//         for duty in (0..=100).rev() {
//             pwm_ch0.set_duty(duty).await;
//             Timer::after(Duration::from_millis(10)).await;
//         }
//     }
// }


#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // Initialize the RP2040 driver, which gives us `Peripherals` with pre-split pins.
    let p: Peripherals = embassy_rp::init(Default::default());

    let mut io_interlock_loop = Output::new(p.PIN_15, Level::Low);

    let mut io_hs_enable = Output::new(p.PIN_5, Level::Low);
    let mut io_ls_enable = Output::new(p.PIN_9, Level::Low);

    let mut c = Config::default();

    let desired_freq_hz = 1_000;
    let clock_freq_hz = embassy_rp::clocks::clk_sys_freq();
    let divider = 2u8;
    let period = ((clock_freq_hz / (desired_freq_hz * divider as u32))/2) as u16 - 1;

    c.top = period;
    c.divider = divider.into();
    c.phase_correct = true;
    c.invert_b = true; // Invert B output

    let mut pwm_ch0: Pwm<'static> = Pwm::new_output_ab(
        p.PWM_SLICE0,   // the underlying hardware PWM channel
        p.PIN_0,     // A output -> GPIO0
        p.PIN_1,     // B output -> GPIO1
        c.clone()
    );

    // For I2C, pick your pins. For example, SDA=GPIO18, SCL=GPIO19
    let mut i2c_cfg = I2cConfig::default();
    i2c_cfg.frequency = 100_000; // or 1_000_000 if you need 1 MHz

    // Create the blocking I2C driver:
    // `p.I2C0` = the I2C0 peripheral, `p.PIN_18` = SDA, `p.PIN_19` = SCL
    let i2c1 = I2c::new_blocking(p.I2C1, p.PIN_19, p.PIN_18, i2c_cfg);
    
    let ads = singleton!(: Ads7828<'static> = {
        Ads7828::new(i2c1, 0x48)
    }).unwrap();

    
    let buffers = singleton!(: SafeChannelBuffers = {
        SafeChannelBuffers::new(ChannelBuffers::new())
    }).unwrap();

    // Now spawn the tasks, giving them &'static references
    spawner.spawn(gather_channels_task(ads, buffers)).unwrap();
    spawner.spawn(log_channels(buffers)).unwrap();



    info!("Hello World!");

    set_dead_time(pwm_ch0, 512, 10000);

    io_interlock_loop.set_high();
    info!("Interlock loop overwritten on");

    io_ls_enable.set_high();
    info!("LS enabled");

    io_hs_enable.set_high();
    info!("HS enabled");
    // spawner.spawn(pwm_sweep_task(hw.pwm_ch0)).unwrap();
    loop {
        embassy_time::Timer::after(Duration::from_secs(1)).await;
    }
}
