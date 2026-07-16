# Ergohaven K:04

No-Qube split BLE firmware for the ordinary K:04 halves.

This target intentionally uses the local K:04 common stack:

- `common/rmk-common-k04`
- `common/rmk-macro-common-k04`
- `common/rmk-types-common-k04`

Keep this separate from `keyboards/k04_qube`: the common path is for ordinary
halves, while the Qube path stays on the root RMK crates because its dongle
connection flow is different and works there.

## Build

```sh
cargo build --release --bin central --bin peripheral
```

The repository build matrix also builds this target:

```sh
./scripts/build_k04_matrix.sh
```
