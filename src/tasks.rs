use defmt::*;
use embassy_time::{Delay, Duration, Instant, Timer};
use futures::FutureExt;
use crate::ads7828::Ads7828;
use crate::channel_buffers::SafeChannelBuffers;
use embassy_rp::gpio::{AnyPin, Input};
use embassy_rp::pio::program::pio_asm;
use embassy_rp::pio::StateMachine;
use libm::log;



#[embassy_executor::task]
pub async fn gather_channels_task(
    ads: &'static Ads7828<'static>, 
    buffers: &'static SafeChannelBuffers
) {
    loop {
        // Wait 500ms
        Timer::after(Duration::from_millis(50)).await;
        // Acquire all 8 channels
        match ads.get_channels(false).await {
            Ok(raw) => {
                let mut guard = buffers.lock().await;
                guard.add_samples(&raw);
            }
            Err(_e) => {
                error!("I2C error {}", _e);
            }
        }
    }
}

/// Example consumer task that reads one channel every second:
#[embassy_executor::task]
pub async fn log_channels(buffers: &'static SafeChannelBuffers) {
    loop {
        Timer::after(Duration::from_millis(1000)).await;
        info!("Running use_channels_task");
        let mut guard = buffers.lock().await;

        // do something with `val`
        info!("C0 Switch current = {} A", (guard.read_and_clear(0) as f32 / (65535.0) * 5.0) * 2.5f32 * 1.25f32 / 800.0f32);
        info!("Channel 1 = {}", guard.read_and_clear(1) as f32 / (65535.0) * 5.0);
        info!("Channel 2 = {}", guard.read_and_clear(2) as f32 / (65535.0) * 5.0);
        info!("Channel 3 = {}", guard.read_and_clear(3) as f32 / (65535.0) * 5.0);
        info!("Channel 4 = {}", guard.read_and_clear(4) as f32 / (65535.0) * 5.0);
        info!("Channel 5 = {}", guard.read_and_clear(5) as f32 / (65535.0) * 5.0);
        let coil_temp_v: f32 = guard.read_and_clear(6) as f32 / (65535.0) * 5.0;
        info!("C6 Coil Temp = {}", -25.84 * log(((coil_temp_v * 10000.0) / (5.0 - coil_temp_v)) as f64) as f32 + 264.18);
        info!("Channel 7 = {}", guard.read_and_clear(7) as f32 / (65535.0) * 5.0);
    }
}

