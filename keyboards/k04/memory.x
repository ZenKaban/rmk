MEMORY
{
  /* NOTE 1 K = 1 KiB = 1024 bytes */
  /* Ergohaven nRF52840 board with Adafruit nRF52 bootloader */
  /* code_partition starts at 0x26000 (see ergohaven-zmk DTS) */
  FLASH : ORIGIN = 0x00026000, LENGTH = 868K
  RAM : ORIGIN = 0x20000008, LENGTH = 255K
}
