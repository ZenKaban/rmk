# Ergohaven RMK Firmware

RMK BLE split firmware for Ergohaven keyboards and trackballs (nRF52840).

## Supported Devices

### Keyboards (BLE split)

| Keyboard    | Layout         | Encoders | Trackball |
|-------------|----------------|----------|-----------|
| K:03        | 5×6 + 5 thumb  | 3+3      | —         |
| K:04        | 5×6 + 5 thumb  | 1+1      | —         |
| K:04 Qube   | 5×6 + 5 thumb  | 1+1      | Qube dongle + ST7789 |
| K:04 Micro  | 3×5 + 3 thumb  | 1+1      | —         |
| Imperial44  | 4×6 + 3 thumb  | 1+1      | —         |
| OP36        | 3×5 + 3 thumb  | —        | —         |
| OP36 Qube   | 3×5 + 3 thumb  | —        | Qube dongle + ST7789 |
| Velvet      | 4×6 + 5 thumb  | —        | —         |
| Velvet UI   | 4×6 + 5 thumb  | —        | PMW3610   |

### Trackballs (standalone BLE)

| Device              | Buttons | Modes                          |
|---------------------|---------|--------------------------------|
| Trackball Royale     | 6       | Normal, Scroll, Sniper, Adjust |
| Trackball Mini v3.1 | 4       | Normal, Scroll, Sniper, Adjust |
| Trackball Mini v3.0 | 2       | Normal, Scroll, Sniper, Adjust |

### Tools

| Tool           | Description                              |
|----------------|------------------------------------------|
| settings_reset | Erases keymap and BLE bonds, resets to bootloader |

## Building

```sh
cd keyboards/k03
cargo build --release --bin central
cargo build --release --bin peripheral
```

Current K:04/OP36 root-RMK regression matrix:

```sh
./scripts/build_k04_matrix.sh
```

## Flashing

1. Put device into bootloader (double-tap reset)
2. Copy `.uf2` file to the mounted USB drive
3. For split keyboards: flash central and peripheral separately

## Settings Reset

Flash `settings_reset.uf2` to erase all saved keymap/BLE data, then re-flash keyboard firmware.

## CI

Every push builds all devices in parallel via GitHub Actions. UF2 artifacts available as build downloads.

## RMK Version

Based on [RMK](https://github.com/HaoboGu/rmk) 0.8.2 with nRF52840 BLE support.

The root `rmk`, `rmk-macro`, `rmk-types`, and `rmk-config` crates are the
current source of truth for all firmware targets in this repository.
