//! Bounded AC2 7148 wire primitives. Evidence: recovery/exports/ac2client.exe/functions.
//! Established-session ISAAC checksum masking is deliberately not implemented here.
use std::fmt;

pub const HEADER_LEN: usize = 20;
pub const LOGON: u32 = 0x10000;
pub const CONNECT: u32 = 0x40000;
pub const CONNECT_ACK: u32 = 0x80000;
pub const MAX_DATAGRAM: usize = 65507;
pub const ENCRYPTED_CHECKSUM: u32 = 2;
pub const ACK: u32 = 0x4000;
pub const LOGOFF: u32 = 0x8000;
pub const TIME_SYNC: u32 = 0x1000000;
pub const ECHO_REQUEST: u32 = 0x2000000;
pub const ECHO_RESPONSE: u32 = 0x4000000;
pub const FLOW: u32 = 0x8000000;

#[derive(Debug, Clone, PartialEq)]
pub struct SessionPacket<'a> {
    pub header: Header,
    pub ack: Option<u32>,
    pub naks: Vec<u32>,
    pub empty_acks: Vec<u32>,
    pub time_sync: Option<f64>,
    pub echo_request: Option<f32>,
    pub echo_response: Option<(f32, f32)>,
    pub flow: Option<(u32, u16)>,
    pub fragments: Vec<Fragment<'a>>,
}
/// Optional headers are registered in ascending mask order (0063b7e0,0063b880).
/// The key masks only the sum of body segments, before adding the header checksum.
pub fn decode_session_packet(
    bytes: &[u8],
    key: Option<u32>,
) -> Result<SessionPacket<'_>, WireError> {
    let h = Header::decode(bytes)?;
    // ClientNet::LogOffServer 004c7590 sends CEmptyHeader<32768,3>.
    // SharedNet::SendOptionalHeader 004c66e0 uses sequence zero, with the
    // current recipient/iteration and no ISAAC masking. Only that form is
    // supported; a combined logoff/application packet must not publish data.
    if h.flags & LOGOFF != 0 && (h.flags != LOGOFF || h.sequence != 0) {
        return Err(WireError::UnsupportedFlags);
    }
    let allowed = ENCRYPTED_CHECKSUM
        | BLOB_FRAGMENTS
        | 0x1000
        | 0x2000
        | ACK
        | LOGOFF
        | TIME_SYNC
        | ECHO_REQUEST
        | ECHO_RESPONSE
        | FLOW;
    if h.flags & !allowed != 0
        || (h.flags & ENCRYPTED_CHECKSUM != 0 && (h.sequence == 0 || key.is_none()))
    {
        return Err(WireError::UnsupportedFlags);
    }
    if bytes.len() > MAX_DATAGRAM || bytes.len() != HEADER_LEN + h.payload_len as usize {
        return Err(WireError::Length);
    }
    let mut p = SessionPacket {
        header: h,
        ack: None,
        naks: vec![],
        empty_acks: vec![],
        time_sync: None,
        echo_request: None,
        echo_response: None,
        flow: None,
        fragments: vec![],
    };
    let body = &bytes[HEADER_LEN..];
    let mut r = Reader::new(body);
    let mut sum = 0u32;
    for flag in [
        0x1000,
        0x2000,
        ACK,
        TIME_SYNC,
        ECHO_REQUEST,
        ECHO_RESPONSE,
        FLOW,
    ] {
        if h.flags & flag == 0 {
            continue;
        }
        let start = r.pos;
        match flag {
            0x1000 | 0x2000 => {
                let count = r.u32()?;
                if count >= 0x73 {
                    return Err(WireError::Length);
                }
                let mut values = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    values.push(r.u32()?);
                }
                if flag == 0x1000 {
                    p.naks = values
                } else {
                    p.empty_acks = values
                }
            }
            ACK => p.ack = Some(r.u32()?),
            TIME_SYNC => p.time_sync = Some(f64::from_bits(r.u64()?)),
            ECHO_REQUEST => p.echo_request = Some(f32::from_bits(r.u32()?)),
            ECHO_RESPONSE => {
                p.echo_response = Some((f32::from_bits(r.u32()?), f32::from_bits(r.u32()?)))
            }
            FLOW => p.flow = Some((r.u32()?, r.u16()?)),
            _ => unreachable!(),
        }
        sum = sum.wrapping_add(checksum(&body[start..r.pos]));
    }
    if h.flags & BLOB_FRAGMENTS != 0 {
        while r.remaining() != 0 {
            if r.remaining() < 16 {
                return Err(WireError::Truncated);
            }
            let n = u16::from_le_bytes(body[r.pos + 10..r.pos + 12].try_into().unwrap()) as usize;
            let segment = r.take(n)?;
            let frag = Fragment::decode(segment)?;
            sum = sum
                .wrapping_add(checksum(&segment[..16]))
                .wrapping_add(checksum(&segment[16..]));
            p.fragments.push(frag);
        }
        if p.fragments.is_empty() {
            return Err(WireError::Length);
        }
    }
    if r.remaining() != 0 {
        return Err(WireError::Length);
    }
    if h.flags & ENCRYPTED_CHECKSUM != 0 {
        sum ^= key.ok_or(WireError::UnsupportedFlags)?;
    }
    if h.checksum != sum.wrapping_add(h.header_checksum()) {
        return Err(WireError::Checksum);
    }
    Ok(p)
}
/// Encode one known fixed-size optional header, e.g. ACK or echo response.
pub fn encode_control_packet(
    mut h: Header,
    payload: &[u8],
    key: Option<u32>,
) -> Result<Vec<u8>, WireError> {
    let flag = h.flags & !ENCRYPTED_CHECKSUM;
    let size = match flag {
        LOGOFF if h.flags == LOGOFF && h.sequence == 0 => 0,
        ACK | ECHO_REQUEST => 4,
        TIME_SYNC | ECHO_RESPONSE => 8,
        FLOW => 6,
        _ => return Err(WireError::UnsupportedFlags),
    };
    if payload.len() != size {
        return Err(WireError::Length);
    }
    let mut sum = checksum(payload);
    if h.flags & ENCRYPTED_CHECKSUM != 0 {
        if h.sequence == 0 {
            return Err(WireError::UnsupportedFlags);
        }
        sum ^= key.ok_or(WireError::UnsupportedFlags)?;
    }
    h.payload_len = payload.len() as u16;
    h.checksum = h.header_checksum().wrapping_add(sum);
    let mut out = h.encode().to_vec();
    out.extend(payload);
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    Truncated,
    Length,
    Checksum,
    UnsupportedFlags,
    InvalidString,
}
impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for WireError {}

/// PortalChecksum::CalcChecksum32 (0065f3d0), including unusual trailing-byte order.
pub fn checksum(bytes: &[u8]) -> u32 {
    let mut sum = (bytes.len() as u32).wrapping_shl(16);
    let mut words = bytes.chunks_exact(4);
    for word in &mut words {
        sum = sum.wrapping_add(u32::from_le_bytes(word.try_into().unwrap()));
    }
    for (i, byte) in words.remainder().iter().enumerate() {
        sum = sum.wrapping_add((*byte as u32) << (24 - i * 8));
    }
    sum
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Header {
    pub sequence: u32,
    pub flags: u32,
    pub checksum: u32,
    pub recipient: u16,
    pub interval: u16,
    pub payload_len: u16,
    pub iteration: u16,
}
impl Header {
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(bytes);
        Ok(Self {
            sequence: r.u32()?,
            flags: r.u32()?,
            checksum: r.u32()?,
            recipient: r.u16()?,
            interval: r.u16()?,
            payload_len: r.u16()?,
            iteration: r.u16()?,
        })
    }
    pub fn encode(self) -> [u8; HEADER_LEN] {
        let mut b = [0; HEADER_LEN];
        b[0..4].copy_from_slice(&self.sequence.to_le_bytes());
        b[4..8].copy_from_slice(&self.flags.to_le_bytes());
        b[8..12].copy_from_slice(&self.checksum.to_le_bytes());
        b[12..14].copy_from_slice(&self.recipient.to_le_bytes());
        b[14..16].copy_from_slice(&self.interval.to_le_bytes());
        b[16..18].copy_from_slice(&self.payload_len.to_le_bytes());
        b[18..20].copy_from_slice(&self.iteration.to_le_bytes());
        b
    }
    pub fn header_checksum(self) -> u32 {
        checksum(
            &Self {
                checksum: 0xbadd70dd,
                ..self
            }
            .encode(),
        )
    }
}

/// Parse only single, unencrypted handshake optional headers. No claim to parse session packets.
pub fn decode_handshake(bytes: &[u8]) -> Result<(Header, &[u8]), WireError> {
    if bytes.len() > MAX_DATAGRAM {
        return Err(WireError::Length);
    }
    let h = Header::decode(bytes)?;
    if !matches!(h.flags, LOGON | CONNECT | CONNECT_ACK) || h.sequence != 0 {
        return Err(WireError::UnsupportedFlags);
    }
    if bytes.len() != HEADER_LEN + usize::from(h.payload_len) {
        return Err(WireError::Length);
    }
    let payload = &bytes[HEADER_LEN..];
    if h.checksum != h.header_checksum().wrapping_add(checksum(payload)) {
        return Err(WireError::Checksum);
    }
    Ok((h, payload))
}
pub fn encode_handshake(mut h: Header, payload: &[u8]) -> Result<Vec<u8>, WireError> {
    if payload.len() > MAX_DATAGRAM - HEADER_LEN {
        return Err(WireError::Length);
    }
    if !matches!(h.flags, LOGON | CONNECT | CONNECT_ACK) || h.sequence != 0 {
        return Err(WireError::UnsupportedFlags);
    }
    h.payload_len = payload.len() as u16;
    h.checksum = h.header_checksum().wrapping_add(checksum(payload));
    let mut out = h.encode().to_vec();
    out.extend_from_slice(payload);
    Ok(out)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Connect {
    pub server_time: f64,
    pub cookie: u64,
    pub net_id: u32,
    pub outgoing_seed: u32,
    pub incoming_seed: u32,
}
impl Connect {
    /// CConnectHeader::CreateFromStream (0063bbe0): native structure includes 4 tail padding bytes.
    pub fn encode(&self) -> [u8; 32] {
        let mut out = [0; 32];
        out[0..8].copy_from_slice(&self.server_time.to_le_bytes());
        out[8..16].copy_from_slice(&self.cookie.to_le_bytes());
        out[16..20].copy_from_slice(&self.net_id.to_le_bytes());
        out[20..24].copy_from_slice(&self.outgoing_seed.to_le_bytes());
        out[24..28].copy_from_slice(&self.incoming_seed.to_le_bytes());
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() != 32 {
            return Err(WireError::Length);
        }
        let mut r = Reader::new(bytes);
        Ok(Self {
            server_time: f64::from_bits(r.u64()?),
            cookie: r.u64()?,
            net_id: r.u32()?,
            outgoing_seed: r.u32()?,
            incoming_seed: r.u32()?,
        })
    }
}
pub fn decode_connect_ack(bytes: &[u8]) -> Result<u64, WireError> {
    if bytes.len() != 8 {
        return Err(WireError::Length);
    }
    Reader::new(bytes).u64()
}

pub const BLOB_FRAGMENTS: u32 = 4;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fragment<'a> {
    pub blob_id: u64,
    pub count: u16,
    pub index: u16,
    pub queue: u16,
    pub data: &'a [u8],
}
impl<'a> Fragment<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, WireError> {
        let mut r = Reader::new(bytes);
        let blob_id = r.u64()?;
        let count = r.u16()?;
        let size = r.u16()? as usize;
        let index = r.u16()?;
        let queue = r.u16()?;
        if size != bytes.len() || size <= 16 || count == 0 || index >= count {
            return Err(WireError::Length);
        }
        Ok(Self {
            blob_id,
            count,
            index,
            queue,
            data: r.take(r.remaining())?,
        })
    }
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        if self.data.is_empty()
            || self.data.len() > MAX_DATAGRAM - HEADER_LEN - 16
            || self.count == 0
            || self.index >= self.count
        {
            return Err(WireError::Length);
        }
        let mut b = self.blob_id.to_le_bytes().to_vec();
        b.extend(self.count.to_le_bytes());
        b.extend(((self.data.len() + 16) as u16).to_le_bytes());
        b.extend(self.index.to_le_bytes());
        b.extend(self.queue.to_le_bytes());
        b.extend(self.data);
        Ok(b)
    }
}
/// Plain single-fragment datagram. Reliable sequence/retry policy belongs to the session layer.
pub fn encode_fragment_packet(mut h: Header, fragment: Fragment<'_>) -> Result<Vec<u8>, WireError> {
    if h.flags != BLOB_FRAGMENTS || h.sequence == 0 {
        return Err(WireError::UnsupportedFlags);
    }
    let b = fragment.encode()?;
    h.payload_len = b.len() as u16;
    h.checksum = h
        .header_checksum()
        .wrapping_add(checksum(&b[..16]))
        .wrapping_add(checksum(&b[16..]));
    let mut out = h.encode().to_vec();
    out.extend(b);
    Ok(out)
}
pub fn decode_fragment_packet(bytes: &[u8]) -> Result<(Header, Fragment<'_>), WireError> {
    let h = Header::decode(bytes)?;
    if h.flags != BLOB_FRAGMENTS || h.sequence == 0 {
        return Err(WireError::UnsupportedFlags);
    }
    if bytes.len() > MAX_DATAGRAM || bytes.len() != HEADER_LEN + h.payload_len as usize {
        return Err(WireError::Length);
    }
    let b = &bytes[HEADER_LEN..];
    let frag = Fragment::decode(b)?;
    let expected = h
        .header_checksum()
        .wrapping_add(checksum(&b[..16]))
        .wrapping_add(checksum(&b[16..]));
    if expected != h.checksum {
        return Err(WireError::Checksum);
    }
    Ok((h, frag))
}

#[derive(Clone, PartialEq, Eq)]
pub struct Login {
    pub version: String,
    pub auth_type: u32,
    pub auth_flags: u32,
    pub connection_sequence: u32,
    pub account: String,
    pub account_override: Option<String>,
    pub crypto_data: Vec<u8>,
    pub extra_data: Vec<u8>,
}
impl fmt::Debug for Login {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Login")
            .field("version", &self.version)
            .field("auth_type", &self.auth_type)
            .field("auth_flags", &self.auth_flags)
            .field("account", &self.account)
            .field("crypto_bytes", &self.crypto_data.len())
            .field("extra_bytes", &self.extra_data.len())
            .finish_non_exhaustive()
    }
}
impl Login {
    /// Native gmClient::InitAuth 00404e10 packs /glsticket as auth 40000002.
    /// Native string obfuscation is not encryption. Never log this result.
    pub fn native_ticket(&self) -> Result<String, WireError> {
        if !matches!(self.version.as_str(), "7148" | "7230")
            || self.auth_type != 0x40000002
            || self.auth_flags & !1 != 0
            || self.account_override.is_some()
            || self.extra_data.len() > 132
            || (self.auth_flags & 1 == 0 && !self.crypto_data.is_empty())
            || (self.auth_flags & 1 != 0 && !(12..=4096).contains(&self.crypto_data.len()))
        {
            return Err(WireError::UnsupportedFlags);
        }
        let mut reader = Reader::new(&self.extra_data);
        let token = reader.string()?;
        if reader.remaining() != 0
            || token.len() != 64
            || !token
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(WireError::Length);
        }
        Ok(token)
    }
    /// Local identity policy only. Native Init (004c9050) exports a CryptoAPI
    /// SIMPLEBLOB and SetCryptoKey (004c4f10) sets bit 0. Its encrypted key remains
    /// opaque here; queue-7 payload encryption is not implemented or authenticated.
    pub fn supports_developer_auth(&self, account: &str) -> bool {
        matches!(self.version.as_str(), "7148" | "7230")
            && self.auth_type == 1
            && self.auth_flags & !1 == 0
            && self.account == account
            && self.account_override.is_none()
            && self.extra_data.is_empty()
            && if self.auth_flags & 1 == 0 {
                self.crypto_data.is_empty()
            } else {
                (12..=4096).contains(&self.crypto_data.len())
            }
    }
    /// 0063c340 and NetAuthenticator::StreamPack (004c5200). Blob bytes remain opaque.
    pub fn decode(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() > MAX_DATAGRAM - HEADER_LEN {
            return Err(WireError::Length);
        }
        let mut r = Reader::new(bytes);
        let version = r.string()?;
        let size = r.u32()? as usize;
        if size != r.remaining() {
            return Err(WireError::Length);
        }
        let auth_type = r.u32()?;
        let auth_flags = r.u32()?;
        let connection_sequence = r.u32()?;
        let account = r.string()?;
        let account_override = if auth_flags & 2 != 0 {
            Some(r.string()?)
        } else {
            None
        };
        let crypto_data = r.blob()?.to_vec();
        let extra_data = r.blob()?.to_vec();
        if r.remaining() != 0 {
            return Err(WireError::Length);
        }
        Ok(Self {
            version,
            auth_type,
            auth_flags,
            connection_sequence,
            account,
            account_override,
            crypto_data,
            extra_data,
        })
    }
}

const STRING_KEYS: [u32; 16] = [
    0xe5849238, 0xeb2d767d, 0xfc0fdd03, 0xcb45a6a6, 0x7f21e6bb, 0xf5a6cdc8, 0x22d9ad12, 0xb2a3d4de,
    0x3b834db7, 0xc406c91c, 0x8392305e, 0x0a9f0099, 0x0943b90c, 0xa66d953c, 0x0d929e1f, 0x0e2e9ad4,
];
fn string_xor(b: &mut [u8]) {
    for (i, word) in b.chunks_exact_mut(4).enumerate() {
        let n = u32::from_le_bytes(word.try_into().unwrap()) ^ STRING_KEYS[i % 16];
        word.copy_from_slice(&n.to_le_bytes());
    }
}
pub fn pack_string(value: &str) -> Result<Vec<u8>, WireError> {
    if value.len() > u16::MAX as usize - 1 {
        return Err(WireError::Length);
    }
    let mut out = (value.len() as u16).to_le_bytes().to_vec();
    let mut body: Vec<u8> = value.bytes().map(|b| b.rotate_left(4)).collect();
    string_xor(&mut body);
    out.extend_from_slice(&body);
    while out.len() % 4 != 0 {
        out.push(0);
    }
    Ok(out)
}
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
    fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        if n > self.remaining() {
            return Err(WireError::Truncated);
        }
        let start = self.pos;
        self.pos += n;
        Ok(&self.bytes[start..self.pos])
    }
    fn u16(&mut self) -> Result<u16, WireError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn blob(&mut self) -> Result<&'a [u8], WireError> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    fn string(&mut self) -> Result<String, WireError> {
        let n = self.u16()?;
        let n = if n == 0xffff {
            self.u32()? as usize
        } else {
            n as usize
        };
        let mut b = self.take(n)?.to_vec();
        string_xor(&mut b);
        for c in &mut b {
            *c = c.rotate_left(4);
        }
        let padding = (4 - self.pos % 4) % 4;
        self.take(padding)?;
        String::from_utf8(b).map_err(|_| WireError::InvalidString)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checksum_golden_tails() {
        assert_eq!(checksum(&[]), 0);
        assert_eq!(checksum(&[1]), 0x01010000);
        assert_eq!(checksum(&[1, 2, 3]), 0x01050300);
        assert_eq!(checksum(&[1, 2, 3, 4]), 0x04070201);
        assert_eq!(checksum(&[255; 8]), 0x0007fffe);
    }
    #[test]
    fn recovered_string_vector() {
        // '7148': nibble-swapped 73 13 43 83 XOR 38 92 84 e5.
        assert_eq!(
            pack_string("7148").unwrap(),
            [4, 0, 0x4b, 0x81, 0xc7, 0x66, 0, 0]
        );
        assert_eq!(
            Reader::new(&[4, 0, 0x4b, 0x81, 0xc7, 0x66, 0, 0])
                .string()
                .unwrap(),
            "7148"
        );
    }
    #[test]
    fn strict_packet_length_and_integrity() {
        let c = Connect {
            server_time: 1.0,
            cookie: 0x1122334455667788,
            net_id: 2,
            outgoing_seed: 3,
            incoming_seed: 4,
        };
        let packet = encode_handshake(
            Header {
                flags: CONNECT,
                recipient: 1,
                iteration: 1,
                ..Default::default()
            },
            &c.encode(),
        )
        .unwrap();
        let (_, body) = decode_handshake(&packet).unwrap();
        assert_eq!(Connect::decode(body).unwrap(), c);
        for len in 0..packet.len() {
            assert!(decode_handshake(&packet[..len]).is_err());
        }
        let mut bad = packet.clone();
        bad[30] ^= 1;
        assert_eq!(decode_handshake(&bad), Err(WireError::Checksum));
        let mut extra = packet;
        extra.push(0);
        assert_eq!(decode_handshake(&extra), Err(WireError::Length));
    }
    #[test]
    fn bounded_login_and_optional_override() {
        let mut auth = Vec::new();
        for n in [1u32, 2, 9] {
            auth.extend(n.to_le_bytes());
        }
        auth.extend(pack_string("tester").unwrap());
        auth.extend(pack_string("override").unwrap());
        auth.extend(3u32.to_le_bytes());
        auth.extend([7, 8, 9]);
        auth.extend(0u32.to_le_bytes());
        let mut body = pack_string("7148").unwrap();
        body.extend((auth.len() as u32).to_le_bytes());
        body.extend(auth);
        let login = Login::decode(&body).unwrap();
        assert_eq!(login.account, "tester");
        assert_eq!(login.account_override.as_deref(), Some("override"));
        assert_eq!(login.crypto_data, [7, 8, 9]);
        for len in 0..body.len() {
            assert!(Login::decode(&body[..len]).is_err());
        }
        let mut invalid = body;
        invalid[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(Login::decode(&invalid), Err(WireError::Length));
    }
    #[test]
    fn fragment_bounds_and_segment_checksum() {
        let frag = Fragment {
            blob_id: 1,
            count: 1,
            index: 0,
            queue: 1,
            data: &[1, 2, 3, 4, 5],
        };
        let p = encode_fragment_packet(
            Header {
                sequence: 1,
                flags: BLOB_FRAGMENTS,
                ..Default::default()
            },
            frag,
        )
        .unwrap();
        assert_eq!(decode_fragment_packet(&p).unwrap().1, frag);
        for n in 0..p.len() {
            assert!(decode_fragment_packet(&p[..n]).is_err());
        }
        let invalid = Fragment { index: 1, ..frag };
        assert_eq!(invalid.encode(), Err(WireError::Length));
    }
    #[test]
    fn session_encryption_and_combined_headers() {
        let key = 0x13579bdf;
        let h = Header {
            sequence: 1,
            flags: ACK | ENCRYPTED_CHECKSUM,
            ..Default::default()
        };
        let packet = encode_control_packet(h, &7u32.to_le_bytes(), Some(key)).unwrap();
        assert_eq!(
            decode_session_packet(&packet, Some(key)).unwrap().ack,
            Some(7)
        );
        assert_eq!(
            decode_session_packet(&packet, Some(key ^ 1)),
            Err(WireError::Checksum)
        );
        assert_eq!(
            decode_session_packet(&packet, None),
            Err(WireError::UnsupportedFlags)
        );
        let ack = 9u32.to_le_bytes();
        let flow = [1, 2, 3, 4, 5, 6];
        let frag = Fragment {
            blob_id: 0x0100000100000001,
            count: 1,
            index: 0,
            queue: 3,
            data: &[5, 4, 3, 2, 1],
        }
        .encode()
        .unwrap();
        let mut h = Header {
            sequence: 2,
            flags: ACK | FLOW | BLOB_FRAGMENTS,
            payload_len: (ack.len() + flow.len() + frag.len()) as u16,
            ..Default::default()
        };
        h.checksum = h
            .header_checksum()
            .wrapping_add(checksum(&ack))
            .wrapping_add(checksum(&flow))
            .wrapping_add(checksum(&frag[..16]))
            .wrapping_add(checksum(&frag[16..]));
        let mut packet = h.encode().to_vec();
        packet.extend(ack);
        packet.extend(flow);
        packet.extend(frag);
        let result = decode_session_packet(&packet, None).unwrap();
        assert_eq!(result.ack, Some(9));
        assert_eq!(result.flow, Some((0x04030201, 0x0605)));
        assert_eq!(result.fragments.len(), 1);
        for n in 0..packet.len() {
            assert!(decode_session_packet(&packet[..n], None).is_err());
        }
    }
}
