/* Qube bootloader: application starts at 0x1000 */
MEMORY
{
    /* Keep the reset utility below the first settings partition at 0xA0000. */
    FLASH : ORIGIN = 0x00001000, LENGTH = 636K
    RAM   : ORIGIN = 0x20000000, LENGTH = 256K
}
