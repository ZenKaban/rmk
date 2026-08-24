# Ergohaven RMK Firmware

Firmware for Ergohaven nRF52840 keyboards and Qube configurations. Built on
[RMK](https://github.com/HaoboGu/rmk).

Ready-to-flash UF2 files are available on the
[Releases](https://github.com/ergohaven/rmk/releases) page.

## Supported Hardware

- Keyboards: K:03, K:04, K:04 Mini, K:04 Micro, Imperial44, OP36, and Velvet
- Qube: K:03, K:04 Series, Imperial44, OP36, and Velvet

## Build

Each firmware package is in `keyboards/`. For example:

```sh
cd keyboards/k03
cargo build --release --bin central --bin peripheral
```

Build every K:04 Series configuration with:

```sh
./scripts/build_k04_matrix.sh
```

Production targets are defined in
[`.github/workflows/build.yml`](.github/workflows/build.yml). Shared firmware
rules are documented in
[`docs/ergohaven-firmware-profile.md`](docs/ergohaven-firmware-profile.md).

## Flash

1. Double-tap reset to enter the bootloader.
2. Copy the correct `.uf2` file to the mounted USB drive.
3. Flash both halves of a standalone split, or the Qube dongle and both halves
   of a Qube configuration.

## Reset and Migration

Use `settings_reset.uf2` for keyboard halves, or `settings_reset_qube.uf2` for
a Qube dongle. Re-flash the normal firmware after the reset.

The one-time `storage_migrate` utilities preserve settings from older
non-K:04 firmware. Upgrade details and the Velvet exception are documented in
the [firmware profile](docs/ergohaven-firmware-profile.md#storage-and-reset).

## Checks

```sh
./scripts/check_ergohaven_profile.sh
./scripts/test_all.sh
```

GitHub Actions validates production profiles and builds all supported devices
on every push and pull request.
