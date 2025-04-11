#![no_std]
#![no_main]


use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::Peripherals;
use cortex_m::singleton;

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Timer};

use {defmt_rtt as _, panic_probe as _};

mod hardware;
mod ads7828;
mod channel_buffers;
mod tasks; // gather_channels_task, etc.

use ads7828::Ads7828;
use channel_buffers::{ChannelBuffers, SafeChannelBuffers};
use tasks::{gather_channels_task, use_channels_task};


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

static mut BUFFERS_INSTANCE: Option<SafeChannelBuffers> = None;



#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // Initialize the RP2040 driver, which gives us `Peripherals` with pre-split pins.
    let p: Peripherals = embassy_rp::init(Default::default());
    let mut hw = hardware::init(p);

    let buffers = singleton!(: SafeChannelBuffers = {
        SafeChannelBuffers::new(ChannelBuffers::new())
    }).unwrap();


    // Now spawn the tasks, giving them &'static references
    spawner.spawn(gather_channels_task(&hw.ads, buffers)).unwrap();
    spawner.spawn(use_channels_task(buffers)).unwrap();
    



    info!("Hello World!");

    hw.set_dead_time(512, 10000);

    hw.io_interlock_loop.set_high();
    info!("Interlock loop overwritten on");

    hw.io_ls_enable.set_high();
    info!("LS enabled");

    hw.io_hs_enable.set_high();
    info!("HS enabled");
    // spawner.spawn(pwm_sweep_task(hw.pwm_ch0)).unwrap();
    loop {
    }
}
