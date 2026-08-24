# Settings Reset for Ergohaven nRF52840 Keyboards

Small reset firmwares that erase the unified Ergohaven RMK storage partition,
clearing all stored settings:

- Keymaps / Vial configuration
- BLE bonds and profiles
- Layout options

## Usage

1. Put the keyboard into bootloader mode (double-tap reset)
2. Drag the correct reset file to the USB drive:
   - `settings_reset.uf2` for keyboard halves
   - `settings_reset_qube.uf2` for a Qube dongle
3. Device will erase settings, verify erased pages, and enter bootloader again
4. Flash your normal keyboard firmware (.uf2)

## Safe Zones

Both files erase only `0xCC000–0xEC000`. Production firmware linkers reserve
that range, so the application and bootloader are preserved. The two files
differ only because keyboard halves and Qube dongles have different
application origins.

## Compatible Devices

All Ergohaven keyboards with nRF52840 + Adafruit bootloader:

- K:04 / Mini / Micro, standalone and with Qube
- K:03 (both halves)
- Imperial44 (both halves)
- Velvet, including the optional right-hand trackball variant (both halves)
- OP36 (both halves)

For a Qube configuration, use `settings_reset_qube.uf2` on the dongle and
`settings_reset.uf2` on both halves. Then flash the normal dongle, left, and
right firmware again.
