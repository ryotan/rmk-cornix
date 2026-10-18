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
  (MIT).
- The pinned RMK fork requests 1M PHY for the split link and 2M PHY for the
  host link. `use_2m_phy = true` enables 2M PHY support in the BLE controller;
  the fork's split-link code requests 1M PHY separately.
- The keymap block in `keyboard.toml` is generated from a Pipette snapshot
  outside this repository. Edit the snapshot and regenerate the block to keep
  changes across future imports.

# Usage

1. Make any changes you want for the firmware.

2. Build the firmware. Execute in the repository root:
   ```sh
   mise run build
   ```
   This writes `firmware/rmk-cornix-central.uf2` and
   `firmware/rmk-cornix-peripheral.uf2`. Requires rustup and `mise install`
   (see `.config/mise/config.toml`).

   For a GitHub build, open Actions, select a successful *Build RMK firmware*
   run for the desired commit on `main`, and download its `rmk-cornix-uf2`
   artifact. Extract both UF2 files into `firmware/`.

3. Flash the left half with `./flash.sh left` and the right half with
   `./flash.sh right`; the script explains when you have to forget the
   keyboard in your computer's Bluetooth settings and pair again.
