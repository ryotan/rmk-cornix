//! The facts the indicator displays, with the time each timed fact last changed.
//!
//! A fact carries a timestamp only when its display depends on time. The
//! timestamp is updated only when the value changes: pulses and blink caps
//! count from that change, so a re-report of the same value must not
//! restart them.

/// Milliseconds since boot, as passed in by the driver.
pub type Ms = u64;

/// The states of the BLE host link that the indicator distinguishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostState {
    Inactive,
    Advertising,
    Connected,
}

/// The split link to the other half.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Peer {
    pub(crate) connected: bool,
    /// When `connected` last changed; boot counts as "not yet linked".
    pub(crate) since: Ms,
}

/// The BLE host link and the active profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Host {
    pub(crate) state: HostState,
    pub(crate) profile: u8,
    /// When `state` or `profile` last changed.
    pub(crate) since: Ms,
}

/// USB power as the driver samples it. Charging is only possible with VBUS
/// present, so the type has no "charging without VBUS" value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vbus {
    Absent,
    Present { charging: bool },
}

/// USB power and the charger's STAT line, as last set from a [`Vbus`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Power {
    pub(crate) vbus: bool,
    pub(crate) charging: bool,
    /// When `charging` last changed; the breathing phase counts from here.
    pub(crate) charging_since: Ms,
    /// Set when charging stops with VBUS present; cleared when VBUS goes
    /// away or charging restarts.
    pub(crate) done_at: Option<Ms>,
}

/// Where key reports go, derived from RMK's `ConnectionStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Output {
    /// A USB host has enumerated the keyboard (`UsbState::Configured` or `Suspended`).
    pub(crate) usb_host: bool,
    /// RMK's `ConnectionStatus::decide_active()` returned BLE.
    pub(crate) output_ble: bool,
}

/// Everything the indicator knows. Fields are crate-private: outside the
/// crate a `Status` is built with [`Status::new`] and changed only through
/// the `set_*` methods, which update a timestamp only when the value
/// changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub(crate) peer: Peer,
    pub(crate) host: Host,
    pub(crate) power: Power,
    /// Highest active layer.
    pub(crate) layer: u8,
    /// Battery percentage; `None` before the first sample and whenever RMK reports no level.
    pub(crate) battery: Option<u8>,
    pub(crate) output: Output,
}

impl Default for Status {
    fn default() -> Self {
        Self::new()
    }
}

impl Status {
    pub const fn new() -> Self {
        Self {
            peer: Peer {
                connected: false,
                since: 0,
            },
            host: Host {
                state: HostState::Inactive,
                profile: 0,
                since: 0,
            },
            power: Power {
                vbus: false,
                charging: false,
                charging_since: 0,
                done_at: None,
            },
            layer: 0,
            battery: None,
            output: Output {
                usb_host: false,
                output_ble: false,
            },
        }
    }

    pub fn set_peer(&mut self, connected: bool, now: Ms) {
        if self.peer.connected != connected {
            self.peer = Peer {
                connected,
                since: now,
            };
        }
    }

    pub fn set_host(&mut self, state: HostState, profile: u8, now: Ms) {
        if self.host.state != state || self.host.profile != profile {
            self.host = Host {
                state,
                profile,
                since: now,
            };
        }
    }

    pub fn set_power(&mut self, vbus: Vbus, now: Ms) {
        let (vbus, charging) = match vbus {
            Vbus::Absent => (false, false),
            Vbus::Present { charging } => (true, charging),
        };
        if charging != self.power.charging {
            self.power.done_at = if !charging && vbus { Some(now) } else { None };
            self.power.charging = charging;
            self.power.charging_since = now;
        }
        if !vbus {
            self.power.done_at = None;
        }
        self.power.vbus = vbus;
    }

    pub fn set_layer(&mut self, layer: u8) {
        self.layer = layer;
    }

    pub fn set_battery(&mut self, level: Option<u8>) {
        self.battery = level;
    }

    pub fn set_output(&mut self, usb_host: bool, output_ble: bool) {
        self.output = Output {
            usb_host,
            output_ble,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_status_is_unlinked_dark_and_on_layer_0() {
        let s = Status::new();
        assert!(!s.peer.connected);
        assert_eq!(s.peer.since, 0);
        assert_eq!(s.host.state, HostState::Inactive);
        assert!(!s.power.vbus);
        assert!(!s.power.charging);
        assert_eq!(s.power.done_at, None);
        assert_eq!(s.layer, 0);
        assert_eq!(s.battery, None);
        assert!(!s.output.usb_host);
        assert!(!s.output.output_ble);
    }

    #[test]
    fn peer_since_is_updated_only_when_the_value_changes() {
        let mut s = Status::new();
        s.set_peer(true, 100);
        assert_eq!(s.peer.since, 100);
        s.set_peer(true, 500);
        assert_eq!(s.peer.since, 100, "a re-report must not restart the pulse");
        s.set_peer(false, 900);
        assert_eq!((s.peer.connected, s.peer.since), (false, 900));
    }

    #[test]
    fn host_state_and_profile_changes_update_since() {
        let mut s = Status::new();
        s.set_host(HostState::Advertising, 0, 10);
        assert_eq!(s.host.since, 10);
        s.set_host(HostState::Advertising, 0, 20);
        assert_eq!(s.host.since, 10);
        s.set_host(HostState::Advertising, 1, 30);
        assert_eq!((s.host.profile, s.host.since), (1, 30));
        s.set_host(HostState::Connected, 1, 40);
        assert_eq!((s.host.state, s.host.since), (HostState::Connected, 40));
    }

    #[test]
    fn charge_done_is_recorded_only_when_charging_stops_with_vbus() {
        let mut s = Status::new();
        s.set_power(Vbus::Present { charging: true }, 100);
        assert_eq!((s.power.charging, s.power.charging_since), (true, 100));
        s.set_power(Vbus::Present { charging: true }, 133);
        assert_eq!(
            s.power.charging_since, 100,
            "re-sampling must not update the timestamp"
        );
        s.set_power(Vbus::Present { charging: false }, 5000);
        assert_eq!(s.power.done_at, Some(5000));
        s.set_power(Vbus::Present { charging: false }, 5033);
        assert_eq!(s.power.done_at, Some(5000));
        s.set_power(Vbus::Absent, 6000);
        assert_eq!(
            s.power.done_at, None,
            "unplugging ends the charge-done state"
        );
    }

    #[test]
    fn unplugging_while_charging_is_not_a_finished_charge() {
        let mut s = Status::new();
        s.set_power(Vbus::Present { charging: true }, 100);
        s.set_power(Vbus::Absent, 200);
        assert_eq!(s.power.done_at, None);
        assert!(!s.power.charging);
    }

    #[test]
    fn charging_again_clears_charge_done() {
        let mut s = Status::new();
        s.set_power(Vbus::Present { charging: true }, 100);
        s.set_power(Vbus::Present { charging: false }, 200);
        s.set_power(Vbus::Present { charging: true }, 300);
        assert_eq!(s.power.done_at, None);
        assert_eq!(s.power.charging_since, 300);
    }

    #[test]
    fn untimed_facts_are_plain_overwrites() {
        let mut s = Status::new();
        s.set_layer(8);
        s.set_battery(Some(42));
        s.set_output(true, true);
        assert_eq!(s.layer, 8);
        assert_eq!(s.battery, Some(42));
        assert_eq!((s.output.usb_host, s.output.output_ble), (true, true));
        s.set_battery(None);
        assert_eq!(s.battery, None);
    }
}
