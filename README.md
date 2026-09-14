# RMK Configuration for Cornix Keyboard

This repository contains an unofficial [RMK](https://rmk.rs/) configuration for
the Cornix keyboard by Jezail Funder. It aims to help users to customize their
own RMK firmware for Cornix, not to replicate the official firmware.

# Features

- It supports all keys and rotary encoders.
- It supports Vial.
- Its Vial layout is roughly compatible with the official firmware, so you can
  load your existing Vial layout (`.vil` file) without much modification.
  Macros, combos, tap dances and encoder keymaps may be lost or changed, so
  check them after loading.

# Notes

- WS2812 status LEDs show the BLE profile and host link, the split link,
  charging, a battery gauge, the active layer on layers 8 and 9, and a warning
  when USB is connected to a host but output goes to BLE. The rendering path is
  ported from [numachang/cornix-rmk-custom](https://github.com/numachang/cornix-rmk-custom)
  (MIT). The logic is in the dependency-free `led/` crate, unit-tested with
  `mise run test`.
- BLE and power consumption are not optimized.

# Usage

1. Make any changes you want for the firmware.

2. Build the firmware. Execute in the repository root:
   ```sh
   mise run build
   ```
   This writes `firmware/rmk-cornix-central.uf2` and
   `firmware/rmk-cornix-peripheral.uf2`. Requires rustup and `mise install`
   (see `.config/mise/config.toml`).

   Otherwise, fork this repository, go to GitHub Actions tab, tap *Build RMK
   firmware*, and download the `rmk-cornix-uf2` artifact into `firmware/`
   when the build is done.

3. Flash the left half with `./flash.sh left` and the right half with
   `./flash.sh right`; the script explains when you have to forget the
   keyboard in your computer's Bluetooth settings and pair again.
