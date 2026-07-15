# Ergohaven OP36 Qube

RMK BLE dongle firmware for OP36 with a Qube ST7789 status screen.

This target is intentionally separate from `keyboards/op36`:

- `qube` is the USB HID central/dongle with the ST7789 display.
- `left` and `right` are BLE peripherals with ids `0` and `1`.
- RMK comes from the root workspace crates (`../../rmk`, `../../rmk-types`),
  synced from official upstream `https://github.com/HaoboGu/rmk` main.

## Build

```sh
cargo build --release --bin qube --features qube
cargo build --release --bin left
cargo build --release --bin right
```

Or build UF2 files with:

```sh
cargo make uf2
```

## Battery

This firmware does not use RMK's `battery_adc_pin` codegen path. The halves use
`src/battery_nrf.rs`, which samples `P0_31` without `calibrate().await` and
re-publishes `BatteryStatusEvent` periodically. Details are in
`docs/known-issues/battery-dongle-split-message.md`.
