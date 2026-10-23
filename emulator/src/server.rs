//! Single-owner UDP runtime: every session mutation is serialized through one loop.
use crate::{
    state::{Phase, Session},
    store::Store,
    transport::Transport,
    wire::{self, Header},
    world,
};
use std::{
    collections::{BTreeMap, hash_map::RandomState},
    hash::BuildHasher,
    io,
    net::{SocketAddr, UdpSocket},
    path::PathBuf,
    time::{Duration, Instant},
};

pub struct Config {
    pub bind: SocketAddr,
    pub account: String,
    pub data: PathBuf,
    pub run_seconds: Option<u64>,
    pub visual_did: Option<u32>,
    pub game_data: Option<PathBuf>,
}
struct Peer {
    client_version: String,
    session: Session,
    transport: Transport,
    connect_packet: Vec<u8>,
    id: u16,
    iteration: u16,
    blob: u32,
    selection_pending: Option<Vec<u8>>,
    ordered_stamp: u16,
}
#[derive(Default)]
struct Counters {
    received: u64,
    rejected: u64,
    logins: u64,
    connected: u64,
    events: u64,
    movement_reports: u64,
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg)
}
fn other(e: impl std::fmt::Debug) -> io::Error {
    io::Error::other(format!("{e:?}"))
}

impl Peer {
    fn send_world_event(
        &mut self,
        bytes: &[u8],
        ordered: bool,
        now: Instant,
    ) -> io::Result<Vec<Vec<u8>>> {
        self.blob = self
            .blob
            .checked_add(1)
            .ok_or_else(|| invalid("blob sequence exhausted"))?;
        let (namespace, stamp) = if ordered {
            self.ordered_stamp = self
                .ordered_stamp
                .checked_add(1)
                .ok_or_else(|| invalid("world ordering stamp exhausted"))?;
            (0x0100_0000u32, u32::from(self.ordered_stamp))
        } else {
            (0x0300_0000u32, self.blob & 0xffff)
        };
        let id = (u64::from(namespace | stamp) << 32) | u64::from(self.blob);
        self.transport.send(id, 1, bytes, now).map_err(other)
    }
    fn send_event(&mut self, bytes: &[u8], now: Instant) -> io::Result<Vec<Vec<u8>>> {
        if matches!(
            self.session.phase,
            Phase::InWorld { .. } | Phase::Entering { .. }
        ) {
            self.send_world_event(bytes, true, now)
        } else {
            self.send_to_queue(1, bytes, now)
        }
    }
    fn send_to_queue(
        &mut self,
        queue: u16,
        bytes: &[u8],
        now: Instant,
    ) -> io::Result<Vec<Vec<u8>>> {
        self.blob = self
            .blob
            .checked_add(1)
            .filter(|v| *v <= 0xffff)
            .ok_or_else(|| invalid("blob ordering stamp exhausted"))?;
        // 00494890 drops ordinary world events while NOT_IN_WORLD. Pre-world
        // messages need bit61 and unordered type3, with no player CBookKeep.
        let preworld = !matches!(
            self.session.phase,
            Phase::InWorld { .. } | Phase::Entering { .. }
        );
        let namespace = if preworld { 0x2300_0000 } else { 0x0100_0000 };
        // Native client's CEventQueue is receive queue1; queue4 is its logon
        // chat queue and discards CharacterSet (0071fb10).
        let id = (u64::from(namespace | self.blob) << 32) | u64::from(self.blob);
        self.transport.send(id, queue, bytes, now).map_err(other)
    }
    fn control(&self, flag: u32, body: &[u8]) -> io::Result<Vec<u8>> {
        wire::encode_control_packet(
            Header {
                flags: flag,
                recipient: self.id,
                iteration: self.iteration,
                ..Default::default()
            },
            body,
            None,
        )
        .map_err(other)
    }
}

pub fn run(config: Config) -> io::Result<()> {
    if !config.bind.ip().is_loopback() || !config.bind.is_ipv4() {
        return Err(invalid(
            "developer authentication requires an IPv4 loopback bind",
        ));
    }
    if config.bind.port() == 0 || config.bind.port() == u16::MAX {
        return Err(invalid("choose a nonzero base port with a free port+1"));
    }
    if config.account.is_empty()
        || config.account.len() > 32
        || !config
            .account
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(invalid(
            "account must have 1-32 ASCII letters, digits or underscores",
        ));
    }
    let mut store = Store::open(&config.data)?;
    let bootstrap = config
        .game_data
        .as_deref()
        .map(crate::bootstrap::Bootstrap::load)
        .transpose()?;
    let primary = UdpSocket::bind(config.bind)?;
    let mut secondary_addr = config.bind;
    secondary_addr.set_port(config.bind.port() + 1);
    let secondary = UdpSocket::bind(secondary_addr)?;
    primary.set_nonblocking(true)?;
    secondary.set_nonblocking(true)?;
    println!(
        "Listening on {} and {}; local developer account configured (7148 / experimental 7230).",
        config.bind, secondary_addr
    );
    if bootstrap.is_some() {
        println!(
            "Local human default-body world bootstrap enabled. Custom appearance and gameplay simulation are unfinished."
        );
    } else {
        println!(
            "Character selection enabled. Supply --game-data to test the local human world bootstrap."
        );
    }
    let start = Instant::now();
    let mut peers: BTreeMap<SocketAddr, Peer> = BTreeMap::new();
    let random = RandomState::new();
    let mut generation = 0u64;
    let mut counters = Counters::default();
    let mut buffer = [0u8; 65536];
    let mut next_report = 10_000;
    let mut next_stop_check = 0;
    loop {
        let now = Instant::now();
        let now_ms = start.elapsed().as_millis() as u64;
        if config
            .run_seconds
            .is_some_and(|seconds| start.elapsed() >= Duration::from_secs(seconds))
        {
            break;
        }
        if now_ms >= next_stop_check {
            let stop = config.data.join("stop.request");
            if stop.exists() {
                std::fs::remove_file(stop)?;
                break;
            }
            next_stop_check = now_ms + 250;
        }
        for (port_index, socket) in [&primary, &secondary].into_iter().enumerate() {
            for _ in 0..64 {
                let (len, addr) = match socket.recv_from(&mut buffer) {
                    Ok(value) => value,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::ConnectionReset | io::ErrorKind::Interrupted
                        ) =>
                    {
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                counters.received += 1;
                if !addr.ip().is_loopback() || len > 4096 {
                    counters.rejected += 1;
                    continue;
                }
                let bytes = &buffer[..len];
                let h = match Header::decode(bytes) {
                    Ok(h) => h,
                    Err(_) => {
                        counters.rejected += 1;
                        continue;
                    }
                };
                let result = (|| -> io::Result<Vec<Vec<u8>>> {
                    if h.flags == wire::LOGON {
                        if port_index != 0 {
                            return Err(invalid("login on wrong port"));
                        }
                        let (_, payload) = wire::decode_handshake(bytes).map_err(other)?;
                        let login = wire::Login::decode(payload).map_err(other)?;
                        // Type 1 is the recovered native admin client's no-ticket path.
                        // This is an explicit local development identity, not password authentication.
                        if !login.supports_developer_auth(&config.account) {
                            return Err(invalid(&format!(
                                "unsupported login: build={:?} type={:#x} flags={:#x} account_match={} crypto_bytes={} extra_bytes={}",
                                login.version,
                                login.auth_type,
                                login.auth_flags,
                                login.account == config.account,
                                login.crypto_data.len(),
                                login.extra_data.len()
                            )));
                        }
                        if let Some(peer) = peers.get(&addr) {
                            if !peer.session.expired(now_ms) {
                                return Ok(vec![peer.connect_packet.clone()]);
                            }
                        }
                        if peers.len() >= 64 {
                            return Err(invalid("session capacity"));
                        }
                        generation = generation
                            .checked_add(1)
                            .ok_or_else(|| invalid("session generation exhausted"))?;
                        let id = (1..=u16::MAX)
                            .find(|id| !peers.values().any(|p| p.id == *id))
                            .ok_or_else(|| invalid("session IDs exhausted"))?;
                        let iteration = u16::try_from(generation)
                            .map_err(|_| invalid("restart required before iteration reuse"))?;
                        let cookie = random.hash_one((generation, now_ms, addr));
                        let incoming = random.hash_one((cookie, 1u32)) as u32;
                        let outgoing = random.hash_one((cookie, 2u32)) as u32;
                        let connect = wire::Connect {
                            server_time: start.elapsed().as_secs_f64(),
                            cookie,
                            net_id: u32::from(id),
                            outgoing_seed: outgoing,
                            incoming_seed: incoming,
                        };
                        let packet = wire::encode_handshake(
                            Header {
                                flags: wire::CONNECT,
                                recipient: id,
                                iteration,
                                ..Default::default()
                            },
                            &connect.encode(),
                        )
                        .map_err(other)?;
                        peers.insert(
                            addr,
                            Peer {
                                client_version: login.version.clone(),
                                session: Session::new(config.account.clone(), cookie, now_ms),
                                transport: Transport::new(incoming, outgoing, id, iteration),
                                connect_packet: packet.clone(),
                                id,
                                iteration,
                                blob: 0,
                                selection_pending: None,
                                ordered_stamp: 0,
                            },
                        );
                        counters.logins += 1;
                        return Ok(vec![packet]);
                    }
                    let peer = peers
                        .get_mut(&addr)
                        .ok_or_else(|| invalid("unknown peer"))?;
                    if h.recipient != peer.id || h.iteration != peer.iteration {
                        return Err(invalid("stale or wrong session"));
                    }
                    if h.flags == wire::CONNECT_ACK {
                        if port_index != 1 {
                            return Err(invalid("connection acknowledgement on wrong port"));
                        }
                        let (_, payload) = wire::decode_handshake(bytes).map_err(other)?;
                        let cookie = wire::decode_connect_ack(payload).map_err(other)?;
                        let first = peer.session.phase == Phase::Connecting;
                        peer.session.connect_ack(cookie, now_ms).map_err(other)?;
                        if !first {
                            return Ok(vec![peer.control(
                                wire::TIME_SYNC,
                                &start.elapsed().as_secs_f64().to_le_bytes(),
                            )?]);
                        }
                        counters.connected += 1;
                        let characters: Vec<_> = store
                            .characters
                            .values()
                            .filter(|c| c.account == config.account)
                            .collect();
                        let list = if let Some(profile) = &bootstrap {
                            let selection: Vec<_> = characters
                                .iter()
                                .filter_map(|c| {
                                    c.creation
                                        .as_ref()
                                        .filter(|r| profile.supports(r))
                                        .map(|r| profile.selection(c.id, r))
                                })
                                .collect();
                            world::character_set(config.account.as_bytes(), 5, &selection)
                                .map_err(other)?
                        } else if let Some(did) = config.visual_did {
                            let selection: Vec<_> = characters
                                .iter()
                                .map(|c| world::SelectionCharacter {
                                    id: c.id,
                                    name: c.name.clone(),
                                    seconds_greyed_out: 0,
                                    parent_visual_did: did,
                                })
                                .collect();
                            world::character_set(config.account.as_bytes(), 5, &selection)
                                .map_err(other)?
                        } else {
                            world::empty_character_set(config.account.as_bytes(), 5)
                                .map_err(other)?
                        };
                        // Local asset mode: finish the zero-download DAT phase before
                        // selection can invoke CharacterGenSystem. OnEndDDD initializes
                        // the native VM; skipping it dereferences a null resolver.
                        peer.selection_pending = Some(list);
                        return peer.send_to_queue(5, &0x1003bu32.to_le_bytes(), now);
                    }
                    if peer.session.phase == Phase::Connecting || peer.session.expired(now_ms) {
                        return Err(invalid("session not connected"));
                    }
                    let received = peer.transport.receive(bytes, now).map_err(other)?;
                    peer.session.touch(now_ms);
                    let mut replies = Vec::new();
                    // Wait for delivery of the DAT notification so a lost first
                    // packet cannot let queue1 race ahead of queue5 initialization.
                    if !peer.transport.has_unacknowledged() {
                        if let Some(list) = peer.selection_pending.take() {
                            replies.extend(peer.send_event(&list, now)?);
                        }
                    }
                    if h.sequence != 0 {
                        replies.push(peer.control(wire::ACK, &received.ack.to_le_bytes())?);
                    }
                    if let Some(echo) = received.echo_request {
                        let mut body = echo.to_le_bytes().to_vec();
                        body.extend(0.0f32.to_le_bytes());
                        replies.push(peer.control(wire::ECHO_RESPONSE, &body)?);
                    }
                    replies.extend(
                        peer.transport
                            .retransmit_requested(&received.naks, now)
                            .map_err(other)?,
                    );
                    for message in received.messages {
                        if !matches!(message.queue, 3 | 4) {
                            counters.rejected += 1;
                            continue;
                        }
                        counters.events += 1;
                        let event = match world::decode_client_event(&message.data) {
                            Ok(event) => event,
                            Err(_) => {
                                counters.rejected += 1;
                                continue;
                            }
                        };
                        let response = match event {
                            world::ClientEvent::Position(position) => {
                                peer.session.report_position(position).map_err(other)?;
                                counters.movement_reports += 1;
                                if counters.movement_reports == 1
                                    || counters.movement_reports % 100 == 0
                                {
                                    let report =
                                        peer.session.last_reported_position.as_ref().unwrap();
                                    eprintln!(
                                        "client movement telemetry: cell={:#010x} xyz={:?} reports={}",
                                        report.cell,
                                        report.offset_bits.map(f32::from_bits),
                                        counters.movement_reports
                                    );
                                }
                                continue;
                            }
                            world::ClientEvent::CreateCharacter { account, request } => {
                                if account != peer.session.account.as_bytes()
                                    || peer.session.phase != Phase::CharacterSelection
                                {
                                    return Err(invalid(
                                        "character creation outside owned selection session",
                                    ));
                                }
                                if let Some(profile) =
                                    bootstrap.as_ref().filter(|p| p.supports(&request))
                                {
                                    if store
                                        .characters
                                        .values()
                                        .any(|c| c.name.eq_ignore_ascii_case(&request.name))
                                    {
                                        world::reject_character_creation(
                                            world::CreationRejection::NameInUse,
                                        )
                                    } else if store
                                        .characters
                                        .values()
                                        .filter(|c| c.account == peer.session.account)
                                        .count()
                                        >= 5
                                    {
                                        world::reject_character_creation(
                                            world::CreationRejection::NoMoreSlots,
                                        )
                                    } else {
                                        match store.create_native(&peer.session.account, &request) {
                                            Ok(character) => {
                                                eprintln!(
                                                    "created character id={} with persisted native choices; default human appearance profile",
                                                    character.id
                                                );
                                                world::confirm_character_creation(
                                                    &profile.selection(character.id, &request),
                                                )
                                                .map_err(other)?
                                            }
                                            Err(error) => {
                                                eprintln!(
                                                    "character persistence rejected: {error}"
                                                );
                                                world::reject_character_creation(
                                                    world::CreationRejection::Corrupt,
                                                )
                                            }
                                        }
                                    }
                                } else {
                                    eprintln!(
                                        "unsupported creation profile: entity={:#x} race={} sex={:#x} start={:#x}",
                                        request.entity_did,
                                        request.race,
                                        request.sex,
                                        request.start_area
                                    );
                                    world::reject_character_creation(
                                        world::CreationRejection::Undefined,
                                    )
                                }
                            }
                            world::ClientEvent::EnterWorld {
                                character_id,
                                account,
                            } => {
                                let owned = account == peer.session.account.as_bytes()
                                    && store
                                        .characters
                                        .get(&character_id)
                                        .is_some_and(|c| c.account == peer.session.account);
                                let error = if owned {
                                    if let Some(profile) = &bootstrap {
                                        if !profile.supports_client(&peer.client_version) {
                                            eprintln!(
                                                "world entry refused: client {} cannot load this DAT bootstrap's GameplayOptionsProfile; use a matching client/data build",
                                                peer.client_version
                                            );
                                            replies.extend(peer.send_event(
                                                &world::character_error(
                                                    world::CharacterError::CouldNotPlaceCharacter,
                                                ),
                                                now,
                                            )?);
                                            continue;
                                        }
                                        if let Some(request) = store
                                            .characters
                                            .get(&character_id)
                                            .and_then(|c| c.creation.as_ref())
                                            .filter(|r| profile.supports(r))
                                        {
                                            let events = profile
                                                .events(character_id, request)
                                                .map_err(other)?;
                                            peer.session
                                                .request_enter(character_id, &config.account, true)
                                                .map_err(other)?;
                                            peer.ordered_stamp = 0;
                                            for (index, event) in events.iter().enumerate() {
                                                replies.extend(peer.send_world_event(
                                                    event,
                                                    index == 2,
                                                    now,
                                                )?);
                                            }
                                            eprintln!(
                                                "world bootstrap sent for character {character_id}"
                                            );
                                            continue;
                                        }
                                    }
                                    world::CharacterError::CouldNotPlaceCharacter
                                } else {
                                    world::CharacterError::CharacterNotOwned
                                };
                                world::character_error(error)
                            }
                            world::ClientEvent::LogOffCharacter { character_id } => {
                                peer.session.can_leave(character_id).map_err(other)?;
                                // Queue the exit before discarding the world ordering context.
                                replies.extend(peer.send_event(&world::char_exit_game(), now)?);
                                peer.session.leave_world(character_id).map_err(other)?;
                                eprintln!("character {character_id} returned to selection");
                                continue;
                            }
                            world::ClientEvent::SceneRenderingComplete => {
                                peer.session.scene_complete().map_err(other)?;
                                eprintln!("native client reported SceneRenderingComplete");
                                continue;
                            }
                        };
                        replies.extend(peer.send_event(&response, now)?);
                    }
                    Ok(replies)
                })();
                match result {
                    Ok(replies) => {
                        for reply in replies {
                            primary.send_to(&reply, addr)?;
                        }
                    }
                    Err(error) => {
                        counters.rejected += 1;
                        if counters.rejected <= 12 {
                            eprintln!(
                                "packet rejected: flags={:#x} sequence={} bytes={} reason={error}",
                                h.flags, h.sequence, len
                            );
                        }
                    }
                }
            }
        }
        let mut remove = Vec::new();
        for (addr, peer) in &mut peers {
            if peer.session.expired(now_ms) {
                remove.push(*addr);
                continue;
            }
            match peer.transport.retransmit_due(now) {
                Ok(packets) => {
                    for packet in packets {
                        primary.send_to(&packet, addr)?;
                    }
                }
                Err(_) => remove.push(*addr),
            }
        }
        for addr in remove {
            peers.remove(&addr);
        }
        if now_ms >= next_report {
            println!(
                "sessions={} received={} rejected={} logins={} connected={} events={} movement_reports={}",
                peers.len(),
                counters.received,
                counters.rejected,
                counters.logins,
                counters.connected,
                counters.events,
                counters.movement_reports
            );
            next_report = now_ms + 10_000;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    println!(
        "Stopped: received={} rejected={} logins={} connected={} events={}",
        counters.received, counters.rejected, counters.logins, counters.connected, counters.events
    );
    Ok(())
}
