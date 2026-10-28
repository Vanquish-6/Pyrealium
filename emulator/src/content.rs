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
    /// Combat stance, swing and death read from each visual's own behavior table.
    poses: BTreeMap<u32, crate::combat_visual::Pose>,
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
        let visual = object
            .spawn
            .get(16..20)
            .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()));
        let flags = object
            .spawn
            .get(20..24)
            .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()));
        let pose = visual.and_then(|visual| self.poses.get(&visual).copied());
        // A missing pose means this visual has no combat mode of its own.
        // Sending another creature's stance faults PhysicsObjRep.
        if pose.is_none() || !matches!(flags, Some(0x8080 | 0x8082)) {
            return Ok(Vec::new());
        }
        let (stamp, mode, events) = crate::combat_visual::present(
            id,
            object.instance,
            object.visual_stamp,
            action,
            pose.unwrap(),
        )
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
    /// Server movement. Outdoor steps may cross the 20 m cell grid. Interior
    /// movement stays in its cell. Returns the native server position (0x10086).
    pub fn move_object(&mut self, id: u64, placement: Placement, motion: u32, time: f64) -> io::Result<Arc<[u8]>> {
        let old_cell = {
            let object = self
                .objects
                .get(&id)
                .filter(|object| object.active && !object.dynamic)
                .ok_or_else(|| invalid("content object not movable"))?;
            if !placement.valid() {
                return Err(invalid("content movement is not a valid position"));
            }
            if placement.cell != object.placement.cell
                && !(crate::movement::outdoor_cell(placement.cell)
                    && crate::movement::outdoor_cell(object.placement.cell))
            {
                return Err(invalid("content movement must stay inside its cell"));
            }
            if placement.cell != object.placement.cell
                && self
                    .cells
                    .get(&placement.cell)
                    .is_some_and(|ids| ids.len() >= MAX_CELL_OBJECTS)
            {
                return Err(invalid("this area cannot hold more objects"));
            }
            object.placement.cell
        };
        let object = self.objects.get_mut(&id).unwrap();
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
        let spawn = Arc::make_mut(&mut object.spawn);
        spawn[offset..offset + 4].copy_from_slice(&placement.cell.to_le_bytes());
        write_position(spawn, offset, placement, placement.orientation());
        object.placement = placement;
        object.position_stamp = stamp;
        if old_cell != placement.cell {
            self.reindex(id, old_cell, placement.cell);
        }
        Ok(wire.into())
    }
    fn reindex(&mut self, id: u64, from: u32, to: u32) {
        if let Some(ids) = self.cells.get_mut(&from) {
            ids.retain(|&other| other != id);
            if ids.is_empty() {
                self.cells.remove(&from);
            }
        }
        let ids = self.cells.entry(to).or_default();
        let at = ids.partition_point(|&other| other < id);
        if ids.get(at) != Some(&id) {
            ids.insert(at, id);
        }
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
        let visual = u32::from_le_bytes(object.spawn.get(16..20).unwrap_or(&[0; 4]).try_into().unwrap());
        let flags = u32::from_le_bytes(object.spawn.get(20..24).unwrap_or(&[0; 4]).try_into().unwrap());
        if let Some(pose) = self.poses.get(&visual).filter(|_| matches!(flags, 0x8080 | 0x8082)) {
            let peace = pose.peace;
            Arc::make_mut(&mut object.spawn)[28..32].copy_from_slice(&peace.to_le_bytes());
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
        let visual_only_state = 0x71000000 | (world::VISUAL_ONLY_ENTITY & 0x00FFFFFF);
        if portal.find(world::VISUAL_ONLY_ENTITY)?.is_some()
            || portal.find(visual_only_state)?.is_some()
        {
            return Err(invalid(
                "visual-only content entity unexpectedly present in portal.dat",
            ));
        }
        let mut cells = Dat::open(game_data.join("cell_1.dat"))?;
        let mut assets = BTreeSet::new();
        let mut visual_motion: BTreeMap<u32, Option<u32>> = BTreeMap::new();
        let mut behavior_motion: BTreeMap<u32, Option<u32>> = BTreeMap::new();
        let mut behavior_pose: BTreeMap<u32, Option<crate::combat_visual::Pose>> = BTreeMap::new();
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
            let name = overrides
                .get(&id)
                .map(|name| (*name).to_string())
                .or(name)
                .or_else(|| {
                    crate::entity_info::name(entity)
                        .filter(|name| presentation_name(name))
                        .map(str::to_string)
                });
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
                let record = portal
                    .read(visual)
                    .map_err(|error| invalid(format!("content asset {visual:#010x}: {error}")))?;
                let mode = table_motion(&mut portal, &record, &mut behavior_motion).map_err(
                    |error| invalid(format!("content visual {visual:#010x}: {error}")),
                )?;
                visual_motion.insert(visual, mode);
                if let Some(table) = behavior_table(&record) {
                    let pose = if let Some(pose) = behavior_pose.get(&table) {
                        *pose
                    } else {
                        let bytes = portal.read(table).unwrap_or_default();
                        let pose = pose_from_table(table, &bytes);
                        behavior_pose.insert(table, pose);
                        pose
                    };
                    if let Some(pose) = pose {
                        content.poses.insert(visual, pose);
                    }
                }
            }
            let cell_record = crate::dat::world_cell_record(placement.cell)?;
            if checked_cells.insert(cell_record) {
                cells.read(cell_record).map_err(|error| {
                    invalid(format!("content cell {:#010x}: {error}", placement.cell))
                })?;
            }
            let in_dat = portal.find(entity)?.is_some();
            // Standing comes from the visual's own behavior table, not from
            // the entity being a creature. A mode on a static mesh faults
            // PhysicsObjRep; a missing mode on an animated visual does too.
            let mode = visual_motion.get(&visual).copied().flatten();
            let spawn = world::create_named_object(
                &StationaryObject {
                    character_id: id,
                    parent_visual_did: visual,
                    entity_did: world::world_content_entity(entity, in_dat),
                    cell_id: placement.cell,
                    position: placement.coordinates,
                    orientation,
                    instance_stamp: 1,
                    visual_ordering_stamp: 0,
                    timestamps: [0; 4],
                },
                scale,
                name.as_deref(),
                mode,
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

/// BehaviorTable::StreamPack (0067d000). Combat stance and swing come from
/// this table only: the monster table keeps its monster stance, and every
/// other table uses one-hand when that mode is actually present.
fn pose_from_table(table: u32, bytes: &[u8]) -> Option<crate::combat_visual::Pose> {
    if bytes.len() < 8 || u32::from_le_bytes(bytes[..4].try_into().ok()?) != table {
        return None;
    }
    let mut walk = Walk { bytes, at: 4 };
    walk.skip_entries(20)?;
    let mut modes = BTreeSet::new();
    let combos = walk.count()?;
    if combos > 100_000 {
        return None;
    }
    for _ in 0..combos {
        let _element = walk.u32()?;
        modes.insert(walk.u32()?);
        walk.skip(12)?;
    }
    walk.skip_entries(28)?;
    walk.skip_words()?;
    walk.skip_words()?;
    let transitions = walk.count()?;
    if transitions > 100_000 {
        return None;
    }
    for _ in 0..transitions {
        walk.skip(8)?;
        let listed = walk.count()?;
        if listed > 10_000 {
            return None;
        }
        walk.skip(listed.checked_mul(8)?)?;
    }
    let listed = walk.count()?;
    if listed > 100_000 {
        return None;
    }
    for _ in 0..listed {
        walk.u32()?;
        modes.insert(walk.u32()?);
        walk.u32()?;
    }
    let mut behaviors = BTreeSet::new();
    let behavior_count = walk.count()?;
    if behavior_count > 100_000 {
        return None;
    }
    for _ in 0..behavior_count {
        behaviors.insert(walk.u32()?);
        walk.skip(8)?;
    }
    walk.skip_words()?;
    let peace = walk.u32()?;
    if walk.at != bytes.len() {
        return None;
    }
    let combat = if table == crate::combat_visual::MONSTER_TABLE
        && modes.contains(&crate::combat_visual::MONSTER_STANCE)
    {
        crate::combat_visual::MONSTER_STANCE
    } else if modes.contains(&crate::combat_visual::ONE_HAND_STANCE) {
        crate::combat_visual::ONE_HAND_STANCE
    } else if modes.contains(&crate::combat_visual::MONSTER_STANCE) {
        crate::combat_visual::MONSTER_STANCE
    } else {
        return None;
    };
    if combat == peace {
        return None;
    }
    let swing = [
        crate::combat_visual::MELEE_AUTO_1,
        crate::combat_visual::MELEE_AUTO_2,
        crate::combat_visual::MELEE_WEAK,
    ]
    .into_iter()
    .find(|id| behaviors.contains(id))
    .unwrap_or(0);
    Some(crate::combat_visual::Pose {
        peace,
        combat,
        swing,
        death_mode: u32::from(modes.contains(&crate::combat_visual::DEATH_MODE))
            * crate::combat_visual::DEATH_MODE,
        death_behavior: u32::from(behaviors.contains(&crate::combat_visual::DEATH_BEHAVIOR))
            * crate::combat_visual::DEATH_BEHAVIOR,
    })
}

struct Walk<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl Walk<'_> {
    fn u32(&mut self) -> Option<u32> {
        let next = self.at.checked_add(4)?;
        let word = u32::from_le_bytes(self.bytes.get(self.at..next)?.try_into().ok()?);
        self.at = next;
        Some(word)
    }
    fn count(&mut self) -> Option<usize> {
        usize::try_from(self.u32()? & 0x00ff_ffff).ok()
    }
    fn skip(&mut self, bytes: usize) -> Option<()> {
        self.at = self.at.checked_add(bytes)?;
        (self.at <= self.bytes.len()).then_some(())
    }
    fn skip_entries(&mut self, entry: usize) -> Option<()> {
        let count = self.count()?;
        self.skip(count.checked_mul(entry)?)
    }
    fn skip_words(&mut self) -> Option<()> {
        let count = self.count()?;
        if count > 100_000 {
            return None;
        }
        self.skip(count.checked_mul(4)?)
    }
}

fn presentation_name(name: &str) -> bool {
    !name.is_empty()
        && name.encode_utf16().count() <= 64
        && !name.chars().any(char::is_control)
}

/// VisualDesc::StreamPack (0063d7f0). Bit 3 is the behavior table, packed
/// after the identity and the optional parent and motion-info words.
fn behavior_table(visual: &[u8]) -> Option<u32> {
    if visual.len() < 8 {
        return None;
    }
    let flags = u32::from_le_bytes(visual[..4].try_into().ok()?);
    if flags & 8 == 0 {
        return None;
    }
    let mut offset = 4usize;
    for bit in [1u32, 2, 4] {
        if flags & bit != 0 {
            offset = offset.checked_add(4)?;
        }
    }
    let behavior = u32::from_le_bytes(visual.get(offset..offset + 4)?.try_into().ok()?);
    (behavior >> 24 == 0x44).then_some(behavior)
}

/// mInitMode is the last word of BehaviorTable::StreamPack (0067d000).
fn table_motion(
    portal: &mut Dat,
    visual: &[u8],
    cache: &mut BTreeMap<u32, Option<u32>>,
) -> io::Result<Option<u32>> {
    let Some(behavior) = behavior_table(visual) else {
        return Ok(None);
    };
    if let Some(mode) = cache.get(&behavior) {
        return Ok(*mode);
    }
    let record = portal.read(behavior).map_err(|error| {
        invalid(format!("behavior table {behavior:#010x}: {error}"))
    })?;
    if record.len() < 8 || u32::from_le_bytes(record[..4].try_into().unwrap()) != behavior {
        return Err(invalid(format!(
            "behavior table {behavior:#010x} is not a 7148 BehaviorTable"
        )));
    }
    let mode = u32::from_le_bytes(record[record.len() - 4..].try_into().unwrap());
    let mode = matches!(mode, 0x40000001 | 0x41000003 | 0x41000010).then_some(mode);
    cache.insert(behavior, mode);
    Ok(mode)
}
