use pyrealium::{
    script::MAX_ARGUMENT_BYTES,
    world::{self, ClientEvent, WorldError},
};

fn event(function: u32, arguments: &[u8]) -> Vec<u8> {
    let mut bytes = world::CLIENT_SCRIPT_EVENT.to_le_bytes().to_vec();
    bytes.extend(function.to_le_bytes());
    bytes.extend((arguments.len() as u32).to_le_bytes());
    bytes.extend(arguments);
    bytes
}

#[test]
fn script_envelope_preserves_opaque_arguments_and_requires_exact_length() {
    // An int-tagged argument: framing does not evaluate or authorize it.
    let arguments = [1, 0, 0, 0, 42, 0, 0, 0];
    let bytes = event(0x123456, &arguments);
    assert_eq!(
        world::decode_client_event(&bytes),
        Ok(ClientEvent::ScriptCall {
            function_id: 0x123456,
            arguments: arguments.to_vec(),
        })
    );
    for length in 0..bytes.len() {
        assert!(world::decode_client_event(&bytes[..length]).is_err());
    }
    let mut extra = bytes;
    extra.push(0);
    assert_eq!(
        world::decode_client_event(&extra),
        Err(WorldError::TrailingBytes)
    );
    assert!(matches!(
        world::decode_client_event(&event(0x123456, &[])),
        Ok(ClientEvent::ScriptCall { .. })
    ));
}

#[test]
fn script_envelope_limits_before_allocating() {
    let mut bytes = event(1, &[]);
    bytes[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        world::decode_client_event(&bytes),
        Err(WorldError::LimitExceeded)
    );
    assert!(world::decode_client_event(&event(1, &vec![0; MAX_ARGUMENT_BYTES])).is_ok());
    assert_eq!(
        world::decode_client_event(&event(1, &vec![0; MAX_ARGUMENT_BYTES + 1])),
        Err(WorldError::LimitExceeded)
    );
}
