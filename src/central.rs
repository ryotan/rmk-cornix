#![no_main]
#![no_std]

use rmk::macros::rmk_central;

#[path = "deadline.rs"]
mod deadline;
#[path = "ws2812.rs"]
mod ws2812;

#[rmk_central]
mod keyboard_central {
    use embassy_nrf::gpio::{Input, Level, Output, OutputDrive, Pull};
    use embassy_nrf::pwm::{Config, Prescaler, SequenceLoad, SequencePwm};

    use crate::ws2812::{Ws2812Driver, PWM_TOP};
    use cornix_led::Role;

    /// Left half status LEDs: WS2812 data on P0.24, LED power on P0.13.
    #[register_processor(event)]
    fn status_led() -> crate::deadline::DeadlineDriven<Ws2812Driver> {
        let mut config = Config::default();
        config.prescaler = Prescaler::Div1; // timing: see ws2812.rs
        config.max_duty = PWM_TOP;
        config.sequence_load = SequenceLoad::Common;
        let pwm = SequencePwm::new_1ch(p.PWM0, p.P0_24, config).unwrap();
        let led_power = Output::new(p.P0_13, Level::Low, OutputDrive::Standard);
        // Charger STAT on P0.01, as in numachang's config; unverified until a charge is observed.
        let charge_stat = Input::new(p.P0_01, Pull::Up);
        Ws2812Driver::processor(pwm, led_power, Role::Central, Some(charge_stat))
    }
}
