use embassy_time::{Delay, Duration, Instant, Timer};
use crate::ads7828::Ads7828;
use crate::channel_buffers::SafeChannelBuffers;

#[embassy_executor::task]
pub async fn gather_channels_task(
    ads: &'static Ads7828<'static>, 
    buffers: &'static SafeChannelBuffers
) {
    loop {
        // Wait 500ms
        Timer::after(Duration::from_millis(500)).await;

        // Acquire all 8 channels
        match ads.get_channels(false).await {
            Ok(raw) => {
                let mut guard = buffers.lock().await;
                guard.add_samples(&raw);
            }
            Err(_e) => {
                // handle i2c error, e.g. log or retry
            }
        }
    }
}

/// Example consumer task that reads one channel every second:
#[embassy_executor::task]
pub async fn use_channels_task(buffers: &'static SafeChannelBuffers) {
    loop {
        Timer::after(Duration::from_secs(1)).await;

        let mut guard = buffers.lock().await;
        let val = guard.read_and_clear(0);
        drop(guard);

        // do something with `val`
        // e.g. defmt::info!("Channel 0 = {}", val);
    }
}
