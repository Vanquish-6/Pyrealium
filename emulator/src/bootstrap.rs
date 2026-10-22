//! Local human-body bootstrap, grounded in installed DAT records.
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
        for id in [
            PLAYER_ENTITY,
            0x0f000000,
            0x470000cd,
            0x470000ce,
            0x1f000023,
            0x1f000024,
        ] {
            portal.read(id)?;
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
        cells.read(0x02990109)?;
        Ok(Self {
            qualities,
            client_version,
        })
    }
    pub fn supports(&self, request: &CharacterCreationRequest) -> bool {
        request.entity_did == PLAYER_ENTITY
            && request.race == 1
            && matches!(request.sex, 0x1000 | 0x2000)
            && matches!(request.start_area, 0 | 0x40000001)
    }
    pub fn selection(&self, id: u64, request: &CharacterCreationRequest) -> SelectionCharacter {
        SelectionCharacter {
            id,
            name: request.name.clone(),
            seconds_greyed_out: 0,
            parent_visual_did: if request.sex == 0x2000 {
                0x1f000024
            } else {
                0x1f000023
            },
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
        if !self.supports(request) {
            return Err(world::WorldError::InvalidValue);
        }
        let selection = self.selection(id, request);
        // Authored human_start_position1, present in installed cell_1.dat.
        let mut object = world::create_stationary_object(&StationaryObject {
            character_id: id,
            parent_visual_did: selection.parent_visual_did,
            entity_did: PLAYER_ENTITY,
            cell_id: 0x02990109,
            position: [59.060577, 240.199, -44.606205],
            orientation: [
                std::f32::consts::FRAC_1_SQRT_2,
                0.0,
                0.0,
                std::f32::consts::FRAC_1_SQRT_2,
            ],
            instance_stamp: 1,
            visual_ordering_stamp: 0,
            timestamps: [0; 4],
        })?;
        object.truncate(object.len() - 8);
        let description = self.weenie_description(&request.name)?;
        object.extend_from_slice(&description);
        let mut player = world::PLAYER_DESC.to_le_bytes().to_vec();
        player.extend_from_slice(&self.qualities[..8]);
        player.extend(description);
        player.extend_from_slice(&self.qualities[52..200]);
        player.extend(
            (if request.sex == 0x2000 {
                0x470000ceu32
            } else {
                0x470000cdu32
            })
            .to_le_bytes(),
        );
        Ok([world::create_player(id, REGION), object, player])
    }
}
