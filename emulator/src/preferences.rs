//! Native UI placement and opacity uploads. No arbitrary script graphs are retained.
use crate::world::WorldError;
use std::collections::BTreeMap;

pub const WINDOW_UPDATE: u32 = 0x82120031;
pub const MAX_BYTES: usize = 16384;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Window {
    coordinates: [u32; 4],
    visible: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiSettings {
    opacities: BTreeMap<u32, u32>,
    layouts: BTreeMap<u32, BTreeMap<u32, Window>>,
}
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl Reader<'_> {
    fn word(&mut self) -> Result<u32, WorldError> {
        let data = self
            .bytes
            .get(self.pos..self.pos + 4)
            .ok_or(WorldError::Truncated)?;
        self.pos += 4;
        Ok(u32::from_le_bytes(data.try_into().unwrap()))
    }
    fn expect(&mut self, expected: u32) -> Result<(), WorldError> {
        if self.word()? != expected {
            return Err(WorldError::InvalidValue);
        }
        Ok(())
    }
    fn count(&mut self, limit: u32) -> Result<u32, WorldError> {
        let header = self.word()?;
        if header >> 24 > 22 || header & 0xffffff > limit {
            return Err(WorldError::LimitExceeded);
        }
        Ok(header & 0xffffff)
    }
    fn graph(&mut self, native: u32) -> Result<(), WorldError> {
        self.expect(5)?;
        self.expect(1)?;
        if self.word()? == u32::MAX {
            return Err(WorldError::InvalidValue);
        }
        self.expect(0x44000100)?;
        self.expect(0)?;
        self.expect(0xffff0000 | native)
    }
}
fn word(out: &mut Vec<u32>, value: u32) {
    out.push(value);
}
// Native add_for_unpacking requires consecutive entries of each hash bucket.
fn bucket_order<T>(map: &BTreeMap<u32, T>) -> Vec<(u32, &T)> {
    let mut entries: Vec<_> = map.iter().map(|(&key, value)| (key, value)).collect();
    entries.sort_unstable_by_key(|(key, _)| (key % 11, *key));
    entries
}
impl UiSettings {
    pub fn merge(&self, patch: &Self) -> Result<Self, WorldError> {
        let mut merged = self.clone();
        merged
            .opacities
            .extend(patch.opacities.iter().map(|(&key, &value)| (key, value)));
        for (&layout, windows) in &patch.layouts {
            merged
                .layouts
                .entry(layout)
                .or_default()
                .extend(windows.iter().map(|(&key, value)| (key, value.clone())));
        }
        if merged.opacities.len() > 64
            || merged.layouts.len() > 8
            || merged.layouts.values().any(|windows| windows.len() > 64)
            || merged
                .layouts
                .values()
                .map(|windows| windows.len())
                .sum::<usize>()
                > 256
        {
            return Err(WorldError::LimitExceeded);
        }
        Ok(merged)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, WorldError> {
        if bytes.len() > MAX_BYTES {
            return Err(WorldError::LimitExceeded);
        }
        let mut r = Reader { bytes, pos: 0 };
        r.graph(1)?;
        let mut opacities = BTreeMap::new();
        for _ in 0..r.count(64)? {
            let key = r.word()?;
            let bits = r.word()?;
            let value = f32::from_bits(bits);
            if !value.is_finite()
                || !(0.0..=1.0).contains(&value)
                || opacities.insert(key, bits).is_some()
            {
                return Err(WorldError::InvalidValue);
            }
        }
        r.expect(0)?; // No interpreted fields on the native hash.
        r.graph(0x6f)?;
        let mut layouts = BTreeMap::new();
        let mut total = 0;
        for _ in 0..r.count(8)? {
            let layout = r.word()?;
            let mut windows = BTreeMap::new();
            for _ in 0..r.count(64)? {
                total += 1;
                if total > 256 {
                    return Err(WorldError::LimitExceeded);
                }
                let key = r.word()?;
                let coordinates = [r.word()?, r.word()?, r.word()?, r.word()?];
                let visible = r.word()?;
                if visible > 1
                    || coordinates.iter().any(|bits| {
                        let value = f32::from_bits(*bits);
                        !value.is_finite() || value.abs() > 1024.0
                    })
                    || windows
                        .insert(
                            key,
                            Window {
                                coordinates,
                                visible: visible != 0,
                            },
                        )
                        .is_some()
                {
                    return Err(WorldError::InvalidValue);
                }
            }
            if layouts.insert(layout, windows).is_some() {
                return Err(WorldError::InvalidValue);
            }
        }
        r.expect(0)?;
        if r.pos != bytes.len() {
            return Err(WorldError::TrailingBytes);
        }
        Ok(Self { opacities, layouts })
    }
    fn opacity_words(&self, out: &mut Vec<u32>) {
        word(out, self.opacities.len() as u32);
        for (key, value) in bucket_order(&self.opacities) {
            out.extend([key, *value]);
        }
    }
    fn layout_words(&self, out: &mut Vec<u32>) {
        word(out, self.layouts.len() as u32);
        for (layout, windows) in bucket_order(&self.layouts) {
            out.extend([layout, windows.len() as u32]);
            for (key, window) in bucket_order(windows) {
                word(out, key);
                out.extend(window.coordinates);
                word(out, u32::from(window.visible));
            }
        }
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut words = vec![5, 1, 1, 0x44000100, 0, 0xffff0001];
        self.opacity_words(&mut words);
        words.extend([0, 5, 1, 1, 0x44000100, 0, 0xffff006f]);
        self.layout_words(&mut words);
        words.push(0);
        words.into_iter().flat_map(u32::to_le_bytes).collect()
    }
    /// GameplayOptionsProfile mask: saved locations, bit field/version, opacity.
    /// StreamPack 006000b0 orders these fields by explicit mask tests.
    pub fn options_words(&self) -> Vec<u32> {
        let mut words = vec![0x890, 0];
        self.layout_words(&mut words);
        words.extend([0x80000000, 4]);
        self.opacity_words(&mut words);
        words
    }
}
