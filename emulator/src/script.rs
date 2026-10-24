//! Bounded native PackBuffer dialect and explicit gameplay message routing.
//! Function IDs are allowlisted; no client-supplied code is executed.
use crate::world::{self, WorldError};

pub const MAX_ARGUMENT_BYTES: usize = 64 * 1024;
pub const COMBAT_START: u32 = 0x82040008;
pub const COMBAT_STOP: u32 = 0x82040010;
pub const COMBAT_STATE: u32 = 0x8204001b;
pub const SAY: u32 = 0x82050006;
pub const TELL: u32 = 0x82050002;
pub const CHAT_STRING: u32 = 0x82050011;
pub const TARGET: u32 = 0x82120016;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Combat(bool),
    Attack(Option<u64>),
    Target(Option<u64>),
    Say(String),
    Tell { recipient: String, text: String },
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl Reader<'_> {
    fn u32(&mut self) -> Result<u32, WorldError> {
        let bytes = self
            .bytes
            .get(self.pos..self.pos + 4)
            .ok_or(WorldError::Truncated)?;
        self.pos += 4;
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
    }
    fn expect(&mut self, value: u32) -> Result<(), WorldError> {
        if self.u32()? != value {
            return Err(WorldError::InvalidValue);
        }
        Ok(())
    }
    fn int(&mut self) -> Result<u32, WorldError> {
        self.expect(1)?;
        self.u32()
    }
    fn iid(&mut self) -> Result<u64, WorldError> {
        self.expect(2)?;
        Ok(u64::from(self.u32()?) | (u64::from(self.u32()?) << 32))
    }
    fn text(&mut self) -> Result<String, WorldError> {
        // One reference to a standalone native StringInfo heap object. Reject
        // general graphs, code packages, substitutions and reserved references.
        self.expect(5)?;
        self.expect(1)?;
        if self.u32()? == u32::MAX {
            return Err(WorldError::InvalidValue);
        }
        self.expect(0x44000100)?;
        self.expect(0)?;
        self.expect(0xffff006a)?;
        let (text, size) = world::read_literal_name(&self.bytes[self.pos..])?;
        self.pos += size;
        self.expect(0)?; // HeapObject has no interpreted fields.
        if text.is_empty() || text.chars().count() > 512 || text.chars().any(char::is_control) {
            return Err(WorldError::InvalidValue);
        }
        Ok(text)
    }
}

pub fn decode(function: u32, bytes: &[u8]) -> Result<Request, WorldError> {
    if bytes.len() > MAX_ARGUMENT_BYTES {
        return Err(WorldError::LimitExceeded);
    }
    let mut r = Reader { bytes, pos: 0 };
    let request = match function {
        COMBAT_START | COMBAT_STOP => {
            if function == COMBAT_START && bytes.len() == 12 {
                let target = r.iid()?;
                Request::Attack((target != 0 && target != u64::MAX).then_some(target))
            } else {
                // Local supported profile: no arguments, or a reserved zero int.
                if !bytes.is_empty() && r.int()? != 0 {
                    return Err(WorldError::InvalidValue);
                }
                Request::Combat(function == COMBAT_START)
            }
        }
        TARGET => {
            let id = r.iid()?;
            Request::Target((id != 0 && id != u64::MAX).then_some(id))
        }
        SAY => {
            r.expect(1)?;
            r.expect(0)?;
            Request::Say(r.text()?)
        }
        TELL => {
            r.expect(1)?;
            r.expect(0)?;
            let text = r.text()?;
            let recipient = r.text()?;
            if recipient.len() > 32 {
                return Err(WorldError::InvalidValue);
            }
            Request::Tell { recipient, text }
        }
        _ => return Err(WorldError::UnsupportedOpcode(function)),
    };
    if r.pos != bytes.len() {
        return Err(WorldError::TrailingBytes);
    }
    Ok(request)
}

fn envelope(opcode: u32, function: u32, arguments: &[u8]) -> Result<Vec<u8>, WorldError> {
    if arguments.len() > MAX_ARGUMENT_BYTES {
        return Err(WorldError::LimitExceeded);
    }
    let mut bytes = Vec::with_capacity(12 + arguments.len());
    bytes.extend(opcode.to_le_bytes());
    bytes.extend(function.to_le_bytes());
    bytes.extend((arguments.len() as u32).to_le_bytes());
    bytes.extend(arguments);
    Ok(bytes)
}
pub fn client_call(function: u32, arguments: &[u8]) -> Result<Vec<u8>, WorldError> {
    envelope(world::CLIENT_SCRIPT_EVENT, function, arguments)
}
pub fn private_call(function: u32, arguments: &[u8]) -> Result<Vec<u8>, WorldError> {
    envelope(0x10066, function, arguments)
}
pub fn integer(value: u32) -> Vec<u8> {
    [1u32.to_le_bytes(), value.to_le_bytes()].concat()
}
pub fn identity(value: u64) -> Vec<u8> {
    let mut bytes = 2u32.to_le_bytes().to_vec();
    bytes.extend(value.to_le_bytes());
    bytes
}
pub fn literal(text: &str) -> Result<Vec<u8>, WorldError> {
    let mut bytes = Vec::new();
    for word in [5u32, 1, 1, 0x44000100, 0, 0xffff006a] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.extend(world::literal_name(text)?);
    bytes.extend(0u32.to_le_bytes());
    Ok(bytes)
}
pub fn chat(text: &str, kind: u32) -> Result<Vec<u8>, WorldError> {
    let mut arguments = literal(text)?;
    arguments.extend(integer(kind));
    private_call(CHAT_STRING, &arguments)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requests_have_exact_types_lengths_and_bounded_text() {
        let mut say = integer(0);
        say.extend(literal("Hello, κόσμε").unwrap());
        assert_eq!(decode(SAY, &say), Ok(Request::Say("Hello, κόσμε".into())));
        for length in 0..say.len() {
            assert!(decode(SAY, &say[..length]).is_err());
        }
        let mut trailing = say.clone();
        trailing.push(0);
        assert_eq!(decode(SAY, &trailing), Err(WorldError::TrailingBytes));
        let mut tell = integer(0);
        tell.extend(literal("hello").unwrap());
        tell.extend(literal("Explorer").unwrap());
        assert_eq!(
            decode(TELL, &tell),
            Ok(Request::Tell {
                recipient: "Explorer".into(),
                text: "hello".into()
            })
        );
        assert_eq!(decode(TARGET, &identity(9)), Ok(Request::Target(Some(9))));
        assert_eq!(
            decode(TARGET, &identity(u64::MAX)),
            Ok(Request::Target(None))
        );
        assert!(decode(TARGET, &integer(9)).is_err());
        assert_eq!(decode(COMBAT_START, &[]), Ok(Request::Combat(true)));
        assert_eq!(decode(COMBAT_STOP, &integer(0)), Ok(Request::Combat(false)));
        assert!(decode(COMBAT_START, &integer(7)).is_err());
        let mut control = integer(0);
        control.extend(literal("a\nb").unwrap());
        assert!(decode(SAY, &control).is_err());
        let mut long = integer(0);
        long.extend(literal(&"x".repeat(513)).unwrap());
        assert!(decode(SAY, &long).is_err());
    }
    #[test]
    fn native_combat_start_preserves_target_and_requires_exact_lint_encoding() {
        for id in [0, u64::MAX, 7, 0x8120000012345678] {
            let args = identity(id);
            assert_eq!(
                decode(COMBAT_START, &args),
                Ok(Request::Attack((id != 0 && id != u64::MAX).then_some(id)))
            );
            for length in 1..args.len() {
                assert!(decode(COMBAT_START, &args[..length]).is_err());
            }
            let mut extra = args;
            extra.push(0);
            assert!(decode(COMBAT_START, &extra).is_err());
        }
        let mut wrong = identity(7);
        wrong[..4].copy_from_slice(&1u32.to_le_bytes());
        assert!(decode(COMBAT_START, &wrong).is_err());
        assert!(decode(COMBAT_STOP, &identity(7)).is_err());
    }
    #[test]
    fn script_envelope_is_bounded_and_exact() {
        let packet = client_call(TARGET, &identity(9)).unwrap();
        assert_eq!(packet.len(), 24);
        assert!(matches!(
            world::decode_client_event(&packet),
            Ok(world::ClientEvent::ScriptCall {
                function_id: TARGET,
                ..
            })
        ));
        for length in 0..packet.len() {
            assert!(world::decode_client_event(&packet[..length]).is_err());
        }
        let mut extra = packet.clone();
        extra.push(0);
        assert_eq!(
            world::decode_client_event(&extra),
            Err(WorldError::TrailingBytes)
        );
        let mut huge = packet;
        huge[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            world::decode_client_event(&huge),
            Err(WorldError::LimitExceeded)
        );
    }
}
