//! Bounded session reliability. State commits only after checksum and structure validation.
use crate::{
    crypto::Isaac,
    wire::{self, Fragment, Header, WireError},
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

const WINDOW: u32 = 512;
const MAX_MESSAGE: usize = 64 * 1024;
const FRAGMENT_DATA: usize = 448;
const MAX_PENDING: usize = 256;

#[derive(Debug)]
pub enum TransportError {
    Wire(WireError),
    Sequence,
    Capacity,
    Fragment,
    RetryLimit,
}
impl From<WireError> for TransportError {
    fn from(e: WireError) -> Self {
        Self::Wire(e)
    }
}
impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for TransportError {}

#[derive(Debug, Clone)]
pub struct Message {
    pub blob_id: u64,
    pub queue: u16,
    pub data: Vec<u8>,
}
#[derive(Debug)]
pub struct Received {
    pub logoff: bool,
    pub messages: Vec<Message>,
    pub duplicate: bool,
    pub ack: u32,
    pub echo_request: Option<f32>,
    pub naks: Vec<u32>,
}
#[derive(Clone)]
struct Assembly {
    queue: u16,
    pieces: Vec<Option<Vec<u8>>>,
    bytes: usize,
    started: Instant,
}
#[derive(Clone)]
struct Pending {
    // Transactional receive clones metadata, not the immutable retry payload.
    bytes: Arc<[u8]>,
    sent: Instant,
    retries: u8,
}
#[derive(Clone)]
pub struct Transport {
    incoming: Isaac,
    outgoing: Isaac,
    recipient: u16,
    iteration: u16,
    generated: u32,
    contiguous: u32,
    seen: BTreeSet<u32>,
    keys: BTreeMap<u32, u32>,
    next_outgoing: u32,
    pending: BTreeMap<u32, Pending>,
    assemblies: BTreeMap<u64, Assembly>,
    completed: VecDeque<u64>,
    next_ordered: u32,
    ordered: BTreeMap<u16, Message>,
    ordered_bytes: usize,
}
impl Transport {
    pub fn new(incoming_seed: u32, outgoing_seed: u32, recipient: u16, iteration: u16) -> Self {
        Self {
            incoming: Isaac::new(incoming_seed),
            outgoing: Isaac::new(outgoing_seed),
            recipient,
            iteration,
            generated: 1,
            contiguous: 1,
            seen: BTreeSet::new(),
            keys: BTreeMap::new(),
            next_outgoing: 2,
            pending: BTreeMap::new(),
            assemblies: BTreeMap::new(),
            completed: VecDeque::new(),
            next_ordered: 1,
            ordered: BTreeMap::new(),
            ordered_bytes: 0,
        }
    }
    pub fn receive(&mut self, bytes: &[u8], now: Instant) -> Result<Received, TransportError> {
        let header = Header::decode(bytes)?;
        if header.sequence == 0
            && header.flags & (wire::ENCRYPTED_CHECKSUM | wire::BLOB_FRAGMENTS) == 0
        {
            if header.recipient != self.recipient || header.iteration != self.iteration {
                return Err(TransportError::Sequence);
            }
            // Fully decode/checksum the packet before mutation. These controls
            // consume neither ISAAC keys nor assembly/order state, so copying
            // pending payloads provides no rollback benefit.
            let packet = wire::decode_session_packet(bytes, None)?;
            if let Some(ack) = packet.ack {
                self.acknowledge(ack)?;
            }
            return Ok(Received {
                logoff: packet.header.flags == wire::LOGOFF,
                messages: Vec::new(),
                duplicate: false,
                ack: self.contiguous,
                echo_request: packet.echo_request,
                naks: packet.naks,
            });
        }
        // A malformed packet must never consume ISAAC state or partially publish a message.
        let mut candidate = self.clone();
        let result = candidate.receive_inner(bytes, now)?;
        *self = candidate;
        Ok(result)
    }
    fn receive_inner(&mut self, bytes: &[u8], now: Instant) -> Result<Received, TransportError> {
        let header = Header::decode(bytes)?;
        if header.recipient != self.recipient || header.iteration != self.iteration {
            return Err(TransportError::Sequence);
        }
        let seq = header.sequence;
        if seq > self.contiguous.saturating_add(WINDOW) {
            return Err(TransportError::Sequence);
        }
        let encrypted = header.flags & wire::ENCRYPTED_CHECKSUM != 0;
        if encrypted && seq < 2 {
            return Err(TransportError::Sequence);
        }
        let key = if encrypted {
            if seq.saturating_add(WINDOW) < self.contiguous {
                return Err(TransportError::Sequence);
            }
            while self.generated < seq {
                self.generated += 1;
                self.keys.insert(self.generated, self.incoming.next_key());
            }
            Some(*self.keys.get(&seq).ok_or(TransportError::Sequence)?)
        } else {
            None
        };
        let packet = wire::decode_session_packet(bytes, key)?;
        // Local emulator policy: application data must use reliable checksum masking.
        if !encrypted && !packet.fragments.is_empty() {
            return Err(TransportError::Sequence);
        }
        let duplicate = encrypted && (seq <= self.contiguous || self.seen.contains(&seq));
        if let Some(ack) = packet.ack {
            self.acknowledge(ack)?;
        }
        let mut messages = Vec::new();
        self.assemblies
            .retain(|_, a| now.saturating_duration_since(a.started) < Duration::from_secs(30));
        if !duplicate {
            for fragment in packet.fragments {
                if let Some(message) = self.add_fragment(fragment, now)? {
                    self.deliver(message, &mut messages)?;
                }
            }
            if encrypted {
                self.seen.insert(seq);
                while self.contiguous < u32::MAX && self.seen.remove(&(self.contiguous + 1)) {
                    self.contiguous += 1;
                }
                let floor = self.contiguous.saturating_sub(WINDOW);
                self.keys.retain(|&id, _| id >= floor);
            }
        }
        Ok(Received {
            logoff: false,
            messages,
            duplicate,
            ack: self.contiguous,
            echo_request: packet.echo_request,
            naks: packet.naks,
        })
    }
    fn deliver(
        &mut self,
        message: Message,
        ready: &mut Vec<Message>,
    ) -> Result<(), TransportError> {
        if !matches!(message.queue, 3 | 7) {
            ready.push(message);
            return Ok(());
        }
        // Proto_UI::SendBuff shares a single ushort counter across queues 3 and 7.
        let stamp = ((message.blob_id >> 32) & 0xffff) as u16;
        if (message.blob_id >> 56) & 0x1f != 1 || stamp == 0 || self.next_ordered > u16::MAX as u32
        {
            return Err(TransportError::Sequence);
        }
        if (stamp as u32) < self.next_ordered {
            return Ok(());
        }
        if stamp as u32 == self.next_ordered {
            ready.push(message);
            self.next_ordered += 1;
            while self.next_ordered <= u16::MAX as u32 {
                let Some(message) = self.ordered.remove(&(self.next_ordered as u16)) else {
                    break;
                };
                self.ordered_bytes -= message.data.len();
                ready.push(message);
                self.next_ordered += 1;
            }
            return Ok(());
        }
        if stamp as u32 >= self.next_ordered + WINDOW || self.ordered.len() >= 16 {
            return Err(TransportError::Capacity);
        }
        if let Some(old) = self.ordered.get(&stamp) {
            if old.blob_id != message.blob_id
                || old.queue != message.queue
                || old.data != message.data
            {
                return Err(TransportError::Fragment);
            }
            return Ok(());
        }
        if self.ordered_bytes + message.data.len() > MAX_MESSAGE * 4 {
            return Err(TransportError::Capacity);
        }
        self.ordered_bytes += message.data.len();
        self.ordered.insert(stamp, message);
        Ok(())
    }
    fn add_fragment(
        &mut self,
        f: Fragment<'_>,
        now: Instant,
    ) -> Result<Option<Message>, TransportError> {
        if self.completed.contains(&f.blob_id) {
            return Ok(None);
        }
        if f.count == 0 || f.count > 256 || f.index >= f.count || f.data.len() > MAX_MESSAGE {
            return Err(TransportError::Fragment);
        }
        if !self.assemblies.contains_key(&f.blob_id) && self.assemblies.len() >= 16 {
            return Err(TransportError::Capacity);
        }
        let a = self
            .assemblies
            .entry(f.blob_id)
            .or_insert_with(|| Assembly {
                queue: f.queue,
                pieces: vec![None; f.count as usize],
                bytes: 0,
                started: now,
            });
        if a.queue != f.queue || a.pieces.len() != f.count as usize {
            return Err(TransportError::Fragment);
        }
        match &a.pieces[f.index as usize] {
            Some(old) if old != f.data => return Err(TransportError::Fragment),
            Some(_) => return Ok(None),
            None => {}
        }
        a.bytes = a
            .bytes
            .checked_add(f.data.len())
            .ok_or(TransportError::Capacity)?;
        if a.bytes > MAX_MESSAGE {
            return Err(TransportError::Capacity);
        }
        a.pieces[f.index as usize] = Some(f.data.to_vec());
        if a.pieces.iter().any(Option::is_none) {
            return Ok(None);
        }
        let a = self.assemblies.remove(&f.blob_id).unwrap();
        let mut data = Vec::with_capacity(a.bytes);
        for piece in a.pieces {
            data.extend(piece.unwrap());
        }
        self.completed.push_back(f.blob_id);
        if self.completed.len() > 512 {
            self.completed.pop_front();
        }
        Ok(Some(Message {
            blob_id: f.blob_id,
            queue: f.queue,
            data,
        }))
    }
    pub fn send_available(&self, bytes: usize) -> Result<bool, TransportError> {
        if bytes == 0 || bytes > MAX_MESSAGE {
            return Err(TransportError::Capacity);
        }
        let count = bytes.div_ceil(FRAGMENT_DATA);
        if self.next_outgoing.checked_add(count as u32).is_none() {
            return Err(TransportError::Sequence);
        }
        Ok(self.pending.len() + count <= MAX_PENDING)
    }
    pub fn send(
        &mut self,
        blob_id: u64,
        queue: u16,
        data: &[u8],
        now: Instant,
    ) -> Result<Vec<Vec<u8>>, TransportError> {
        if data.is_empty() || data.len() > MAX_MESSAGE {
            return Err(TransportError::Capacity);
        }
        let count = data.len().div_ceil(FRAGMENT_DATA);
        if self.pending.len() + count > MAX_PENDING
            || self.next_outgoing.checked_add(count as u32).is_none()
        {
            return Err(TransportError::Capacity);
        }
        let mut output = Vec::with_capacity(count);
        for (index, part) in data.chunks(FRAGMENT_DATA).enumerate() {
            let fragment = Fragment {
                blob_id,
                count: count as u16,
                index: index as u16,
                queue,
                data: part,
            }
            .encode()?;
            let body =
                wire::checksum(&fragment[..16]).wrapping_add(wire::checksum(&fragment[16..]));
            let mut h = Header {
                sequence: self.next_outgoing,
                flags: wire::BLOB_FRAGMENTS | wire::ENCRYPTED_CHECKSUM,
                recipient: self.recipient,
                iteration: self.iteration,
                payload_len: fragment.len() as u16,
                ..Default::default()
            };
            h.checksum = h
                .header_checksum()
                .wrapping_add(body ^ self.outgoing.next_key());
            let mut bytes = h.encode().to_vec();
            bytes.extend(fragment);
            self.pending.insert(
                self.next_outgoing,
                Pending {
                    bytes: Arc::from(bytes.as_slice()),
                    sent: now,
                    retries: 0,
                },
            );
            self.next_outgoing += 1;
            output.push(bytes);
        }
        Ok(output)
    }
    pub fn acknowledge(&mut self, sequence: u32) -> Result<(), TransportError> {
        if sequence >= self.next_outgoing {
            return Err(TransportError::Sequence);
        }
        while self
            .pending
            .first_key_value()
            .is_some_and(|(&seq, _)| seq <= sequence)
        {
            self.pending.pop_first();
        }
        Ok(())
    }
    pub fn has_unacknowledged(&self) -> bool {
        !self.pending.is_empty()
    }
    pub fn retransmit_requested(
        &mut self,
        sequences: &[u32],
        now: Instant,
    ) -> Result<Vec<Vec<u8>>, TransportError> {
        if sequences.len() > 256 {
            return Err(TransportError::Capacity);
        }
        let mut result = Vec::new();
        for seq in sequences.iter().copied().collect::<BTreeSet<_>>() {
            if let Some(p) = self.pending.get_mut(&seq) {
                if p.retries >= 8 {
                    return Err(TransportError::RetryLimit);
                }
                // Rate-limit repeated NAKs; retry packet bytes and checksum remain identical.
                if now.saturating_duration_since(p.sent) >= Duration::from_millis(100) {
                    p.retries += 1;
                    p.sent = now;
                    result.push(p.bytes.to_vec());
                }
            }
        }
        Ok(result)
    }
    pub fn retransmit_due(&mut self, now: Instant) -> Result<Vec<Vec<u8>>, TransportError> {
        let mut result = Vec::new();
        for p in self.pending.values_mut() {
            let delay = Duration::from_millis(500u64 << p.retries.min(3));
            if now.saturating_duration_since(p.sent) >= delay {
                if p.retries >= 8 {
                    return Err(TransportError::RetryLimit);
                }
                p.retries += 1;
                p.sent = now;
                result.push(p.bytes.to_vec());
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn control_fast_path_validates_before_retiring_pending_payloads() {
        let mut transport = Transport::new(7, 9, 1, 1);
        let now = Instant::now();
        for id in 1..=16 {
            transport.send(id, 1, &[42; 1000], now).unwrap();
        }
        let before = transport.pending.len();
        let control = |ack: u32| {
            wire::encode_control_packet(
                Header {
                    flags: wire::ACK,
                    recipient: 1,
                    iteration: 1,
                    ..Default::default()
                },
                &ack.to_le_bytes(),
                None,
            )
            .unwrap()
        };
        assert!(transport.receive(&control(u32::MAX), now).is_err());
        assert_eq!(transport.pending.len(), before);
        let mut corrupt = control(3);
        corrupt[8] ^= 1;
        assert!(transport.receive(&corrupt, now).is_err());
        assert_eq!(transport.pending.len(), before);
        transport.receive(&control(3), now).unwrap();
        assert_eq!(transport.pending.len(), before - 2);
        transport.receive(&control(1), now).unwrap();
        assert_eq!(transport.pending.len(), before - 2);
        assert_eq!(transport.generated, 1);
        assert_eq!(transport.contiguous, 1);
    }
    fn pair() -> (Transport, Transport) {
        (Transport::new(7, 9, 1, 1), Transport::new(9, 7, 1, 1))
    }
    #[test]
    fn reordering_reassembly_duplicates_and_ack() {
        let (mut a, mut b) = pair();
        let now = Instant::now();
        let data = vec![42; 1000];
        let packets = a.send(1, 2, &data, now).unwrap();
        assert_eq!(b.receive(&packets[2], now).unwrap().ack, 1);
        assert!(b.receive(&packets[2], now).unwrap().duplicate);
        assert_eq!(b.receive(&packets[0], now).unwrap().ack, 2);
        let result = b.receive(&packets[1], now).unwrap();
        assert_eq!(result.ack, 4);
        assert_eq!(result.messages[0].data, data);
        a.acknowledge(4).unwrap();
        assert!(
            a.retransmit_due(now + Duration::from_secs(1))
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn bad_checksum_does_not_advance_crypto() {
        let (mut a, mut b) = pair();
        let now = Instant::now();
        let packet = a.send(1, 2, b"hello", now).unwrap().remove(0);
        let mut bad = packet.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(b.receive(&bad, now).is_err());
        assert_eq!(b.receive(&packet, now).unwrap().messages[0].data, b"hello");
    }
    #[test]
    fn huge_gap_and_conflicting_fragments_are_transactional() {
        let (mut a, mut b) = pair();
        let now = Instant::now();
        let packet = a.send(1, 2, b"hello", now).unwrap().remove(0);
        let mut bad = packet.clone();
        bad[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(b.receive(&bad, now).is_err());
        assert_eq!(b.generated, 1);
        assert!(
            b.add_fragment(
                Fragment {
                    blob_id: 2,
                    count: 2,
                    index: 0,
                    queue: 2,
                    data: b"a"
                },
                now
            )
            .unwrap()
            .is_none()
        );
        assert!(
            b.add_fragment(
                Fragment {
                    blob_id: 2,
                    count: 2,
                    index: 0,
                    queue: 2,
                    data: b"b"
                },
                now
            )
            .is_err()
        );
    }
    #[test]
    fn retransmission_reuses_exact_bytes_and_stops() {
        let (mut a, _) = pair();
        let now = Instant::now();
        let packets = a.send(1, 2, b"hello", now).unwrap();
        for i in 1..=8 {
            assert_eq!(
                a.retransmit_due(now + Duration::from_secs(i * 5)).unwrap(),
                packets
            );
        }
        assert!(matches!(
            a.retransmit_due(now + Duration::from_secs(50)),
            Err(TransportError::RetryLimit)
        ));
    }
    #[test]
    fn invalid_fragment_after_ack_preserves_shared_retry_payload_and_crypto() {
        let (mut a, mut b) = pair();
        let now = Instant::now();
        let outgoing = b.send(1, 2, b"must survive rollback", now).unwrap();
        let retry_bytes = b.pending[&2].bytes.clone();
        let fragments: Vec<_> = [2, 3]
            .into_iter()
            .map(|count| {
                Fragment {
                    blob_id: 99,
                    count,
                    index: 0,
                    queue: 4,
                    data: b"x",
                }
                .encode()
                .unwrap()
            })
            .collect();
        let ack = 2u32.to_le_bytes();
        let mut sum = wire::checksum(&ack);
        let mut payload = ack.to_vec();
        for fragment in fragments {
            sum = sum
                .wrapping_add(wire::checksum(&fragment[..16]))
                .wrapping_add(wire::checksum(&fragment[16..]));
            payload.extend(fragment);
        }
        let mut header = Header {
            sequence: 2,
            flags: wire::ACK | wire::BLOB_FRAGMENTS | wire::ENCRYPTED_CHECKSUM,
            recipient: 1,
            iteration: 1,
            payload_len: payload.len() as u16,
            ..Default::default()
        };
        header.checksum = header
            .header_checksum()
            .wrapping_add(sum ^ b.incoming.clone().next_key());
        let mut packet = header.encode().to_vec();
        packet.extend(payload);
        assert!(matches!(
            b.receive(&packet, now),
            Err(TransportError::Fragment)
        ));
        assert_eq!(b.pending.len(), 1);
        assert!(Arc::ptr_eq(&b.pending[&2].bytes, &retry_bytes));
        assert_eq!(b.generated, 1);
        assert!(b.assemblies.is_empty());
        assert_eq!(
            b.retransmit_due(now + Duration::from_secs(1)).unwrap(),
            outgoing
        );
        let packet = a
            .send(2, 4, b"valid after rollback", now)
            .unwrap()
            .remove(0);
        assert_eq!(
            b.receive(&packet, now).unwrap().messages[0].data,
            b"valid after rollback"
        );
    }
    #[test]
    fn logoff_requires_exact_control_and_current_session_identity() {
        let (mut a, mut b) = pair();
        let now = Instant::now();
        let header = Header {
            flags: wire::LOGOFF,
            recipient: 1,
            iteration: 1,
            ..Default::default()
        };
        let logoff = wire::encode_control_packet(header, &[], None).unwrap();
        for length in 0..logoff.len() {
            assert!(b.receive(&logoff[..length], now).is_err());
        }
        for header in [
            Header {
                recipient: 2,
                ..header
            },
            Header {
                iteration: 2,
                ..header
            },
        ] {
            let packet = wire::encode_control_packet(header, &[], None).unwrap();
            assert!(b.receive(&packet, now).is_err());
        }
        let mut corrupt = logoff.clone();
        corrupt[8] ^= 1;
        assert!(b.receive(&corrupt, now).is_err());
        for header in [
            Header {
                sequence: 2,
                ..header
            },
            Header {
                flags: wire::LOGOFF | wire::ACK,
                ..header
            },
            Header {
                flags: wire::LOGOFF | wire::ENCRYPTED_CHECKSUM,
                ..header
            },
        ] {
            assert!(wire::encode_control_packet(header, &[], None).is_err());
            assert!(wire::decode_session_packet(&header.encode(), None).is_err());
        }
        assert!(wire::encode_control_packet(header, &[0], None).is_err());
        let received = b.receive(&logoff, now).unwrap();
        assert!(received.logoff);
        assert!(received.messages.is_empty());
        assert_eq!(received.ack, 1);
        assert_eq!(b.generated, 1);
        // Connection removal belongs to the caller. The transport itself must
        // not advance sequence or ISAAC state on this unsequenced control.
        let packet = a.send(1, 2, b"unchanged state", now).unwrap().remove(0);
        assert_eq!(
            b.receive(&packet, now).unwrap().messages[0].data,
            b"unchanged state"
        );
    }
    #[test]
    fn plain_controls_do_not_consume_reliable_sequence_or_keys() {
        let (mut a, mut b) = pair();
        let now = Instant::now();
        let control = wire::encode_control_packet(
            Header {
                sequence: 1,
                flags: wire::ACK,
                recipient: 1,
                iteration: 1,
                ..Default::default()
            },
            &1u32.to_le_bytes(),
            None,
        )
        .unwrap();
        assert_eq!(b.receive(&control, now).unwrap().ack, 1);
        assert_eq!(b.generated, 1);
        let packet = a.send(1, 2, b"first reliable", now).unwrap().remove(0);
        assert_eq!(Header::decode(&packet).unwrap().sequence, 2);
        assert_eq!(
            b.receive(&packet, now).unwrap().messages[0].data,
            b"first reliable"
        );
    }
    #[test]
    fn send_capacity_failure_leaves_key_stream_unchanged() {
        let (mut a, mut b) = pair();
        let now = Instant::now();
        assert!(a.send(1, 2, &vec![0; MAX_MESSAGE + 1], now).is_err());
        let packet = a.send(2, 2, b"after rejection", now).unwrap().remove(0);
        assert_eq!(
            b.receive(&packet, now).unwrap().messages[0].data,
            b"after rejection"
        );
    }
    #[test]
    fn ordered_messages_share_stamp_across_queues() {
        let (mut a, mut b) = pair();
        let now = Instant::now();
        let first = a
            .send((0x01000001u64 << 32) | 1, 3, b"first", now)
            .unwrap()
            .remove(0);
        let second = a
            .send((0x01000002u64 << 32) | 2, 7, b"second", now)
            .unwrap()
            .remove(0);
        assert!(b.receive(&second, now).unwrap().messages.is_empty());
        let ready = b.receive(&first, now).unwrap().messages;
        assert_eq!(ready.len(), 2);
        assert_eq!(ready[0].data, b"first");
        assert_eq!(ready[1].data, b"second");
    }
    #[test]
    fn plain_application_fragments_rejected_without_state_change() {
        let (_, mut b) = pair();
        let now = Instant::now();
        let packet = wire::encode_fragment_packet(
            Header {
                sequence: 2,
                flags: wire::BLOB_FRAGMENTS,
                recipient: 1,
                iteration: 1,
                ..Default::default()
            },
            Fragment {
                blob_id: 1,
                count: 1,
                index: 0,
                queue: 2,
                data: b"plain",
            },
        )
        .unwrap();
        assert!(b.receive(&packet, now).is_err());
        assert_eq!(b.contiguous, 1);
        assert_eq!(b.generated, 1);
    }
}
