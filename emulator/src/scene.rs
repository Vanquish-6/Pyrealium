//! Single-owner player presence. Cell-indexed recipients and shared immutable payloads.
//! Interior interest uses native visible-cell sets; outdoor interest is a
//! separately labeled development policy. Neither grants collision/LOS authority.
use crate::{
    movement::{self, ClientPosition, Placement},
    world::{self, WorldError},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Debug)]
pub struct Body {
    pub id: u64,
    pub placement: Placement,
    instance: u16,
    spawn: Vec<u8>,
    /// Start of PhysicsDesc: 12 + the VisualDesc length.
    physics: usize,
    position_offset: usize,
    ready: bool,
    public_stats: BTreeMap<(u32, u32), (u32, Arc<[u8]>)>,
    hands: BTreeMap<u64, Arc<[u8]>>,
    item_instances: BTreeMap<u64, u16>,
}
impl Body {
    pub fn new(id: u64, placement: Placement, spawn: Vec<u8>) -> Result<Self, WorldError> {
        if !placement.valid()
            || spawn.len() < 68
            || spawn.len() > 65536
            || spawn[..4] != world::CREATE_OBJECT.to_le_bytes()
            || spawn[4..12] != id.to_le_bytes()
            || id == 0
        {
            return Err(WorldError::InvalidValue);
        }
        let physics = 12 + crate::appearance::visual_len(&spawn[12..]).ok_or(WorldError::InvalidValue)?;
        if spawn.len() < physics + 8 {
            return Err(WorldError::InvalidValue);
        }
        let instance = u16::from_le_bytes(spawn[physics + 4..physics + 6].try_into().unwrap());
        if instance == 0 {
            return Err(WorldError::InvalidValue);
        }
        // Generated native subsets place optional mode/slider data before
        // Position. Never overwrite those fields using a fixed cell offset.
        let flags = u32::from_le_bytes(spawn[physics..physics + 4].try_into().unwrap());
        let position_offset = physics
            + match flags {
                0x8000 => 8,
                0x8080 => 12,
                0x8082 => 28,
                _ => return Err(WorldError::InvalidValue),
            };
        if spawn.len() < position_offset + 32 {
            return Err(WorldError::InvalidValue);
        }
        Ok(Self {
            id,
            placement,
            instance,
            spawn,
            physics,
            position_offset,
            ready: false,
            public_stats: BTreeMap::new(),
            hands: BTreeMap::new(),
            item_instances: BTreeMap::new(),
        })
    }
    pub fn with_progression(mut self, state: &crate::progression::Progression) -> Self {
        self.save_public_stats(state);
        self
    }
    pub fn with_hands(mut self, spawns: Vec<Vec<u8>>) -> Result<Self, WorldError> {
        if spawns.len() > 2 {
            return Err(WorldError::LimitExceeded);
        }
        for spawn in spawns {
            if world::hand_parent(&spawn)? != (self.id, self.instance) {
                return Err(WorldError::InvalidValue);
            }
            let id = u64::from_le_bytes(spawn[4..12].try_into().unwrap());
            let instance = u16::from_le_bytes(spawn[24..26].try_into().unwrap());
            if id == self.id || instance == 0 || self.hands.contains_key(&id) {
                return Err(WorldError::InvalidValue);
            }
            self.item_instances.insert(id, instance);
            self.hands.insert(id, spawn.into());
        }
        Ok(self)
    }
    fn hand_spawns(&self, recipient: u64, deliveries: &mut Vec<Delivery>) {
        for data in self.hands.values() {
            deliveries.push(Delivery {
                recipient,
                data: data.clone(),
            });
        }
    }
    fn hand_destroys(&self, recipient: u64, deliveries: &mut Vec<Delivery>) {
        for id in self.hands.keys() {
            deliveries.push(Delivery {
                recipient,
                data: world::destroy_inventory_item(*id, self.item_instances[id]).into(),
            });
        }
    }
    fn save_public_stats(&mut self, state: &crate::progression::Progression) -> Vec<Arc<[u8]>> {
        let mut updates = Vec::new();
        for (key, value) in [
            (0x107, state.level().health),
            (0x102, state.health),
            (0x12e, u32::from(state.level().number)),
        ] {
            if self
                .public_stats
                .get(&(0x1006a, key))
                .is_some_and(|(old, _)| *old == value)
            {
                continue;
            }
            let wire: Arc<[u8]> = world::visual_int(self.id, self.instance, key, value).into();
            self.public_stats
                .insert((0x1006a, key), (value, wire.clone()));
            updates.push(wire);
        }
        let dead = u32::from(!state.alive());
        if !self
            .public_stats
            .get(&(0x1006c, 0x117))
            .is_some_and(|(value, _)| *value == dead)
        {
            let wire: Arc<[u8]> =
                world::visual_bool(self.id, self.instance, 0x117, !state.alive()).into();
            self.public_stats
                .insert((0x1006c, 0x117), (dead, wire.clone()));
            updates.push(wire);
        }
        updates
    }
    fn relocate(&mut self, placement: Placement) {
        self.placement = placement;
        let offset = self.position_offset;
        self.spawn[offset..offset + 4].copy_from_slice(&placement.cell.to_le_bytes());
        for (i, n) in placement
            .coordinates
            .iter()
            .chain(placement.orientation().iter())
            .enumerate()
        {
            self.spawn[offset + 4 + i * 4..offset + 8 + i * 4].copy_from_slice(&n.to_le_bytes());
        }
    }
    fn destroy(&self) -> Arc<[u8]> {
        // DispatchEvent 00494010 -> 0x100c2; RecvEvt_DestroyObject 004951f0
        // consumes InstanceIDWithStamp (004946d0).
        let mut bytes = Vec::with_capacity(16);
        bytes.extend(0x100c2u32.to_le_bytes());
        bytes.extend(self.id.to_le_bytes());
        bytes.extend(self.instance.to_le_bytes());
        bytes.extend(0u16.to_le_bytes());
        bytes.into()
    }
}

#[derive(Debug)]
pub struct Delivery {
    pub recipient: u64,
    pub data: Arc<[u8]>,
}

#[derive(Default)]
pub struct Scene {
    visibility: Option<crate::visibility::Visibility>,
    bodies: BTreeMap<u64, Body>,
    cells: BTreeMap<u32, BTreeSet<u64>>,
    content: crate::content::Content,
}
impl Scene {
    pub fn with_visibility(mut self, visibility: crate::visibility::Visibility) -> Self {
        self.visibility = Some(visibility);
        self
    }
    pub fn prepare_cell(&mut self, cell: u32) -> std::io::Result<()> {
        if let Some(visibility) = &mut self.visibility {
            visibility.prepare(cell)?;
        }
        Ok(())
    }
    fn sees_cell(&self, observer: u32, target: u32) -> bool {
        self.visibility
            .as_ref()
            .map_or(observer == target, |visibility| {
                visibility.visible(observer, target)
            })
    }
    fn visible_bodies(&self, cell: u32, except: u64) -> BTreeSet<u64> {
        self.bodies
            .values()
            .filter(|body| {
                body.ready && body.id != except && self.sees_cell(cell, body.placement.cell)
            })
            .map(|body| body.id)
            .collect()
    }
    fn visible_content(&self, cell: u32) -> BTreeSet<u64> {
        let cells = self
            .visibility
            .as_ref()
            .map_or_else(|| vec![cell], |visibility| visibility.cells(cell));
        cells
            .into_iter()
            .flat_map(|cell| self.content.at(cell).map(|object| object.id))
            .collect()
    }
    /// Keep the inherited mode in late-observer CreateObject snapshots.
    pub fn save_combat_mode(
        &mut self,
        owner: u64,
        fighting: bool,
        stamp: u16,
    ) -> Result<(), WorldError> {
        let body = self
            .bodies
            .get_mut(&owner)
            .filter(|body| body.ready)
            .ok_or(WorldError::InvalidValue)?;
        if body.position_offset >= body.physics + 12 {
            let p = body.physics;
            body.spawn[p + 6..p + 8].copy_from_slice(&stamp.to_le_bytes());
            body.spawn[p + 8..p + 12].copy_from_slice(
                &(if fighting {
                    crate::combat_visual::ONE_HAND
                } else {
                    crate::combat_visual::PEACE
                })
                .to_le_bytes(),
            );
        }
        Ok(())
    }
    /// Advance the CreateObject ordering stamp without changing combat mode.
    pub fn save_visual_stamp(&mut self, owner: u64, stamp: u16) -> Result<(), WorldError> {
        let body = self
            .bodies
            .get_mut(&owner)
            .filter(|body| body.ready)
            .ok_or(WorldError::InvalidValue)?;
        if stamp == 0 {
            return Err(WorldError::InvalidValue);
        }
        let p = body.physics;
        body.spawn[p + 6..p + 8].copy_from_slice(&stamp.to_le_bytes());
        Ok(())
    }
    pub fn inventory_instance(&self, owner: u64, item: u64) -> Result<u16, WorldError> {
        let body = self
            .bodies
            .get(&owner)
            .filter(|body| body.ready)
            .ok_or(WorldError::InvalidValue)?;
        Ok(body
            .item_instances
            .get(&item)
            .copied()
            .unwrap_or(body.instance))
    }
    pub fn admit_inventory_grant(&self, owner: u64, item: u64) -> Result<u16, WorldError> {
        let body = self
            .bodies
            .get(&owner)
            .filter(|body| body.ready)
            .ok_or(WorldError::InvalidValue)?;
        if item == 0
            || item == owner
            || body.item_instances.contains_key(&item)
            || body.item_instances.len() >= 89
        {
            return Err(WorldError::LimitExceeded);
        }
        Ok(body.instance)
    }
    pub fn inventory_granted(
        &mut self,
        owner: u64,
        item: u64,
        instance: u16,
    ) -> Result<(), WorldError> {
        if self.admit_inventory_grant(owner, item)? != instance {
            return Err(WorldError::InvalidValue);
        }
        self.bodies
            .get_mut(&owner)
            .unwrap()
            .item_instances
            .insert(item, instance);
        Ok(())
    }
    pub fn inventory_removed(&mut self, owner: u64, item: u64) -> Result<(), WorldError> {
        let body = self
            .bodies
            .get_mut(&owner)
            .filter(|body| body.ready)
            .ok_or(WorldError::InvalidValue)?;
        if body.hands.contains_key(&item) {
            return Err(WorldError::InvalidValue);
        }
        body.item_instances.remove(&item);
        Ok(())
    }
    pub fn next_item_instance(&self, owner: u64, item: u64) -> Result<u16, WorldError> {
        let body = self
            .bodies
            .get(&owner)
            .filter(|body| body.ready)
            .ok_or(WorldError::InvalidValue)?;
        body.item_instances
            .get(&item)
            .copied()
            .unwrap_or(body.instance)
            .checked_add(1)
            .ok_or(WorldError::LimitExceeded)
    }
    pub fn validate_hand_change(
        &self,
        owner: u64,
        item: u64,
        spawn: &[u8],
        equipped: bool,
    ) -> Result<u16, WorldError> {
        let instance = self.next_item_instance(owner, item)?;
        let body = &self.bodies[&owner];
        if (equipped && world::hand_parent(spawn)? != (owner, body.instance))
            || (!equipped && (spawn.len() != 56 || spawn[20..24] != 0u32.to_le_bytes()))
            || spawn[..4] != world::CREATE_OBJECT.to_le_bytes()
            || spawn[4..12] != item.to_le_bytes()
            || spawn[24..26] != instance.to_le_bytes()
        {
            return Err(WorldError::InvalidValue);
        }
        if body.item_instances.len() >= 89 && !body.item_instances.contains_key(&item)
            || equipped && body.hands.len() >= 2 && !body.hands.contains_key(&item)
        {
            return Err(WorldError::LimitExceeded);
        }
        Ok(instance)
    }
    pub fn hand_change(
        &mut self,
        owner: u64,
        item: u64,
        spawn: Vec<u8>,
        equipped: bool,
    ) -> Result<Vec<Delivery>, WorldError> {
        let instance = self.validate_hand_change(owner, item, &spawn, equipped)?;
        let body = self.bodies.get_mut(&owner).unwrap();
        let old_instance = body
            .item_instances
            .get(&item)
            .copied()
            .unwrap_or(body.instance);
        let previously_equipped = body.hands.remove(&item).is_some();
        let spawn: Arc<[u8]> = spawn.into();
        body.item_instances.insert(item, instance);
        if equipped {
            body.hands.insert(item, spawn.clone());
        }
        let cell = body.placement.cell;
        let destroy: Arc<[u8]> = world::destroy_inventory_item(item, old_instance).into();
        let mut deliveries = vec![
            Delivery {
                recipient: owner,
                data: destroy.clone(),
            },
            Delivery {
                recipient: owner,
                data: spawn.clone(),
            },
        ];
        for recipient in self.neighbor_ids(cell, owner) {
            if previously_equipped {
                deliveries.push(Delivery {
                    recipient,
                    data: destroy.clone(),
                });
            }
            if equipped {
                deliveries.push(Delivery {
                    recipient,
                    data: spawn.clone(),
                });
            }
        }
        Ok(deliveries)
    }
    pub fn player_stats(
        &mut self,
        id: u64,
        state: &crate::progression::Progression,
    ) -> Vec<Delivery> {
        let Some(body) = self.bodies.get_mut(&id) else {
            return Vec::new();
        };
        let updates = body.save_public_stats(state);
        let cell = body.placement.cell;
        if !body.ready {
            return Vec::new();
        }
        let mut deliveries = Vec::new();
        for recipient in self.neighbor_ids(cell, id) {
            for data in &updates {
                deliveries.push(Delivery {
                    recipient,
                    data: data.clone(),
                });
            }
        }
        deliveries
    }
    pub fn player_placement(&self, id: u64) -> Option<Placement> {
        self.bodies
            .get(&id)
            .filter(|body| body.ready)
            .map(|body| body.placement)
    }
    pub fn content_placement(&self, id: u64) -> Option<Placement> {
        self.content
            .object(id)
            .filter(|_| self.content.cell(id).is_some())
            .map(|object| object.placement)
    }
    pub fn examination_content(
        &self,
        observer: u64,
        target: u64,
    ) -> Option<&crate::content::Object> {
        self.can_observe(observer, target)
            .then(|| self.content.object(target))
            .flatten()
    }
    /// Find an active, visible service of the pinned entity type within reach.
    /// Coordinates are only comparable inside the same cell; this does not
    /// infer reach through walls or between unrelated interior coordinate spaces.
    pub fn nearby_service(&self, observer: u64, entity: u32, target: Option<u64>) -> Option<u64> {
        let player = self.player_placement(observer)?;
        self.content.at(player.cell)
            .filter(|object| object.entity == entity
                && target.is_none_or(|id| object.id == id)
                && self.can_observe(observer, object.id))
            .filter_map(|object| {
                let distance: f32 = player.coordinates.iter().zip(object.placement.coordinates)
                    .map(|(a, b)| (a - b).powi(2)).sum();
                (distance.is_finite() && distance <= 16.0).then_some((distance, object.id))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)))
            .map(|(_, id)| id)
    }
    /// Nearest visible vendor NPC within four metres in the observer's cell.
    pub fn nearby_vendor(&self, observer: u64, target: Option<u64>) -> Option<u64> {
        let player = self.player_placement(observer)?;
        self.content.at(player.cell)
            .filter(|object| crate::entity_info::is_vendor(object.entity)
                && target.is_none_or(|id| object.id == id)
                && self.can_observe(observer, object.id))
            .filter_map(|object| {
                let distance: f32 = player.coordinates.iter().zip(object.placement.coordinates)
                    .map(|(a, b)| (a - b).powi(2)).sum();
                (distance.is_finite() && distance <= 16.0).then_some((distance, object.id))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)))
            .map(|(_, id)| id)
    }
    pub fn visible_hand_definition(&self, observer: u64, target: u64) -> Option<u32> {
        for body in self.bodies.values() {
            if self.can_observe(observer, body.id) {
                if let Some(spawn) = body.hands.get(&target) {
                    // Cached native hand-object subset has a fixed 72-byte shape.
                    return Some(u32::from_le_bytes(spawn[56..60].try_into().unwrap()));
                }
            }
        }
        None
    }
    pub fn content_stat(
        &mut self,
        id: u64,
        key: u32,
        value: u32,
    ) -> std::io::Result<Vec<Delivery>> {
        let Some(data) = self.content.set_stat(id, key, value)? else {
            return Ok(Vec::new());
        };
        let cell = self.content.cell(id).unwrap();
        Ok(self
            .neighbor_ids(cell, 0)
            .map(|recipient| Delivery {
                recipient,
                data: data.clone(),
            })
            .collect())
    }
    pub fn despawn_content(&mut self, id: u64) -> std::io::Result<Vec<Delivery>> {
        let cell = self
            .content
            .cell(id)
            .ok_or_else(|| std::io::Error::other("content object inactive"))?;
        let data = self.content.despawn(id)?;
        Ok(self
            .neighbor_ids(cell, 0)
            .map(|recipient| Delivery {
                recipient,
                data: data.clone(),
            })
            .collect())
    }
    pub fn content_damage_story(
        &mut self,
        victim: u64,
        attacker: u64,
        health_change: i32,
    ) -> std::io::Result<Vec<Delivery>> {
        let cell = self
            .content
            .cell(victim)
            .ok_or_else(|| std::io::Error::other("creature damage story inactive"))?;
        let data = self.content.damage_story(victim, attacker, health_change)?;
        Ok(self
            .neighbor_ids(cell, 0)
            .map(|recipient| Delivery {
                recipient,
                data: data.clone(),
            })
            .collect())
    }
    pub fn content_fx(&mut self, id: u64, fx: u32) -> std::io::Result<Vec<Delivery>> {
        let cell = self
            .content
            .cell(id)
            .ok_or_else(|| std::io::Error::other("object effect inactive"))?;
        let data = self.content.fx(id, fx)?;
        Ok(self
            .neighbor_ids(cell, 0)
            .map(|recipient| Delivery {
                recipient,
                data: data.clone(),
            })
            .collect())
    }
    pub fn idol_presentation(
        &mut self,
        id: u64,
        action: crate::combat_visual::IdolAction,
    ) -> std::io::Result<Vec<Delivery>> {
        let cell = self
            .content
            .cell(id)
            .ok_or_else(|| std::io::Error::other("creature inactive"))?;
        let events = self.content.idol_presentation(id, action)?;
        Ok(self
            .neighbor_ids(cell, 0)
            .flat_map(|recipient| {
                events
                    .iter()
                    .cloned()
                    .map(move |data| Delivery { recipient, data })
            })
            .collect())
    }
    pub fn move_content(
        &mut self,
        id: u64,
        placement: Placement,
        motion: u32,
        time: f64,
    ) -> std::io::Result<Vec<Delivery>> {
        let old = self
            .content
            .cell(id)
            .ok_or_else(|| std::io::Error::other("content object is not active"))?;
        let data = self.content.move_object(id, placement, motion, time)?;
        let new = self.content.cell(id).unwrap_or(old);
        if old == new {
            return Ok(self
                .neighbor_ids(new, 0)
                .map(|recipient| Delivery {
                    recipient,
                    data: data.clone(),
                })
                .collect());
        }
        let previous = self.neighbor_ids(old, 0).collect::<BTreeSet<_>>();
        let current = self.neighbor_ids(new, 0).collect::<BTreeSet<_>>();
        let object = self.content.object(id).unwrap();
        let mut deliveries = Vec::new();
        for recipient in previous.difference(&current) {
            deliveries.push(Delivery {
                recipient: *recipient,
                data: object.destroy.clone(),
            });
        }
        for recipient in current.difference(&previous) {
            deliveries.push(Delivery {
                recipient: *recipient,
                data: object.spawn.clone(),
            });
            for update in object.updates() {
                deliveries.push(Delivery {
                    recipient: *recipient,
                    data: update.clone(),
                });
            }
        }
        for recipient in current.intersection(&previous) {
            deliveries.push(Delivery {
                recipient: *recipient,
                data: data.clone(),
            });
        }
        Ok(deliveries)
    }
    /// Create a corpse where creature `of` currently stands.
    pub fn spawn_corpse(&mut self, id: u64, of: u64, name: &str) -> std::io::Result<Vec<Delivery>> {
        let object = self
            .content
            .object(of)
            .ok_or_else(|| std::io::Error::other("corpse source creature unknown"))?;
        let (placement, orientation) = (object.placement, object.orientation());
        let spawn = crate::world::create_corpse(id, placement.cell, placement.coordinates, orientation, name)
            .map_err(|error| std::io::Error::other(format!("corpse: {error}")))?;
        let data: Arc<[u8]> = spawn.clone().into();
        self.content.insert_corpse(id, placement, orientation, spawn)?;
        Ok(self
            .neighbor_ids(placement.cell, 0)
            .map(|recipient| Delivery {
                recipient,
                data: data.clone(),
            })
            .collect())
    }
    pub fn remove_corpse(&mut self, id: u64) -> std::io::Result<Vec<Delivery>> {
        let (cell, data) = self.content.remove_corpse(id)?;
        Ok(self
            .neighbor_ids(cell, 0)
            .map(|recipient| Delivery {
                recipient,
                data: data.clone(),
            })
            .collect())
    }
    pub fn respawn_content(&mut self, id: u64) -> std::io::Result<Vec<Delivery>> {
        self.content.respawn(id)?;
        let object = self.content.object(id).unwrap();
        let mut events = Vec::new();
        for recipient in self.neighbor_ids(object.placement.cell, 0) {
            events.push(Delivery {
                recipient,
                data: object.spawn.clone(),
            });
            for data in object.updates() {
                events.push(Delivery {
                    recipient,
                    data: data.clone(),
                });
            }
        }
        Ok(events)
    }
    pub fn with_content(content: crate::content::Content) -> Self {
        Self {
            content,
            ..Self::default()
        }
    }
    pub fn can_observe(&self, observer: u64, target: u64) -> bool {
        if let Some(cell) = self.content.cell(target) {
            return self
                .bodies
                .get(&observer)
                .is_some_and(|body| body.ready && self.sees_cell(body.placement.cell, cell));
        }
        self.bodies
            .get(&observer)
            .zip(self.bodies.get(&target))
            .is_some_and(|(a, b)| {
                a.ready && b.ready && self.sees_cell(a.placement.cell, b.placement.cell)
            })
    }
    pub fn visible_content_count(&self, observer: u64) -> Option<usize> {
        self.player_placement(observer)
            .map(|point| self.visible_content(point.cell).len())
    }
    pub fn broadcast(&self, source: u64, bytes: Vec<u8>) -> Result<Vec<Delivery>, WorldError> {
        let body = self.bodies.get(&source).ok_or(WorldError::InvalidValue)?;
        if !body.ready {
            return Err(WorldError::InvalidValue);
        }
        if bytes.len() > crate::script::MAX_ARGUMENT_BYTES {
            return Err(WorldError::LimitExceeded);
        }
        let data: Arc<[u8]> = bytes.into();
        Ok(self
            .neighbor_ids(body.placement.cell, 0)
            .map(|recipient| Delivery {
                recipient,
                data: data.clone(),
            })
            .collect())
    }
    fn neighbor_ids(&self, cell: u32, except: u64) -> impl Iterator<Item = u64> + '_ {
        self.bodies
            .values()
            .filter(move |body| {
                body.ready && body.id != except && self.sees_cell(body.placement.cell, cell)
            })
            .map(|body| body.id)
    }
    fn neighbors(&self, cell: u32, except: u64) -> BTreeSet<u64> {
        self.neighbor_ids(cell, except).collect()
    }
    pub fn enter(&mut self, body: Body) -> Result<Vec<Delivery>, WorldError> {
        let id = body.id;
        self.enter_hidden(body)?;
        self.activate(id)
    }
    pub fn enter_hidden(&mut self, body: Body) -> Result<(), WorldError> {
        if self.bodies.len() >= 64
            || self.bodies.contains_key(&body.id)
            || self.content.object(body.id).is_some()
        {
            return Err(WorldError::LimitExceeded);
        }
        self.prepare_cell(body.placement.cell)
            .map_err(|_| WorldError::InvalidValue)?;
        self.bodies.insert(body.id, body);
        Ok(())
    }
    pub fn activate(&mut self, id: u64) -> Result<Vec<Delivery>, WorldError> {
        let body = self.bodies.get_mut(&id).ok_or(WorldError::InvalidValue)?;
        if body.ready {
            return Ok(Vec::new());
        }
        body.ready = true;
        self.cells
            .entry(body.placement.cell)
            .or_default()
            .insert(id);
        let body = &self.bodies[&id];
        let mut deliveries = Vec::new();
        for object in self
            .visible_content(body.placement.cell)
            .into_iter()
            .filter_map(|id| self.content.object(id))
        {
            deliveries.push(Delivery {
                recipient: body.id,
                data: object.spawn.clone(),
            });
            for data in object.updates() {
                deliveries.push(Delivery {
                    recipient: body.id,
                    data: data.clone(),
                });
            }
        }
        let spawn: Arc<[u8]> = body.spawn.clone().into();
        for id in self.neighbors(body.placement.cell, body.id) {
            deliveries.push(Delivery {
                recipient: id,
                data: spawn.clone(),
            });
            body.hand_spawns(id, &mut deliveries);
            for (_, data) in body.public_stats.values() {
                deliveries.push(Delivery {
                    recipient: id,
                    data: data.clone(),
                });
            }
        }
        for id in self.visible_bodies(body.placement.cell, body.id) {
            deliveries.push(Delivery {
                recipient: body.id,
                data: self.bodies[&id].spawn.clone().into(),
            });
            self.bodies[&id].hand_spawns(body.id, &mut deliveries);
            for (_, data) in self.bodies[&id].public_stats.values() {
                deliveries.push(Delivery {
                    recipient: body.id,
                    data: data.clone(),
                });
            }
        }
        Ok(deliveries)
    }
    fn unindex(&mut self, cell: u32, id: u64) {
        if let Some(ids) = self.cells.get_mut(&cell) {
            ids.remove(&id);
            if ids.is_empty() {
                self.cells.remove(&cell);
            }
        }
    }
    pub fn remove(&mut self, id: u64) -> Vec<Delivery> {
        let Some(body) = self.bodies.remove(&id) else {
            return Vec::new();
        };
        if !body.ready {
            return Vec::new();
        }
        self.unindex(body.placement.cell, id);
        let event = body.destroy();
        let mut deliveries = Vec::new();
        for recipient in self.neighbors(body.placement.cell, id) {
            body.hand_destroys(recipient, &mut deliveries);
            deliveries.push(Delivery {
                recipient,
                data: event.clone(),
            });
        }
        deliveries
    }
    pub fn update(
        &mut self,
        id: u64,
        stamp: u16,
        update: &ClientPosition,
    ) -> Result<Vec<Delivery>, WorldError> {
        update.validate()?;
        self.prepare_cell(update.placement.cell)
            .map_err(|_| WorldError::InvalidValue)?;
        let body = self.bodies.get(&id).ok_or(WorldError::InvalidValue)?;
        if body.instance != update.stamps[0] {
            return Err(WorldError::InvalidValue);
        }
        let old_cell = body.placement.cell;
        if !body.ready {
            self.bodies.get_mut(&id).unwrap().relocate(update.placement);
            return Ok(Vec::new());
        }
        if old_cell == update.placement.cell {
            let instance = body.instance;
            self.bodies.get_mut(&id).unwrap().relocate(update.placement);
            let mut recipients = self.neighbor_ids(old_cell, id).peekable();
            if recipients.peek().is_none() {
                return Ok(Vec::new());
            }
            let data: Arc<[u8]> = movement::server_position(id, instance, stamp, update)?.into();
            return Ok(recipients
                .map(|recipient| Delivery {
                    recipient,
                    data: data.clone(),
                })
                .collect());
        }
        let previous = self.neighbors(old_cell, id);
        let current = self.neighbors(update.placement.cell, id);
        let previous_seen = self.visible_bodies(old_cell, id);
        let current_seen = self.visible_bodies(update.placement.cell, id);
        let previous_content = self.visible_content(old_cell);
        let current_content = self.visible_content(update.placement.cell);
        let destroy = body.destroy();
        let movement: Arc<[u8]> =
            movement::server_position(id, body.instance, stamp, update)?.into();
        let mut deliveries = Vec::new();
        for object in previous_content
            .difference(&current_content)
            .filter_map(|id| self.content.object(*id))
        {
            deliveries.push(Delivery {
                recipient: id,
                data: object.destroy.clone(),
            });
        }
        for object in current_content
            .difference(&previous_content)
            .filter_map(|id| self.content.object(*id))
        {
            deliveries.push(Delivery {
                recipient: id,
                data: object.spawn.clone(),
            });
            for data in object.updates() {
                deliveries.push(Delivery {
                    recipient: id,
                    data: data.clone(),
                });
            }
        }
        for other in previous.difference(&current) {
            body.hand_destroys(*other, &mut deliveries);
            deliveries.push(Delivery {
                recipient: *other,
                data: destroy.clone(),
            });
        }
        for other in previous_seen.difference(&current_seen) {
            self.bodies[other].hand_destroys(id, &mut deliveries);
            deliveries.push(Delivery {
                recipient: id,
                data: self.bodies[other].destroy(),
            });
        }
        self.bodies.get_mut(&id).unwrap().relocate(update.placement);
        if old_cell != update.placement.cell {
            self.unindex(old_cell, id);
            self.cells
                .entry(update.placement.cell)
                .or_default()
                .insert(id);
        }
        let spawn: Arc<[u8]> = self.bodies[&id].spawn.clone().into();
        for other in current.difference(&previous) {
            deliveries.push(Delivery {
                recipient: *other,
                data: spawn.clone(),
            });
            self.bodies[&id].hand_spawns(*other, &mut deliveries);
            for (_, data) in self.bodies[&id].public_stats.values() {
                deliveries.push(Delivery {
                    recipient: *other,
                    data: data.clone(),
                });
            }
        }
        for other in current_seen.difference(&previous_seen) {
            deliveries.push(Delivery {
                recipient: id,
                data: self.bodies[other].spawn.clone().into(),
            });
            self.bodies[other].hand_spawns(id, &mut deliveries);
            for (_, data) in self.bodies[other].public_stats.values() {
                deliveries.push(Delivery {
                    recipient: id,
                    data: data.clone(),
                });
            }
        }
        for recipient in current.intersection(&previous) {
            deliveries.push(Delivery {
                recipient: *recipient,
                data: movement.clone(),
            });
        }
        Ok(deliveries)
    }
    pub fn look_at(&self, id: u64, target: u64) -> Result<Vec<Delivery>, WorldError> {
        let body = self.bodies.get(&id).ok_or(WorldError::InvalidValue)?;
        if !body.ready || (target != 0 && target != u64::MAX && !self.can_observe(id, target)) {
            return Err(WorldError::InvalidValue);
        }
        let mut recipients = self.neighbor_ids(body.placement.cell, id).peekable();
        if recipients.peek().is_none() {
            return Ok(Vec::new());
        }
        // RecvEvt_LookAt 00495ac0 consumes the source InstanceIDWithStamp,
        // then the target InstanceID. DispatchEvent 00494010 uses 0x10096.
        let mut event = Vec::with_capacity(24);
        event.extend(0x10096u32.to_le_bytes());
        event.extend(id.to_le_bytes());
        event.extend(body.instance.to_le_bytes());
        event.extend(0u16.to_le_bytes());
        event.extend(target.to_le_bytes());
        let data: Arc<[u8]> = event.into();
        Ok(recipients
            .map(|recipient| Delivery {
                recipient,
                data: data.clone(),
            })
            .collect())
    }
    pub fn look(&self, id: u64, yaw: f32, pitch: f32) -> Result<Vec<Delivery>, WorldError> {
        if !yaw.is_finite() || !pitch.is_finite() {
            return Err(WorldError::InvalidValue);
        }
        let body = self.bodies.get(&id).ok_or(WorldError::InvalidValue)?;
        if !body.ready {
            return Err(WorldError::InvalidValue);
        }
        let mut recipients = self.neighbor_ids(body.placement.cell, id).peekable();
        if recipients.peek().is_none() {
            return Ok(Vec::new());
        }
        let mut event = Vec::with_capacity(24);
        // DispatchEvent 00494010 -> 0x100c9; receiver 00495b80.
        event.extend(0x100c9u32.to_le_bytes());
        event.extend(id.to_le_bytes());
        event.extend(body.instance.to_le_bytes());
        event.extend(0u16.to_le_bytes());
        event.extend(yaw.to_le_bytes());
        event.extend(pitch.to_le_bytes());
        let data: Arc<[u8]> = event.into();
        Ok(recipients
            .map(|recipient| Delivery {
                recipient,
                data: data.clone(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn body(id: u64, cell: u32) -> Body {
        let placement = Placement {
            cell,
            ..Placement::HUMAN_START
        };
        let bytes = world::create_stationary_object(&world::StationaryObject {
            character_id: id,
            parent_visual_did: 1,
            entity_did: 2,
            cell_id: cell,
            position: placement.coordinates,
            orientation: placement.orientation(),
            instance_stamp: 1,
            visual_ordering_stamp: 0,
            timestamps: [0; 4],
        })
        .unwrap();
        Body::new(id, placement, bytes).unwrap()
    }
    #[test]
    fn target_look_requires_ready_visible_objects_and_preserves_clear_sentinels() {
        let mut scene = Scene::default();
        scene.enter(body(1, 1)).unwrap();
        scene.enter(body(2, 1)).unwrap();
        scene.enter(body(3, 2)).unwrap();
        scene.enter_hidden(body(4, 1)).unwrap();
        for (source, target) in [(1, 3), (1, 4), (1, 99), (4, 1), (99, 1)] {
            assert!(scene.look_at(source, target).is_err());
        }
        for target in [2, 0, u64::MAX] {
            let events = scene.look_at(1, target).unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].recipient, 2);
            assert_eq!(&events[0].data[..4], &0x10096u32.to_le_bytes());
            assert_eq!(&events[0].data[4..12], &1u64.to_le_bytes());
            assert_eq!(&events[0].data[12..16], &[1, 0, 0, 0]);
            assert_eq!(&events[0].data[16..24], &target.to_le_bytes());
        }
        assert!(scene.look_at(3, 0).unwrap().is_empty());
    }
    #[test]
    fn visibility_movement_cross_cell_and_removal() {
        let mut scene = Scene::default();
        assert!(scene.enter(body(1, 1)).unwrap().is_empty());
        let enter = scene.enter(body(2, 1)).unwrap();
        assert_eq!(
            enter.iter().map(|d| d.recipient).collect::<Vec<_>>(),
            [1, 2]
        );
        assert!(scene.enter(body(3, 2)).unwrap().is_empty());
        let update = ClientPosition {
            time: 1.0,
            placement: Placement {
                cell: 1,
                ..Placement::HUMAN_START
            },
            motion: 0xbf7f7f7f,
            cancel_moveto: false,
            stamps: [1, 0, 0, 0],
            jump_velocity: None,
        };
        let moves = scene.update(1, 1, &update).unwrap();
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].recipient, 2);
        assert_eq!(
            &moves[0].data[..4],
            &movement::SERVER_POSITION.to_le_bytes()
        );
        let transition = scene
            .update(
                1,
                2,
                &ClientPosition {
                    placement: Placement {
                        cell: 2,
                        ..update.placement
                    },
                    time: 2.0,
                    ..update
                },
            )
            .unwrap();
        assert_eq!(transition.len(), 4);
        assert_eq!(&transition[0].data[..4], &0x100c2u32.to_le_bytes());
        let gone = scene.remove(1);
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].recipient, 3);
        assert_eq!(scene.bodies.len(), 2);
        assert!(scene.remove(1).is_empty());
        assert!(scene.update(1, 3, &update).is_err());
    }
    #[test]
    fn pending_scene_is_hidden_until_ready_and_activation_is_idempotent() {
        let mut scene = Scene::default();
        scene.enter(body(1, 1)).unwrap();
        scene.enter_hidden(body(2, 1)).unwrap();
        assert!(!scene.can_observe(1, 2));
        assert!(scene.broadcast(2, vec![1]).is_err());
        assert!(scene.look(2, 0.2, 0.3).is_err());
        let update = ClientPosition {
            time: 1.0,
            placement: Placement {
                cell: 1,
                coordinates: [1.0, 2.0, 3.0],
                ..Placement::HUMAN_START
            },
            motion: 0xbf7f7f7f,
            cancel_moveto: false,
            stamps: [1, 0, 0, 0],
            jump_velocity: None,
        };
        assert!(scene
            .update(
                2,
                1,
                &ClientPosition {
                    placement: Placement {
                        cell: 2,
                        ..update.placement
                    },
                    ..update
                }
            )
            .unwrap()
            .is_empty());
        assert!(scene.enter(body(3, 2)).unwrap().is_empty());
        assert!(!scene.can_observe(3, 2));
        assert!(scene.update(2, 1, &update).unwrap().is_empty());
        let visible = scene.activate(2).unwrap();
        assert_eq!(visible.len(), 2);
        assert_eq!(&visible[0].data[32..36], &1f32.to_le_bytes());
        assert!(scene.can_observe(1, 2));
        assert!(scene.activate(2).unwrap().is_empty());
        scene.enter_hidden(body(4, 1)).unwrap();
        assert_eq!(scene.broadcast(1, vec![1]).unwrap().len(), 2);
        assert!(scene.remove(4).is_empty());
        assert!(scene.activate(99).is_err());
    }
    #[test]
    fn unobserved_movement_is_visible_to_later_arrivals() {
        let mut scene = Scene::default();
        scene.enter(body(1, 1)).unwrap();
        let placement = Placement {
            cell: 1,
            coordinates: [10.0, 20.0, 30.0],
            heading: 64,
        };
        let update = ClientPosition {
            time: 1.0,
            placement,
            motion: (u32::from(placement.heading) << 24) | 0x7f7f7f,
            cancel_moveto: false,
            stamps: [1, 0, 0, 0],
            jump_velocity: None,
        };
        assert!(scene.update(1, 1, &update).unwrap().is_empty());
        assert!(scene.look(1, 0.2, 0.3).unwrap().is_empty());
        assert!(scene.look(99, 0.2, 0.3).is_err());
        let deliveries = scene.enter(body(2, 1)).unwrap();
        let previous = deliveries.iter().find(|d| d.recipient == 2).unwrap();
        assert_eq!(&previous.data[4..12], &1u64.to_le_bytes());
        for (index, value) in placement
            .coordinates
            .iter()
            .chain(placement.orientation().iter())
            .enumerate()
        {
            assert_eq!(
                &previous.data[32 + index * 4..36 + index * 4],
                &value.to_le_bytes()
            );
        }
    }
    #[test]
    fn bounds_duplicate_identity_and_shared_delivery() {
        let mut scene = Scene::default();
        for id in 1..=64 {
            scene.enter(body(id, 1)).unwrap();
        }
        assert!(scene.enter(body(1, 1)).is_err());
        assert!(scene.enter(body(65, 1)).is_err());
        let looks = scene.look(1, 0.2, 0.3).unwrap();
        assert_eq!(looks.len(), 63);
        assert!(Arc::ptr_eq(&looks[0].data, &looks[1].data));
        assert!(scene.look(1, f32::NAN, 0.0).is_err());
    }
}
