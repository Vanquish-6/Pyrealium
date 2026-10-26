//! Local Human/Lugian/Tumerok bootstrap, grounded in installed DAT records.
//! Appearance overrides and gameplay simulation remain separate from world loading.
use crate::{
    dat::Dat,
    world::{self, CharacterCreationRequest, SelectionCharacter, StationaryObject},
};
use std::{io, path::Path};

pub const PLAYER_ENTITY: u32 = 0x4700110f;
// CreatePlayer passes through WorldManager::SetRegion -> SetRegion (0051e7a0),
// which maps the region number through gmregionid.imp before SetRegionByGID.
pub const REGION: u32 = 1;
pub struct Bootstrap {
    qualities: Vec<u8>,
    client_version: &'static str,
    cells: Dat,
    validated_cells: std::collections::BTreeSet<u32>,
}
// Content identity for the two narrowly supported experimental layouts. This
// fingerprint is a compatibility check, not authentication of untrusted assets.
fn fingerprint(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
/// Start areas are the creation screen's choice; each race still starts at its own point.
fn supported(request: &CharacterCreationRequest) -> bool {
    request.entity_did >> 24 == 0x47
        && matches!(request.start_area, 0 | 0x40000001..=0x40000010)
        && request.class <= 0xff
        && crate::appearance::body(request.race, request.sex)
            .is_some_and(|body| body.visual_desc(&request.physical_values).is_ok())
}
impl Bootstrap {
    /// Native 7148 cannot consume the original installed 7230 DATs'
    /// GameplayOptionsProfile's 0x2000 field in WState 7100110F: it mistakes
    /// trailing data for HeapObject.m_size and then dereferences a failed load.
    /// A pinned converted state is eligible for 7148. Both profiles remain
    /// experimental; eligibility is not a claim of native world acceptance.
    pub fn supports_client(&self, version: &str) -> bool {
        version == self.client_version
    }
    pub fn load(directory: &Path) -> io::Result<Self> {
        let mut portal = Dat::open(directory.join("portal.dat"))?;
        let player_state = portal.read(0x7100110f)?;
        let package_state = portal.read(0x7000037f)?;
        let client_version = match (fingerprint(&player_state), fingerprint(&package_state)) {
            (0x0fb49f0fddeda7f2, 0x3594e7ba65dcd613) => "7230",
            (0xc0b0230d626998c2, 0x83c30e31275ab0e3) => "7148",
            _ => return Err(invalid("unrecognized or mixed player script-state layouts")),
        };
        for id in [PLAYER_ENTITY, 0x0f000000] {
            portal.read(id)?;
        }
        for body in crate::appearance::bodies() {
            for id in body.records() {
                portal.read(id)?;
            }
        }
        let qualities = portal.read(0x8100110f)?;
        // Narrow known layout: never apply offsets to an unrecognized record.
        for (offset, expected) in [
            (0, 0x2f_u32),
            (4, 0x8100110f),
            (8, 0x00ffa002),
            (52, 5),
            (96, 10),
            (180, 1),
            (192, 1),
            (196, 1),
            (200, 0x470000cf),
        ] {
            if qualities.len() != 204 || qualities[offset..offset + 4] != expected.to_le_bytes() {
                return Err(invalid(
                    "unsupported player qualities layout in supplied DAT",
                ));
            }
        }
        let mut cells = Dat::open(directory.join("cell_1.dat"))?;
        for body in crate::appearance::bodies() {
            cells.read(crate::dat::world_cell_record(body.start.cell)?)?;
        }
        Ok(Self {
            qualities,
            client_version,
            cells,
            validated_cells: std::collections::BTreeSet::new(),
        })
    }
    /// Human, Lugian and Tumerok of either sex with valid appearance choices.
    /// The requested entity is ignored; every avatar uses PLAYER_ENTITY.
    pub fn supports(&self, request: &CharacterCreationRequest) -> bool {
        supported(request)
    }
    fn body(request: &CharacterCreationRequest) -> &'static crate::appearance::Body {
        crate::appearance::body(request.race, request.sex)
            .expect("caller checked supports()")
    }
    /// The race's start point for a new character.
    pub fn start(&self, request: &CharacterCreationRequest) -> crate::movement::Placement {
        Self::body(request).start
    }
    pub fn selection(&self, id: u64, request: &CharacterCreationRequest) -> SelectionCharacter {
        let body = Self::body(request);
        SelectionCharacter {
            id,
            name: request.name.clone(),
            seconds_greyed_out: 0,
            parent_visual_did: body.visual,
            appearance: body.visual_desc(&request.physical_values).ok(),
        }
    }
    fn weenie_description(&self, name: &str) -> Result<Vec<u8>, world::WorldError> {
        let mut data = (0x00ffa002u32 | 0x02000004).to_le_bytes().to_vec();
        data.extend(PLAYER_ENTITY.to_le_bytes());
        data.extend_from_slice(&self.qualities[12..16]);
        data.extend(world::literal_name(name)?);
        data.extend_from_slice(&self.qualities[16..52]);
        Ok(data)
    }
    pub fn events(
        &self,
        id: u64,
        request: &CharacterCreationRequest,
    ) -> Result<[Vec<u8>; 3], world::WorldError> {
        self.events_at(id, request, self.start(request), 1)
    }
    pub fn contains_cell(&mut self, cell: u32) -> bool {
        if self.validated_cells.contains(&cell) {
            return true;
        }
        let Ok(record) = crate::dat::world_cell_record(cell) else {
            return false;
        };
        if self.cells.read(record).is_err() {
            return false;
        }
        if self.validated_cells.len() >= 4096 {
            self.validated_cells.clear();
        }
        self.validated_cells.insert(cell);
        true
    }
    pub fn events_at(
        &self,
        id: u64,
        request: &CharacterCreationRequest,
        placement: crate::movement::Placement,
        instance_stamp: u16,
    ) -> Result<[Vec<u8>; 3], world::WorldError> {
        self.events_at_state(
            id,
            request,
            placement,
            instance_stamp,
            &crate::progression::Progression::default(),
        )
    }
    pub fn events_at_state(
        &self,
        id: u64,
        request: &CharacterCreationRequest,
        placement: crate::movement::Placement,
        instance_stamp: u16,
        progression: &crate::progression::Progression,
    ) -> Result<[Vec<u8>; 3], world::WorldError> {
        if !self.supports(request) {
            return Err(world::WorldError::InvalidValue);
        }
        let selection = self.selection(id, request);
        let body = Self::body(request);
        let mut object = world::create_stationary_object(&StationaryObject {
            character_id: id,
            parent_visual_did: selection.parent_visual_did,
            entity_did: PLAYER_ENTITY,
            cell_id: placement.cell,
            position: placement.coordinates,
            orientation: placement.orientation(),
            instance_stamp,
            visual_ordering_stamp: 0,
            timestamps: [0; 4],
        })?;
        // Replace the parent-only VisualDesc (flags 2 + DID, bytes 12..20) with
        // the full appearance, as the retail CreateObject carried it.
        if let Some(visual) = &selection.appearance {
            object.splice(12..20, visual.iter().copied());
        }
        object.truncate(object.len() - 8);
        let description = self.weenie_description(&request.name)?;
        object.extend_from_slice(&description);
        let mut player = world::PLAYER_DESC.to_le_bytes().to_vec();
        let header = u32::from_le_bytes(self.qualities[..4].try_into().unwrap()) | 0x800;
        player.extend(header.to_le_bytes()); // Native long-int quality table present.
        player.extend_from_slice(&self.qualities[4..8]);
        player.extend(description);
        // CBaseQualities::UnPack 005192f0: int table follows the description.
        // Preserve authored base entries, then initialize the local level-one
        // test profile. Stat IDs are recovered gmintstats.emp (2300002D).
        let mut ints = std::collections::BTreeMap::new();
        for entry in self.qualities[56..96].chunks_exact(8) {
            ints.insert(
                u32::from_le_bytes(entry[..4].try_into().unwrap()),
                u32::from_le_bytes(entry[4..].try_into().unwrap()),
            );
        }
        for (key, value) in progression.int_stats() {
            ints.insert(key, value);
        }
        // Species, Sex and Class (gmintstats 0x122..0x124), as in the retail
        // new-character PlayerDesc.
        ints.insert(0x122, request.race);
        ints.insert(0x123, request.sex);
        ints.insert(0x124, request.class);
        player.extend((ints.len() as u32).to_le_bytes());
        // PHashTable::StreamPack 00517fd0 encodes the bucket-size index in
        // the high byte (zero here => 11 buckets, client table 00a7b140).
        // add_for_unpacking 004fc9f0 links only consecutive entries in a
        // bucket; revisiting a bucket overwrites its head and corrupts the
        // element count. Numeric key order is not a valid wire order.
        let mut ints: Vec<_> = ints.into_iter().collect();
        ints.sort_unstable_by_key(|(key, _)| (key % 11, *key));
        for (key, value) in ints {
            player.extend(key.to_le_bytes());
            player.extend(value.to_le_bytes());
        }
        let mut booleans = std::collections::BTreeMap::new();
        for entry in self.qualities[100..180].chunks_exact(8) {
            booleans.insert(
                u32::from_le_bytes(entry[..4].try_into().unwrap()),
                u32::from_le_bytes(entry[4..].try_into().unwrap()),
            );
        }
        booleans.insert(0x117, u32::from(!progression.alive()));
        let mut booleans = booleans.into_iter().collect::<Vec<_>>();
        booleans.sort_unstable_by_key(|(key, _)| (key % 11, *key));
        player.extend((booleans.len() as u32).to_le_bytes());
        for (key, value) in booleans {
            player.extend(key.to_le_bytes());
            player.extend(value.to_le_bytes());
        }
        player.extend_from_slice(&self.qualities[180..200]);
        player.extend(body.entity.to_le_bytes());
        player.extend(2u32.to_le_bytes());
        for (key, value) in [
            (0x12cu32, progression.xp),
            (0x12d, progression.available_xp),
        ] {
            player.extend(key.to_le_bytes());
            player.extend(value.to_le_bytes());
        }
        Ok([world::create_player(id, REGION), object, player])
    }
}
