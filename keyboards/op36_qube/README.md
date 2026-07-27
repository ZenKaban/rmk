# Ergohaven Classic Splits + Qube

One shared RMK BLE dongle firmware crate for K:03, Velvet, Imperial44, and
OP36 with a Qube ST7789 status screen. Rust code is shared; every model keeps
its own matrix, factory keymap, Vial definition, Product ID, and layer labels.

This target is intentionally separate from the standalone split targets:

- `qube` is the USB HID central/dongle with the ST7789 display.
- `left` and `right` are BLE peripherals with ids `0` and `1`.
- RMK comes from the root workspace crates (`../../rmk`, `../../rmk-types`),
  synced from official upstream `https://github.com/HaoboGu/rmk` main.

| Profile | Keyboard config | Vial config | Product ID |
|---------|-----------------|-------------|------------|
| OP36 | `keyboard.toml` | `vial.json` | `0x0036` |
| K:03 | `keyboard_k03.toml` | `../k03/vial.json` | `0x0070` |
| Velvet | `keyboard_velvet.toml` | `../velvet/vial.json` | `0x00BE` |
| Imperial44 | `keyboard_imperial44.toml` | `../imperial44/vial.json` | `0x0044` |

## Build

The default profile remains OP36:

```sh
cargo build --release --bin qube --features qube
cargo build --release --bin left
cargo build --release --bin right
```

Select another profile with explicit config paths:

```sh
KEYBOARD_TOML_PATH="$PWD/keyboard_k03.toml" \
VIAL_JSON_PATH="$PWD/../k03/vial.json" \
CARGO_TARGET_DIR=target/k03/qube \
cargo build --release --bin qube --features qube

KEYBOARD_TOML_PATH="$PWD/keyboard_k03.toml" \
VIAL_JSON_PATH="$PWD/../k03/vial.json" \
CARGO_TARGET_DIR=target/k03/halves \
cargo build --release --bin left --bin right
```

## Battery

This firmware does not use RMK's `battery_adc_pin` codegen path. The halves use
`src/battery_nrf.rs`, which samples `P0_31` without `calibrate().await` and
re-publishes `BatteryStatusEvent` periodically. Details are in
`docs/known-issues/battery-dongle-split-message.md`.
