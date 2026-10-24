//! Build-7148 native examination request/profile subset. Never executes scripts.
use crate::{
    dat::Dat,
    script,
    world::{self, WorldError},
};
use std::{collections::BTreeMap, io, path::Path};

pub const REQUEST: u32 = 0x820a0007;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Object(u64),
    Clear,
}

/// Decode only the object and cleared request profiles proved by build 7148.
/// Clear/default construction (00609530 / 006095f0) sets type and admin to
/// zero; StreamPack 00609660 then emits those two words without a union value.
pub fn decode_request(bytes: &[u8]) -> Result<Request, WorldError> {
    if !matches!(bytes.len(), 36 | 44) {
        return Err(WorldError::InvalidValue);
    }
    let word = |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    if word(0) != 5
        || word(4) != 1
        || [0, u32::MAX].contains(&word(8))
        || word(12) != 0x44000100
        || word(16) != 0
        || word(20) != 0xffff009a
        || word(28) != 0
    {
        return Err(WorldError::InvalidValue);
    }
    match (word(24), bytes.len()) {
        (0, 36) if word(32) == 0 => Ok(Request::Clear),
        (0x40000002, 44) if word(40) == 0 => {
            let id = u64::from(word(32)) | (u64::from(word(36)) << 32);
            if [0, u64::MAX].contains(&id) {
                return Err(WorldError::InvalidValue);
            }
            Ok(Request::Object(id))
        }
        _ => Err(WorldError::InvalidValue),
    }
}

/// Legacy object-only entry point; clear requests are intentionally distinct.
pub fn decode(bytes: &[u8]) -> Result<u64, WorldError> {
    match decode_request(bytes)? {
        Request::Object(id) => Ok(id),
        Request::Clear => Err(WorldError::InvalidValue),
    }
}

#[derive(Clone, Debug)]
pub enum Name {
    Literal(String),
    Localized([u32; 3]),
}
#[derive(Default)]
pub struct Names(BTreeMap<u32, Option<Name>>);
impl Names {
    pub fn load(directory: &Path, definitions: impl Iterator<Item = u32>) -> io::Result<Self> {
        let mut portal = Dat::open(directory.join("portal.dat"))?;
        let mut names = Self::default();
        for id in definitions {
            if names.0.contains_key(&id) {
                continue;
            }
            if names.0.len() >= 16384 {
                return Err(io::Error::other("examination name capacity exceeded"));
            }
            // An unavailable default name cannot prevent entering the world.
            // No substituted StringInfo or unsupported prefix is offset-guessed.
            names.0.insert(id, Self::read(&mut portal, id).ok());
        }
        Ok(names)
    }
    pub fn get(&self, definition: u32, fallback: &str) -> Name {
        self.0
            .get(&definition)
            .and_then(|name| name.clone())
            .unwrap_or_else(|| Name::Literal(fallback.to_owned()))
    }
    fn read(portal: &mut Dat, definition: u32) -> io::Result<Name> {
        let invalid = || io::Error::other("unsupported definition name prefix");
        if definition >> 24 != 0x47 {
            return Err(invalid());
        }
        let did = 0x81000000 | (definition & 0xffffff);
        let bytes = portal.read(did)?;
        if bytes.len() > 65536 {
            return Err(invalid());
        }
        let mut pos = 0;
        let mut word = || -> io::Result<u32> {
            let data = bytes.get(pos..pos + 4).ok_or_else(invalid)?;
            pos += 4;
            Ok(u32::from_le_bytes(data.try_into().unwrap()))
        };
        if word()? & 1 == 0 || word()? != did {
            return Err(invalid());
        }
        let desc = word()?;
        if desc & 4 == 0 {
            return Err(invalid());
        }
        // Only these fields precede the name in gmWeenieDesc::StreamPack.
        for bit in [1, 0x2000000, 2] {
            if desc & bit != 0 {
                word()?;
            }
        }
        let name = [word()?, word()?, word()?];
        if name[0] == 0 || name[1] >> 24 != 0x25 || name[2] != 0 {
            return Err(invalid());
        }
        Ok(Name::Localized(name))
    }
}

/// ExaminationProfile::StreamPack 00608380, node pack 00608490 and list pack
/// 00609140. The matching 7148 profile ends after its node list, unlike newer
/// retained capture profiles. HeapObject::Pack 006dbb00 adds zero field count.
pub fn response(target: u64, name: Name, lines: &[String]) -> Result<Vec<u8>, WorldError> {
    if lines.len() > 12
        || lines
            .iter()
            .any(|line| line.len() > 512 || line.chars().any(char::is_control))
    {
        return Err(WorldError::LimitExceeded);
    }
    let mut bytes = Vec::new();
    for word in [
        5,
        1,
        1,
        0x44000100,
        0,
        0xffff0099,
        0x40000002,
        0,
        target as u32,
        (target >> 32) as u32,
        ((lines.len() + 1) * 2) as u32,
    ] {
        bytes.extend(u32::to_le_bytes(word));
    }
    let text: Vec<_> = std::iter::once(name)
        .chain(lines.iter().cloned().map(Name::Literal))
        .collect();
    for (index, text) in text.into_iter().enumerate() {
        for word in [
            0x40000004,
            2,
            if index == 0 { 0xbe5b6513 } else { 0x33be4035 },
        ] {
            bytes.extend(u32::to_le_bytes(word));
        }
        match text {
            Name::Literal(text) => bytes.extend(world::literal_name(&text)?),
            Name::Localized(words) => {
                for word in words {
                    bytes.extend(word.to_le_bytes());
                }
            }
        }
        bytes.extend(0x40000006u32.to_le_bytes()); // Break closes the current row.
        bytes.extend(2u32.to_le_bytes());
    }
    bytes.extend(0u32.to_le_bytes());
    script::private_call(0x820a0009, &bytes)
}
