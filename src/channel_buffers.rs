use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;

/// Store sums + counts for 8 channels.
pub struct ChannelBuffers {
    sums: [f64; 8],
    counts: [u32; 8],
}

impl ChannelBuffers {
    pub fn new() -> Self {
        Self {
            sums: [0.0; 8],
            counts: [0; 8],
        }
    }

    /// Add the new raw readings (0..4095).
    pub fn add_samples(&mut self, raw: &[u16; 8]) {
        for (i, &val) in raw.iter().enumerate() {
            let scaled = val as f64 / 4095.0;
            self.sums[i] += scaled;
            self.counts[i] += 1;
        }
    }

    /// Read and clear the average for `channel`.
    /// Returns a 16-bit scaled value (0..65535).
    pub fn read_and_clear(&mut self, channel: u8) -> u16 {
        let ch = channel as usize % 8;
        let sum = self.sums[ch];
        let c = self.counts[ch];

        self.sums[ch] = 0.0;
        self.counts[ch] = 0;

        if c == 0 {
            return 0;
        }
        let avg = sum / (c as f64); // 0..1
        // If you want rounding:
        // let val = libm::round(avg * 65535.0) as u16;
        let val = (avg * 65535.0) as u16;
        val
    }
}

/// Type alias for a threadsafe `ChannelBuffers`.
pub type SafeChannelBuffers = Mutex<CriticalSectionRawMutex, ChannelBuffers>;
