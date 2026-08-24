# Legacy Storage Migration

One-time upgrade utility for non-K:04 firmware that stored RMK settings at
`0xA0000–0xC0000`. It copies the raw 128 KiB partition to the unified
`0xCC000–0xEC000` partition and verifies every page.

Run it **before** flashing firmware that uses the unified profile:

1. Use `storage_migrate.uf2` on keyboard halves.
2. Use `storage_migrate_qube.uf2` on a Qube dongle.
3. The device returns to the bootloader after the copy.
4. Flash the normal new firmware.

For a Qube split, migrate the dongle and both halves. The source partition is
not erased. The utility refuses to write when the destination contains data
that is neither erased nor an exact partial copy, protecting existing K:04
settings and already-initialized unified profiles.

Do not run this utility on K:04 Series; those devices already use the unified
partition.
