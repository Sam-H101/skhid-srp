//! Reads a binary config into an owned value tree.
//!
//! Unlike [`crate::BinaryConfig::parse_n_print`], this never panics on bad
//! input: every offset is bounds-checked and errors come back as
//! [`ConfigError`], so a server can load game data it does not control.

use std::fmt;

use encoding_rs::WINDOWS_1251;
use num_traits::FromPrimitive;

use crate::{BinaryType, BinaryValue};

/// Deeper nesting than any shipped config uses; guards against cyclic offsets.
const MAX_DEPTH: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub enum ConfigValue {
    Boolean(bool),
    Integer(i32),
    Float(f32),
    String(String),
    Float2([f32; 2]),
    Float3([f32; 3]),
    Float4([f32; 4]),
    /// Named entries, in file order.
    Named(Vec<(String, ConfigValue)>),
    Indexed(Vec<ConfigValue>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "binary config: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}

impl ConfigValue {
    /// Parses the root value at the start of `bytes`.
    pub fn parse(bytes: &[u8]) -> Result<Self, ConfigError> {
        let root = read_value(bytes, 0)?;
        build(bytes, &root, 0)
    }

    /// A named child.
    pub fn get(&self, key: &str) -> Option<&ConfigValue> {
        match self {
            Self::Named(entries) => entries.iter().find(|(name, _)| name == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// A named child, or an error naming the missing key.
    pub fn req(&self, key: &str) -> Result<&ConfigValue, ConfigError> {
        self.get(key)
            .ok_or_else(|| ConfigError(format!("missing key `{key}`")))
    }

    /// The children of either kind of table, in file order.
    pub fn children(&self) -> Vec<&ConfigValue> {
        match self {
            Self::Named(entries) => entries.iter().map(|(_, v)| v).collect(),
            Self::Indexed(values) => values.iter().collect(),
            _ => Vec::new(),
        }
    }

    /// Named entries, or nothing for any other value.
    pub fn entries(&self) -> &[(String, ConfigValue)] {
        match self {
            Self::Named(entries) => entries,
            _ => &[],
        }
    }

    /// A number as the engine reads it: `(float)value` accepts integers too.
    pub fn as_f32(&self) -> Option<f32> {
        match *self {
            Self::Float(value) => Some(value),
            Self::Integer(value) => Some(value as f32),
            _ => None,
        }
    }

    pub fn as_i32(&self) -> Option<i32> {
        match *self {
            Self::Integer(value) => Some(value),
            Self::Float(value) => Some(value as i32),
            Self::Boolean(value) => Some(value.into()),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Self::Boolean(value) => Some(value),
            Self::Integer(value) => Some(value != 0),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_float3(&self) -> Option<[f32; 3]> {
        match *self {
            Self::Float3(value) => Some(value),
            _ => None,
        }
    }

    /// `key` as a number, or an error naming it.
    pub fn f32_at(&self, key: &str) -> Result<f32, ConfigError> {
        self.req(key)?
            .as_f32()
            .ok_or_else(|| ConfigError(format!("`{key}` is not a number")))
    }

    pub fn i32_at(&self, key: &str) -> Result<i32, ConfigError> {
        self.req(key)?
            .as_i32()
            .ok_or_else(|| ConfigError(format!("`{key}` is not an integer")))
    }

    pub fn str_at(&self, key: &str) -> Result<&str, ConfigError> {
        self.req(key)?
            .as_str()
            .ok_or_else(|| ConfigError(format!("`{key}` is not a string")))
    }

    pub fn float3_at(&self, key: &str) -> Result<[f32; 3], ConfigError> {
        self.req(key)?
            .as_float3()
            .ok_or_else(|| ConfigError(format!("`{key}` is not a float3")))
    }
}

fn slice(bytes: &[u8], offset: usize, len: usize) -> Result<&[u8], ConfigError> {
    offset
        .checked_add(len)
        .and_then(|end| bytes.get(offset..end))
        .ok_or_else(|| ConfigError(format!("offset {offset}+{len} is out of bounds")))
}

fn read_value(bytes: &[u8], offset: usize) -> Result<BinaryValue, ConfigError> {
    let raw = slice(bytes, offset, BinaryValue::SIZE)?;
    let u64_at = |at: usize| u64::from_le_bytes(raw[at..at + 8].try_into().unwrap());
    let type_ = u16::from_le_bytes([raw[20], raw[21]]);
    Ok(BinaryValue {
        data: u64_at(0),
        id: u64_at(8),
        id_crc: crate::IdCrc(u32::from_le_bytes(raw[16..20].try_into().unwrap())),
        type_: BinaryType::from_u16(type_)
            .ok_or_else(|| ConfigError(format!("unknown value type {type_}")))?,
        count: u16::from_le_bytes([raw[22], raw[23]]),
    })
}

fn c_string(bytes: &[u8], offset: usize) -> Result<String, ConfigError> {
    let tail = bytes
        .get(offset..)
        .ok_or_else(|| ConfigError(format!("name offset {offset} is out of bounds")))?;
    let end = tail
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| ConfigError(format!("unterminated name at {offset}")))?;
    Ok(WINDOWS_1251.decode(&tail[..end]).0.into_owned())
}

fn floats<const N: usize>(bytes: &[u8], offset: usize) -> Result<[f32; N], ConfigError> {
    let raw = slice(bytes, offset, 4 * N)?;
    Ok(std::array::from_fn(|i| {
        f32::from_le_bytes(raw[4 * i..4 * i + 4].try_into().unwrap())
    }))
}

fn build(bytes: &[u8], value: &BinaryValue, depth: usize) -> Result<ConfigValue, ConfigError> {
    if depth > MAX_DEPTH {
        return Err(ConfigError("nesting is too deep".to_owned()));
    }
    let offset = usize::try_from(value.data)
        .map_err(|_| ConfigError(format!("offset {} does not fit", value.data)))?;
    Ok(match value.type_ {
        BinaryType::Boolean => ConfigValue::Boolean(value.data != 0),
        BinaryType::Integer => ConfigValue::Integer(value.data as i32),
        BinaryType::Float => ConfigValue::Float(f32::from_bits(value.data as u32)),
        BinaryType::String => {
            // `count` includes the terminating NUL.
            let len = usize::from(value.count.max(1)) - 1;
            ConfigValue::String(
                WINDOWS_1251
                    .decode(slice(bytes, offset, len)?)
                    .0
                    .into_owned(),
            )
        }
        BinaryType::Float2 => ConfigValue::Float2(floats(bytes, offset)?),
        BinaryType::Float3 => ConfigValue::Float3(floats(bytes, offset)?),
        BinaryType::Float4 => ConfigValue::Float4(floats(bytes, offset)?),
        BinaryType::TableNamed | BinaryType::TableIndexed => {
            let mut children = Vec::with_capacity(usize::from(value.count));
            for i in 0..usize::from(value.count) {
                let child = read_value(bytes, offset + i * BinaryValue::SIZE)?;
                children.push(child);
            }
            if value.type_ == BinaryType::TableIndexed {
                ConfigValue::Indexed(
                    children
                        .iter()
                        .map(|child| build(bytes, child, depth + 1))
                        .collect::<Result<_, _>>()?,
                )
            } else {
                ConfigValue::Named(
                    children
                        .iter()
                        .map(|child| {
                            let id = usize::try_from(child.id).map_err(|_| {
                                ConfigError(format!("name offset {} does not fit", child.id))
                            })?;
                            Ok((c_string(bytes, id)?, build(bytes, child, depth + 1)?))
                        })
                        .collect::<Result<_, ConfigError>>()?,
                )
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_config_built_from_json() {
        let json = serde_json::json!({
            "parameters": { "bullet_damage": 0.4, "magazine_capacity": 30, "name": "ak" },
            "list": [1, 2, 3],
        });
        let config = crate::BinaryConfig::parse_json(&json);
        let value = ConfigValue::parse(config.as_bytes()).unwrap();
        let parameters = value.req("parameters").unwrap();
        assert_eq!(parameters.f32_at("bullet_damage").unwrap(), 0.4);
        assert_eq!(parameters.i32_at("magazine_capacity").unwrap(), 30);
        assert_eq!(parameters.f32_at("magazine_capacity").unwrap(), 30.0);
        assert_eq!(parameters.str_at("name").unwrap(), "ak");
        let list: Vec<i32> = value
            .req("list")
            .unwrap()
            .children()
            .iter()
            .map(|v| v.as_i32().unwrap())
            .collect();
        assert_eq!(list, [1, 2, 3]);
        assert!(value.get("missing").is_none());
    }

    #[test]
    fn rejects_truncated_input_without_panicking() {
        let json = serde_json::json!({ "a": { "b": "text" } });
        let config = crate::BinaryConfig::parse_json(&json);
        let bytes = config.as_bytes();
        for len in 0..bytes.len() {
            let _ = ConfigValue::parse(&bytes[..len]);
        }
    }
}
