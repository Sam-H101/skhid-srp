//! Lenient readers for binary config values.

use binary_config_parser::ConfigValue;

use crate::math::Vec3;
use crate::resources::Error;

/// A `float3`, or a table of three numbers (what a config built from JSON
/// holds, as in the tests).
pub fn float3(value: &ConfigValue, key: &str) -> Option<Vec3> {
    match value.get(key)? {
        ConfigValue::Float3(v) => Some(Vec3::from_array(*v)),
        ConfigValue::Indexed(items) if items.len() == 3 => Some(Vec3::new(
            items[0].as_f32()?,
            items[1].as_f32()?,
            items[2].as_f32()?,
        )),
        _ => None,
    }
}

pub fn float3_req(value: &ConfigValue, key: &str) -> Result<Vec3, Error> {
    float3(value, key).ok_or_else(|| Error(format!("`{key}` is missing or not a float3")))
}

pub fn float3_or(value: &ConfigValue, key: &str, default: Vec3) -> Vec3 {
    float3(value, key).unwrap_or(default)
}

pub fn children<'a>(value: &'a ConfigValue, key: &str) -> Vec<&'a ConfigValue> {
    value
        .get(key)
        .map(ConfigValue::children)
        .unwrap_or_default()
}

pub fn str_or<'a>(value: &'a ConfigValue, key: &str) -> &'a str {
    value.get(key).and_then(ConfigValue::as_str).unwrap_or("")
}

pub fn i32_or(value: &ConfigValue, key: &str, default: i32) -> i32 {
    value
        .get(key)
        .and_then(ConfigValue::as_i32)
        .unwrap_or(default)
}
