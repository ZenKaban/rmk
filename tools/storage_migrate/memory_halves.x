/* Adafruit nRF52 bootloader: application starts at 0x26000 */
MEMORY
{
    /* Keep the migration utility below the legacy partition at 0xA0000. */
    FLASH : ORIGIN = 0x00026000, LENGTH = 488K
    RAM   : ORIGIN = 0x20000000, LENGTH = 256K
}
