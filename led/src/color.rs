//! Colours in WS2812 GRB order and the configurable colour table.
//!
//! The values a user is likely to change are defined here: the layer
//! colours, the output warning colour, the gauge thresholds and the
//! brightness.

/// A pixel colour in the WS2812 transmission order (green, red, blue).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Grb {
    pub g: u8,
    pub r: u8,
    pub b: u8,
}

impl Grb {
    /// Build from the usual red, green, blue order.
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { g, r, b }
    }

    pub const fn is_off(self) -> bool {
        self.g == 0 && self.r == 0 && self.b == 0
    }
}

/// Steady brightness of every indication.
pub const LEVEL: u8 = 0x10;
/// Peak of the charging breathing curve.
pub const BREATH_PEAK: u8 = 0x20;

pub const OFF: Grb = Grb::new(0, 0, 0);
pub const RED: Grb = Grb::new(LEVEL, 0, 0);
pub const GREEN: Grb = Grb::new(0, LEVEL, 0);
pub const BLUE: Grb = Grb::new(0, 0, LEVEL);
pub const YELLOW: Grb = Grb::new(LEVEL, LEVEL, 0);
pub const WHITE: Grb = Grb::new(LEVEL, LEVEL, LEVEL);
pub const PURPLE: Grb = Grb::new(LEVEL, 0, LEVEL);
pub const ORANGE: Grb = Grb::new(LEVEL, LEVEL / 4, 0);

/// Inner pixel while USB is enumerated by a host but output goes to BLE.
pub const OUTPUT_WARNING: Grb = ORANGE;

/// Colour shown on the outer pixel while `layer` is the highest active
/// layer, or `None` for layers without a colour.
pub const fn layer_color(layer: u8) -> Option<Grb> {
    match layer {
        8 => Some(WHITE),  // Numpad, TG(8)
        9 => Some(PURPLE), // System, OSL(9)
        _ => None,
    }
}

/// Battery percentage at or under which the low-battery warning shows and
/// the gauge turns red.
pub const BATTERY_LOW: u8 = 20;
/// Battery percentage above which the gauge is green; yellow in between.
pub const GAUGE_GREEN_ABOVE: u8 = 50;

/// Gauge colour for a battery level.
pub const fn gauge_color(level: u8) -> Grb {
    if level > GAUGE_GREEN_ABOVE {
        GREEN
    } else if level > BATTERY_LOW {
        YELLOW
    } else {
        RED
    }
}

/// The two pixel colours of one frame, at least one of them lit. The fields
/// are private and [`Lit::try_new`] is the only constructor, so a value of
/// this type never means "both dark".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lit {
    inner: Grb,
    outer: Grb,
}

impl Lit {
    /// `None` when both pixels are off.
    pub const fn try_new(inner: Grb, outer: Grb) -> Option<Self> {
        if inner.is_off() && outer.is_off() {
            None
        } else {
            Some(Self { inner, outer })
        }
    }

    pub const fn inner(self) -> Grb {
        self.inner
    }

    pub const fn outer(self) -> Grb {
        self.outer
    }
}

/// Colour of a BLE profile: BT0 green, BT1 red, BT2 (and anything else) blue.
pub const fn profile_color(profile: u8) -> Grb {
    match profile {
        0 => GREEN,
        1 => RED,
        _ => BLUE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_table_colours_only_the_configured_layers() {
        assert_eq!(layer_color(8), Some(WHITE));
        assert_eq!(layer_color(9), Some(PURPLE));
        for layer in [0, 1, 2, 3, 4, 5, 6, 7, 10, 255] {
            assert_eq!(layer_color(layer), None, "layer {layer}");
        }
    }

    #[test]
    fn gauge_bands_split_at_50_and_20() {
        assert_eq!(gauge_color(100), GREEN);
        assert_eq!(gauge_color(51), GREEN);
        assert_eq!(gauge_color(50), YELLOW);
        assert_eq!(gauge_color(21), YELLOW);
        assert_eq!(gauge_color(20), RED);
        assert_eq!(gauge_color(0), RED);
    }

    #[test]
    fn profile_colours_follow_the_manual() {
        assert_eq!(profile_color(0), GREEN);
        assert_eq!(profile_color(1), RED);
        assert_eq!(profile_color(2), BLUE);
        assert_eq!(profile_color(7), BLUE);
    }

    #[test]
    fn lit_needs_at_least_one_lit_pixel() {
        assert_eq!(Lit::try_new(OFF, OFF), None);
        let lit = Lit::try_new(WHITE, OFF).unwrap();
        assert_eq!((lit.inner(), lit.outer()), (WHITE, OFF));
        let lit = Lit::try_new(OFF, GREEN).unwrap();
        assert_eq!((lit.inner(), lit.outer()), (OFF, GREEN));
    }

    #[test]
    fn off_is_all_zero() {
        assert!(OFF.is_off());
        assert!(!RED.is_off());
        assert_eq!(Grb::new(1, 2, 3), Grb { g: 2, r: 1, b: 3 });
    }
}
