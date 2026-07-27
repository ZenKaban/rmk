# Ergohaven RMK Firmware

RMK BLE split firmware for Ergohaven keyboards and trackballs (nRF52840).

## Supported Devices

### Keyboards (BLE split)

| Keyboard    | Layout         | Encoders | Trackball |
|-------------|----------------|----------|-----------|
| K:03        | 5×6 + 5 thumb  | 3+3      | —         |
| K:04 Series | K:04 / Mini / Micro | 1+1 | —         |
| K:04 Series + Qube | K:04 / Mini / Micro | 1+1 | Qube dongle + ST7789 |
| Imperial44  | 4×6 + 3 thumb  | 1+1      | —         |
| OP36        | 3×5 + 3 thumb  | —        | —         |
| Classic splits + Qube | K:03 / Velvet / Imperial44 / OP36 | model-specific | Qube dongle + ST7789 |
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

Current K:04/OP36 regression matrix:

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

## Releases

Packaged UF2 firmware is published on the
[GitHub Releases](https://github.com/ergohaven/rmk/releases) page. The current
firmware release is `v0.1.3`.

## RMK Version

Based on [RMK](https://github.com/HaoboGu/rmk) 0.8.2 with nRF52840 BLE support.

The root `rmk`, `rmk-macro`, `rmk-types`, and `rmk-config` crates are the
source of truth for firmware targets in this repository. K:04 without Qube
and K:04 with Qube each build K:04, Mini, and Micro profiles from one shared
crate. They use the same BLE split connection engine; only the central topology
differs: a central with a local matrix and one peripheral, or a matrix-less
Qube central with two peripheral halves.
