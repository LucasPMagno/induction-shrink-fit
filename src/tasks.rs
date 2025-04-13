use defmt::*;
use embassy_time::{Delay, Duration, Instant, Timer};
use futures::FutureExt;
use crate::ads7828::Ads7828;
use crate::channel_buffers::SafeChannelBuffers;
use embassy_rp::gpio::{AnyPin, Input};
use embassy_rp::pio::program::pio_asm;
use embassy_rp::pio::StateMachine;



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
        Timer::after(Duration::from_millis(100)).await;
        info!("Running use_channels_task");
        let mut guard = buffers.lock().await;

        // do something with `val`
        info!("C0 Switch current = {} A", (guard.read_and_clear(0) as f32 / (65535.0) * 5.0) * 2.5f32 * 1.25f32 / 800.0f32);
        // info!("Channel 1 = {}", guard.read_and_clear(1) as f32 / (65535.0) * 5.0);
        // info!("Channel 2 = {}", guard.read_and_clear(2) as f32 / (65535.0) * 5.0);
        // info!("Channel 3 = {}", guard.read_and_clear(3) as f32 / (65535.0) * 5.0);
        // info!("Channel 4 = {}", guard.read_and_clear(4) as f32 / (65535.0) * 5.0);
        // info!("Channel 5 = {}", guard.read_and_clear(5) as f32 / (65535.0) * 5.0);
        // info!("Channel 6 = {}", guard.read_and_clear(6) as f32 / (65535.0) * 5.0);
        // info!("Channel 7 = {}", guard.read_and_clear(7) as f32 / (65535.0) * 5.0);
    }
}

// ------------------------------------------------------------------------------------------
// PIO PWM Input for SiC Temperature
// ------------------------------------------------------------------------------------------
#[embassy_executor::task]
pub async fn measure_duty_cycle(mut pin: Input<'static>) {
    embassy_time::Ticker::every(Duration::from_nanos(128)).next().await;



    loop {
        //PWM frequency is ~400kHz
        Timer::after(Duration::from_millis(100)).await;
        // 1. Wait for rising edge
        pin.wait_for_rising_edge();
        let rise1: Instant = Instant::now();
        info!("Rising 1 edge detected at {:?}", rise1);

        // 2. Wait for falling edge -> measure high time
        pin.wait_for_falling_edge();
        let fall = Instant::now();
        info!("Falling edge detected at {:?}", fall);
        let high_time_us = fall.duration_since(rise1).as_ticks();

        // 3. Wait for next rising edge -> measure period
        pin.wait_for_rising_edge();
        let rise2 = Instant::now();
        info!("Rising 2 edge detected at {:?}", rise2);
        let period_us = rise2.duration_since(rise1).as_ticks();

        // Calculate the duty cycle as a fraction from 0..1
        // (protect against divide-by-zero)
        let duty = if period_us == 0 {
            0.0
        } else {
            high_time_us as f32 / period_us as f32
        };
        // Linear mapping: V(d) = -5 * d + 5
        // (Maps 10% → 4.5 V, 88% → 0.6 V)
        let voltage = (5.0 - 5.0 * duty).clamp(0.6, 4.5);

        info!(
            "Duty cycle: {}%,  Mapped Voltage: {} V",
            duty * 100.0,
            voltage
        );
    }
}