//! Read the player's original launcher resources; no game artwork is distributed.
use crate::update::{Layout, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::collections::BTreeMap;

#[derive(Default, serde::Serialize)]
pub struct Skin {
    pub images: BTreeMap<u32, String>,
    pub strings: BTreeMap<u32, BTreeMap<u32, String>>,
}
fn bytes(d: &[u8], at: usize, n: usize) -> Result<&[u8]> {
    d.get(at..at.checked_add(n).ok_or("Invalid PE range")?)
        .ok_or("Truncated launcher resource".into())
}
fn word(d: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(bytes(d, at, 2)?.try_into().unwrap()))
}
fn dword(d: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(bytes(d, at, 4)?.try_into().unwrap()))
}
fn wide(d: &[u8], at: usize, n: usize) -> Result<String> {
    String::from_utf16(
        &bytes(d, at, n.checked_mul(2).ok_or("Invalid string size")?)?
            .chunks_exact(2)
            .map(|p| u16::from_le_bytes([p[0], p[1]]))
            .collect::<Vec<_>>(),
    )
    .map_err(|e| e.to_string())
}
impl Skin {
    pub fn load(layout: &Layout) -> Result<Self> {
        let mut errors = Vec::new();
        for name in ["survarium_launcher.exe", "launcher_skhid.exe"] {
            let path = layout.bin.join(name);
            match std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|b| Self::parse(&b))
            {
                Ok(skin) => return Ok(skin),
                Err(e) => errors.push(format!("{name}: {e}")),
            }
        }
        Err(format!(
            "The original 0.26g launcher is needed for its artwork. {}",
            errors.join("; ")
        ))
    }
    pub fn parse(d: &[u8]) -> Result<Self> {
        if bytes(d, 0, 2)? != b"MZ" {
            return Err("Not a PE executable".into());
        }
        let pe = dword(d, 0x3c)? as usize;
        if bytes(d, pe, 4)? != b"PE\0\0" || word(d, pe + 24)? != 0x10b {
            return Err("Not a PE32 launcher".into());
        }
        let count = word(d, pe + 6)? as usize;
        if count > 96 {
            return Err("Too many PE sections".into());
        }
        let sections = pe + 24 + word(d, pe + 20)? as usize;
        let offset = |rva: u32, size: usize| -> Result<usize> {
            for i in 0..count {
                let at = sections + i * 40;
                let start = dword(d, at + 12)?;
                let len = dword(d, at + 16)?;
                if let Some(delta) = rva.checked_sub(start) {
                    if (delta as u64) + (size as u64) <= len as u64 {
                        let file = dword(d, at + 20)? as usize + delta as usize;
                        bytes(d, file, size)?;
                        return Ok(file);
                    }
                }
            }
            Err("Resource outside PE sections".into())
        };
        let root = offset(dword(d, pe + 24 + 112)?, 16)?;
        let resource_size = dword(d, pe + 24 + 116)? as usize;
        let resource = bytes(d, root, resource_size)?;
        let entries = |dir: usize| -> Result<Vec<(u32, u32)>> {
            let n = word(resource, dir + 12)? as usize + word(resource, dir + 14)? as usize;
            if n > 4096 {
                return Err("Too many resources".into());
            }
            (0..n)
                .map(|i| {
                    Ok((
                        dword(resource, dir + 16 + i * 8)?,
                        dword(resource, dir + 20 + i * 8)?,
                    ))
                })
                .collect()
        };
        let dir = |v: u32| -> Result<usize> {
            if v & 0x80000000 == 0 {
                return Err("Expected resource directory".into());
            }
            Ok((v & 0x7fffffff) as usize)
        };
        let mut skin = Self::default();
        let mut visited = 0;
        for (kind, types) in entries(0)? {
            let name = if kind & 0x80000000 != 0 {
                let at = (kind & 0x7fffffff) as usize;
                wide(resource, at + 2, word(resource, at)? as usize)?
            } else {
                String::new()
            };
            if kind != 6 && name != "PNG" && name != "JPG" {
                continue;
            }
            for (id, langs) in entries(dir(types)?)? {
                if kind == 6 && !(1..=4096).contains(&id) {
                    return Err("Invalid string block number".into());
                }
                for (lang, leaf) in entries(dir(langs)?)? {
                    visited += 1;
                    if visited > 4096 {
                        return Err("Too many launcher resources".into());
                    }
                    if leaf & 0x80000000 != 0 {
                        return Err("Unexpected resource nesting".into());
                    }
                    let at = leaf as usize;
                    let size = dword(resource, at + 4)? as usize;
                    if size > 4 * 1024 * 1024 {
                        return Err("Oversized launcher resource".into());
                    }
                    let data = bytes(d, offset(dword(resource, at)?, size)?, size)?;
                    if kind == 6 {
                        let mut pos = 0;
                        for i in 0..16 {
                            let n = word(data, pos)? as usize;
                            pos += 2;
                            let text = wide(data, pos, n)?;
                            pos += n * 2;
                            if n != 0 {
                                skin.strings.entry(lang).or_default().insert(
                                    (id.checked_sub(1).ok_or("Invalid string block")?) * 16 + i,
                                    text,
                                );
                            }
                        }
                    } else if lang == 1033 {
                        let mime = if name == "PNG" {
                            "image/png"
                        } else {
                            "image/jpeg"
                        };
                        skin.images
                            .insert(id, format!("data:{mime};base64,{}", STANDARD.encode(data)));
                    }
                }
            }
        }
        for id in [
            300, 301, 302, 303, 304, 305, 306, 311, 312, 313, 314, 315, 316, 317, 318, 319, 320,
            321, 325, 326, 327, 357, 358, 359, 360, 361, 362, 363, 364,
        ] {
            if !skin.images.contains_key(&id) {
                return Err(format!("Missing 0.26g skin resource {id}"));
            }
        }
        for lang in [1033, 1049, 1058, 1031, 1045] {
            for id in [134, 135, 194, 196, 197, 198] {
                if !skin.strings.get(&lang).is_some_and(|s| s.contains_key(&id)) {
                    return Err("Missing launcher translations".into());
                }
            }
        }
        Ok(skin)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn truncated_and_hostile_pe_is_rejected() {
        for n in 0..256 {
            assert!(Skin::parse(&vec![0; n]).is_err());
        }
        let mut d = vec![0; 128];
        d[..2].copy_from_slice(b"MZ");
        d[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Skin::parse(&d).is_err());
    }
}
