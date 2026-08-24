#![no_std]

pub const ERASED_WORD: u32 = 0xFFFF_FFFF;

#[inline]
pub const fn word_is_migration_compatible(source: u32, destination: u32) -> bool {
    destination == ERASED_WORD || destination == source
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erased_destination_is_safe() {
        assert!(word_is_migration_compatible(0x1234_5678, ERASED_WORD));
    }

    #[test]
    fn matching_partial_copy_is_safe() {
        assert!(word_is_migration_compatible(0x1234_5678, 0x1234_5678));
    }

    #[test]
    fn unrelated_destination_data_is_rejected() {
        assert!(!word_is_migration_compatible(0x1234_5678, 0x1234_5670));
    }
}
