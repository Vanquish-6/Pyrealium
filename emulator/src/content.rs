//! Bounded server-authored placements. DAT presence is checked before publication.
use crate::{
    dat::Dat,
    movement::Placement,
    world::{self, StationaryObject},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{self, Read},
    path::Path,
    sync::Arc,
};

pub const NAMESPACE: u64 = 0x8140000000000000;
const MAX_BYTES: u64 = 4 * 1024 * 1024;
const MAX_OBJECTS: usize = 65536;
const MAX_CELL_OBJECTS: usize = 64;
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
pub struct Object {
    pub id: u64,
    pub entity: u32,
    pub placement: Placement,
    pub spawn: Arc<[u8]>,
    pub destroy: Arc<[u8]>,
    active: bool,
    instance: u16,
    visual_stamp: u16,
    stats: BTreeMap<u32, (u32, Arc<[u8]>)>,
    home: (Placement, [f32; 4]),
    position_stamp: u16,
    dynamic: bool,
    /// Placement-specific display name sent in the CreateObject, if any.
    pub name: Option<String>,
}
/// Start of Position in a generated stationary CreateObject, by physics flags.
fn position_offset(spawn: &[u8]) -> Option<usize> {
    match u32::from_le_bytes(spawn.get(20..24)?.try_into().ok()?) {
        0x8000 => Some(28),
        0x8080 => Some(32),
        0x8082 => Some(48),
        _ => None,
    }
    .filter(|offset| spawn.len() >= offset + 32)
}
fn write_position(spawn: &mut [u8], offset: usize, placement: Placement, orientation: [f32; 4]) {
    for (i, value) in placement.coordinates.iter().chain(orientation.iter()).enumerate() {
        spawn[offset + 4 + i * 4..offset + 8 + i * 4].copy_from_slice(&value.to_le_bytes());
    }
}
impl Object {
    /// Current quaternion from the cached CreateObject (w, x, y, z).
    pub fn orientation(&self) -> [f32; 4] {
        position_offset(&self.spawn).map_or(self.home.1, |offset| {
            std::array::from_fn(|i| {
                let at = offset + 16 + i * 4;
                f32::from_le_bytes(self.spawn[at..at + 4].try_into().unwrap())
            })
        })
    }
    pub fn home(&self) -> Placement {
        self.home.0
    }
    /// Placement name, else the entity's English default name.
    pub fn display_name(&self) -> Option<&str> {
        self.name.as_deref().or_else(|| crate::entity_info::name(self.entity))
    }
    pub fn stat(&self, key: u32) -> Option<u32> {
        self.stats.get(&key).map(|(value, _)| *value)
    }
    pub fn updates(&self) -> impl Iterator<Item = &Arc<[u8]>> {
        self.stats
            .get(&0x107)
            .map(|(_, wire)| wire)
            .into_iter()
            .chain(
                self.stats
                    .iter()
                    .filter(|(key, _)| **key != 0x107)
                    .map(|(_, (_, wire))| wire),
            )
    }
    fn stat_wire(&self, key: u32, value: u32) -> Arc<[u8]> {
        world::visual_int(self.id, self.instance, key, value).into()
    }
}
#[derive(Default)]
pub struct Content {
    objects: BTreeMap<u64, Object>,
    cells: BTreeMap<u32, Vec<u64>>,
}
impl Content {
    /// The shipped snapshot works independently of the current directory.
    pub fn bundled(game_data: &Path) -> io::Result<Self> {
        Self::parse(include_str!("../content/world.tsv"), game_data)
    }
    pub fn observed(game_data: &Path) -> io::Result<Self> {
        Self::parse(
            include_str!("../content/world-observed-expanded.tsv"),
            game_data,
        )
    }
    pub fn training(game_data: &Path) -> io::Result<Self> {
        let mut combined = String::from("pyrealium-world-v3\n");
        // Keep the retained actor snapshot when enabling the training preset;
        // replacing it with the portal/lifestone-only base leaves known actors
        // absent even though the server reports that world content is enabled.
        for line in include_str!("../content/world-observed-expanded.tsv")
            .lines()
            .skip(1)
        {
            combined.push_str(line);
            combined.push('\n');
        }
        for line in include_str!("../content/training-world.tsv")
            .lines()
            .skip(1)
        {
            combined.push_str(line);
            combined.push('\n');
        }
        for line in include_str!("../content/capture-spawns.tsv")
            .lines()
            .skip(1)
        {
            combined.push_str(line);
            combined.push('\n');
        }
        Self::parse(&combined, game_data)
    }
    pub fn len(&self) -> usize {
        self.objects.len()
    }
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }
    pub fn cell(&self, id: u64) -> Option<u32> {
        self.objects
            .get(&id)
            .filter(|object| object.active)
            .map(|object| object.placement.cell)
    }
    pub fn objects(&self) -> impl Iterator<Item = &Object> {
        self.objects.values()
    }
    pub fn object(&self, id: u64) -> Option<&Object> {
        self.objects.get(&id)
    }
    pub fn set_stat(&mut self, id: u64, key: u32, value: u32) -> io::Result<Option<Arc<[u8]>>> {
        let object = self
            .objects
            .get_mut(&id)
            .ok_or_else(|| invalid("unknown content object"))?;
        if object.stats.get(&key).is_some_and(|(old, _)| *old == value) {
            return Ok(None);
        }
        if object.stats.len() >= 16 && !object.stats.contains_key(&key) {
            return Err(invalid("content stat limit"));
        }
        let wire = object.stat_wire(key, value);
        object.stats.insert(key, (value, wire.clone()));
        Ok(object.active.then_some(wire))
    }
    pub fn despawn(&mut self, id: u64) -> io::Result<Arc<[u8]>> {
        let object = self
            .objects
            .get_mut(&id)
            .filter(|object| object.active)
            .ok_or_else(|| invalid("content object not active"))?;
        object.active = false;
        Ok(object.destroy.clone())
    }
    pub fn idol_presentation(
        &mut self,
        id: u64,
        action: crate::combat_visual::IdolAction,
    ) -> io::Result<Vec<Arc<[u8]>>> {
        let object = self
            .objects
            .get_mut(&id)
            .filter(|object| object.active)
            .ok_or_else(|| invalid("inactive creature presentation"))?;
        // Do not apply this behavior table to unrelated observed creatures.
        if object.entity != crate::quests::TARGET_ENTITY
            || object.spawn.get(16..20) != Some(0x1f0003d1u32.to_le_bytes().as_slice())
        {
            return Ok(Vec::new());
        }
        if object.spawn.get(20..24) != Some(0x8082u32.to_le_bytes().as_slice()) {
            return Err(invalid("unsupported Idol physics presentation"));
        }
        let (stamp, mode, events) =
            crate::combat_visual::idol(id, object.instance, object.visual_stamp, action)
                .map_err(|error| invalid(format!("creature presentation: {error}")))?;
        let spawn = Arc::make_mut(&mut object.spawn);
        spawn[26..28].copy_from_slice(&stamp.to_le_bytes());
        spawn[28..32].copy_from_slice(&mode.to_le_bytes());
        object.visual_stamp = stamp;
        Ok(events.into_iter().map(Arc::from).collect())
    }
    /// Next visual stamp on the victim. The story is presentation only.
    pub fn damage_story(
        &mut self,
        victim: u64,
        attacker: u64,
        health_change: i32,
    ) -> io::Result<Arc<[u8]>> {
        let object = self
            .objects
            .get_mut(&victim)
            .filter(|object| object.active)
            .ok_or_else(|| invalid("inactive creature damage story"))?;
        let stamp = object
            .visual_stamp
            .checked_add(1)
            .ok_or_else(|| invalid("creature visual stamp exhausted"))?;
        let wire = crate::combat_visual::damage_story(
            victim,
            object.instance,
            stamp,
            attacker,
            health_change,
        )
        .map_err(|error| invalid(format!("creature damage story: {error}")))?;
        if object.spawn.len() >= 28 {
            Arc::make_mut(&mut object.spawn)[26..28].copy_from_slice(&stamp.to_le_bytes());
        }
        object.visual_stamp = stamp;
        Ok(wire.into())
    }
    pub fn fx(&mut self, id: u64, fx: u32) -> io::Result<Arc<[u8]>> {
        let object = self
            .objects
            .get_mut(&id)
            .filter(|object| object.active)
            .ok_or_else(|| invalid("inactive object effect"))?;
        let stamp = object
            .visual_stamp
            .checked_add(1)
            .ok_or_else(|| invalid("object visual stamp exhausted"))?;
        let wire = crate::combat_visual::fx(id, object.instance, stamp, fx)
            .map_err(|error| invalid(format!("object effect: {error}")))?;
        if object.spawn.len() >= 28 {
            Arc::make_mut(&mut object.spawn)[26..28].copy_from_slice(&stamp.to_le_bytes());
        }
        object.visual_stamp = stamp;
        Ok(wire.into())
    }
    /// Same-cell server movement. Updates the cached CreateObject for late
    /// observers and returns the native server position update (0x10086).
    pub fn move_object(&mut self, id: u64, placement: Placement, motion: u32, time: f64) -> io::Result<Arc<[u8]>> {
        let object = self
            .objects
            .get_mut(&id)
            .filter(|object| object.active && !object.dynamic)
            .ok_or_else(|| invalid("content object not movable"))?;
        if placement.cell != object.placement.cell || !placement.valid() {
            return Err(invalid("content movement must stay inside its cell"));
        }
        let offset = position_offset(&object.spawn).ok_or_else(|| invalid("unsupported content physics layout"))?;
        let update = crate::movement::ClientPosition {
            time,
            placement,
            motion: (u32::from(placement.heading) << 24) | (motion & 0x00ff_ffff),
            cancel_moveto: false,
            stamps: [object.instance, 0, 0, 0],
            jump_velocity: None,
        };
        let stamp = object.position_stamp.wrapping_add(1);
        let wire = crate::movement::server_position(id, object.instance, stamp, &update)
            .map_err(|error| invalid(format!("content movement: {error}")))?;
        write_position(Arc::make_mut(&mut object.spawn), offset, placement, placement.orientation());
        object.placement = placement;
        object.position_stamp = stamp;
        Ok(wire.into())
    }
    /// Add a runtime corpse to its cell. The caller has built a valid spawn.
    pub fn insert_corpse(&mut self, id: u64, placement: Placement, orientation: [f32; 4], spawn: Vec<u8>) -> io::Result<()> {
        if self.objects.contains_key(&id) || self.objects.len() >= MAX_OBJECTS {
            return Err(invalid("corpse identity or object capacity"));
        }
        let cell_ids = self.cells.entry(placement.cell).or_default();
        if cell_ids.len() >= MAX_CELL_OBJECTS {
            return Err(invalid("this area cannot hold more objects"));
        }
        let mut destroy = 0x100c2u32.to_le_bytes().to_vec();
        destroy.extend(id.to_le_bytes());
        destroy.extend(1u16.to_le_bytes());
        destroy.extend(0u16.to_le_bytes());
        let at = cell_ids.partition_point(|&other| other < id);
        cell_ids.insert(at, id);
        self.objects.insert(id, Object {
            id,
            entity: world::CORPSE_ENTITY,
            placement,
            spawn: spawn.into(),
            destroy: destroy.into(),
            active: true,
            instance: 1,
            visual_stamp: 0,
            stats: BTreeMap::new(),
            home: (placement, orientation),
            position_stamp: 0,
            dynamic: true,
            name: Some("Creature Corpse".into()),
        });
        Ok(())
    }
    /// Remove a runtime corpse, returning its DestroyObject.
    pub fn remove_corpse(&mut self, id: u64) -> io::Result<(u32, Arc<[u8]>)> {
        if !self.objects.get(&id).is_some_and(|object| object.dynamic) {
            return Err(invalid("not a runtime corpse"));
        }
        let object = self.objects.remove(&id).unwrap();
        if let Some(ids) = self.cells.get_mut(&object.placement.cell) {
            ids.retain(|&other| other != id);
            if ids.is_empty() {
                self.cells.remove(&object.placement.cell);
            }
        }
        Ok((object.placement.cell, object.destroy))
    }
    pub fn respawn(&mut self, id: u64) -> io::Result<()> {
        let object = self
            .objects
            .get_mut(&id)
            .filter(|object| !object.active && !object.dynamic)
            .ok_or_else(|| invalid("content object already active"))?;
        if object.placement != object.home.0 {
            if let Some(offset) = position_offset(&object.spawn) {
                write_position(Arc::make_mut(&mut object.spawn), offset, object.home.0, object.home.1);
            }
            object.placement = object.home.0;
        }
        object.position_stamp = 0;
        object.instance = object
            .instance
            .checked_add(1)
            .ok_or_else(|| invalid("content lifetime exhausted"))?;
        Arc::make_mut(&mut object.spawn)[24..26].copy_from_slice(&object.instance.to_le_bytes());
        object.visual_stamp = 0;
        Arc::make_mut(&mut object.spawn)[26..28].copy_from_slice(&0u16.to_le_bytes());
        if object.entity == crate::quests::TARGET_ENTITY
            && object.spawn.get(20..24) == Some(0x8082u32.to_le_bytes().as_slice())
        {
            Arc::make_mut(&mut object.spawn)[28..32]
                .copy_from_slice(&crate::combat_visual::PEACE.to_le_bytes());
        }
        Arc::make_mut(&mut object.destroy)[12..14].copy_from_slice(&object.instance.to_le_bytes());
        let keys = object
            .stats
            .iter()
            .map(|(&key, (value, _))| (key, *value))
            .collect::<Vec<_>>();
        for (key, value) in keys {
            let wire = object.stat_wire(key, value);
            object.stats.insert(key, (value, wire));
        }
        object.active = true;
        Ok(())
    }
    pub fn at(&self, cell: u32) -> impl Iterator<Item = &Object> {
        self.cells
            .get(&cell)
            .into_iter()
            .flatten()
            .map(|id| &self.objects[id])
            .filter(|object| object.active)
    }
    /// V1 uses IID/entity/visual/cell/X/Y/Z/heading; v2 replaces heading with
    /// quaternion W/X/Y/Z. Numeric identifiers are hex; positions are decimal.
    pub fn load(path: &Path, game_data: &Path) -> io::Result<Self> {
        let mut bytes = Vec::new();
        File::open(path)?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(invalid("world content exceeds 4 MiB"));
        }
        let text =
            std::str::from_utf8(&bytes).map_err(|_| invalid("world content is not UTF-8"))?;
        Self::parse(text, game_data)
    }
    fn parse(text: &str, game_data: &Path) -> io::Result<Self> {
        if text.len() as u64 > MAX_BYTES {
            return Err(invalid("world content exceeds 4 MiB"));
        }
        let mut lines = text.lines();
        let version = match lines.next() {
            Some("pyrealium-world-v1") => 1,
            Some("pyrealium-world-v2") => 2,
            Some("pyrealium-world-v3") => 3,
            Some("pyrealium-world-v4") => 4,
            _ => return Err(invalid("unsupported world content header")),
        };
        let overrides: BTreeMap<u64, &str> = crate::entity_info::object_names().collect();
        let mut content = Self::default();
        let mut portal = Dat::open(game_data.join("portal.dat"))?;
        let mut cells = Dat::open(game_data.join("cell_1.dat"))?;
        let mut assets = BTreeSet::new();
        let mut checked_cells = BTreeSet::new();
        for (line, text) in lines.enumerate() {
            if text.is_empty() || text.starts_with('#') {
                continue;
            }
            let parse = || -> io::Result<(u64, u32, u32, Placement, [f32; 4], Option<f32>, Option<String>)> {
                if text.len() > 512 {
                    return Err(invalid("placement exceeds 512 bytes"));
                }
                let fields: Vec<_> = text.split('\t').collect();
                // v3 and v4 rows may carry an optional trailing name column.
                let expected = match version {
                    1 => 8..=8,
                    2 => 11..=11,
                    _ => 12..=13,
                };
                if !expected.contains(&fields.len()) {
                    return Err(invalid(
                        "placement field count does not match manifest version",
                    ));
                }
                let hex = |value: &str| {
                    u64::from_str_radix(value.strip_prefix("0x").unwrap_or(value), 16)
                        .map_err(|_| invalid("invalid hex identifier"))
                };
                let narrow = |index: usize| {
                    u32::try_from(hex(fields[index])?)
                        .map_err(|_| invalid("identifier exceeds u32"))
                };
                let id = hex(fields[0])?;
                let entity = narrow(1)?;
                let visual = narrow(2)?;
                let placement = Placement {
                    cell: narrow(3)?,
                    coordinates: [
                        fields[4].parse().map_err(|_| invalid("invalid X"))?,
                        fields[5].parse().map_err(|_| invalid("invalid Y"))?,
                        fields[6].parse().map_err(|_| invalid("invalid Z"))?,
                    ],
                    heading: if version >= 2 {
                        0
                    } else {
                        fields[7].parse().map_err(|_| invalid("invalid heading"))?
                    },
                };
                let orientation = if version >= 2 {
                    let mut rotation = [0.0f32; 4];
                    for (value, text) in rotation.iter_mut().zip(&fields[7..11]) {
                        *value = text.parse().map_err(|_| invalid("invalid quaternion"))?;
                    }
                    rotation
                } else {
                    placement.orientation()
                };
                let norm: f32 = orientation.iter().map(|value| value * value).sum();
                if !orientation.iter().all(|value| value.is_finite())
                    || !norm.is_finite()
                    || (norm - 1.0).abs() > 0.01
                {
                    return Err(invalid("invalid or non-normalized content quaternion"));
                }
                if id >> 48 != NAMESPACE >> 48
                    || id == NAMESPACE
                    || entity >> 24 != 0x47
                    || visual >> 24 != 0x1f
                    || !placement.valid()
                {
                    return Err(invalid(
                        "invalid placement identity, asset type or position",
                    ));
                }
                let name = fields
                    .get(12)
                    .filter(|name| **name != "-")
                    .map(|name| name.to_string())
                    .filter(|name| {
                        !name.is_empty()
                            && name.encode_utf16().count() <= 64
                            && !name.chars().any(char::is_control)
                    });
                let scale = if version >= 3 && fields[11] != "-" {
                    let scale: f32 = fields[11]
                        .parse()
                        .map_err(|_| invalid("invalid content scale"))?;
                    if !scale.is_finite() || scale <= 0.0 || scale > 100.0 {
                        return Err(invalid("content scale outside local bounds"));
                    }
                    Some(scale)
                } else {
                    None
                };
                Ok((id, entity, visual, placement, orientation, scale, name))
            };
            let (id, entity, visual, placement, orientation, scale, name) = parse()
                .map_err(|error| invalid(format!("world content line {}: {error}", line + 2)))?;
            let name = overrides.get(&id).map(|name| name.to_string()).or(name);
            if content.objects.len() >= MAX_OBJECTS || content.objects.contains_key(&id) {
                return Err(invalid("world content object limit or duplicate IID"));
            }
            let cell_ids = content.cells.entry(placement.cell).or_default();
            if cell_ids.len() >= MAX_CELL_OBJECTS {
                return Err(invalid("world content exceeds 64 objects in one cell"));
            }
            // Retail server-only weenies are absent from the client DAT; the
            // client draws those objects from their visual alone.
            if assets.insert(visual) {
                portal
                    .read(visual)
                    .map_err(|error| invalid(format!("content asset {visual:#010x}: {error}")))?;
            }
            let cell_record = crate::dat::world_cell_record(placement.cell)?;
            if checked_cells.insert(cell_record) {
                cells.read(cell_record).map_err(|error| {
                    invalid(format!("content cell {:#010x}: {error}", placement.cell))
                })?;
            }
            let spawn = world::create_named_object(
                &StationaryObject {
                    character_id: id,
                    parent_visual_did: visual,
                    entity_did: entity,
                    cell_id: placement.cell,
                    position: placement.coordinates,
                    orientation,
                    instance_stamp: 1,
                    visual_ordering_stamp: 0,
                    timestamps: [0; 4],
                },
                scale,
                name.as_deref(),
            )
            .map_err(|error| invalid(format!("content object: {error}")))?;
            let mut destroy = 0x100c2u32.to_le_bytes().to_vec();
            destroy.extend(id.to_le_bytes());
            destroy.extend(1u16.to_le_bytes());
            destroy.extend(0u16.to_le_bytes());
            content.objects.insert(
                id,
                Object {
                    id,
                    entity,
                    placement,
                    spawn: spawn.into(),
                    destroy: destroy.into(),
                    active: true,
                    instance: 1,
                    visual_stamp: 0,
                    stats: BTreeMap::new(),
                    home: (placement, orientation),
                    position_stamp: 0,
                    dynamic: false,
                    name,
                },
            );
            cell_ids.push(id);
        }
        for ids in content.cells.values_mut() {
            ids.sort_unstable();
        }
        Ok(content)
    }
}
