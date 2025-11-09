use defmt::*;
use embassy_hal_internal::PeripheralRef;
use embassy_rp::{
    adc::{Adc, Async, Channel},
    clocks,
    gpio::Input,
};
use embassy_time::{Duration, Instant, Timer};
use libm::{logf, sqrtf};

use crate::{ads7828::Ads7828, mlx90614::Mlx90614, state::MEASUREMENTS};

const TARGET_SAMPLE_RATE_HZ: u32 = 150_000;
const PAIRS_PER_BATCH: usize = 512;
const DMA_BUFFER_LEN: usize = PAIRS_PER_BATCH * 2;
const ADC_REF_V: f32 = 3.321;
const VDC_GAIN: f32 = 0.0018615088;
const CURRENT_CENTER_V: f32 = 1.245; //1.252 in theory but measured slightly lower
const CURRENT_SENSITIVITY_A_PER_V: f32 = 1280.0; // 0.625 V -> 800 A
const POWER_SMOOTH_FACTOR: f32 = 0.2;
const MAX_VOLTAGE_V: f32 = 1000.0;
const MAX_CURRENT_A: f32 = 900.0;
const PWM_MIN_DUTY: f32 = 0.05;
const PWM_MAX_DUTY: f32 = 0.95;
const PWM_LOW_DUTY: f32 = 0.10;
const PWM_HIGH_DUTY: f32 = 0.88;
const PWM_LOW_V: f32 = 0.6;
const PWM_HIGH_V: f32 = 4.5;
const MODULE_NTC_BETA: f32 = 3468.0;
const MODULE_NTC_R0: f32 = 5_000.0;
const MODULE_NTC_T0_C: f32 = 25.0;

#[embassy_executor::task]
pub async fn adc_task(
    adc: &'static mut Adc<'static, Async>,
    channels: &'static mut [Channel<'static>; 2],
    mut dma: PeripheralRef<'static, embassy_rp::peripherals::DMA_CH0>,
) {
    static mut DMA_BUFFER: [u16; DMA_BUFFER_LEN] = [0; DMA_BUFFER_LEN];
    let channel_count = channels.len() as u32;
    let adc_clk = clocks::clk_adc_freq();
    let div = 0;
    // let mut div = if channel_count == 0 {
    //     0
    // } else {
    //     adc_clk
    //         .saturating_div(TARGET_SAMPLE_RATE_HZ.saturating_mul(channel_count))
    //         .saturating_sub(1)
    // };
    // if div > u16::MAX as u32 {
    //     div = u16::MAX as u32;
    // }
    // let div = div as u16;

    loop {
        let buffer = unsafe { &mut DMA_BUFFER };
        if let Err(_e) = adc
            .read_many_multichannel(&mut channels[..], buffer, div, dma.reborrow())
            .await
        {
            warn!("ADC DMA error");
            Timer::after(Duration::from_millis(5)).await;
            continue;
        }

        let mut sum_v_sq = 0.0f32;
        let mut sum_i_sq = 0.0f32;
        let mut sum_vi = 0.0f32;

        for pair in buffer.chunks_exact(2) {
            let v_sample = pair[0] as f32;
            let i_sample = pair[1] as f32;

            let v_adc = v_sample * (ADC_REF_V / 4095.0);
            let i_adc = i_sample * (ADC_REF_V / 4095.0);

            let dc_voltage = (v_adc / VDC_GAIN).clamp(0.0, MAX_VOLTAGE_V);
            let coil_current = ((i_adc - CURRENT_CENTER_V) * CURRENT_SENSITIVITY_A_PER_V)
                .clamp(-MAX_CURRENT_A, MAX_CURRENT_A);

            sum_v_sq += dc_voltage * dc_voltage;
            sum_i_sq += coil_current * coil_current;
            sum_vi += dc_voltage * coil_current;
        }

        let samples = PAIRS_PER_BATCH as f32;
        let vrms = sqrtf((sum_v_sq / samples).max(0.0));
        let irms = sqrtf((sum_i_sq / samples).max(0.0));
        let power_kw = ((sum_vi / samples) / 1000.0).clamp(0.0, 20.0);
        info!("Vdc: {} V, Irms: {} A, P: {} kW", vrms, irms, power_kw);
        {
            let mut guard = MEASUREMENTS.lock().await;
            guard.dc_voltage_v = smooth_value(guard.dc_voltage_v, vrms);
            guard.coil_current_rms_a = smooth_value(guard.coil_current_rms_a, irms);
            guard.coil_power_kw = smooth_value(guard.coil_power_kw, power_kw);
            guard.valid = true;
        }
        Timer::after(Duration::from_millis(50)).await;
    }
}

#[embassy_executor::task]
pub async fn ads_task(ads: &'static Ads7828<'static>) {
    loop {
        match ads.get_channels(false).await {
            Ok(raw) => {
                let coil_temp_v = code_to_voltage(raw[6]);
                let pcb_temp_v = code_to_voltage(raw[3]);

                let coil_temp_c = ntc_pullup_temp(coil_temp_v);
                let pcb_temp_c = pcb_temp_v_to_c(pcb_temp_v);

                {
                    let mut guard = MEASUREMENTS.lock().await;
                    guard.coil_temp_c = smooth_value(guard.coil_temp_c, coil_temp_c);
                    guard.pcb_temp_c = smooth_value(guard.pcb_temp_c, pcb_temp_c);
                    info!("Coil temp: {} C, PCB temp: {} C", coil_temp_c, pcb_temp_c);
                }
            }
            Err(_e) => warn!("ADS7828 error"),
        }

        Timer::after(Duration::from_millis(50)).await;
    }
}

#[embassy_executor::task]
pub async fn mlx_task(
    mut mlx: Mlx90614<'static, embassy_rp::peripherals::I2C0, embassy_rp::i2c::Blocking>,
) {
    loop {
        match mlx.read_object_temp().await {
            Ok(t) => {
                let mut guard = MEASUREMENTS.lock().await;
                guard.object_temp_c = smooth_value(guard.object_temp_c, t);
                info!("IR object temp: {} C", t);
            }
            Err(_e) => warn!("MLX90614 read error"),
        }
        Timer::after(Duration::from_millis(100)).await;
    }
}

#[embassy_executor::task]
pub async fn sic_temp_task(mut pin: Input<'static>) {
    const SAMPLES: usize = 128;

    loop {
        let mut duty_sum = 0.0f32;
        let mut collected = 0usize;

        while collected < SAMPLES {
            pin.wait_for_rising_edge().await;
            let rise = Instant::now();
            pin.wait_for_falling_edge().await;
            let fall = Instant::now();
            pin.wait_for_rising_edge().await;
            let next_rise = Instant::now();

            let high = duration_to_secs(fall - rise);
            let period = duration_to_secs(next_rise - rise);
            if period > 0.0 {
                let duty = (high / period).clamp(PWM_MIN_DUTY, PWM_MAX_DUTY);
                duty_sum += duty;
                collected += 1;
            }
        }

        let duty = (duty_sum / SAMPLES as f32).clamp(PWM_MIN_DUTY, PWM_MAX_DUTY);
        let voltage = duty_to_voltage(duty);
        let resistance = voltage / 0.000203;
        let module_temp_c = ntc_beta_temp(resistance);

        {
            let mut guard = MEASUREMENTS.lock().await;
            guard.module_temp_c = smooth_value(guard.module_temp_c, module_temp_c);
        }
        info!("SiC module temp: {} C", module_temp_c);

        Timer::after(Duration::from_millis(50)).await;
    }
}

fn smooth_value(previous: f32, new_value: f32) -> f32 {
    if !previous.is_finite() || previous == 0.0 {
        new_value
    } else {
        previous + POWER_SMOOTH_FACTOR * (new_value - previous)
    }
}

fn code_to_voltage(code: u16) -> f32 {
    (code as f32 / 4095.0) * 5.0
}

fn ntc_pullup_temp(voltage: f32) -> f32 {
    const SERIES_R: f32 = 10_000.0;
    const BETA: f32 = 3950.0;
    const R0: f32 = 10_000.0;
    const T0_K: f32 = 298.15;

    if voltage <= 0.01 || voltage >= 4.99 {
        return 0.0;
    }

    let resistance = SERIES_R * voltage / (5.0 - voltage);
    let inv_t = 1.0 / T0_K + logf(resistance / R0) / BETA;
    1.0 / inv_t - 273.15
}

fn pcb_temp_v_to_c(voltage: f32) -> f32 {
    ((voltage - 0.5) / 0.01).clamp(-40.0, 150.0)
}

fn duty_to_voltage(duty: f32) -> f32 {
    let slope = (PWM_HIGH_V - PWM_LOW_V) / (PWM_HIGH_DUTY - PWM_LOW_DUTY);
    PWM_LOW_V + slope * (duty - PWM_LOW_DUTY)
}

fn ntc_beta_temp(resistance: f32) -> f32 {
    if resistance <= 10.0 {
        return 0.0;
    }
    let t0_k = MODULE_NTC_T0_C + 273.15;
    let inv_t = 1.0 / t0_k + logf(resistance / MODULE_NTC_R0) / MODULE_NTC_BETA;
    1.0 / inv_t - 273.15
}

fn duration_to_secs(duration: Duration) -> f32 {
    duration.as_micros() as f32 / 1_000_000.0
}
