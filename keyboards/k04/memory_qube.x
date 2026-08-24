MEMORY
{
  /* NOTE 1 K = 1 KiB = 1024 bytes */
  /* Qube dongle with Adafruit nRF52 bootloader */
  /* Reserve 0xCC000..0xEC000 for RMK storage. */
  FLASH : ORIGIN = 0x00001000, LENGTH = 812K
  RAM : ORIGIN = 0x20000008, LENGTH = 255K
}
