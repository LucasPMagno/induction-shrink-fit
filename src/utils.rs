use defmt::info;
use embassy_rp::{
    pwm::{Config, Pwm, SetDutyCycle}, clocks
};

pub fn pwm_enable(pwm_ch: &mut Pwm<'_>, dt_ns: u32, desired_freq_hz: u32) {
    let clock_freq_hz = clocks::clk_sys_freq();
    let divider = 2u8;
    let period = ((clock_freq_hz / (desired_freq_hz * divider as u32))/2) as u16 - 1;

    // Calculate the dead time in clock cycles - dt_ns * 125MHz / 16 / 1_000_000_000
    // dt = 1 = 1 period of 125MHz clock divider by divider = 8ns * 16 = 128ns
    let dt = (((dt_ns * (clock_freq_hz / 1_000_000))/(divider as u32)) / 1_000) as u16;

    info!("PWM period: {}", period);
    info!("PWM divider: {}", divider);
    info!("PWM dt: {}", dt);
    info!("PWM dt_ns: {}", dt_ns);

    let mut c = Config::default();
    c.top = period;
    c.divider = divider.into();
    c.phase_correct = true;
    c.invert_b = true; // Invert B output
    pwm_ch.set_config(&c);

    let (pwm_ch0_a, pwm_ch0_b) = pwm_ch.split_by_ref();
    if let (Some(ref mut a), Some(ref mut b)) = (pwm_ch0_a, pwm_ch0_b) {
        a.set_duty_cycle_fraction((period - dt)/2, period).unwrap();
        b.set_duty_cycle_fraction((period + dt)/2, period).unwrap();
    }
}

pub fn pwm_disable(pwm_ch: &mut Pwm<'static>) {
    let _ = pwm_ch.set_duty_cycle_fully_off();
}