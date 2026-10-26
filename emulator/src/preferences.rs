//! Native UI placement and opacity uploads. No arbitrary script graphs are retained.
use crate::world::WorldError;
use std::collections::BTreeMap;

pub const WINDOW_UPDATE: u32 = 0x82120031;
pub const HOTBAR_UPDATE: u32 = 0x82120018;
pub const GAMEPLAY_UPDATE: u32 = 0x8212000c;
pub const MAX_BYTES: usize = 16384;

/// The four native UploadGameOptions values. Bits remain client preferences,
/// never combat, visibility or ownership authority. Store floats by bits so
/// journal replay and duplicate suppression are exact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameplayOptions {
    bitfield: u32,
    radar: u32,
    shortcut_height: u32,
    damage_range: u32,
}
impl GameplayOptions {
    pub fn decode(bytes: &[u8]) -> Result<Self, WorldError> {
        if bytes.len() != 32 {
            return Err(WorldError::InvalidValue);
        }
        let word = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        if word(0) != 1 || word(8) != 1 || word(16) != 1 || word(24) != 3 {
            return Err(WorldError::InvalidValue);
        }
        let value = Self {
            bitfield: word(4),
            radar: word(12),
            shortcut_height: word(20),
            damage_range: word(28),
        };
        let range = f32::from_bits(value.damage_range);
        if !(1..=5).contains(&value.shortcut_height)
            || !range.is_finite()
            || (range < 0.0 && range != -1.0)
        {
            return Err(WorldError::InvalidValue);
        }
        Ok(value)
    }
    pub fn encode(&self) -> Vec<u8> {
        [1, self.bitfield, 1, self.radar, 1, self.shortcut_height, 3, self.damage_range]
            .into_iter().flat_map(u32::to_le_bytes).collect()
    }
}

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
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Shortcut {
    Empty,
    Skill(u32),
    Item(u64),
    Text(String),
    Enumeration(u32),
    Craft(u32),
}
impl Shortcut {
    fn words(&self) -> Result<Vec<u32>, WorldError> {
        Ok(match self {
            Self::Empty => vec![0],
            Self::Skill(id) => vec![0x40000001, *id],
            Self::Item(id) => vec![0x40000002, *id as u32, (*id >> 32) as u32],
            Self::Text(text) => {
                let mut words = vec![0x40000003];
                words.extend(
                    crate::world::ui_text(text)?
                        .chunks_exact(4)
                        .map(|b| u32::from_le_bytes(b.try_into().unwrap())),
                );
                words
            }
            Self::Enumeration(id) => vec![0x40000004, *id],
            Self::Craft(id) => vec![0x40000006, *id],
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HotbarUpdate {
    slot: u32,
    shortcut: Shortcut,
}
impl HotbarUpdate {
    pub fn decode(bytes: &[u8]) -> Result<Self, WorldError> {
        if bytes.len() > 2048 {
            return Err(WorldError::LimitExceeded);
        }
        let mut r = Reader { bytes, pos: 0 };
        r.graph(0xac)?; // gmCNativeFactory 00466c10 -> ShortcutInfo.
        let shortcut = match r.word()? {
            0 => Shortcut::Empty,
            0x40000001 => Shortcut::Skill(r.word()?),
            0x40000002 => Shortcut::Item(u64::from(r.word()?) | (u64::from(r.word()?) << 32)),
            0x40000003 => {
                let (text, size) = crate::world::read_ui_text(&bytes[r.pos..])?;
                r.pos += size;
                if text.is_empty()
                    || text.chars().count() > 256
                    || text.chars().any(char::is_control)
                {
                    return Err(WorldError::InvalidValue);
                }
                Shortcut::Text(text)
            }
            0x40000004 => Shortcut::Enumeration(r.word()?),
            0x40000006 => Shortcut::Craft(r.word()?),
            _ => return Err(WorldError::InvalidValue),
        };
        r.expect(0)?;
        r.expect(1)?;
        let slot = r.word()?;
        if slot >= 100 {
            return Err(WorldError::LimitExceeded);
        }
        if r.pos != bytes.len() {
            return Err(WorldError::TrailingBytes);
        }
        Ok(Self { slot, shortcut })
    }
    pub fn encode(&self) -> Result<Vec<u8>, WorldError> {
        let mut words = vec![5, 1, 1, 0x44000100, 0, 0xffff00ac];
        words.extend(self.shortcut.words()?);
        words.extend([0, 1, self.slot]);
        Ok(words.into_iter().flat_map(u32::to_le_bytes).collect())
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hotbar {
    slots: BTreeMap<u32, Shortcut>,
}
impl Hotbar {
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
    /// First slot is the authored training attack. Shown only while the saved bar is empty.
    pub fn training_attack() -> Self {
        let mut slots = BTreeMap::new();
        slots.insert(0, Shortcut::Skill(crate::skills::TRAINING_ATTACK));
        Self { slots }
    }
    /// The race's three basic attacks in the first slots of an empty bar.
    pub fn starter(race: u32, sex: u32) -> Self {
        let Some(body) = crate::appearance::body(race, sex) else {
            return Self::training_attack();
        };
        let slots = body
            .starters
            .iter()
            .enumerate()
            .map(|(slot, &skill)| (slot as u32, Shortcut::Skill(skill)))
            .collect();
        Self { slots }
    }
    pub fn apply(&self, update: &HotbarUpdate) -> Result<Self, WorldError> {
        let mut next = self.clone();
        if update.shortcut == Shortcut::Empty {
            next.slots.remove(&update.slot);
        } else {
            next.slots.insert(update.slot, update.shortcut.clone());
        }
        if next.words()?.len() * 4 > MAX_BYTES {
            return Err(WorldError::LimitExceeded);
        }
        Ok(next)
    }
    fn words(&self) -> Result<Vec<u32>, WorldError> {
        let mut words = vec![100];
        for slot in 0..100 {
            words.extend(self.slots.get(&slot).unwrap_or(&Shortcut::Empty).words()?);
        }
        Ok(words)
    }
}
pub fn options_words(
    settings: Option<&UiSettings>,
    hotbar: Option<&Hotbar>,
) -> Result<Vec<u32>, WorldError> {
    options_words_with_gameplay(settings, hotbar, None)
}
pub fn options_words_with_gameplay(
    settings: Option<&UiSettings>,
    hotbar: Option<&Hotbar>,
    gameplay: Option<&GameplayOptions>,
) -> Result<Vec<u32>, WorldError> {
    let mut words = vec![
        (if settings.is_some() { 0x890 } else { 0 })
            | (if hotbar.is_some() { 2 } else { 0 })
            | (if gameplay.is_some() { 0x10a8 } else { 0 }),
        0,
    ];
    if let Some(hotbar) = hotbar {
        words.extend(hotbar.words()?);
    }
    if let Some(options) = gameplay {
        words.extend([options.shortcut_height, options.damage_range]);
    }
    if let Some(settings) = settings {
        settings.layout_words(&mut words);
    }
    if let Some(options) = gameplay {
        words.push(options.radar);
    }
    if settings.is_some() || gameplay.is_some() {
        // The high bit announces the version word needed for window opacity.
        let bits = gameplay.map_or(0, |options| options.bitfield)
            | if settings.is_some() { 0x80000000 } else { 0 };
        words.push(bits);
        if bits & 0x80000000 != 0 { words.push(4); }
    }
    if let Some(settings) = settings {
        settings.opacity_words(&mut words);
    }
    Ok(words)
}
