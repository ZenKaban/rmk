/// Config for storage
#[derive(Clone, Copy, Debug)]
pub struct StorageConfig {
    /// Start address of local storage, MUST BE start of a sector.
    /// If start_addr is set to 0(this is the default value), the last `num_sectors` sectors will be used.
    pub start_addr: usize,
    // Number of sectors used for storage, >= 2.
    pub num_sectors: u8,
    pub clear_storage: bool,
    pub clear_layout: bool,
    /// Use a new storage key namespace from this layer onward, allowing a
    /// firmware to replace legacy tail-layer defaults without touching lower
    /// user-configured layers.
    pub no_action_layer_start: Option<u8>,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            start_addr: 0,
            num_sectors: 2,
            clear_storage: false,
            clear_layout: false,
            no_action_layer_start: None,
        }
    }
}
