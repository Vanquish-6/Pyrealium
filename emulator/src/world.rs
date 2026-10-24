//! Build 7148 application events. These are blob bodies, not UDP packets.
//! See docs/world-evidence.md for recovered evidence and the world-entry boundary.

pub const CHAR_EXIT_GAME: u32 = 0x10027;
pub const CHARACTER_DELETION: u32 = 0x10029;
pub const CREATE_CHARACTER: u32 = 0x1002a;
pub const ENTER_WORLD: u32 = 0x1002b;
pub const CHARACTER_SET: u32 = 0x10055;
pub const CHARACTER_ERROR: u32 = 0x10056;
pub const CHAR_GEN_VERIFICATION: u32 = 0x10057;
pub const CREATE_OBJECT: u32 = 0x10058;
pub const CREATE_PLAYER: u32 = 0x10060;
pub const PLAYER_DESC: u32 = 0x10068;
pub const MIN_CHARACTER_SET: u32 = 0x100b8;
pub const SCENE_RENDERING_COMPLETE: u32 = 0x100d1;
pub const CLIENT_SCRIPT_EVENT: u32 = 0x10053;
pub const MAX_STRING_BYTES: usize = 4096;
pub const MAX_CHARACTERS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldError {
    Truncated,
    TrailingBytes,
    LimitExceeded,
    InvalidValue,
    UnsupportedOpcode(u32),
}

impl std::fmt::Display for WorldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for WorldError {}

#[derive(Debug, Clone, PartialEq)]
pub enum ClientEvent {
    ScriptCall {
        function_id: u32,
        arguments: Vec<u8>,
    },
    DeleteCharacter {
        account: Vec<u8>,
        character_id: u64,
    },
    Position(crate::movement::ClientPosition),
    Look {
        yaw: f32,
        pitch: f32,
    },
    LookAt {
        target: u64,
    },
    CreateCharacter {
        account: Vec<u8>,
        request: CharacterCreationRequest,
    },
    EnterWorld {
        character_id: u64,
        account: Vec<u8>,
    },
    LogOffCharacter {
        character_id: u64,
    },
    SceneRenderingComplete,
}

/// Values are retained as received, not trusted as authority to create characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterCreationRequest {
    pub name: String,
    pub entity_did: u32,
    pub start_area: u32,
    pub race: u32,
    pub sex: u32,
    pub class: u32,
    pub implement: u32,
    /// (physical-type key, IEEE754 float bits), preserving exact client values.
    pub physical_values: Vec<(u32, u32)>,
}

const MUNGE_KEYS: [u32; 16] = [
    0xe5849238, 0xeb2d767d, 0xfc0fdd03, 0xcb45a6a6, 0x7f21e6bb, 0xf5a6cdc8, 0x22d9ad12, 0xb2a3d4de,
    0x3b834db7, 0xc406c91c, 0x8392305e, 0x0a9f0099, 0x0943b90c, 0xa66d953c, 0x0d929e1f, 0x0e2e9ad4,
];

fn xor_words(bytes: &mut [u8]) {
    // The executable transforms complete words only; a partial tail is not XORed.
    for (index, word) in bytes.chunks_exact_mut(4).enumerate() {
        let key = MUNGE_KEYS[index & 15].to_le_bytes();
        for (value, key) in word.iter_mut().zip(key) {
            *value ^= key;
        }
    }
}

fn pack_string(out: &mut Vec<u8>, bytes: &[u8], units: usize) -> Result<(), WorldError> {
    if bytes.len() > MAX_STRING_BYTES {
        return Err(WorldError::LimitExceeded);
    }
    out.extend_from_slice(&(units as u16).to_le_bytes());
    let mut encoded: Vec<u8> = bytes.iter().map(|b| b.rotate_left(4)).collect();
    xor_words(&mut encoded);
    out.extend(encoded);
    while out.len() % 4 != 0 {
        out.push(0);
    }
    Ok(())
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], WorldError> {
        let end = self
            .pos
            .checked_add(count)
            .ok_or(WorldError::LimitExceeded)?;
        let bytes = self.bytes.get(self.pos..end).ok_or(WorldError::Truncated)?;
        self.pos = end;
        Ok(bytes)
    }
    fn u32(&mut self) -> Result<u32, WorldError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, WorldError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<Vec<u8>, WorldError> {
        self.string_units(1)
    }
    fn string_units(&mut self, width: usize) -> Result<Vec<u8>, WorldError> {
        let count = u16::from_le_bytes(self.take(2)?.try_into().unwrap());
        let count = if count == 0xffff {
            self.u32()? as usize
        } else {
            count as usize
        };
        let count = count.checked_mul(width).ok_or(WorldError::LimitExceeded)?;
        if count > MAX_STRING_BYTES {
            return Err(WorldError::LimitExceeded);
        }
        let mut bytes = self.take(count)?.to_vec();
        xor_words(&mut bytes);
        bytes.iter_mut().for_each(|b| *b = b.rotate_left(4));
        self.take((4 - self.pos % 4) % 4)?;
        Ok(bytes)
    }
    fn wide_string(&mut self) -> Result<String, WorldError> {
        let bytes = self.string_units(2)?;
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16(&units).map_err(|_| WorldError::InvalidValue)
    }
}

pub(crate) fn read_ui_text(bytes: &[u8]) -> Result<(String, usize), WorldError> {
    let mut reader = Reader { bytes, pos: 0 };
    let text = reader.wide_string()?;
    Ok((text, reader.pos))
}
pub(crate) fn ui_text(text: &str) -> Result<Vec<u8>, WorldError> {
    let mut bytes = Vec::new();
    let units: Vec<u16> = text.encode_utf16().collect();
    let data: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
    pack_string(&mut bytes, &data, units.len())?;
    Ok(bytes)
}
pub fn decode_client_event(bytes: &[u8]) -> Result<ClientEvent, WorldError> {
    let mut reader = Reader { bytes, pos: 0 };
    let event = match reader.u32()? {
        CLIENT_SCRIPT_EVENT => {
            let function_id = reader.u32()?;
            let length = reader.u32()? as usize;
            if length > crate::script::MAX_ARGUMENT_BYTES {
                return Err(WorldError::LimitExceeded);
            }
            ClientEvent::ScriptCall {
                function_id,
                arguments: reader.take(length)?.to_vec(),
            }
        }
        CHARACTER_DELETION => ClientEvent::DeleteCharacter {
            account: reader.string()?,
            character_id: reader.u64()?,
        },
        crate::movement::CLIENT_POSITION => {
            let update = crate::movement::ClientPosition::decode(reader.take(bytes.len() - 4)?)?;
            ClientEvent::Position(update)
        }
        crate::movement::CLIENT_LOOK => {
            let yaw = f32::from_bits(reader.u32()?);
            let pitch = f32::from_bits(reader.u32()?);
            if !yaw.is_finite() || !pitch.is_finite() {
                return Err(WorldError::InvalidValue);
            }
            ClientEvent::Look { yaw, pitch }
        }
        // ECM_Physics::SendEvt_CLookAt 00496980: opcode and one InstanceID.
        0x100c7 => ClientEvent::LookAt {
            target: reader.u64()?,
        },
        CREATE_CHARACTER => {
            let account = reader.string()?;
            let mut request = CharacterCreationRequest {
                name: reader.wide_string()?,
                entity_did: reader.u32()?,
                start_area: reader.u32()?,
                race: reader.u32()?,
                sex: reader.u32()?,
                class: reader.u32()?,
                implement: reader.u32()?,
                physical_values: Vec::new(),
            };
            let header = reader.u32()?;
            let count = (header & 0x00ffffff) as usize;
            if count > 256 {
                return Err(WorldError::LimitExceeded);
            }
            for _ in 0..count {
                let key = reader.u32()?;
                let value = reader.u32()?;
                if !f32::from_bits(value).is_finite()
                    || request.physical_values.iter().any(|p| p.0 == key)
                {
                    return Err(WorldError::InvalidValue);
                }
                request.physical_values.push((key, value));
            }
            ClientEvent::CreateCharacter { account, request }
        }
        ENTER_WORLD => ClientEvent::EnterWorld {
            character_id: reader.u64()?,
            account: reader.string()?,
        },
        CHAR_EXIT_GAME => ClientEvent::LogOffCharacter {
            character_id: reader.u64()?,
        },
        SCENE_RENDERING_COMPLETE => ClientEvent::SceneRenderingComplete,
        opcode => return Err(WorldError::UnsupportedOpcode(opcode)),
    };
    if reader.pos != bytes.len() {
        return Err(WorldError::TrailingBytes);
    }
    Ok(event)
}

pub fn encode_enter_world(character_id: u64, account: &[u8]) -> Result<Vec<u8>, WorldError> {
    let mut bytes = ENTER_WORLD.to_le_bytes().to_vec();
    bytes.extend(character_id.to_le_bytes());
    pack_string(&mut bytes, account, account.len())?;
    Ok(bytes)
}

/// Full CharacterSet with empty active/deleted arrays, default status and chat disabled.
/// For a nonempty set with inherited VisualDesc, use `character_set`.
pub fn empty_character_set(account: &[u8], allowed_slots: u32) -> Result<Vec<u8>, WorldError> {
    if allowed_slots as usize > MAX_CHARACTERS {
        return Err(WorldError::LimitExceeded);
    }
    let mut bytes = CHARACTER_SET.to_le_bytes().to_vec();
    for value in [0_u32, 0, 0, allowed_slots] {
        bytes.extend(value.to_le_bytes());
    }
    pack_string(&mut bytes, account, account.len())?;
    bytes.extend(0_u32.to_le_bytes());
    Ok(bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinimalCharacter {
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionCharacter {
    pub id: u64,
    pub name: String,
    pub seconds_greyed_out: u32,
    /// Must refer to a valid VisualDesc in the matching client's DATs.
    pub parent_visual_did: u32,
}

fn pack_identity(bytes: &mut Vec<u8>, character: &SelectionCharacter) -> Result<(), WorldError> {
    if character.name.len() > MAX_STRING_BYTES {
        return Err(WorldError::LimitExceeded);
    }
    bytes.extend(character.id.to_le_bytes());
    let units: Vec<u16> = character.name.encode_utf16().collect();
    let utf16: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
    pack_string(bytes, &utf16, units.len())?;
    bytes.extend(character.seconds_greyed_out.to_le_bytes());
    bytes.extend(2_u32.to_le_bytes());
    bytes.extend(character.parent_visual_did.to_le_bytes());
    Ok(())
}

/// Full selection list using parent VisualDesc inheritance, with no appearance overrides.
/// Asset existence/appropriateness must be checked separately against the matching DATs.
pub fn character_set(
    account: &[u8],
    allowed_slots: u32,
    characters: &[SelectionCharacter],
) -> Result<Vec<u8>, WorldError> {
    if characters.len() > MAX_CHARACTERS || allowed_slots as usize > MAX_CHARACTERS {
        return Err(WorldError::LimitExceeded);
    }
    let mut bytes = CHARACTER_SET.to_le_bytes().to_vec();
    bytes.extend((characters.len() as u32).to_le_bytes());
    for character in characters {
        pack_identity(&mut bytes, character)?;
    }
    for value in [0_u32, 0, allowed_slots] {
        bytes.extend(value.to_le_bytes());
    }
    pack_string(&mut bytes, account, account.len())?;
    bytes.extend(0_u32.to_le_bytes());
    Ok(bytes)
}

/// The minimal-list wire event does not provide the visual data needed by selection UI.
pub fn minimal_character_set(
    account: &[u8],
    allowed_slots: u32,
    characters: &[MinimalCharacter],
) -> Result<Vec<u8>, WorldError> {
    if characters.len() > MAX_CHARACTERS || allowed_slots as usize > MAX_CHARACTERS {
        return Err(WorldError::LimitExceeded);
    }
    let mut bytes = MIN_CHARACTER_SET.to_le_bytes().to_vec();
    bytes.extend(allowed_slots.to_le_bytes());
    pack_string(&mut bytes, account, account.len())?;
    bytes.extend((characters.len() as u32).to_le_bytes());
    for character in characters {
        // Check before allocating an arbitrarily large caller-supplied name.
        if character.name.len() > MAX_STRING_BYTES {
            return Err(WorldError::LimitExceeded);
        }
        let units: Vec<u16> = character.name.encode_utf16().collect();
        let utf16: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
        pack_string(&mut bytes, &utf16, units.len())?;
    }
    bytes.extend((characters.len() as u32).to_le_bytes());
    for character in characters {
        bytes.extend(character.id.to_le_bytes());
    }
    Ok(bytes)
}

/// This notification sets player/region IDs; it does NOT create the world object.
pub fn create_player(character_id: u64, region_id: u32) -> Vec<u8> {
    let mut bytes = CREATE_PLAYER.to_le_bytes().to_vec();
    bytes.extend(character_id.to_le_bytes());
    bytes.extend(region_id.to_le_bytes());
    bytes
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CharacterError {
    EnterGameGeneric = 9,
    CharacterNotOwned = 13,
    CouldNotPlaceCharacter = 18,
}

pub fn character_error(error: CharacterError) -> Vec<u8> {
    let mut bytes = CHARACTER_ERROR.to_le_bytes().to_vec();
    bytes.extend((error as u32).to_le_bytes());
    bytes
}

pub fn char_exit_game() -> Vec<u8> {
    CHAR_EXIT_GAME.to_le_bytes().to_vec()
}
pub fn encode_delete_character(character_id: u64, account: &[u8]) -> Result<Vec<u8>, WorldError> {
    let mut bytes = CHARACTER_DELETION.to_le_bytes().to_vec();
    pack_string(&mut bytes, account, account.len())?;
    bytes.extend(character_id.to_le_bytes());
    Ok(bytes)
}
pub fn character_deleted(character_id: u64) -> Vec<u8> {
    let mut bytes = CHARACTER_DELETION.to_le_bytes().to_vec();
    bytes.extend(character_id.to_le_bytes());
    bytes
}

pub(crate) fn literal_name(name: &str) -> Result<Vec<u8>, WorldError> {
    if name.len() > MAX_STRING_BYTES {
        return Err(WorldError::LimitExceeded);
    }
    let mut bytes = vec![0; 10]; // table, string ID, variable count
    bytes.extend(1u16.to_le_bytes()); // literal present
    let units: Vec<_> = name.encode_utf16().collect();
    let utf16: Vec<_> = units.iter().flat_map(|v| v.to_le_bytes()).collect();
    pack_string(&mut bytes, &utf16, units.len())?;
    Ok(bytes)
}

pub(crate) fn read_literal_name(bytes: &[u8]) -> Result<(String, usize), WorldError> {
    let mut reader = Reader { bytes, pos: 0 };
    if reader.u32()? != 0 || reader.u32()? != 0 || reader.u32()? != 0x10000 {
        return Err(WorldError::InvalidValue);
    }
    let text = reader.wide_string()?;
    Ok((text, reader.pos))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CreationRejection {
    Undefined = 0,
    NameInUse = 3,
    NameBanned = 4,
    Corrupt = 5,
    DatabaseDown = 6,
    AdminPrivilegeDenied = 7,
    NoMoreSlots = 8,
}

/// Non-success CharGenVerification, carrying no created identity or visual asset.
pub fn reject_character_creation(reason: CreationRejection) -> Vec<u8> {
    let mut bytes = CHAR_GEN_VERIFICATION.to_le_bytes().to_vec();
    bytes.extend((reason as u32).to_le_bytes());
    bytes.extend(0_u64.to_le_bytes()); // no newly allocated character
    bytes.extend([0; 4]); // empty aligned UTF16 name
    bytes.extend(0_u32.to_le_bytes()); // seconds greyed out
    bytes.extend(0_u32.to_le_bytes()); // empty VisualDesc mask
    bytes.extend(0_u32.to_le_bytes()); // trailing verification argument
    bytes
}

/// A persisted, server-validated identity. The success response alone does not
/// enter the world; the native UI subsequently sends EnterWorld for this IID.
pub fn confirm_character_creation(character: &SelectionCharacter) -> Result<Vec<u8>, WorldError> {
    if character.id == 0 || character.parent_visual_did == 0 {
        return Err(WorldError::InvalidValue);
    }
    let mut bytes = CHAR_GEN_VERIFICATION.to_le_bytes().to_vec();
    bytes.extend(1u32.to_le_bytes());
    pack_identity(&mut bytes, character)?;
    bytes.extend(0u32.to_le_bytes());
    Ok(bytes)
}

/// Recovered stationary/unparented object subset. Callers supply real matching-world
/// placement and asset identities; these checks cannot prove DAT compatibility.
#[derive(Debug, Clone, PartialEq)]
pub struct StationaryObject {
    pub character_id: u64,
    pub parent_visual_did: u32,
    pub entity_did: u32,
    pub cell_id: u32,
    pub position: [f32; 3],
    /// Native serialized order is w,x,y,z.
    pub orientation: [f32; 4],
    pub instance_stamp: u16,
    pub visual_ordering_stamp: u16,
    pub timestamps: [u16; 4],
}

pub fn visual_int(id: u64, instance: u16, key: u32, value: u32) -> Vec<u8> {
    let mut wire = 0x1006au32.to_le_bytes().to_vec();
    wire.extend(id.to_le_bytes());
    wire.extend(instance.to_le_bytes());
    wire.extend(0u16.to_le_bytes());
    wire.extend(key.to_le_bytes());
    wire.extend(value.to_le_bytes());
    wire
}
pub fn visual_bool(id: u64, instance: u16, key: u32, value: bool) -> Vec<u8> {
    let mut wire = visual_int(id, instance, key, u32::from(value));
    wire[..4].copy_from_slice(&0x1006cu32.to_le_bytes());
    wire
}

/// Builds a syntactically recovered CreateObject subset; not a complete player bootstrap.
pub fn create_stationary_object(object: &StationaryObject) -> Result<Vec<u8>, WorldError> {
    create_stationary_object_with_scale(object, None)
}

/// Optional native gmWeenieDesc scale (bit 0x80, StreamPack 006037c0).
/// The positive scale bound is local content policy, not a recovered game limit.
pub fn create_stationary_object_with_scale(
    object: &StationaryObject,
    scale: Option<f32>,
) -> Result<Vec<u8>, WorldError> {
    if scale.is_some_and(|scale| !scale.is_finite() || scale <= 0.0 || scale > 100.0) {
        return Err(WorldError::InvalidValue);
    }
    let norm: f32 = object.orientation.iter().map(|v| v * v).sum();
    if object.character_id == 0
        || object.parent_visual_did == 0
        || object.entity_did == 0
        || object.cell_id == 0
        || object
            .position
            .iter()
            .chain(object.orientation.iter())
            .any(|v| !v.is_finite())
        || !norm.is_finite()
        || (norm - 1.0).abs() > 0.01
    {
        return Err(WorldError::InvalidValue);
    }
    let mut bytes = CREATE_OBJECT.to_le_bytes().to_vec();
    bytes.extend(object.character_id.to_le_bytes());
    bytes.extend(2_u32.to_le_bytes());
    bytes.extend(object.parent_visual_did.to_le_bytes());
    bytes.extend(0x8000_u32.to_le_bytes()); // PhysicsDesc: position present
    bytes.extend(object.instance_stamp.to_le_bytes());
    bytes.extend(object.visual_ordering_stamp.to_le_bytes());
    bytes.extend(object.cell_id.to_le_bytes());
    for value in object.position.iter().chain(object.orientation.iter()) {
        bytes.extend(value.to_le_bytes());
    }
    for stamp in object.timestamps {
        bytes.extend(stamp.to_le_bytes());
    }
    bytes.extend((0x02000000_u32 | if scale.is_some() { 0x80 } else { 0 }).to_le_bytes());
    bytes.extend(object.entity_did.to_le_bytes());
    if let Some(scale) = scale {
        bytes.extend(scale.to_le_bytes());
    }
    Ok(bytes)
}

/// Empty CBaseQualities: present-table mask 0, uncategorized DBObj DID.
/// The receiver requires an existing player object. This carries no player stats,
/// abilities or inventory and MUST NOT be treated as complete gameplay state.
pub fn empty_player_description(qualities_did: u32) -> Vec<u8> {
    let mut bytes = PLAYER_DESC.to_le_bytes().to_vec();
    bytes.extend(0_u32.to_le_bytes());
    bytes.extend(qualities_did.to_le_bytes());
    bytes
}

/// Contained item subset: no world position or physics parent. Ownership is in
/// gmWeenieDesc's container/wielder fields, not a client-supplied transform.
pub fn create_inventory_item(
    item: &crate::inventory::Item,
    owner: u64,
    instance: u16,
) -> Result<Vec<u8>, WorldError> {
    if owner == 0
        || instance == 0
        || item.id == 0
        || item.visual == 0
        || item.definition >> 24 != 0x47
        || item.quantity == 0
    {
        return Err(WorldError::InvalidValue);
    }
    let mut bytes = CREATE_OBJECT.to_le_bytes().to_vec();
    bytes.extend(item.id.to_le_bytes());
    bytes.extend(2u32.to_le_bytes()); // VisualDesc parent-DID bit, 0063d7f0.
    bytes.extend(item.visual.to_le_bytes());
    bytes.extend(0u32.to_le_bytes()); // PhysicsDesc has no optional fields.
    bytes.extend(instance.to_le_bytes());
    bytes.extend(0u16.to_le_bytes());
    bytes.extend([0; 8]); // Four mandatory PhysicsDesc timestamp ushorts.
    let owner_flag = match item.location {
        crate::inventory::Location::Bag(_) => 0x20,
        crate::inventory::Location::Equipped(_) => 0x40,
    };
    bytes.extend((0x02000800u32 | owner_flag).to_le_bytes());
    bytes.extend(item.definition.to_le_bytes());
    bytes.extend(owner.to_le_bytes());
    bytes.extend(u32::from(item.quantity).to_le_bytes());
    Ok(bytes)
}

/// Hand item with the native PhysicsDesc parent subset. The player must be
/// created first. No standalone cell/position is attached to a contained item.
pub fn create_hand_item(
    item: &crate::inventory::Item,
    owner: u64,
    instance: u16,
    parent_instance: u16,
    location: u32,
) -> Result<Vec<u8>, WorldError> {
    if !matches!(
        item.location,
        crate::inventory::Location::Equipped(0x40 | 0x80)
    ) || !matches!(location, 1 | 2)
        || parent_instance == 0
    {
        return Err(WorldError::InvalidValue);
    }
    let mut bytes = create_inventory_item(item, owner, instance)?;
    bytes[20..24].copy_from_slice(&0x20u32.to_le_bytes());
    let mut parent = owner.to_le_bytes().to_vec();
    parent.extend(location.to_le_bytes());
    parent.extend(parent_instance.to_le_bytes());
    parent.extend(0u16.to_le_bytes()); // Native alignment before timestamps.
    bytes.splice(28..28, parent);
    Ok(bytes)
}

pub fn destroy_inventory_item(id: u64, instance: u16) -> Vec<u8> {
    let mut bytes = 0x100c2u32.to_le_bytes().to_vec();
    bytes.extend(id.to_le_bytes());
    bytes.extend(instance.to_le_bytes());
    bytes.extend(0u16.to_le_bytes());
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deletion_uses_account_then_identity_and_exact_lengths() {
        let bytes = encode_delete_character(7, b"a").unwrap();
        assert_eq!(
            bytes,
            [0x29, 0, 1, 0, 1, 0, 0x16, 0, 7, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            decode_client_event(&bytes),
            Ok(ClientEvent::DeleteCharacter {
                account: b"a".to_vec(),
                character_id: 7
            })
        );
        for end in 0..bytes.len() {
            assert!(decode_client_event(&bytes[..end]).is_err());
        }
        assert_eq!(
            character_deleted(7),
            [0x29, 0, 1, 0, 7, 0, 0, 0, 0, 0, 0, 0]
        );
    }
    #[test]
    fn native_creation_request_and_all_truncations() {
        // account a, UTF16 name A, 6 scalar choices, one key/float appearance entry.
        let mut wire = vec![0x2a, 0, 1, 0, 1, 0, 0x16, 0, 1, 0, 0x14, 0];
        for value in [0x12345678_u32, 2, 3, 4, 5, 6, 0x05000001, 9, 0x3f000000] {
            wire.extend(value.to_le_bytes());
        }
        assert_eq!(
            decode_client_event(&wire),
            Ok(ClientEvent::CreateCharacter {
                account: b"a".to_vec(),
                request: CharacterCreationRequest {
                    name: "A".into(),
                    entity_did: 0x12345678,
                    start_area: 2,
                    race: 3,
                    sex: 4,
                    class: 5,
                    implement: 6,
                    physical_values: vec![(9, 0x3f000000)],
                },
            })
        );
        for len in 0..wire.len() {
            assert!(decode_client_event(&wire[..len]).is_err());
        }
        let end = wire.len();
        wire[end - 4..].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
        assert_eq!(decode_client_event(&wire), Err(WorldError::InvalidValue));
        wire[36..40].copy_from_slice(&257_u32.to_le_bytes());
        assert_eq!(decode_client_event(&wire), Err(WorldError::LimitExceeded));
        assert_eq!(
            reject_character_creation(CreationRejection::Undefined),
            [
                0x57, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0
            ]
        );
    }
    #[test]
    fn stationary_object_offsets_and_validation() {
        let mut object = StationaryObject {
            character_id: 1,
            parent_visual_did: 2,
            entity_did: 3,
            cell_id: 4,
            position: [1.0, 2.0, 3.0],
            orientation: [1.0, 0.0, 0.0, 0.0],
            instance_stamp: 5,
            visual_ordering_stamp: 6,
            timestamps: [7, 8, 9, 10],
        };
        let bytes = create_stationary_object(&object).unwrap();
        assert_eq!(bytes.len(), 76);
        assert_eq!(
            &bytes[..32],
            &[
                0x58, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 0, 0x80, 0, 0, 5, 0,
                6, 0, 4, 0, 0, 0
            ]
        );
        assert_eq!(
            &bytes[60..],
            &[7, 0, 8, 0, 9, 0, 10, 0, 0, 0, 0, 2, 3, 0, 0, 0]
        );
        assert_eq!(
            empty_player_description(0x80000001),
            [0x68, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0x80]
        );
        object.orientation = [0.0; 4];
        assert_eq!(
            create_stationary_object(&object),
            Err(WorldError::InvalidValue)
        );
        object.orientation = [1.0, 0.0, 0.0, 0.0];
        object.position[0] = f32::NAN;
        assert_eq!(
            create_stationary_object(&object),
            Err(WorldError::InvalidValue)
        );
    }
    #[test]
    fn enter_world_known_wire_and_every_truncation() {
        // abcd -> nibble swap 16 26 36 46 -> XOR 38 92 84 e5.
        let wire = [
            0x2b, 0, 1, 0, 8, 7, 6, 5, 4, 3, 2, 1, 4, 0, 0x2e, 0xb4, 0xb2, 0xa3, 0, 0,
        ];
        assert_eq!(
            encode_enter_world(0x0102030405060708, b"abcd").unwrap(),
            wire
        );
        assert_eq!(
            decode_client_event(&wire),
            Ok(ClientEvent::EnterWorld {
                character_id: 0x0102030405060708,
                account: b"abcd".to_vec(),
            })
        );
        for len in 0..wire.len() {
            assert!(decode_client_event(&wire[..len]).is_err());
        }
        let mut extra = wire.to_vec();
        extra.push(0);
        assert_eq!(decode_client_event(&extra), Err(WorldError::TrailingBytes));
    }
    #[test]
    fn partial_word_is_only_nibble_swapped() {
        let bytes = encode_enter_world(1, b"abc").unwrap();
        assert_eq!(&bytes[12..], &[3, 0, 0x16, 0x26, 0x36, 0, 0, 0]);
        assert_eq!(
            decode_client_event(&bytes),
            Ok(ClientEvent::EnterWorld {
                character_id: 1,
                account: b"abc".to_vec(),
            })
        );
    }
    #[test]
    fn limits_apply_before_untrusted_allocation() {
        let mut wire = ENTER_WORLD.to_le_bytes().to_vec();
        wire.extend(1_u64.to_le_bytes());
        wire.extend(0xffff_u16.to_le_bytes());
        wire.extend(u32::MAX.to_le_bytes());
        assert_eq!(decode_client_event(&wire), Err(WorldError::LimitExceeded));
        assert_eq!(
            empty_character_set(b"a", 65),
            Err(WorldError::LimitExceeded)
        );
    }
    #[test]
    fn minimal_list_uses_separate_names_and_ids_arrays() {
        let bytes = minimal_character_set(
            b"a",
            5,
            &[MinimalCharacter {
                id: 7,
                name: "A".into(),
            }],
        )
        .unwrap();
        assert_eq!(
            bytes,
            [
                0xb8, 0, 1, 0, 5, 0, 0, 0, 1, 0, 0x16, 0, 1, 0, 0, 0, 1, 0, 0x14, 0, 1, 0, 0, 0, 7,
                0, 0, 0, 0, 0, 0, 0
            ]
        );
    }
    #[test]
    fn full_empty_list_and_control_events() {
        assert_eq!(
            character_set(b"account", 5, &[]),
            empty_character_set(b"account", 5)
        );
        assert_eq!(
            empty_character_set(b"", 5).unwrap(),
            [
                0x55, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0
            ]
        );
        assert_eq!(
            create_player(1, 2),
            [0x60, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0]
        );
        assert_eq!(
            character_error(CharacterError::CharacterNotOwned),
            [0x56, 0, 1, 0, 13, 0, 0, 0]
        );
        assert_eq!(
            decode_client_event(&[0xd1, 0, 1, 0]),
            Ok(ClientEvent::SceneRenderingComplete)
        );
        assert_eq!(
            decode_client_event(&[0xd1, 0, 1, 0, 0]),
            Err(WorldError::TrailingBytes)
        );
        assert_eq!(
            decode_client_event(&[0xff, 0, 0, 0]),
            Err(WorldError::UnsupportedOpcode(255))
        );
    }
    #[test]
    fn full_selection_identity_layout() {
        let bytes = character_set(
            b"",
            5,
            &[SelectionCharacter {
                id: 7,
                name: "A".into(),
                seconds_greyed_out: 0,
                parent_visual_did: 0x01020304,
            }],
        )
        .unwrap();
        assert_eq!(
            &bytes[..32],
            &[
                0x55, 0, 1, 0, 1, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0x14, 0, 0, 0, 0, 0, 2, 0,
                0, 0, 4, 3, 2, 1
            ]
        );
        assert_eq!(
            &bytes[32..],
            &[0, 0, 0, 0, 0, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
    }
}
