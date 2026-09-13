// WS2812 rendering (PWM + EasyDMA) ported from numachang/cornix-rmk-custom
// (https://github.com/numachang/cornix-rmk-custom, src/ws2812.rs at commit
// d5e7498bcd7f9c4861a26e587ad2d4e7842035be).
//
// Copyright (c) 2026 numachang
// SPDX-License-Identifier: MIT
// Full license text: licenses/numachang-cornix-rmk-custom.LICENSE-MIT

//! WS2812 status indicator for the Cornix: the hardware side.
//!
//! What to show is decided in the `cornix-led` crate: this module feeds it
//! facts and renders its frames. Facts arrive two ways: RMK events (each
//! handler calls a `Status::set_*` and refreshes) and a poll of USB power and
//! the charger's STAT line on every wake-up. A refresh asks
//! `Indicator::evaluate` for the two colours and the next moment the display
//! can change. That moment or the next poll, whichever is earlier, is the
//! deadline of the [`DeadlineProcessor`] loop. So the task wakes every
//! [`cornix_led::indicator::FRAME_MS`] while something blinks or breathes,
//! at the end of a pulse, and at least once per [`SAMPLE_INTERVAL_MS`].
//!
//! Each half carries two serial RGB LEDs, driven by the nRF52840 PWM
//! peripheral with EasyDMA. Each WS2812 data bit is one PWM period and the
//! whole 2-pixel frame is a sequence the PWM hardware clocks out by DMA. The
//! CPU is not in the timing loop: no interrupt masking, no busy-wait, so the
//! BLE radio (nrf-sdc / MPSL) is not blocked. Bit-banging inside
//! `interrupt::free` blocked the radio and dropped the BLE link; SPIM
//! produced no output on this board. See [`Ws2812Driver::render`] for when
//! a frame is written and when the LED power is on.
//!
//! `cornix_led::indicator` documents which pixel is which and what each
//! shows.

use cornix_led::color::OFF;
use cornix_led::{Grb, HostState, Indicator, Lit, Ms, Role, Status, Vbus};

use crate::deadline::DeadlineDriven;
use embassy_nrf::gpio::{Input, Output};
use embassy_nrf::pwm::{SequenceConfig, SequencePwm, SingleSequenceMode, SingleSequencer};
use embassy_time::{Duration, Instant, Timer};
use rmk::event::{
    BatteryStatusEvent, CentralConnectedEvent, ConnectionStatusChangeEvent, LayerChangeEvent,
    PeripheralConnectedEvent,
};
use rmk::macros::processor;
use rmk::processor::DeadlineProcessor;
use rmk::types::battery::BatteryStatus;
use rmk::types::ble::BleState;
use rmk::types::connection::{ConnectionType, UsbState};

// WS2812 encoding for the PWM peripheral. The PWM runs at 16 MHz (Div1) with a
// COUNTERTOP of 20, so one period is 20 ticks = 1.25 µs — exactly one WS2812
// bit. Each sequence word is an embassy `DutyCycle` raw value: bit 15 set
// (inverted polarity) makes the output HIGH for the low 15-bit count of ticks
// at the start of the period, then LOW. So `0x8000 | n` = HIGH for n/20 of the
// period:
//   `0` bit: 6 ticks high  ≈ 0.375 µs        `1` bit: 13 ticks high ≈ 0.81 µs
pub const PWM_TOP: u16 = 20;
const W0: u16 = 0x8000 | 6;
const W1: u16 = 0x8000 | 13;
/// All-low word (HIGH for 0 ticks) for the reset/latch tail.
const WRESET: u16 = 0x8000;
/// 2 pixels × 3 colour bytes × 8 bits.
const SEQ_BITS: usize = 2 * 3 * 8;
/// Reset/latch tail: 40 words × 1.25 µs = 50 µs of low after the data.
const SEQ_RESET: usize = 40;
const SEQ_LEN: usize = SEQ_BITS + SEQ_RESET;

/// RMK 0.9 publishes no event for USB power, and none for the STAT line
/// either (see `sample_power`), so the driver polls them this often and
/// feeds the result to the status like an event.
const SAMPLE_INTERVAL_MS: Ms = 1000;

/// Status indicator: RMK events and pin sampling in, WS2812 frames out.
///
/// `#[processor]` generates the event enum, the subscriber and the dispatch
/// to the `on_<event>_event` handlers below. The subscribe list is the union
/// of what both halves need; events that never fire on a half are never
/// delivered. The loop is [`DeadlineProcessor::deadline_loop`], run through
/// [`DeadlineDriven`] (see `deadline.rs`). `new` is private and
/// [`Ws2812Driver::processor`] returns the wrapper, so the bare driver, whose
/// `process_loop` would have no timed wake-ups, cannot be registered.
#[processor(subscribe = [
    BatteryStatusEvent,
    PeripheralConnectedEvent,
    CentralConnectedEvent,
    ConnectionStatusChangeEvent,
    LayerChangeEvent,
])]
pub struct Ws2812Driver {
    pwm: SequencePwm<'static>,
    led_power: Output<'static>,
    /// Charger STAT line (open-drain, low while charging), if wired. `None`
    /// means charging is never reported.
    charge_stat: Option<Input<'static>>,
    status: Status,
    indicator: Indicator,
    /// Next wake-up: the earlier of the frame's own deadline and the next
    /// poll; `0` until the first refresh, so the first wake-up is immediate.
    next_deadline: Ms,
    /// When VBUS and STAT were last polled.
    last_sample: Ms,
    strip: Strip,
}

/// What the strip holds, and with it whether the LED power is on. The power
/// is on exactly when this is not `Off`; only [`Ws2812Driver::render`]
/// changes either, and only from the result of a write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Strip {
    /// Power off. The strip shows nothing.
    Off,
    /// Power on. The strip holds these colours.
    Showing(Lit),
    /// Power on. A write failed, so what the strip holds is unknown; the
    /// next frame is written whatever it is.
    ContentUnknown,
}

impl Ws2812Driver {
    /// The driver as an RMK processor, ready for `#[register_processor(event)]`.
    /// `led_power` may be passed at any level; the driver drives it low before
    /// use.
    pub fn processor(
        pwm: SequencePwm<'static>,
        led_power: Output<'static>,
        role: Role,
        charge_stat: Option<Input<'static>>,
    ) -> DeadlineDriven<Self> {
        DeadlineDriven(Self::new(pwm, led_power, role, charge_stat))
    }

    fn new(
        pwm: SequencePwm<'static>,
        mut led_power: Output<'static>,
        role: Role,
        charge_stat: Option<Input<'static>>,
    ) -> Self {
        led_power.set_low();
        Self {
            pwm,
            led_power,
            charge_stat,
            status: Status::new(),
            indicator: Indicator::new(role),
            next_deadline: 0,
            last_sample: 0,
            strip: Strip::Off,
        }
    }

    fn now() -> Ms {
        Instant::now().as_millis()
    }

    /// Poll USB power and the charger's STAT line into the status.
    ///
    /// VBUS comes straight from the POWER peripheral's USBREGSTATUS register
    /// (independent of the battery switch and of USB enumeration); a bare
    /// register read is passive, so it is safe alongside MPSL / nrf-sdc.
    /// RMK 0.9 accepts `charge_state.pin` in keyboard.toml but never starts
    /// its `ChargingStateReader`, so the STAT line is sampled here. Once RMK
    /// runs the reader, delete this sampling and subscribe to
    /// `ChargingStateEvent` instead.
    fn sample_power(&mut self, now: Ms) {
        let vbus = if embassy_nrf::pac::POWER.usbregstatus().read().vbusdetect() {
            Vbus::Present {
                charging: self.charge_stat.as_ref().is_some_and(|stat| stat.is_low()),
            }
        } else {
            Vbus::Absent
        };
        self.status.set_power(vbus, now);
        self.last_sample = now;
    }

    /// Ask the indicator for a frame, render it if it changed, and remember
    /// when to wake up next: the frame's own deadline or the next poll,
    /// whichever comes first.
    async fn refresh(&mut self, now: Ms) {
        let frame = self.indicator.evaluate(&self.status, now);
        let next_poll = self.last_sample + SAMPLE_INTERVAL_MS;
        self.next_deadline = frame.next_deadline.map_or(next_poll, |d| d.min(next_poll));
        self.render(frame.inner, frame.outer).await;
    }

    /// Encode both pixels (GRB, MSB first) into the PWM sequence buffer, with a
    /// trailing low tail for the WS2812 reset/latch.
    fn encode(buf: &mut [u16; SEQ_LEN], inner: Grb, outer: Grb) {
        let bytes = [inner.g, inner.r, inner.b, outer.g, outer.r, outer.b];
        let mut k = 0;
        for byte in bytes {
            let mut b = byte;
            for _ in 0..8 {
                buf[k] = if b & 0x80 != 0 { W1 } else { W0 };
                k += 1;
                b <<= 1;
            }
        }
        while k < SEQ_LEN {
            buf[k] = WRESET;
            k += 1;
        }
    }

    /// Show both pixels; the LED power is on only while something is lit.
    /// Nothing is written when the strip already holds the frame: the WS2812
    /// latch keeps the previous colour, so an idle indicator does nothing.
    async fn render(&mut self, inner: Grb, outer: Grb) {
        match Lit::try_new(inner, outer) {
            None => {
                if self.strip == Strip::Off {
                    return;
                }
                // Blank the strip, then cut its power.
                self.strip = match self.write(OFF, OFF).await {
                    Ok(()) => {
                        self.led_power.set_low();
                        Strip::Off
                    }
                    Err(()) => Strip::ContentUnknown,
                };
            }
            Some(lit) => {
                if self.strip == Strip::Showing(lit) {
                    return;
                }
                if self.strip == Strip::Off {
                    // Power the strip and wait 5 ms before the first frame,
                    // as numachang's port did.
                    self.led_power.set_high();
                    Timer::after(Duration::from_millis(5)).await;
                }
                self.strip = match self.write(lit.inner(), lit.outer()).await {
                    Ok(()) => Strip::Showing(lit),
                    Err(()) => Strip::ContentUnknown,
                };
            }
        }
    }

    /// Clock one frame out to the strip by DMA. `Ok` means the strip holds it.
    async fn write(&mut self, inner: Grb, outer: Grb) -> Result<(), ()> {
        let mut buf = [WRESET; SEQ_LEN];
        Self::encode(&mut buf, inner, outer);
        let seq = SingleSequencer::new(&mut self.pwm, &buf, SequenceConfig::default());
        match seq.start(SingleSequenceMode::Times(1)) {
            Ok(()) => {
                // Let the DMA finish before `seq` (and `buf`) drop: the whole
                // sequence is ~ SEQ_LEN × 1.25 µs ≈ 110 µs; 1 ms is ample.
                Timer::after(Duration::from_millis(1)).await;
                Ok(())
            }
            Err(e) => {
                // Not reachable with the current constants (one playback, a
                // RAM buffer, SEQ_LEN well below the hardware limit).
                defmt::warn!("ws2812: PWM sequence start failed: {:?}", e);
                Err(())
            }
        }
    }

    // --- Event handlers (dispatched by the `#[processor]` macro) ---

    /// Local battery level. The event's `charge_state` is ignored because RMK
    /// never runs its charging reader (see `sample_power`).
    async fn on_battery_status_event(&mut self, event: BatteryStatusEvent) {
        let level = match event.0 {
            BatteryStatus::Available { level, .. } => level,
            BatteryStatus::Unavailable => None,
        };
        self.status.set_battery(level);
        self.refresh(Self::now()).await;
    }

    /// Central view of the split link.
    async fn on_peripheral_connected_event(&mut self, event: PeripheralConnectedEvent) {
        if self.indicator.role() == Role::Central {
            let now = Self::now();
            self.status.set_peer(event.connected, now);
            self.refresh(now).await;
        }
    }

    /// Peripheral view of the split link.
    async fn on_central_connected_event(&mut self, event: CentralConnectedEvent) {
        if self.indicator.role() == Role::Peripheral {
            let now = Self::now();
            self.status.set_peer(event.connected, now);
            self.refresh(now).await;
        }
    }

    /// Host link state, profile and output route. On the peripheral this is
    /// the central's status, mirrored over the split link.
    async fn on_connection_status_change_event(&mut self, event: ConnectionStatusChangeEvent) {
        let status = event.0;
        let now = Self::now();
        let state = match status.ble.state {
            BleState::Advertising => HostState::Advertising,
            BleState::Connected => HostState::Connected,
            BleState::Inactive => HostState::Inactive,
        };
        self.status.set_host(state, status.ble.profile, now);
        // Mirrors RMK's private `ConnectionStatus::usb_ready` (rmk-types/src/connection.rs).
        let usb_host = matches!(status.usb, UsbState::Configured | UsbState::Suspended);
        let output_ble = status.decide_active() == Some(ConnectionType::Ble);
        self.status.set_output(usb_host, output_ble);
        self.refresh(now).await;
    }

    /// Highest active layer, published by the keymap (central) or mirrored
    /// over the split link (peripheral).
    async fn on_layer_change_event(&mut self, event: LayerChangeEvent) {
        self.status.set_layer(event.0);
        self.refresh(Self::now()).await;
    }
}

impl DeadlineProcessor for Ws2812Driver {
    fn deadline(&self) -> Option<Instant> {
        // `as_millis` truncates ticks to whole milliseconds, so a timer set to
        // exactly `next_deadline` wakes up reading one millisecond less and
        // would re-arm the same deadline; aim one millisecond past it.
        Some(Instant::from_millis(self.next_deadline + 1))
    }

    async fn on_deadline(&mut self) {
        let now = Self::now();
        self.sample_power(now);
        self.refresh(now).await;
    }
}
