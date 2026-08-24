use embedded_storage_async::nor_flash::NorFlash as AsyncNorFlash;
use serde::de::{Error as DeError, SeqAccess, Visitor};
use serde::{Deserializer, Serializer};

use crate::MACRO_SPACE_SIZE;
use crate::keyboard::combo::Combo;
use crate::storage::{Storage, StorageData, StorageKey, print_storage_error};

pub(crate) mod macro_bytes_serde {
    use super::*;

    pub(crate) fn serialize<S>(value: &[u8; MACRO_SPACE_SIZE], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let used = value
            .iter()
            .rposition(|byte| *byte != 0)
            .map(|last| (last + 2).min(MACRO_SPACE_SIZE))
            .unwrap_or(1);
        serializer.serialize_bytes(&value[..used])
    }

    pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<[u8; MACRO_SPACE_SIZE], D::Error>
    where
        D: Deserializer<'de>,
    {
        struct MacroBytesVisitor;

        impl<'de> Visitor<'de> for MacroBytesVisitor {
            type Value = [u8; MACRO_SPACE_SIZE];

            fn expecting(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(formatter, "at most {MACRO_SPACE_SIZE} macro bytes")
            }

            fn visit_bytes<E>(self, value: &[u8]) -> Result<Self::Value, E>
            where
                E: DeError,
            {
                if value.len() > MACRO_SPACE_SIZE {
                    return Err(E::invalid_length(value.len(), &self));
                }

                let mut bytes = [0u8; MACRO_SPACE_SIZE];
                bytes[..value.len()].copy_from_slice(value);
                Ok(bytes)
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut bytes = [0u8; MACRO_SPACE_SIZE];
                let mut len = 0;
                while let Some(byte) = seq.next_element()? {
                    if len == MACRO_SPACE_SIZE {
                        return Err(A::Error::invalid_length(len + 1, &self));
                    }
                    bytes[len] = byte;
                    len += 1;
                }

                Ok(bytes)
            }
        }

        deserializer.deserialize_bytes(MacroBytesVisitor)
    }
}

impl<F: AsyncNorFlash, const ROW: usize, const COL: usize, const NUM_LAYER: usize, const NUM_ENCODER: usize>
    Storage<F, ROW, COL, NUM_LAYER, NUM_ENCODER>
{
    pub(crate) async fn read_runtime_state(
        &mut self,
        data: &mut crate::keymap::KeymapData<ROW, COL, NUM_LAYER, NUM_ENCODER>,
        behavior: &mut crate::config::BehaviorConfig,
    ) -> Result<(), ()> {
        let no_action_layer_start = self.tail_key_namespace_start();
        let uses_tail_key_namespace = |layer: u8| no_action_layer_start.is_some_and(|start| layer >= start);

        // Restore every host-owned setting in one flash traversal. Calling
        // fetch_item once per combo/fork/morse repeatedly scanned the complete
        // sequential-storage map and delayed K04 runtime startup by ~20 s.
        let mut key_iterator = self
            .flash
            .fetch_all_items(&mut self.buffer)
            .await
            .map_err(|e| print_storage_error::<F>(e))?;

        // Read all keymap keys and encoder configs
        while let Some((key, item)) = key_iterator
            .next::<StorageData>(&mut self.buffer)
            .await
            .map_err(|e| print_storage_error::<F>(e))?
        {
            match (key, item) {
                (StorageKey::KeymapV2 { layer, row, col }, StorageData::KeyAction(action))
                    if !uses_tail_key_namespace(layer) =>
                {
                    let layer = layer as usize;
                    let row = row as usize;
                    let col = col as usize;
                    if layer < NUM_LAYER && row < ROW && col < COL {
                        data.keymap[layer][row][col] = action;
                    }
                }
                (StorageKey::KeymapTailV3 { layer, row, col }, StorageData::KeyAction(action))
                    if uses_tail_key_namespace(layer) =>
                {
                    let layer = layer as usize;
                    let row = row as usize;
                    let col = col as usize;
                    if layer < NUM_LAYER && row < ROW && col < COL {
                        data.keymap[layer][row][col] = action;
                    }
                }
                (StorageKey::EncoderV2 { layer, idx }, StorageData::EncoderAction(action))
                    if !uses_tail_key_namespace(layer) =>
                {
                    let idx = idx as usize;
                    let layer = layer as usize;
                    if layer < NUM_LAYER && idx < NUM_ENCODER {
                        data.encoder_map[layer][idx] = action;
                    }
                }
                (StorageKey::EncoderTailV3 { layer, idx }, StorageData::EncoderAction(action))
                    if uses_tail_key_namespace(layer) =>
                {
                    let idx = idx as usize;
                    let layer = layer as usize;
                    if layer < NUM_LAYER && idx < NUM_ENCODER {
                        data.encoder_map[layer][idx] = action;
                    }
                }
                (StorageKey::LayoutConfig, StorageData::LayoutConfig(config)) => {
                    // Restore the default (base) layer set via a `PDF` key
                    behavior.default_layer = config.default_layer;
                }
                (StorageKey::BehaviorConfig, StorageData::BehaviorConfig(config)) => {
                    behavior.morse.prior_idle_time = embassy_time::Duration::from_millis(config.prior_idle_time as u64);
                    behavior.morse.default_profile = config.morse_default_profile;
                    behavior.combo.timeout = embassy_time::Duration::from_millis(config.combo_timeout as u64);
                    behavior.one_shot.timeout = embassy_time::Duration::from_millis(config.one_shot_timeout as u64);
                    behavior.tap.tap_interval = config.tap_interval;
                    behavior.tap.tap_capslock_interval = config.tap_capslock_interval;
                }
                (StorageKey::MacroData, StorageData::MacroData(macro_data)) => {
                    behavior.keyboard_macros.macro_sequences.copy_from_slice(&macro_data);
                }
                (StorageKey::Combo(idx), StorageData::Combo(config)) => {
                    if let Some(combo) = behavior.combo.combos.get_mut(idx as usize) {
                        debug!("Read combo config: {:?}", config);
                        *combo = Some(Combo::new(config));
                    }
                }
                (StorageKey::Fork(idx), StorageData::Fork(fork)) => {
                    if let Some(item) = behavior.fork.forks.get_mut(idx as usize) {
                        *item = fork;
                    }
                }
                (StorageKey::Morse(idx), StorageData::Morse(morse)) => {
                    if let Some(item) = behavior.morse.morses.get_mut(idx as usize) {
                        *item = morse;
                    }
                }
                _ => continue,
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rmk_types::action::Action;
    use rmk_types::keycode::{HidKeyCode, KeyCode};
    use rmk_types::morse::{HOLD, Morse, MorseMode, MorsePattern, MorseProfile, TAP};
    use sequential_storage::map::Value;

    use super::*;

    #[test]
    fn macro_storage_compacts_trailing_zeroes_and_restores_the_fixed_buffer() {
        let mut macro_bytes = [0u8; MACRO_SPACE_SIZE];
        macro_bytes[..4].copy_from_slice(&[1, 2, 3, 0]);
        let storage_data = StorageData::MacroData(macro_bytes);
        let mut buffer = [0u8; MACRO_SPACE_SIZE + 32];

        let serialized_size = Value::serialize_into(&storage_data, &mut buffer).unwrap();
        assert!(serialized_size < 32);

        let (decoded, consumed) = StorageData::deserialize_from(&buffer[..serialized_size]).unwrap();
        assert_eq!(consumed, serialized_size);
        match decoded {
            StorageData::MacroData(decoded) => assert_eq!(decoded, macro_bytes),
            _ => panic!("Expected MacroData"),
        }
    }

    #[test]
    fn empty_macro_storage_keeps_one_terminator() {
        let storage_data = StorageData::MacroData([0u8; MACRO_SPACE_SIZE]);
        let mut buffer = [0u8; 64];

        let serialized_size = Value::serialize_into(&storage_data, &mut buffer).unwrap();
        let (decoded, _) = StorageData::deserialize_from(&buffer[..serialized_size]).unwrap();

        match decoded {
            StorageData::MacroData(decoded) => assert!(decoded.iter().all(|byte| *byte == 0)),
            _ => panic!("Expected MacroData"),
        }
    }

    #[test]
    fn test_morse_serialization_deserialization() {
        let morse = Morse::new_from_vial(
            Action::Key(KeyCode::Hid(HidKeyCode::A)),
            Action::Key(KeyCode::Hid(HidKeyCode::B)),
            Action::Key(KeyCode::Hid(HidKeyCode::C)),
            Action::Key(KeyCode::Hid(HidKeyCode::D)),
            MorseProfile::new(Some(true), Some(MorseMode::PermissiveHold), Some(190u16), Some(180u16)),
        );

        // Serialization
        let mut buffer = [0u8; 64];
        let storage_data = StorageData::Morse(morse.clone());
        let serialized_size = Value::serialize_into(&storage_data, &mut buffer).unwrap();

        // Deserialization
        let deserialized_data = StorageData::deserialize_from(&buffer[..serialized_size]).unwrap();

        // Validation
        match deserialized_data {
            (StorageData::Morse(deserialized_morse), _) => {
                // actions
                assert_eq!(deserialized_morse.actions.len(), morse.actions.len());
                for (original, deserialized) in morse.actions.iter().zip(deserialized_morse.actions.iter()) {
                    assert_eq!(original, deserialized);
                }
                // profile
                assert_eq!(deserialized_morse.profile, morse.profile);
            }
            _ => panic!("Expected MorseData"),
        }
    }

    #[test]
    fn test_morse_with_partial_actions() {
        // Create a Morse with partial actions
        let mut morse = Morse::default();
        _ = morse.put(TAP, Action::Key(KeyCode::Hid(HidKeyCode::A)));
        _ = morse.put(HOLD, Action::Key(KeyCode::Hid(HidKeyCode::B)));

        // Serialization
        let mut buffer = [0u8; 64];
        let storage_data = StorageData::Morse(morse.clone());
        let serialized_size = Value::serialize_into(&storage_data, &mut buffer).unwrap();

        // Deserialization
        let deserialized_data = StorageData::deserialize_from(&buffer[..serialized_size]).unwrap();

        // Validation
        match deserialized_data {
            (StorageData::Morse(deserialized_morse), _) => {
                // actions
                assert_eq!(deserialized_morse.actions.len(), morse.actions.len());
                for (original, deserialized) in morse.actions.iter().zip(deserialized_morse.actions.iter()) {
                    assert_eq!(original, deserialized);
                }
                // profile
                assert_eq!(deserialized_morse.profile, morse.profile);
            }
            _ => panic!("Expected MorseData"),
        }
    }

    #[test]
    fn test_morse_with_morse_serialization_deserialization() {
        let mut morse = Morse {
            profile: MorseProfile::new(
                Some(false),
                Some(MorseMode::HoldOnOtherPress),
                Some(210u16),
                Some(220u16),
            ),
            actions: heapless::LinearMap::default(),
        };
        morse
            .actions
            .insert(MorsePattern::from_u16(0b1_01), Action::Key(KeyCode::Hid(HidKeyCode::A)))
            .ok();
        morse
            .actions
            .insert(
                MorsePattern::from_u16(0b1_1000),
                Action::Key(KeyCode::Hid(HidKeyCode::B)),
            )
            .ok();
        morse
            .actions
            .insert(
                MorsePattern::from_u16(0b1_1010),
                Action::Key(KeyCode::Hid(HidKeyCode::C)),
            )
            .ok();

        // Serialization
        let mut buffer = [0u8; 64];
        let storage_data = StorageData::Morse(morse.clone());
        let serialized_size = Value::serialize_into(&storage_data, &mut buffer).unwrap();

        // Deserialization
        let deserialized_data = StorageData::deserialize_from(&buffer[..serialized_size]).unwrap();

        // Validation
        match deserialized_data {
            (StorageData::Morse(deserialized_morse), _) => {
                // actions
                assert_eq!(deserialized_morse.actions.len(), morse.actions.len());
                for (original, deserialized) in morse.actions.iter().zip(deserialized_morse.actions.iter()) {
                    assert_eq!(original, deserialized);
                }
                // profile
                assert_eq!(deserialized_morse.profile, morse.profile);
            }
            _ => panic!("Expected MorseData"),
        }
    }
}
