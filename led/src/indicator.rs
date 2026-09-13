//! Picks a colour per pixel from the behaviour tables, plus the next wake-up.
//!
//! Both pixels are normally dark. Rows are checked top to bottom and the
//! first one that applies decides the colour. The tables are the `inner` and
//! `outer` functions below, in the same order. Pixel 0 is the inner LED
//! (towards the middle of the keyboard), pixel 1 the outer one. Every pulse
//! lasts [`PULSE_MS`]; blinks use [`BLINK_PERIOD_MS`] / [`BLINK_ON_MS`]; the
//! split-lost blink stops after [`BLINK_MAX_PERIODS`].
//!
//! Left half (central), inner:
//! 1. Split link lost (including not yet linked after boot): blue blink, capped.
//! 2. Split link just established: blue pulse.
//! 3. USB enumerated by a host but output goes to BLE: orange, steady.
//! 4. A coloured layer is active, level known, no VBUS: battery gauge, steady
//!    (green above 50 %, yellow 21–50 %, red at or below 20 %).
//! 5. A coloured layer is active, VBUS present, not charging: green, steady.
//! 6. No VBUS, level at or below 20 %: red double blink for the first 5 s of
//!    every minute, dark for the rest.
//! 7. Charging stopped with VBUS present, first 3 s: green pulse, once.
//! 8. Charging (VBUS present, charger reports charging): green breathing.
//! 9. Otherwise off.
//!
//! Left half, outer:
//! 1. A coloured layer is active: the layer colour, steady (see
//!    [`crate::color::layer_color`]).
//! 2. Advertising: blink in the profile colour, for as long as it advertises.
//! 3. Host just connected: profile colour pulse.
//! 4. Otherwise off.
//!
//! Right half (peripheral), inner: rows 3–9 of the left inner pixel, with the
//! right half's own level, VBUS and charging state.
//!
//! Right half, outer: the split link as blink and pulse (rows 1–2 of the left
//! inner pixel), then the layer colour, then off. The right half receives the
//! output route and the layer from the central over the split link.
//!
//! On the left half nothing hides the layer colour. The profile keys and
//! Switch Output are on layer 9, so the layer must stay visible while a
//! profile advertises and while the output warning shows; that is why the
//! warning is on the inner pixel. On the right half the split link stays
//! above the layer colour: while unlinked, the right half neither reaches the
//! host nor receives layer changes.
//!
//! "VBUS present, not charging" cannot tell a full battery from the power
//! switch being off (measured: the level reads 100 % either way), so row 5
//! also lights with the switch off on USB.

use crate::color::{
    gauge_color, layer_color, profile_color, Grb, BATTERY_LOW, BLUE, BREATH_PEAK, GREEN, OFF,
    OUTPUT_WARNING, RED,
};
use crate::status::{HostState, Ms, Status};

/// Which half this indicator runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Central,
    Peripheral,
}

/// One rendered frame and when it can next change on its own, or `None`
/// when only a new fact can change it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub inner: Grb,
    pub outer: Grb,
    pub next_deadline: Option<Ms>,
}

/// Re-evaluate this often while a blink or the breathing is showing.
pub const FRAME_MS: Ms = 33;
/// Length of every pulse: shown for this long, then dark.
pub const PULSE_MS: Ms = 3000;
pub const BLINK_PERIOD_MS: Ms = 1000;
pub const BLINK_ON_MS: Ms = 400;
/// The split-lost blink stops after this many periods.
pub const BLINK_MAX_PERIODS: u64 = 10;
pub const BREATH_PERIOD_MS: Ms = 2000;
/// The low-battery warning flashes twice per [`BLINK_PERIOD_MS`], each flash this long.
pub const DOUBLE_BLINK_FLASH_MS: Ms = 200;
/// Period of the low-battery warning.
pub const LOW_BATTERY_PERIOD_MS: Ms = 60_000;
/// The low-battery warning blinks for this long at the start of each
/// [`LOW_BATTERY_PERIOD_MS`], and is dark for the rest of it.
pub const LOW_BATTERY_BURST_MS: Ms = 5_000;

/// Time since `since`, or 0 when `now` is earlier: a clock that goes
/// backwards must not produce a wrapped elapsed time.
fn elapsed(now: Ms, since: Ms) -> Ms {
    now.saturating_sub(since)
}

/// How the colour picked for a pixel changes with time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    /// Steady or off: only a new fact changes it.
    Never,
    /// A blink or the breathing: re-evaluate every frame.
    EveryFrame,
    /// Steady until this moment, then something else.
    At(Ms),
}

impl Change {
    fn deadline(self, now: Ms) -> Option<Ms> {
        match self {
            Change::Never => None,
            Change::EveryFrame => Some(now + FRAME_MS),
            Change::At(end) => Some(end),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pick {
    color: Grb,
    change: Change,
}

impl Pick {
    const fn steady(color: Grb) -> Self {
        Self {
            color,
            change: Change::Never,
        }
    }

    const fn animated(color: Grb) -> Self {
        Self {
            color,
            change: Change::EveryFrame,
        }
    }

    const fn until(color: Grb, end: Ms) -> Self {
        Self {
            color,
            change: Change::At(end),
        }
    }
}

/// `color` during the on part of each blink period, dark otherwise.
fn blink(elapsed: Ms, color: Grb) -> Grb {
    if elapsed % BLINK_PERIOD_MS < BLINK_ON_MS {
        color
    } else {
        OFF
    }
}

/// The charging curve: a triangle from dark to `BREATH_PEAK` and back over
/// one period (visually close enough to a cosine, and integer-only).
fn breath(elapsed: Ms) -> Grb {
    let t = elapsed % BREATH_PERIOD_MS;
    let half = BREATH_PERIOD_MS / 2;
    let up = if t <= half { t } else { BREATH_PERIOD_MS - t };
    // `up <= half`, so the level never exceeds the peak; the fallback only
    // guards the conversion, it is not a reachable value.
    let level = u8::try_from(up * u64::from(BREATH_PEAK) / half).unwrap_or(BREATH_PEAK);
    Grb::new(0, level, 0)
}

/// On phase of the low-battery double blink; see [`DOUBLE_BLINK_FLASH_MS`].
fn double_blink_on(now: Ms) -> bool {
    let phase = now % BLINK_PERIOD_MS;
    let second_flash = 2 * DOUBLE_BLINK_FLASH_MS..3 * DOUBLE_BLINK_FLASH_MS;
    phase < DOUBLE_BLINK_FLASH_MS || second_flash.contains(&phase)
}

/// The low-battery warning: double blinks during the first
/// [`LOW_BATTERY_BURST_MS`] of every [`LOW_BATTERY_PERIOD_MS`], then dark until
/// the next period starts (no wake-ups in between).
fn low_battery_warning(now: Ms) -> Pick {
    let phase = now % LOW_BATTERY_PERIOD_MS;
    if phase < LOW_BATTERY_BURST_MS {
        Pick::animated(if double_blink_on(now) { RED } else { OFF })
    } else {
        Pick::until(OFF, now - phase + LOW_BATTERY_PERIOD_MS)
    }
}

/// Picks colours for one half. Holds only the role; the colours are computed
/// from the `Status` and `now` passed to [`Indicator::evaluate`].
#[derive(Clone, Copy, Debug)]
pub struct Indicator {
    role: Role,
}

impl Indicator {
    pub const fn new(role: Role) -> Self {
        Self { role }
    }

    pub const fn role(&self) -> Role {
        self.role
    }

    /// Pick the colour of both pixels and the earliest moment either changes
    /// on its own (the next blink frame or a pulse end); `next_deadline` is
    /// `None` when both are steady or off.
    ///
    /// `now` must be the same clock the `Status::set_*` timestamps came from
    /// and must not run backwards; if it does, elapsed times clamp to zero.
    pub fn evaluate(&self, s: &Status, now: Ms) -> Frame {
        let inner = self.inner(s, now);
        let outer = self.outer(s, now);
        Frame {
            inner: inner.color,
            outer: outer.color,
            next_deadline: match (inner.change.deadline(now), outer.change.deadline(now)) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            },
        }
    }

    fn inner(&self, s: &Status, now: Ms) -> Pick {
        if self.role == Role::Central {
            if let Some(pick) = Self::peer_link(s, now) {
                return pick;
            }
        }
        if s.output.usb_host && s.output.output_ble {
            return Pick::steady(OUTPUT_WARNING);
        }
        let coloured_layer = layer_color(s.layer).is_some();
        if coloured_layer && !s.power.vbus {
            if let Some(level) = s.battery {
                return Pick::steady(gauge_color(level));
            }
        }
        if coloured_layer && s.power.vbus && !s.power.charging {
            return Pick::steady(GREEN);
        }
        if !s.power.vbus && s.battery.is_some_and(|level| level <= BATTERY_LOW) {
            return low_battery_warning(now);
        }
        if let Some(done_at) = s.power.done_at {
            if elapsed(now, done_at) < PULSE_MS {
                return Pick::until(GREEN, done_at + PULSE_MS);
            }
        }
        if s.power.charging {
            return Pick::animated(breath(elapsed(now, s.power.charging_since)));
        }
        Pick::steady(OFF)
    }

    /// The split link as a pick: capped blue blink while lost, blue pulse
    /// just after linking, otherwise `None` (the row does not apply).
    fn peer_link(s: &Status, now: Ms) -> Option<Pick> {
        let age = elapsed(now, s.peer.since);
        if !s.peer.connected {
            (age < BLINK_PERIOD_MS * BLINK_MAX_PERIODS).then(|| Pick::animated(blink(age, BLUE)))
        } else {
            (age < PULSE_MS).then(|| Pick::until(BLUE, s.peer.since + PULSE_MS))
        }
    }

    fn outer(&self, s: &Status, now: Ms) -> Pick {
        match self.role {
            Role::Central => {
                if let Some(color) = layer_color(s.layer) {
                    return Pick::steady(color);
                }
                let age = elapsed(now, s.host.since);
                let color = profile_color(s.host.profile);
                match s.host.state {
                    HostState::Advertising => return Pick::animated(blink(age, color)),
                    HostState::Connected if age < PULSE_MS => {
                        return Pick::until(color, s.host.since + PULSE_MS);
                    }
                    HostState::Connected | HostState::Inactive => {}
                }
            }
            Role::Peripheral => {
                if let Some(pick) = Self::peer_link(s, now) {
                    return pick;
                }
            }
        }
        if let Some(color) = layer_color(s.layer) {
            return Pick::steady(color);
        }
        Pick::steady(OFF)
    }
}

#[cfg(test)]
mod outer_tests {
    use super::*;
    use crate::color::*;
    use crate::status::{HostState, Status};

    fn central() -> Indicator {
        Indicator::new(Role::Central)
    }

    fn peripheral() -> Indicator {
        Indicator::new(Role::Peripheral)
    }

    fn outer(ind: &Indicator, s: &Status, now: Ms) -> Grb {
        ind.evaluate(s, now).outer
    }

    #[test]
    fn dark_when_nothing_is_happening() {
        let mut s = Status::new();
        s.set_peer(true, 0);
        assert_eq!(outer(&central(), &s, 5_000), OFF);
        assert_eq!(outer(&peripheral(), &s, 5_000), OFF);
    }

    #[test]
    fn the_output_warning_never_hides_the_layer_colour() {
        let mut s = Status::new();
        s.set_peer(true, 0);
        s.set_host(HostState::Connected, 0, 0);
        s.set_layer(9);
        s.set_output(true, true);
        assert_eq!(outer(&central(), &s, 10_000), PURPLE);
        assert_eq!(outer(&peripheral(), &s, 10_000), PURPLE);
    }

    #[test]
    fn central_blinks_the_profile_colour_while_advertising_without_a_cap() {
        let mut s = Status::new();
        s.set_peer(true, 0);
        s.set_host(HostState::Advertising, 1, 0);
        let ind = central();
        assert_eq!(outer(&ind, &s, 100), RED);
        assert_eq!(outer(&ind, &s, 500), OFF);
        assert_eq!(outer(&ind, &s, 1_100), RED);
        assert_eq!(
            outer(&ind, &s, 60_100),
            RED,
            "still blinking after a minute"
        );
        s.set_host(HostState::Advertising, 2, 60_000);
        assert_eq!(
            outer(&ind, &s, 60_100),
            BLUE,
            "profile change changes the colour"
        );
    }

    #[test]
    fn central_shows_the_profile_colour_for_three_seconds_after_connecting() {
        let mut s = Status::new();
        s.set_peer(true, 0);
        s.set_host(HostState::Connected, 0, 1_000);
        let ind = central();
        assert_eq!(outer(&ind, &s, 1_000), GREEN);
        assert_eq!(outer(&ind, &s, 3_999), GREEN);
        assert_eq!(outer(&ind, &s, 4_000), OFF);
    }

    #[test]
    fn a_re_reported_connected_does_not_pulse_again() {
        let mut s = Status::new();
        s.set_peer(true, 0);
        s.set_host(HostState::Connected, 0, 1_000);
        s.set_host(HostState::Connected, 0, 2_500);
        assert_eq!(
            outer(&central(), &s, 2_600),
            GREEN,
            "still inside the first pulse"
        );
        assert_eq!(outer(&central(), &s, 4_500), OFF, "no second pulse");
    }

    #[test]
    fn layer_colour_is_steady_on_both_halves() {
        let mut s = Status::new();
        s.set_peer(true, 0);
        s.set_layer(8);
        assert_eq!(outer(&central(), &s, 10_000), WHITE);
        assert_eq!(outer(&peripheral(), &s, 10_000), WHITE);
        s.set_layer(9);
        assert_eq!(outer(&central(), &s, 10_000), PURPLE);
        s.set_layer(3);
        assert_eq!(outer(&central(), &s, 10_000), OFF);
    }

    #[test]
    fn central_layer_colour_outranks_the_host_link() {
        let mut s = Status::new();
        s.set_peer(true, 0);
        s.set_layer(9);
        s.set_host(HostState::Advertising, 1, 10_000);
        assert_eq!(outer(&central(), &s, 10_100), PURPLE, "blink hidden");
        assert_eq!(
            outer(&central(), &s, 10_500),
            PURPLE,
            "also in the off phase"
        );
        s.set_layer(0);
        assert_eq!(
            outer(&central(), &s, 11_100),
            RED,
            "blink once the layer clears"
        );
        s.set_layer(8);
        s.set_host(HostState::Connected, 2, 12_000);
        assert_eq!(outer(&central(), &s, 12_500), WHITE, "pulse hidden");
        s.set_layer(0);
        assert_eq!(
            outer(&central(), &s, 13_000),
            BLUE,
            "pulse once the layer clears"
        );
        assert_eq!(outer(&central(), &s, 15_000), OFF, "pulse over");
    }

    #[test]
    fn peripheral_layer_colour_stays_below_the_link_rows() {
        let mut s = Status::new();
        s.set_layer(8);
        assert_eq!(outer(&peripheral(), &s, 100), BLUE, "unlinked blink first");
        assert_eq!(
            outer(&peripheral(), &s, 500),
            OFF,
            "blink off phase stays dark"
        );
        assert_eq!(
            outer(&peripheral(), &s, 10_100),
            WHITE,
            "layer colour after the cap"
        );
        s.set_peer(true, 20_000);
        assert_eq!(outer(&peripheral(), &s, 21_000), BLUE, "link pulse first");
        assert_eq!(
            outer(&peripheral(), &s, 23_000),
            WHITE,
            "layer colour when it ends"
        );
    }

    #[test]
    fn peripheral_blinks_blue_while_unlinked_for_ten_periods() {
        let s = Status::new();
        let ind = peripheral();
        assert_eq!(outer(&ind, &s, 100), BLUE);
        assert_eq!(outer(&ind, &s, 500), OFF);
        assert_eq!(outer(&ind, &s, 9_100), BLUE);
        assert_eq!(outer(&ind, &s, 10_100), OFF, "cap reached");
        assert_eq!(outer(&ind, &s, 20_100), OFF);
    }

    #[test]
    fn peripheral_shows_blue_for_three_seconds_after_linking() {
        let mut s = Status::new();
        s.set_peer(true, 2_000);
        let ind = peripheral();
        assert_eq!(outer(&ind, &s, 2_500), BLUE);
        assert_eq!(outer(&ind, &s, 5_000), OFF);
    }

    #[test]
    fn peripheral_ignores_the_host_link_and_central_ignores_peer_on_the_outer() {
        let mut s = Status::new();
        s.set_host(HostState::Advertising, 0, 0);
        assert_eq!(
            outer(&peripheral(), &s, 100),
            BLUE,
            "peer blink, not the profile blink"
        );
        s.set_peer(true, 0);
        assert_eq!(outer(&peripheral(), &s, 5_000), OFF);
        let c = Status::new();
        assert_eq!(
            outer(&central(), &c, 100),
            OFF,
            "the central shows the split link on the inner pixel"
        );
    }
}

#[cfg(test)]
mod inner_tests {
    use super::*;
    use crate::color::*;
    use crate::status::{Status, Vbus};

    fn central() -> Indicator {
        Indicator::new(Role::Central)
    }

    fn peripheral() -> Indicator {
        Indicator::new(Role::Peripheral)
    }

    fn inner(ind: &Indicator, s: &Status, now: Ms) -> Grb {
        ind.evaluate(s, now).inner
    }

    /// A linked status on battery power with no layer colour: the baseline.
    fn quiet() -> Status {
        let mut s = Status::new();
        s.set_peer(true, 0);
        s.set_battery(Some(80));
        s
    }

    #[test]
    fn dark_when_nothing_is_happening() {
        assert_eq!(inner(&central(), &quiet(), 20_000), OFF);
        assert_eq!(inner(&peripheral(), &quiet(), 20_000), OFF);
    }

    #[test]
    fn central_blinks_blue_while_unlinked_for_ten_periods_then_stops() {
        let s = Status::new();
        let ind = central();
        assert_eq!(inner(&ind, &s, 100), BLUE);
        assert_eq!(inner(&ind, &s, 500), OFF);
        assert_eq!(inner(&ind, &s, 9_100), BLUE);
        assert_eq!(inner(&ind, &s, 10_100), OFF);
    }

    #[test]
    fn central_shows_blue_for_three_seconds_after_linking() {
        let mut s = quiet();
        s.set_peer(false, 1_000);
        s.set_peer(true, 2_000);
        let ind = central();
        assert_eq!(inner(&ind, &s, 2_000), BLUE);
        assert_eq!(inner(&ind, &s, 4_999), BLUE);
        assert_eq!(inner(&ind, &s, 5_000), OFF);
    }

    #[test]
    fn peripheral_keeps_the_peer_link_off_the_inner_pixel() {
        let s = Status::new();
        assert_eq!(inner(&peripheral(), &s, 100), OFF);
    }

    #[test]
    fn gauge_shows_on_a_coloured_layer_on_battery_power() {
        let mut s = quiet();
        s.set_layer(8);
        assert_eq!(inner(&central(), &s, 20_000), GREEN);
        s.set_battery(Some(30));
        assert_eq!(inner(&central(), &s, 20_000), YELLOW);
        assert_eq!(inner(&peripheral(), &s, 20_000), YELLOW);
        s.set_battery(Some(10));
        assert_eq!(
            inner(&central(), &s, 20_100),
            RED,
            "steady red, not the double blink"
        );
        assert_eq!(inner(&central(), &s, 20_300), RED);
        s.set_layer(3);
        s.set_battery(Some(80));
        assert_eq!(inner(&central(), &s, 20_000), OFF, "layer 3 has no colour");
    }

    #[test]
    fn gauge_needs_a_known_level() {
        let mut s = quiet();
        s.set_layer(9);
        s.set_battery(None);
        assert_eq!(inner(&central(), &s, 20_000), OFF);
    }

    #[test]
    fn on_vbus_a_coloured_layer_shows_steady_green_unless_charging() {
        let mut s = quiet();
        s.set_layer(8);
        s.set_power(Vbus::Present { charging: false }, 0);
        assert_eq!(inner(&central(), &s, 20_000), GREEN);
        assert_eq!(inner(&central(), &s, 60_000), GREEN, "steady, no timeout");
        assert_eq!(inner(&peripheral(), &s, 60_000), GREEN);
        s.set_power(Vbus::Present { charging: true }, 60_000);
        assert_eq!(
            inner(&central(), &s, 61_000),
            Grb::new(0, BREATH_PEAK, 0),
            "breathing, not the gauge"
        );
    }

    #[test]
    fn on_vbus_the_steady_green_does_not_need_a_known_level() {
        let mut s = quiet();
        s.set_layer(8);
        s.set_battery(None);
        s.set_power(Vbus::Present { charging: false }, 0);
        assert_eq!(inner(&central(), &s, 20_000), GREEN);
    }

    #[test]
    fn peripheral_inner_ignores_a_fresh_link_too() {
        let mut s = Status::new();
        s.set_peer(true, 2_000);
        assert_eq!(inner(&peripheral(), &s, 2_500), OFF);
    }

    #[test]
    fn low_battery_double_blinks_on_battery_power_only() {
        let mut s = quiet();
        s.set_battery(Some(20));
        let ind = central();
        // Inside the 5 s burst at the start of a minute.
        assert_eq!(inner(&ind, &s, 60_100), RED);
        assert_eq!(inner(&ind, &s, 60_300), OFF);
        assert_eq!(inner(&ind, &s, 60_500), RED);
        assert_eq!(inner(&ind, &s, 60_700), OFF);
        assert_eq!(inner(&ind, &s, 64_100), RED, "last second of the burst");
        s.set_battery(Some(21));
        assert_eq!(inner(&ind, &s, 60_100), OFF);
        s.set_battery(Some(5));
        s.set_power(Vbus::Present { charging: false }, 0);
        assert_eq!(
            inner(&ind, &s, 60_100),
            OFF,
            "the level is meaningless on VBUS"
        );
    }

    #[test]
    fn low_battery_rests_for_the_remaining_fifty_five_seconds() {
        let mut s = quiet();
        s.set_battery(Some(10));
        let ind = central();
        assert_eq!(inner(&ind, &s, 65_100), OFF);
        assert_eq!(inner(&ind, &s, 119_900), OFF);
        assert_eq!(
            inner(&ind, &s, 120_100),
            RED,
            "the next minute starts a new burst"
        );
    }

    #[test]
    fn charging_breathes_green_from_the_edge() {
        // Past the link pulse that `quiet()` starts at 0.
        let mut s = quiet();
        s.set_power(Vbus::Present { charging: true }, 10_000);
        let ind = central();
        assert_eq!(inner(&ind, &s, 10_000), OFF, "the curve starts dark");
        assert_eq!(inner(&ind, &s, 10_500), Grb::new(0, BREATH_PEAK / 2, 0));
        assert_eq!(inner(&ind, &s, 11_000), Grb::new(0, BREATH_PEAK, 0));
        assert_eq!(inner(&ind, &s, 12_000), OFF);
        assert_eq!(
            inner(&peripheral(), &s, 11_000),
            Grb::new(0, BREATH_PEAK, 0)
        );
    }

    #[test]
    fn charge_done_pulses_green_once() {
        let mut s = quiet();
        s.set_power(Vbus::Present { charging: true }, 0);
        s.set_power(Vbus::Present { charging: false }, 5_000);
        let ind = central();
        assert_eq!(inner(&ind, &s, 5_000), GREEN);
        assert_eq!(inner(&ind, &s, 7_999), GREEN);
        assert_eq!(inner(&ind, &s, 8_000), OFF);
        assert_eq!(inner(&peripheral(), &s, 6_000), GREEN);
    }

    #[test]
    fn a_link_edge_does_not_replay_the_charge_done_pulse() {
        let mut s = quiet();
        s.set_power(Vbus::Present { charging: true }, 0);
        s.set_power(Vbus::Present { charging: false }, 5_000);
        s.set_peer(false, 6_000);
        s.set_peer(true, 6_500);
        let ind = central();
        assert_eq!(inner(&ind, &s, 6_600), BLUE, "the link pulse is above");
        assert_eq!(inner(&ind, &s, 9_500), OFF, "no green after the link pulse");
        assert_eq!(inner(&ind, &s, 20_000), OFF);
    }

    #[test]
    fn output_warning_outranks_the_power_rows_on_both_halves() {
        let mut s = quiet();
        s.set_layer(8);
        s.set_power(Vbus::Present { charging: false }, 0);
        assert_eq!(
            inner(&central(), &s, 10_000),
            GREEN,
            "baseline: layer on USB"
        );
        s.set_output(true, true);
        assert_eq!(inner(&central(), &s, 10_000), ORANGE);
        assert_eq!(inner(&peripheral(), &s, 10_000), ORANGE);
        s.set_power(Vbus::Present { charging: true }, 10_000);
        assert_eq!(inner(&central(), &s, 11_000), ORANGE, "above breathing");
        s.set_output(true, false);
        assert_ne!(
            inner(&central(), &s, 11_000),
            ORANGE,
            "USB output: no warning"
        );
        s.set_output(false, true);
        assert_ne!(
            inner(&central(), &s, 11_000),
            ORANGE,
            "no USB host: no warning"
        );
    }

    #[test]
    fn link_rows_outrank_the_output_warning_on_the_central() {
        let mut s = Status::new();
        s.set_output(true, true);
        assert_eq!(
            inner(&central(), &s, 100),
            BLUE,
            "unlinked blink above the warning"
        );
        assert_eq!(
            inner(&peripheral(), &s, 100),
            ORANGE,
            "peripheral: no link row on the inner pixel"
        );
        s.set_peer(true, 1_000);
        assert_eq!(
            inner(&central(), &s, 1_100),
            BLUE,
            "link pulse above the warning"
        );
        assert_eq!(inner(&central(), &s, 5_000), ORANGE, "after the pulse");
    }

    #[test]
    fn link_rows_outrank_power_rows_on_the_central() {
        let mut s = Status::new();
        s.set_power(Vbus::Present { charging: true }, 0);
        assert_eq!(
            inner(&central(), &s, 0),
            BLUE,
            "unlinked blink above breathing"
        );
        assert_eq!(
            inner(&peripheral(), &s, 0),
            OFF,
            "peripheral: breathing starts dark"
        );
        assert_eq!(
            inner(&central(), &s, 1_000),
            BLUE,
            "still the blink at the breathing peak"
        );
    }
}

#[cfg(test)]
mod deadline_tests {
    use super::*;
    use crate::status::{HostState, Status, Vbus};

    fn deadline(role: Role, s: &Status, now: Ms) -> Option<Ms> {
        Indicator::new(role).evaluate(s, now).next_deadline
    }

    #[test]
    fn steady_or_dark_pixels_need_no_wake_up() {
        let mut s = Status::new();
        s.set_peer(true, 0);
        assert_eq!(deadline(Role::Central, &s, 10_000), None);
        s.set_layer(8);
        s.set_battery(Some(70));
        assert_eq!(
            deadline(Role::Central, &s, 10_000),
            None,
            "steady colours are steady"
        );
        s.set_output(true, true);
        assert_eq!(deadline(Role::Peripheral, &s, 10_000), None);
    }

    #[test]
    fn the_low_battery_warning_sleeps_between_bursts() {
        let mut s = Status::new();
        s.set_peer(true, 0);
        s.set_battery(Some(10));
        assert_eq!(
            deadline(Role::Central, &s, 62_000),
            Some(62_033),
            "blinking"
        );
        assert_eq!(
            deadline(Role::Central, &s, 75_000),
            Some(120_000),
            "dark until the next burst"
        );
        assert_eq!(deadline(Role::Peripheral, &s, 75_000), Some(120_000));
    }

    #[test]
    fn a_blink_or_breathing_wakes_every_frame() {
        let mut s = Status::new();
        s.set_peer(true, 0);
        s.set_host(HostState::Advertising, 0, 0);
        assert_eq!(deadline(Role::Central, &s, 10_000), Some(10_033));
        let mut c = Status::new();
        c.set_peer(true, 0);
        c.set_power(Vbus::Present { charging: true }, 0);
        assert_eq!(deadline(Role::Peripheral, &c, 10_000), Some(10_033));
        assert_eq!(
            deadline(Role::Central, &Status::new(), 100),
            Some(133),
            "unlinked blink at boot"
        );
    }

    #[test]
    fn a_pulse_wakes_at_its_end() {
        let mut s = Status::new();
        s.set_peer(false, 500);
        s.set_peer(true, 1_000);
        assert_eq!(deadline(Role::Central, &s, 1_500), Some(4_000));
        assert_eq!(deadline(Role::Peripheral, &s, 1_500), Some(4_000));
        // Past the link pulse that starts at 0.
        let mut c = Status::new();
        c.set_peer(true, 0);
        c.set_host(HostState::Connected, 1, 5_000);
        assert_eq!(deadline(Role::Central, &c, 5_100), Some(8_000));
        c.set_power(Vbus::Present { charging: true }, 0);
        c.set_power(Vbus::Present { charging: false }, 6_000);
        assert_eq!(
            deadline(Role::Central, &c, 6_100),
            Some(8_000),
            "earliest of outer 8000 and inner 9000"
        );
        assert_eq!(
            deadline(Role::Central, &c, 8_500),
            Some(9_000),
            "the inner pulse remains"
        );
    }

    #[test]
    fn the_earliest_pixel_wins_and_a_lone_pixel_counts() {
        let mut s = Status::new();
        s.set_peer(false, 500);
        s.set_peer(true, 1_000);
        s.set_host(HostState::Advertising, 0, 0);
        assert_eq!(
            deadline(Role::Central, &s, 1_500),
            Some(1_533),
            "the outer blink is earlier than the inner pulse end"
        );
        s.set_host(HostState::Inactive, 0, 1_500);
        assert_eq!(
            deadline(Role::Central, &s, 1_500),
            Some(4_000),
            "only the inner pulse is timed"
        );
    }
}
