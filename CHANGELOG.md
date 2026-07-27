# Changelog

## v0.1.3

### Features

- Added complete K:04 Series firmware for K:04, Mini, and Micro in standalone and Qube configurations
- Added left/right battery telemetry and Qube live display data across the complete K:04 Series
- Added module-aware pointing settings, configurable encoder steps, and factory-enabled touchpad acceleration and gestures for the K:04 Series
- Embedded the Ergohaven manufacturer and firmware version `0.1.3` in the released K:04 and trackball definitions and exposed the same version through VIA `id_firmware_version`

### Fixes

- Fixed split wake latency after idle while retaining the power-saving connection interval
- Fixed excessive idle polling for trackball and touchpad modules
- Fixed BLE Vial report framing, host-session responsiveness, discovery compatibility, and split battery updates
- Fixed K:04 settings persistence, Layer LED writes, touch gestures, and Qube pointing runtime parity
