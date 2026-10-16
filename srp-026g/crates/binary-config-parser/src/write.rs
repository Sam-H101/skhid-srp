//! Writes a value tree as a binary config of the game's ([`ConfigValue::to_bytes`]).
//!
//! The layout is the one the game's own files have: the root value at
//! offset 0, then every table's values, one table after another in the order
//! they are reached (a table's children are side by side, as the reader
//! takes them), then the names, strings and float vectors the values point
//! at. Each value is 24 bytes (`BinaryValue`): its data (the number, or the
//! offset of what it points at), its name's offset and CRC-32 (a named
//! table's child; an indexed one's has its index and no CRC), its type and
//! its count: a table's children, a string's bytes with the NUL, 4 for a
//! number or a boolean, 8, 12 or 16 for a float vector, as the game's files
//! have them. Strings and names are written in Windows-1251, which is what
//! [`ConfigValue::parse`] decodes them from; so `parse(to_bytes(v)) == v`.

use std::collections::{HashMap, VecDeque};

use encoding_rs::WINDOWS_1251;

use crate::tree::{ConfigError, ConfigValue};
use crate::{BinaryType, BinaryValue, IdCrc};

/// A value's place: its name (a named table's child) or index.
enum Key<'a> {
    Root,
    Name(&'a str),
    Index(usize),
}

impl ConfigValue {
    /// The value as a binary config: what [`ConfigValue::parse`] reads back
    /// as this same value. Fails only for what the format cannot hold: a
    /// table of more than 65 535 children or a string of 65 535 bytes or
    /// more.
    pub fn to_bytes(&self) -> Result<Vec<u8>, ConfigError> {
        // Every value's slot, in the order they are laid out; a table's
        // children are given slots side by side when the table is reached.
        let mut slots: Vec<(&ConfigValue, Key)> = vec![(self, Key::Root)];
        let mut first_child: Vec<usize> = Vec::new();
        let mut queue = VecDeque::from([0usize]);
        while let Some(at) = queue.pop_front() {
            let start = slots.len();
            if first_child.len() <= at {
                first_child.resize(at + 1, 0);
            }
            first_child[at] = start;
            match slots[at].0 {
                Self::Named(entries) => {
                    for (name, value) in entries {
                        slots.push((value, Key::Name(name)));
                    }
                }
                Self::Indexed(values) => {
                    for (index, value) in values.iter().enumerate() {
                        slots.push((value, Key::Index(index)));
                    }
                }
                _ => {}
            }
            queue.extend(start..slots.len());
        }

        let mut bytes = vec![0u8; slots.len() * BinaryValue::SIZE];
        let mut names: HashMap<&str, u64> = HashMap::new();
        for (at, (value, key)) in slots.iter().enumerate() {
            let (id, id_crc) = match key {
                Key::Root => (0, IdCrc::default()),
                Key::Index(index) => (*index as u64, IdCrc::default()),
                Key::Name(name) => {
                    let offset = match names.get(name) {
                        Some(&offset) => offset,
                        None => {
                            let offset = bytes.len() as u64;
                            let encoded = encode(name)?;
                            bytes.extend_from_slice(&encoded);
                            bytes.push(0);
                            names.insert(name, offset);
                            offset
                        }
                    };
                    (offset, IdCrc::get_hash_of(&encode(name)?))
                }
            };
            let tail = bytes.len() as u64;
            let (type_, data, count) = match value {
                Self::Boolean(value) => (BinaryType::Boolean, u64::from(*value), 4),
                Self::Integer(value) => (BinaryType::Integer, u64::from(*value as u32), 4),
                Self::Float(value) => (BinaryType::Float, u64::from(value.to_bits()), 4),
                Self::String(text) => {
                    let encoded = encode(text)?;
                    let count = u16::try_from(encoded.len() + 1).map_err(|_| {
                        ConfigError(format!("a string of {} bytes is too long", encoded.len()))
                    })?;
                    bytes.extend_from_slice(&encoded);
                    bytes.push(0);
                    (BinaryType::String, tail, count)
                }
                Self::Float2(values) => (BinaryType::Float2, floats(&mut bytes, values), 8),
                Self::Float3(values) => (BinaryType::Float3, floats(&mut bytes, values), 12),
                Self::Float4(values) => (BinaryType::Float4, floats(&mut bytes, values), 16),
                Self::Named(entries) => (
                    BinaryType::TableNamed,
                    (first_child[at] * BinaryValue::SIZE) as u64,
                    children(entries.len())?,
                ),
                Self::Indexed(values) => (
                    BinaryType::TableIndexed,
                    (first_child[at] * BinaryValue::SIZE) as u64,
                    children(values.len())?,
                ),
            };
            let slot = BinaryValue {
                data,
                id,
                id_crc,
                type_,
                count,
            };
            bytes[at * BinaryValue::SIZE..(at + 1) * BinaryValue::SIZE]
                .copy_from_slice(slot.as_bytes());
        }
        Ok(bytes)
    }
}

impl IdCrc {
    /// The CRC-32 of a name's bytes as the file has them.
    fn get_hash_of(bytes: &[u8]) -> Self {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(bytes);
        Self(hasher.finalize())
    }
}

fn children(count: usize) -> Result<u16, ConfigError> {
    u16::try_from(count).map_err(|_| ConfigError(format!("a table of {count} values is too big")))
}

/// `text` in Windows-1251, every character of it.
fn encode(text: &str) -> Result<Vec<u8>, ConfigError> {
    let (encoded, _, unmappable) = WINDOWS_1251.encode(text);
    if unmappable {
        return Err(ConfigError(format!("{text:?} is not Windows-1251")));
    }
    if encoded.contains(&0) {
        return Err(ConfigError(format!("{text:?} has a NUL")));
    }
    Ok(encoded.into_owned())
}

/// Appends `values` and gives their offset.
fn floats<const N: usize>(bytes: &mut Vec<u8>, values: &[f32; N]) -> u64 {
    let offset = bytes.len() as u64;
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_kind() -> ConfigValue {
        ConfigValue::Named(vec![
            ("flag".into(), ConfigValue::Boolean(true)),
            ("off".into(), ConfigValue::Boolean(false)),
            ("count".into(), ConfigValue::Integer(-7)),
            ("big".into(), ConfigValue::Integer(i32::MAX)),
            ("ratio".into(), ConfigValue::Float(0.8)),
            (
                "name".into(),
                ConfigValue::String("Рудня \"Восток\"".into()),
            ),
            ("empty".into(), ConfigValue::String(String::new())),
            ("uv".into(), ConfigValue::Float2([1.0, -2.5])),
            ("position".into(), ConfigValue::Float3([1.0, 2.0, 3.0])),
            ("color".into(), ConfigValue::Float4([0.0, 0.25, 0.5, 1.0])),
            (
                "points".into(),
                ConfigValue::Indexed(vec![
                    ConfigValue::Named(vec![
                        ("team".into(), ConfigValue::Integer(0)),
                        ("position".into(), ConfigValue::Float3([4.0, 5.0, 6.0])),
                    ]),
                    ConfigValue::Named(vec![("team".into(), ConfigValue::Integer(1))]),
                    ConfigValue::Indexed(Vec::new()),
                ]),
            ),
            ("nothing".into(), ConfigValue::Named(Vec::new())),
        ])
    }

    #[test]
    fn what_is_written_reads_back_the_same() {
        for value in [
            every_kind(),
            ConfigValue::Named(Vec::new()),
            ConfigValue::Indexed(vec![ConfigValue::Integer(1), ConfigValue::Integer(2)]),
            ConfigValue::Integer(5),
        ] {
            let bytes = value.to_bytes().unwrap();
            assert_eq!(ConfigValue::parse(&bytes).unwrap(), value);
        }
    }

    #[test]
    fn it_is_laid_out_as_the_games_files() {
        let bytes = every_kind().to_bytes().unwrap();
        let value = |at: usize| {
            let raw = &bytes[at * 24..at * 24 + 24];
            (
                u64::from_le_bytes(raw[0..8].try_into().unwrap()),
                u32::from_le_bytes(raw[16..20].try_into().unwrap()),
                u16::from_le_bytes([raw[20], raw[21]]),
                u16::from_le_bytes([raw[22], raw[23]]),
            )
        };
        // The root's children follow it.
        assert_eq!(value(0), (24, 0, BinaryType::TableNamed as u16, 12));
        // A boolean and a number count 4, a float3 12; a negative number is
        // its 32 bits.
        assert_eq!(value(1).3, 4);
        assert_eq!(value(3).0, 0xFFFF_FFF9);
        assert_eq!(value(9).3, 12);
        // A named child has its name's CRC-32.
        assert_eq!(value(1).1, IdCrc::get_hash("flag").get());
        // The string counts its NUL.
        assert_eq!(value(6).3, "Рудня \"Восток\"".chars().count() as u16 + 1);
    }

    #[test]
    fn what_the_format_cannot_hold_is_refused() {
        let long = ConfigValue::String("x".repeat(70_000));
        assert!(long.to_bytes().is_err());
        let wide = ConfigValue::Indexed(vec![ConfigValue::Integer(0); 70_000]);
        assert!(wide.to_bytes().is_err());
        assert!(ConfigValue::String("日本".into()).to_bytes().is_err());
    }
}
