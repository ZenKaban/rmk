# Ergohaven K:04 Qube

RMK BLE dongle firmware for K:04 with a Qube ST7789 status screen.

This target is intentionally separate from `keyboards/k04`:

- `qube` is the USB HID central/dongle with the ST7789 display.
- `left` and `right` are BLE peripherals with ids `0` and `1`.
- RMK comes from `../../common/rmk-main-upstream`, vendored from official
  upstream `https://github.com/HaoboGu/rmk` main.

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

## Scope

The first K:04 Qube target covers matrix, split BLE, two encoders, battery
telemetry, and the Qube status screen. The existing `keyboards/k04` target keeps
the older custom layer LED, trackball, touchpad, and module-settings runtime
until those modules are ported to RMK main.

## Battery

This firmware does not use RMK's `battery_adc_pin` codegen path. The halves use
`src/battery_nrf.rs`, which samples `P0_31` without `calibrate().await` and
re-publishes `BatteryStatusEvent` periodically.
