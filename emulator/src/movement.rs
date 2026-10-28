//! Build-7148 movement: CPositionPack::StreamPack 0069c060,
//! PositionPack::StreamPack 0069be20, PositionOffset::StreamPack 00625930.
use crate::world::WorldError;

pub const CLIENT_POSITION: u32 = 0x1008e;
pub const CLIENT_LOOK: u32 = 0x100c8;
pub const SERVER_POSITION: u32 = 0x10086;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub cell: u32,
    pub coordinates: [f32; 3],
    pub heading: u8,
}
impl Placement {
    pub const HUMAN_START: Self = Self {
        cell: 0x02990109,
        coordinates: [59.060577, 240.199, -44.606205],
        heading: 191,
    };
    pub fn valid(&self) -> bool {
        self.cell != 0
            && self
                .coordinates
                .iter()
                .all(|n| n.is_finite() && n.abs() < 1_000_000.0)
    }
    pub fn orientation(&self) -> [f32; 4] {
        // Heading::ToUInt8 uses 255 units per revolution (0061a940).
        let half = f32::from(self.heading) * std::f32::consts::PI / 255.0;
        // Native heading is clockwise from +Y; quaternion rotation is opposite.
        // Frame::set_heading 0061a810 and Frame::cache_matrix 00619f70.
        [half.cos(), 0.0, 0.0, -half.sin()]
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClientPosition {
    pub time: f64,
    pub placement: Placement,
    pub motion: u32,
    pub cancel_moveto: bool,
    /// instance, teleport, forced-position, move-to; four distinct native stamps.
    pub stamps: [u16; 4],
    pub jump_velocity: Option<[f32; 3]>,
}
impl ClientPosition {
    pub fn validate(&self) -> Result<(), WorldError> {
        if !self.time.is_finite()
            || self.time < 0.0
            || !self.placement.valid()
            || self.placement.heading != (self.motion >> 24) as u8
            || self
                .jump_velocity
                .is_some_and(|v| v.iter().any(|n| !n.is_finite() || n.abs() > 10000.0))
        {
            return Err(WorldError::InvalidValue);
        }
        Ok(())
    }
    /// Payload after the opcode; fixed-size reads and no untrusted allocations.
    pub fn decode(bytes: &[u8]) -> Result<Self, WorldError> {
        if bytes.len() < 40 {
            return Err(WorldError::Truncated);
        }
        let word = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let flags = word(28);
        if flags & !6 != 0 {
            return Err(WorldError::InvalidValue);
        }
        let jump = flags & 2 != 0;
        let length = if jump { 52 } else { 40 };
        if bytes.len() < length {
            return Err(WorldError::Truncated);
        }
        if bytes.len() > length {
            return Err(WorldError::TrailingBytes);
        }
        let motion = word(24);
        let result = Self {
            time: f64::from_le_bytes(bytes[..8].try_into().unwrap()),
            placement: Placement {
                cell: word(8),
                coordinates: std::array::from_fn(|i| f32::from_bits(word(12 + i * 4))),
                heading: (motion >> 24) as u8,
            },
            motion,
            cancel_moveto: flags & 4 != 0,
            stamps: std::array::from_fn(|i| {
                u16::from_le_bytes(bytes[32 + i * 2..34 + i * 2].try_into().unwrap())
            }),
            jump_velocity: jump.then(|| std::array::from_fn(|i| f32::from_bits(word(40 + i * 4)))),
        };
        result.validate()?;
        Ok(result)
    }
    pub fn encode(&self) -> Result<Vec<u8>, WorldError> {
        self.validate()?;
        let mut bytes = Vec::with_capacity(56);
        bytes.extend(CLIENT_POSITION.to_le_bytes());
        bytes.extend(self.time.to_le_bytes());
        pack_placement(&mut bytes, self.placement);
        bytes.extend(self.motion.to_le_bytes());
        bytes.extend(
            ((u32::from(self.cancel_moveto) * 4) | (u32::from(self.jump_velocity.is_some()) * 2))
                .to_le_bytes(),
        );
        for stamp in self.stamps {
            bytes.extend(stamp.to_le_bytes());
        }
        if let Some(v) = self.jump_velocity {
            for value in v {
                bytes.extend(value.to_le_bytes());
            }
        }
        Ok(bytes)
    }
}
/// LandscapeDefs `36000000`: outdoor cells are 20 m, eight along each side of a
/// 160 m landblock. Local coordinates are landblock-relative, so a creature can
/// cross those cell edges without a new origin.
pub const OUTDOOR_CELL_SIDE: f32 = 20.0;
pub const OUTDOOR_BLOCK_SIDE: f32 = 160.0;
const OUTDOOR_GRID: i32 = 8;

pub fn outdoor_cell(cell: u32) -> bool {
    (1..=64).contains(&(cell & 0xffff))
}

fn block_local(coordinates: [f32; 3]) -> bool {
    (0.0..OUTDOOR_BLOCK_SIDE).contains(&coordinates[0])
        && (0.0..OUTDOOR_BLOCK_SIDE).contains(&coordinates[1])
        && coordinates[2].is_finite()
}

/// Landblock-local outdoor position in a continuous frame. Interior cells and
/// coordinates outside the block stay local to their cell.
pub fn world_point(placement: Placement) -> Option<[f32; 3]> {
    if !outdoor_cell(placement.cell) || !block_local(placement.coordinates) {
        return None;
    }
    let block_x = (placement.cell >> 24) as f32;
    let block_y = ((placement.cell >> 16) & 255) as f32;
    Some([
        block_x * OUTDOOR_BLOCK_SIDE + placement.coordinates[0],
        block_y * OUTDOOR_BLOCK_SIDE + placement.coordinates[1],
        placement.coordinates[2],
    ])
}

/// Inverse of `world_point`. The cell index follows `placement_from_point`.
pub fn placement_at(world: [f32; 3], heading: u8) -> Option<Placement> {
    if world.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let block_x = (world[0] / OUTDOOR_BLOCK_SIDE).floor();
    let block_y = (world[1] / OUTDOOR_BLOCK_SIDE).floor();
    if !(0.0..255.0).contains(&block_x) || !(0.0..255.0).contains(&block_y) {
        return None;
    }
    let x = world[0] - block_x * OUTDOOR_BLOCK_SIDE;
    let y = world[1] - block_y * OUTDOOR_BLOCK_SIDE;
    if !(0.0..OUTDOOR_BLOCK_SIDE).contains(&x) || !(0.0..OUTDOOR_BLOCK_SIDE).contains(&y) {
        return None;
    }
    let column = ((x / OUTDOOR_CELL_SIDE) as i32).clamp(0, OUTDOOR_GRID - 1);
    let row = ((y / OUTDOOR_CELL_SIDE) as i32).clamp(0, OUTDOOR_GRID - 1);
    let low = 1 + column * OUTDOOR_GRID + row;
    let placement = Placement {
        cell: (block_x as u32) << 24 | (block_y as u32) << 16 | low as u32,
        coordinates: [x, y, world[2]],
        heading,
    };
    placement.valid().then_some(placement)
}

/// Same-cell distance, or outdoor distance across the 20 m cell grid.
pub fn separation(a: Placement, b: Placement) -> f32 {
    match (world_point(a), world_point(b)) {
        (Some(a), Some(b)) => a
            .iter()
            .zip(b)
            .map(|(a, b)| (a - b) * (a - b))
            .sum::<f32>()
            .sqrt(),
        _ if a.cell == b.cell => a
            .coordinates
            .iter()
            .zip(b.coordinates)
            .map(|(a, b)| (a - b) * (a - b))
            .sum::<f32>()
            .sqrt(),
        _ => f32::MAX,
    }
}

fn pack_placement(bytes: &mut Vec<u8>, placement: Placement) {
    bytes.extend(placement.cell.to_le_bytes());
    for value in placement.coordinates {
        bytes.extend(value.to_le_bytes());
    }
}

/// Server PositionPack has three stamps, unlike the client pack's four stamps.
pub fn server_position(
    id: u64,
    instance: u16,
    position_stamp: u16,
    update: &ClientPosition,
) -> Result<Vec<u8>, WorldError> {
    if id == 0 || instance == 0 {
        return Err(WorldError::InvalidValue);
    }
    update.validate()?;
    let mut bytes = Vec::with_capacity(68);
    bytes.extend(SERVER_POSITION.to_le_bytes());
    bytes.extend(id.to_le_bytes());
    bytes.extend(instance.to_le_bytes());
    bytes.extend(0u16.to_le_bytes());
    bytes.extend(update.time.to_le_bytes());
    pack_placement(&mut bytes, update.placement);
    bytes.extend(update.motion.to_le_bytes());
    // The native server pack uses bit 2 for contact, not cancel-move-to.
    bytes.extend(
        (if update.jump_velocity.is_some() {
            2u32
        } else {
            4u32
        })
        .to_le_bytes(),
    );
    for stamp in [position_stamp, update.stamps[2], update.stamps[1]] {
        bytes.extend(stamp.to_le_bytes());
    }
    if let Some(v) = update.jump_velocity {
        for value in v {
            bytes.extend(value.to_le_bytes());
        }
    }
    while bytes.len() % 4 != 0 {
        bytes.push(0);
    }
    Ok(bytes)
}

#[derive(Debug, Clone)]
pub struct Motion {
    pub placement: Placement,
    pub instance: u16,
    pub position_stamp: u16,
    pub last: Option<ClientPosition>,
    force_stamp: u16,
}
impl Motion {
    pub fn new(placement: Placement, instance: u16) -> Self {
        Self {
            placement,
            instance,
            position_stamp: 0,
            last: None,
            force_stamp: 0,
        }
    }
    pub fn accept(&mut self, update: ClientPosition) -> Result<(), WorldError> {
        if update.stamps[..3] != [self.instance, 0, self.force_stamp]
            || self.last.is_some_and(|last| update.time <= last.time)
        {
            return Err(WorldError::InvalidValue);
        }
        // Decode validation also applies to synthetic/direct callers.
        update.validate()?;
        self.position_stamp = self.position_stamp.wrapping_add(1);
        self.placement = update.placement;
        self.last = Some(update);
        Ok(())
    }
    /// Development instant relocation using the recovered forced-position
    /// stamp. Prepare on a candidate before making durable world-state changes.
    pub fn forced(&self, destination: Placement) -> Result<(Self, ClientPosition), WorldError> {
        if !destination.valid() {
            return Err(WorldError::InvalidValue);
        }
        let mut next = self.clone();
        next.force_stamp = self
            .force_stamp
            .checked_add(1)
            .ok_or(WorldError::LimitExceeded)?;
        next.position_stamp = self.position_stamp.wrapping_add(1);
        next.placement = destination;
        let update = ClientPosition {
            time: self.last.map_or(0.0, |last| last.time),
            placement: destination,
            motion: (u32::from(destination.heading) << 24) | 0x007f7f7f,
            cancel_moveto: true,
            stamps: [self.instance, 0, next.force_stamp, 0],
            jump_velocity: None,
        };
        update.validate()?;
        Ok((next, update))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outdoor_grid_crosses_cell_and_block_edges() {
        let origin = Placement {
            cell: 0x0100_0001,
            coordinates: [1.0, 1.0, 5.0],
            heading: 0,
        };
        let world = world_point(origin).unwrap();
        assert_eq!(world, [161.0, 1.0, 5.0]);
        assert_eq!(placement_at(world, 0).unwrap().cell, 0x0100_0001);
        assert_eq!(
            placement_at([181.0, 1.0, 5.0], 0).unwrap().cell,
            0x0100_0009
        );
        assert_eq!(
            placement_at([320.0, 1.0, 5.0], 0).unwrap().cell,
            0x0200_0001
        );
    }
    #[test]
    fn heading_matches_original_x86_rotation_axes() {
        // tools/movement_binary_vectors.py executes Frame::cache_matrix 00619f70.
        for (heading, x, y) in [
            (0, 0.0, 1.0),
            (63, 0.9998293, 0.018478882),
            (64, 0.99998105, -0.006159997),
            (127, 0.01231966, -0.9999242),
            (128, -0.01231966, -0.9999242),
            (191, -0.99998105, -0.006159997),
            (192, -0.9998293, 0.018478882),
            (255, 0.0, 1.0),
        ] {
            let p = Placement {
                heading,
                ..Placement::HUMAN_START
            };
            let [w, _, _, z] = p.orientation();
            assert!((-2.0 * w * z - x).abs() < 0.000001);
            assert!((1.0 - 2.0 * z * z - y).abs() < 0.000001);
        }
    }
    pub fn sample() -> ClientPosition {
        ClientPosition {
            time: 12.5,
            placement: Placement::HUMAN_START,
            motion: 0xbf7f7f7f,
            cancel_moveto: true,
            stamps: [1, 0, 0, 9],
            jump_velocity: None,
        }
    }
    #[test]
    fn exact_layout_and_all_truncations() {
        for velocity in [None, Some([1.0, -2.0, 3.5])] {
            let update = ClientPosition {
                jump_velocity: velocity,
                ..sample()
            };
            let bytes = update.encode().unwrap();
            assert_eq!(bytes.len(), if velocity.is_some() { 56 } else { 44 });
            assert_eq!(&bytes[36..44], &[1, 0, 0, 0, 0, 0, 9, 0]);
            assert_eq!(ClientPosition::decode(&bytes[4..]).unwrap(), update);
            for end in 4..bytes.len() {
                assert!(ClientPosition::decode(&bytes[4..end]).is_err());
            }
            let server = server_position(0x0102030405060708, 1, 2, &update).unwrap();
            assert_eq!(server.len(), if velocity.is_some() { 68 } else { 56 });
            assert_eq!(&server[48..54], &[2, 0, 0, 0, 0, 0]);
        }
    }
    #[test]
    fn rejects_stale_stamps_nonfinite_and_trailing_transactionally() {
        let mut state = Motion::new(Placement::HUMAN_START, 1);
        state.accept(sample()).unwrap();
        assert!(state.accept(sample()).is_err());
        assert_eq!(state.position_stamp, 1);
        for update in [
            ClientPosition {
                time: f64::NAN,
                ..sample()
            },
            ClientPosition {
                stamps: [2, 0, 0, 0],
                time: 14.0,
                ..sample()
            },
            ClientPosition {
                time: 11.0,
                ..sample()
            },
        ] {
            assert!(state.accept(update).is_err());
            assert_eq!(state.position_stamp, 1);
        }
        let mut bytes = sample().encode().unwrap();
        bytes.push(0);
        assert_eq!(
            ClientPosition::decode(&bytes[4..]),
            Err(WorldError::TrailingBytes)
        );
        bytes.truncate(44);
        bytes[32..36].copy_from_slice(&8u32.to_le_bytes());
        assert_eq!(
            ClientPosition::decode(&bytes[4..]),
            Err(WorldError::InvalidValue)
        );
    }
}
