#![no_main]
#![no_std]

use rmk::macros::rmk_peripheral;

#[path = "deadline.rs"]
mod deadline;
#[path = "ws2812.rs"]
mod ws2812;

#[rmk_peripheral(id = 0)]
mod keyboard_peripheral {
    use embassy_nrf::gpio::{Input, Level, Output, OutputDrive, Pull};
    use embassy_nrf::pwm::{Config, Prescaler, SequenceLoad, SequencePwm};

    use crate::ws2812::{Ws2812Driver, PWM_TOP};
    use cornix_led::Role;

    /// Right half status LEDs: WS2812 data on P0.13, LED power on P0.24; the
    /// two pins are swapped relative to the left half.
    #[register_processor(event)]
    fn status_led() -> crate::deadline::DeadlineDriven<Ws2812Driver> {
        let mut config = Config::default();
        config.prescaler = Prescaler::Div1; // timing: see ws2812.rs
        config.max_duty = PWM_TOP;
        config.sequence_load = SequenceLoad::Common;
        let pwm = SequencePwm::new_1ch(p.PWM0, p.P0_13, config).unwrap();
        let led_power = Output::new(p.P0_24, Level::Low, OutputDrive::Standard);
        // Charger STAT assumed on P0.01 like the left half; unverified until a charge is observed.
        let charge_stat = Input::new(p.P0_01, Pull::Up);
        Ws2812Driver::processor(pwm, led_power, Role::Peripheral, Some(charge_stat))
    }
}
