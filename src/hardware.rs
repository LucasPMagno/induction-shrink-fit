use defmt::info;
use embassy_rp::{
    gpio::{Drive, Level, Output}, pwm::{Config, Pwm, PwmOutput, SetDutyCycle}, Peripherals
};
use embassy_rp::i2c::I2c;
use embassy_rp::i2c;
use embassy_rp::peripherals::{I2C1};
use cortex_m::singleton;


use crate::ads7828::Ads7828;
static mut ADS7828_INSTANCE: Option<Ads7828<'static>> = None;


pub struct Hardware {
    pub io_interlock_loop: Output<'static>,
    pub io_hs_enable: Output<'static>,
    pub io_ls_enable: Output<'static>,
    pub pwm_ch0: Pwm<'static>,
    pub ads: &'static mut Ads7828<'static>,
}

pub fn init(p: Peripherals) -> Hardware {
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
    let mut i2c_cfg = i2c::Config::default();
    i2c_cfg.frequency = 100_000; // or 1_000_000 if you need 1 MHz

    // Create the blocking I2C driver:
    // `p.I2C0` = the I2C0 peripheral, `p.PIN_18` = SDA, `p.PIN_19` = SCL
    let i2c1 = I2c::new_blocking(p.I2C1, p.PIN_19, p.PIN_18, i2c_cfg);
    
    let ads = singleton!(: Ads7828<'static> = {
        Ads7828::new(i2c1, 0x48)
    }).unwrap();


    // Return a struct bundling everything
    Hardware { io_interlock_loop, io_hs_enable, io_ls_enable, pwm_ch0, ads }
}

impl Hardware {
    pub fn set_interlock_loop(&mut self, state: bool) {
        if state {
            self.io_interlock_loop.set_high();
        } else {
            self.io_interlock_loop.set_low();
        }
    }

    pub fn get_interlock_loop(&self) -> bool {
        self.io_interlock_loop.is_set_high()
    }

    pub fn set_dead_time(&mut self, dt_ns: u32, desired_freq_hz: u32) {
        let clock_freq_hz = embassy_rp::clocks::clk_sys_freq();
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
        self.pwm_ch0.set_config(&c);

        let (pwm_ch0_a, pwm_ch0_b) = self.pwm_ch0.split_by_ref();
        if let (Some(ref mut a), Some(ref mut b)) = (pwm_ch0_a, pwm_ch0_b) {
            a.set_duty_cycle_fraction((period - dt)/2, period).unwrap();
            b.set_duty_cycle_fraction((period + dt)/2, period).unwrap();
        }
        
    }
}

