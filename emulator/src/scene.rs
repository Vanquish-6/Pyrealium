//! Single-owner player presence. Cell-indexed recipients and shared immutable payloads.
//! Same-cell interest is an explicit initial policy, not recovered visibility geometry.
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
    ready: bool,
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
        let instance = u16::from_le_bytes(spawn[24..26].try_into().unwrap());
        if instance == 0 {
            return Err(WorldError::InvalidValue);
        }
        Ok(Self {
            id,
            placement,
            instance,
            spawn,
            ready: false,
        })
    }
    fn relocate(&mut self, placement: Placement) {
        self.placement = placement;
        self.spawn[28..32].copy_from_slice(&placement.cell.to_le_bytes());
        for (i, n) in placement
            .coordinates
            .iter()
            .chain(placement.orientation().iter())
            .enumerate()
        {
            self.spawn[32 + i * 4..36 + i * 4].copy_from_slice(&n.to_le_bytes());
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
    bodies: BTreeMap<u64, Body>,
    cells: BTreeMap<u32, BTreeSet<u64>>,
}
impl Scene {
    pub fn can_observe(&self, observer: u64, target: u64) -> bool {
        self.bodies
            .get(&observer)
            .zip(self.bodies.get(&target))
            .is_some_and(|(a, b)| a.ready && b.ready && a.placement.cell == b.placement.cell)
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
        Ok(self.cells[&body.placement.cell]
            .iter()
            .map(|&recipient| Delivery {
                recipient,
                data: data.clone(),
            })
            .collect())
    }
    fn neighbor_ids(&self, cell: u32, except: u64) -> impl Iterator<Item = u64> + '_ {
        self.cells
            .get(&cell)
            .into_iter()
            .flatten()
            .copied()
            .filter(move |id| *id != except)
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
        if self.bodies.len() >= 64 || self.bodies.contains_key(&body.id) {
            return Err(WorldError::LimitExceeded);
        }
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
        let spawn: Arc<[u8]> = body.spawn.clone().into();
        for id in self.neighbors(body.placement.cell, body.id) {
            deliveries.push(Delivery {
                recipient: id,
                data: spawn.clone(),
            });
            deliveries.push(Delivery {
                recipient: body.id,
                data: self.bodies[&id].spawn.clone().into(),
            });
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
        self.neighbors(body.placement.cell, id)
            .into_iter()
            .map(|recipient| Delivery {
                recipient,
                data: event.clone(),
            })
            .collect()
    }
    pub fn update(
        &mut self,
        id: u64,
        stamp: u16,
        update: &ClientPosition,
    ) -> Result<Vec<Delivery>, WorldError> {
        update.validate()?;
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
        let destroy = body.destroy();
        let movement: Arc<[u8]> =
            movement::server_position(id, body.instance, stamp, update)?.into();
        let mut deliveries = Vec::new();
        for other in previous.difference(&current) {
            deliveries.push(Delivery {
                recipient: *other,
                data: destroy.clone(),
            });
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
            deliveries.push(Delivery {
                recipient: id,
                data: self.bodies[other].spawn.clone().into(),
            });
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
        assert!(
            scene
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
                .is_empty()
        );
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
