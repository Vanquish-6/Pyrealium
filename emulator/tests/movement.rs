use pyrealium::{
    movement::{CLIENT_POSITION, Motion},
    state::{Phase, Session, StateError},
    world::{self, ClientEvent, WorldError},
};

// CPositionPack: double time, PositionOffset, packed motion/heading,
// flags, four ushort stamps, optional Vector. All fields end on 4-byte boundaries.
fn packet(jump: bool) -> Vec<u8> {
    let mut bytes = CLIENT_POSITION.to_le_bytes().to_vec();
    bytes.extend(10.0f64.to_le_bytes());
    bytes.extend(0x02990109u32.to_le_bytes());
    for value in [59.060577f32, 240.199, -44.606205] {
        bytes.extend(value.to_le_bytes());
    }
    bytes.extend(0x40000000u32.to_le_bytes());
    bytes.extend((if jump { 6u32 } else { 4u32 }).to_le_bytes());
    for stamp in [1u16, 0, 0, 0] {
        bytes.extend(stamp.to_le_bytes());
    }
    if jump {
        for value in [0.0f32, 0.0, 5.0] {
            bytes.extend(value.to_le_bytes());
        }
    }
    bytes
}

#[test]
fn native_position_bounds_and_optional_jump() {
    for jump in [false, true] {
        let bytes = packet(jump);
        assert_eq!(bytes.len(), if jump { 56 } else { 44 });
        let ClientEvent::Position(position) = world::decode_client_event(&bytes).unwrap() else {
            panic!()
        };
        assert_eq!(position.placement.cell, 0x02990109);
        assert_eq!(position.stamps, [1, 0, 0, 0]);
        assert_eq!(position.jump_velocity.is_some(), jump);
        for length in 0..bytes.len() {
            assert!(world::decode_client_event(&bytes[..length]).is_err());
        }
        let mut extra = bytes;
        extra.push(0);
        assert_eq!(
            world::decode_client_event(&extra),
            Err(WorldError::TrailingBytes)
        );
    }
}

#[test]
fn malformed_position_is_rejected() {
    for (offset, value) in [
        (12, 0u32),
        (16, f32::NAN.to_bits()),
        (32, 8),
        (52, f32::INFINITY.to_bits()),
    ] {
        let mut bytes = packet(true);
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert_eq!(
            world::decode_client_event(&bytes),
            Err(WorldError::InvalidValue)
        );
    }
    for time in [f64::NAN, f64::INFINITY, -1.0] {
        let mut bytes = packet(false);
        bytes[4..12].copy_from_slice(&time.to_le_bytes());
        assert_eq!(
            world::decode_client_event(&bytes),
            Err(WorldError::InvalidValue)
        );
    }
}

#[test]
fn placement_replay_spawn_and_session_transitions() {
    let ClientEvent::Position(position) = world::decode_client_event(&packet(false)).unwrap()
    else {
        panic!()
    };
    let mut session = Session::new("local".into(), 42, 0);
    assert_eq!(session.scene_complete(), Err(StateError::InvalidTransition));
    session.connect_ack(42, 1).unwrap();
    session.request_enter(1, "local", true).unwrap();
    // Loading reports may update hidden placement, while scene publication and
    // gameplay remain gated separately by the session in the actual runtime.
    let mut motion = Motion::new(position.placement, 1);
    session.scene_complete().unwrap();
    motion.accept(position).unwrap();
    assert_eq!(motion.accept(position), Err(WorldError::InvalidValue));
    let mut stale_spawn = position;
    stale_spawn.time = 11.0;
    stale_spawn.stamps[0] = 2;
    assert_eq!(motion.accept(stale_spawn), Err(WorldError::InvalidValue));
    assert_eq!(motion.last, Some(position));
    assert_eq!(session.phase, Phase::InWorld { character: 1 });
    assert_eq!(session.leave_world(2), Err(StateError::InvalidTransition));
    assert_eq!(session.phase, Phase::InWorld { character: 1 });
    session.leave_world(1).unwrap();
    assert_eq!(session.phase, Phase::CharacterSelection);
    assert_eq!(session.leave_world(1), Err(StateError::InvalidTransition));
    session.request_enter(1, "local", true).unwrap();
    session.scene_complete().unwrap();
    let ClientEvent::Position(position) = world::decode_client_event(&packet(false)).unwrap()
    else {
        panic!()
    };
    let mut motion = Motion::new(position.placement, 2);
    assert!(motion.accept(position).is_err());
    motion
        .accept(pyrealium::movement::ClientPosition {
            stamps: [2, 0, 0, 0],
            ..position
        })
        .unwrap();
}
