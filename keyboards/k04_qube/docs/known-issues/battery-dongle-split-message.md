# Known issue: split battery on Qube dongle

This target uses the same temporary battery path as `op36_qube`:

- `src/battery_nrf.rs` samples `P0_31` / AIN7 without `calibrate().await`.
- `left` and `right` re-publish `BatteryStatusEvent` periodically.
- `SplitMessage::BatteryStatus` is kept above display-gated variants in the
  vendored RMK main tree so postcard enum indexes do not depend on the
  `display` feature.

The stock RMK `battery_adc_pin` path is not enabled here until nRF SAADC
calibration and split battery heartbeat are hardened upstream.
