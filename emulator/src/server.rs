//! Single-owner UDP runtime: every session mutation is serialized through one loop.
use crate::{
    movement::{Motion, Placement},
    script,
    state::{Phase, Session},
    store::Store,
    transport::Transport,
    wire::{self, Header},
    world,
};
use std::{
    collections::{BTreeMap, BTreeSet, hash_map::RandomState},
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
    pub initialize_ui: bool,
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
    motion: Option<Motion>,
    target: Option<u64>,
    combat: bool,
    last_chat_ms: Option<u64>,
    unhandled_scripts: BTreeSet<u32>,
}
#[derive(Default)]
struct Counters {
    received: u64,
    rejected: u64,
    logins: u64,
    connected: u64,
    events: u64,
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
        let blob = self
            .blob
            .checked_add(1)
            .ok_or_else(|| invalid("blob sequence exhausted"))?;
        let ordered_stamp = if ordered {
            self.ordered_stamp
                .checked_add(1)
                .ok_or_else(|| invalid("world ordering stamp exhausted"))?
        } else {
            self.ordered_stamp
        };
        let (namespace, stamp) = if ordered {
            (0x0100_0000u32, u32::from(ordered_stamp))
        } else {
            (0x0300_0000u32, blob & 0xffff)
        };
        let id = (u64::from(namespace | stamp) << 32) | u64::from(blob);
        let packets = self.transport.send(id, 1, bytes, now).map_err(other)?;
        self.blob = blob;
        self.ordered_stamp = ordered_stamp;
        Ok(packets)
    }
    fn send_event(&mut self, bytes: &[u8], now: Instant) -> io::Result<Vec<Vec<u8>>> {
        if matches!(
            self.session.phase,
            Phase::Entering { .. } | Phase::InWorld { .. }
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
        let blob = self
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
        let id = (u64::from(namespace | blob) << 32) | u64::from(blob);
        let packets = self.transport.send(id, queue, bytes, now).map_err(other)?;
        self.blob = blob;
        Ok(packets)
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

fn selection_list(
    store: &Store,
    account: &str,
    profile: Option<&crate::bootstrap::Bootstrap>,
    visual: Option<u32>,
) -> io::Result<Vec<u8>> {
    if profile.is_none() && visual.is_none() {
        return world::empty_character_set(account.as_bytes(), 5).map_err(other);
    }
    let selection: Vec<_> = store
        .characters
        .values()
        .filter(|c| c.account == account && !c.deleted)
        .filter_map(|c| {
            if let Some(profile) = profile {
                c.creation
                    .as_ref()
                    .filter(|r| profile.supports(r))
                    .map(|r| profile.selection(c.id, r))
            } else {
                Some(world::SelectionCharacter {
                    id: c.id,
                    name: c.name.clone(),
                    seconds_greyed_out: 0,
                    parent_visual_did: visual.unwrap(),
                })
            }
        })
        .collect();
    world::character_set(account.as_bytes(), 5, &selection).map_err(other)
}

fn deliver_scene(
    deliveries: Vec<crate::scene::Delivery>,
    active: &BTreeMap<u64, SocketAddr>,
    peers: &mut BTreeMap<SocketAddr, Peer>,
    socket: &UdpSocket,
    now: Instant,
    failed: &mut BTreeSet<SocketAddr>,
) -> io::Result<()> {
    for delivery in deliveries {
        let Some(addr) = active.get(&delivery.recipient) else {
            continue;
        };
        let Some(peer) = peers.get_mut(addr) else {
            continue;
        };
        if failed.contains(addr) {
            continue;
        }
        match peer.send_world_event(&delivery.data, false, now) {
            Ok(packets) => {
                for packet in packets {
                    socket.send_to(&packet, addr)?;
                }
            }
            Err(_) => {
                failed.insert(*addr);
            }
        }
    }
    Ok(())
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
    let mut bootstrap = config
        .game_data
        .as_deref()
        .map(crate::bootstrap::Bootstrap::load)
        .transpose()?;
    if config.initialize_ui
        && !bootstrap
            .as_ref()
            .is_some_and(|profile| profile.supports_client("7148"))
    {
        return Err(invalid(
            "experimental UI initialization requires converted build-7148 game data",
        ));
    }
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
    let mut active_characters = BTreeMap::new();
    let mut scene = crate::scene::Scene::default();
    let mut failed_peers = BTreeSet::new();
    let random = RandomState::new();
    let mut generation = 0u64;
    let mut counters = Counters::default();
    let mut buffer = [0u8; 65536];
    let mut next_report = 10_000;
    let mut next_stop_check = 0;
    let mut next_checkpoint = 5000;
    let mut diagnosed_events = BTreeSet::new();
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
                if failed_peers.contains(&addr) {
                    continue;
                }
                let mut deliveries = Vec::new();
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
                        let previous = peers.insert(
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
                                motion: None,
                                target: None,
                                combat: false,
                                last_chat_ms: None,
                                unhandled_scripts: BTreeSet::new(),
                            },
                        );
                        if let Some(previous) = previous {
                            if let Phase::Entering { character } | Phase::InWorld { character } =
                                previous.session.phase
                            {
                                active_characters.remove(&character);
                                deliveries.extend(scene.remove(character));
                            }
                        }
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
                            .filter(|c| c.account == config.account && !c.deleted)
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
                    if received.logoff {
                        // Validation includes checksum and current session identity.
                        // The end-of-loop removal releases presence and ownership.
                        store.flush_positions()?;
                        failed_peers.insert(addr);
                        return Ok(Vec::new());
                    }
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
                            Err(error) => {
                                counters.rejected += 1;
                                let opcode = message
                                    .data
                                    .get(..4)
                                    .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
                                    .unwrap_or(0);
                                if diagnosed_events.len() < 32 && diagnosed_events.insert(opcode) {
                                    eprintln!(
                                        "application decode rejected: opcode={opcode:#x} queue={} bytes={} reason={error}",
                                        message.queue,
                                        message.data.len()
                                    );
                                }
                                continue;
                            }
                        };
                        let application = (|| -> io::Result<Vec<Vec<u8>>> {
                            let mut replies = Vec::new();
                            let response = match event {
                                world::ClientEvent::ScriptCall {
                                    function_id,
                                    arguments,
                                } => {
                                    let character = match peer.session.phase {
                                        Phase::InWorld { character } => character,
                                        _ => {
                                            return Err(invalid(
                                                "gameplay request outside ready world session",
                                            ));
                                        }
                                    };
                                    let request = match script::decode(function_id, &arguments) {
                                        Ok(request) => request,
                                        Err(error) => {
                                            if peer.unhandled_scripts.len() < 16
                                                && peer.unhandled_scripts.insert(function_id)
                                            {
                                                eprintln!(
                                                    "script request rejected: function={function_id:#010x} bytes={} reason={error}",
                                                    arguments.len()
                                                );
                                            }
                                            return Err(other(error));
                                        }
                                    };
                                    match request {
                                        script::Request::Target(target) => {
                                            if target.is_some_and(|target| {
                                                !scene.can_observe(character, target)
                                            }) {
                                                return Err(invalid("target is not visible"));
                                            }
                                            peer.target = target;
                                        }
                                        script::Request::Combat(enabled) => {
                                            let response = script::private_call(
                                                script::COMBAT_STATE,
                                                &script::integer(u32::from(enabled)),
                                            )
                                            .map_err(other)?;
                                            replies.extend(peer.send_event(&response, now)?);
                                            peer.combat = enabled;
                                        }
                                        request @ (script::Request::Say(_)
                                        | script::Request::Tell { .. }) => {
                                            if peer.last_chat_ms.is_some_and(|last| {
                                                now_ms.saturating_sub(last) < 500
                                            }) {
                                                return Err(invalid("chat rate limit"));
                                            }
                                            let name = &store.characters[&character].name;
                                            match request {
                                                script::Request::Say(text) => {
                                                    let packet = script::chat(
                                                        &format!("{name}: {text}"),
                                                        0x10,
                                                    )
                                                    .map_err(other)?;
                                                    deliveries.extend(
                                                        scene
                                                            .broadcast(character, packet)
                                                            .map_err(other)?,
                                                    );
                                                }
                                                script::Request::Tell { recipient, text } => {
                                                    let recipient_id = store
                                                        .characters
                                                        .values()
                                                        .find(|c| {
                                                            !c.deleted
                                                                && c.name.eq_ignore_ascii_case(
                                                                    &recipient,
                                                                )
                                                                && active_characters
                                                                    .contains_key(&c.id)
                                                                && scene.can_observe(c.id, c.id)
                                                        })
                                                        .map(|c| c.id);
                                                    if let Some(recipient_id) = recipient_id {
                                                        let packet = script::chat(
                                                            &format!("{name} tells you: {text}"),
                                                            0x20,
                                                        )
                                                        .map_err(other)?;
                                                        deliveries.push(crate::scene::Delivery {
                                                            recipient: recipient_id,
                                                            data: packet.into(),
                                                        });
                                                        replies.extend(peer.send_event(&script::chat(&format!("You tell {recipient}: {text}"), 0x20).map_err(other)?, now)?);
                                                    } else {
                                                        replies.extend(
                                                            peer.send_event(
                                                                &script::chat(
                                                                    "That player is not online.",
                                                                    1,
                                                                )
                                                                .map_err(other)?,
                                                                now,
                                                            )?,
                                                        );
                                                    }
                                                }
                                                _ => unreachable!(),
                                            }
                                            peer.last_chat_ms = Some(now_ms);
                                        }
                                    }
                                    return Ok(replies);
                                }
                                world::ClientEvent::DeleteCharacter {
                                    account,
                                    character_id,
                                } => {
                                    if peer.session.phase != Phase::CharacterSelection
                                        || account != peer.session.account.as_bytes()
                                        || active_characters.contains_key(&character_id)
                                    {
                                        return Err(invalid(
                                            "deletion outside owned inactive selection",
                                        ));
                                    }
                                    store.delete(&peer.session.account, character_id)?;
                                    world::character_deleted(character_id)
                                }
                                world::ClientEvent::Position(update) => {
                                    let character = match peer.session.phase {
                                        Phase::Entering { character }
                                        | Phase::InWorld { character } => character,
                                        _ => return Err(invalid("movement outside world session")),
                                    };
                                    let motion = peer
                                        .motion
                                        .as_mut()
                                        .ok_or_else(|| invalid("missing world placement"))?;
                                    if update.placement.cell != motion.placement.cell
                                        && !bootstrap
                                            .as_mut()
                                            .is_some_and(|p| p.contains_cell(update.placement.cell))
                                    {
                                        return Err(invalid(
                                            "movement references unavailable cell",
                                        ));
                                    }
                                    motion.accept(update).map_err(other)?;
                                    store.set_placement(character, motion.placement)?;
                                    deliveries.extend(
                                        scene
                                            .update(character, motion.position_stamp, &update)
                                            .map_err(other)?,
                                    );
                                    return Ok(replies);
                                }
                                world::ClientEvent::LookAt { target } => {
                                    let character = match peer.session.phase {
                                        Phase::InWorld { character } => character,
                                        _ => {
                                            return Err(invalid(
                                                "target look outside ready world session",
                                            ));
                                        }
                                    };
                                    deliveries
                                        .extend(scene.look_at(character, target).map_err(other)?);
                                    return Ok(replies);
                                }
                                world::ClientEvent::Look { yaw, pitch } => {
                                    if peer.motion.is_none() {
                                        return Err(invalid("look outside world session"));
                                    }
                                    let character = match peer.session.phase {
                                        Phase::Entering { character }
                                        | Phase::InWorld { character } => character,
                                        _ => return Err(invalid("look outside world session")),
                                    };
                                    deliveries
                                        .extend(scene.look(character, yaw, pitch).map_err(other)?);
                                    return Ok(replies);
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
                                            .filter(|c| {
                                                c.account == peer.session.account && !c.deleted
                                            })
                                            .count()
                                            >= 5
                                        {
                                            world::reject_character_creation(
                                                world::CreationRejection::NoMoreSlots,
                                            )
                                        } else {
                                            match store
                                                .create_native(&peer.session.account, &request)
                                            {
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
                                        && store.characters.get(&character_id).is_some_and(|c| {
                                            c.account == peer.session.account && !c.deleted
                                        });
                                    let error = if owned {
                                        if peer.session.phase != Phase::CharacterSelection {
                                            return Err(invalid("world entry outside selection"));
                                        }
                                        if active_characters.contains_key(&character_id) {
                                            return Err(invalid("character already active"));
                                        }
                                        if let Some(profile) = &mut bootstrap {
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
                                                return Ok(replies);
                                            }
                                            if let Some(request) = store
                                                .characters
                                                .get(&character_id)
                                                .and_then(|c| c.creation.as_ref())
                                                .filter(|r| profile.supports(r))
                                                .cloned()
                                            {
                                                let placement = store
                                                    .placement(character_id)
                                                    .unwrap_or(Placement::HUMAN_START);
                                                if !profile.contains_cell(placement.cell) {
                                                    return Err(invalid(
                                                        "saved placement references unavailable cell",
                                                    ));
                                                }
                                                let instance = store.begin_instance(
                                                    &peer.session.account,
                                                    character_id,
                                                )?;
                                                let events = profile
                                                    .events_at(
                                                        character_id,
                                                        &request,
                                                        placement,
                                                        instance,
                                                    )
                                                    .map_err(other)?;
                                                peer.session
                                                    .request_enter(
                                                        character_id,
                                                        &config.account,
                                                        true,
                                                    )
                                                    .map_err(other)?;
                                                peer.ordered_stamp = 0;
                                                peer.target = None;
                                                peer.combat = false;
                                                peer.motion =
                                                    Some(Motion::new(placement, instance));
                                                active_characters.insert(character_id, addr);
                                                scene
                                                    .enter_hidden(
                                                        crate::scene::Body::new(
                                                            character_id,
                                                            placement,
                                                            events[1].clone(),
                                                        )
                                                        .map_err(other)?,
                                                    )
                                                    .map_err(other)?;
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
                                                return Ok(replies);
                                            }
                                        }
                                        world::CharacterError::CouldNotPlaceCharacter
                                    } else {
                                        world::CharacterError::CharacterNotOwned
                                    };
                                    world::character_error(error)
                                }
                                world::ClientEvent::LogOffCharacter { character_id } => {
                                    if !matches!(peer.session.phase, Phase::Entering { character } | Phase::InWorld { character } if character == character_id)
                                    {
                                        return Err(invalid(
                                            "logout does not own the active character",
                                        ));
                                    }
                                    store.flush_positions()?;
                                    replies.extend(peer.send_event(&world::char_exit_game(), now)?);
                                    peer.session.leave_world(character_id).map_err(other)?;
                                    peer.motion = None;
                                    peer.target = None;
                                    peer.combat = false;
                                    active_characters.remove(&character_id);
                                    deliveries.extend(scene.remove(character_id));
                                    let list = selection_list(
                                        &store,
                                        &peer.session.account,
                                        bootstrap.as_ref(),
                                        config.visual_did,
                                    )?;
                                    replies.extend(peer.send_event(&list, now)?);
                                    return Ok(replies);
                                }
                                world::ClientEvent::SceneRenderingComplete => {
                                    let first =
                                        matches!(peer.session.phase, Phase::Entering { .. });
                                    if first && config.initialize_ui {
                                        let packet = crate::ui::empty_player().map_err(other)?;
                                        replies.extend(peer.send_event(&packet, now)?);
                                    }
                                    peer.session.scene_complete().map_err(other)?;
                                    if let Phase::InWorld { character } = peer.session.phase {
                                        deliveries
                                            .extend(scene.activate(character).map_err(other)?);
                                    }
                                    eprintln!("client reported SceneRenderingComplete");
                                    return Ok(replies);
                                }
                            };
                            replies.extend(peer.send_event(&response, now)?);
                            Ok(replies)
                        })();
                        match application {
                            Ok(events) => replies.extend(events),
                            Err(error) => {
                                counters.rejected += 1;
                                if counters.rejected <= 12 {
                                    eprintln!("application event rejected: {error}");
                                }
                            }
                        }
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
                deliver_scene(
                    deliveries,
                    &active_characters,
                    &mut peers,
                    &primary,
                    now,
                    &mut failed_peers,
                )?;
            }
        }
        let mut remove = std::mem::take(&mut failed_peers);
        for (addr, peer) in &mut peers {
            if peer.session.expired(now_ms) {
                remove.insert(*addr);
                continue;
            }
            match peer.transport.retransmit_due(now) {
                Ok(packets) => {
                    for packet in packets {
                        primary.send_to(&packet, addr)?;
                    }
                }
                Err(_) => {
                    remove.insert(*addr);
                }
            }
        }
        let mut departures = Vec::new();
        for addr in remove {
            if let Some(peer) = peers.remove(&addr) {
                if let Phase::Entering { character } | Phase::InWorld { character } =
                    peer.session.phase
                {
                    active_characters.remove(&character);
                    departures.extend(scene.remove(character));
                }
            }
        }
        deliver_scene(
            departures,
            &active_characters,
            &mut peers,
            &primary,
            now,
            &mut failed_peers,
        )?;
        if now_ms >= next_checkpoint {
            store.flush_positions()?;
            next_checkpoint = now_ms + 5000;
        }
        if now_ms >= next_report {
            println!(
                "sessions={} received={} rejected={} logins={} connected={} events={}",
                peers.len(),
                counters.received,
                counters.rejected,
                counters.logins,
                counters.connected,
                counters.events
            );
            next_report = now_ms + 10_000;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    store.flush_positions()?;
    println!(
        "Stopped: received={} rejected={} logins={} connected={} events={}",
        counters.received, counters.rejected, counters.logins, counters.connected, counters.events
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> Peer {
        Peer {
            client_version: "7148".into(),
            session: Session::new("local".into(), 1, 0),
            transport: Transport::new(1, 2, 1, 1),
            connect_packet: Vec::new(),
            id: 1,
            iteration: 1,
            blob: 0,
            selection_pending: None,
            ordered_stamp: 0,
            motion: None,
            target: None,
            combat: false,
            last_chat_ms: None,
            unhandled_scripts: BTreeSet::new(),
        }
    }

    #[test]
    fn rejected_sends_do_not_consume_blob_or_world_order_stamps() {
        let now = Instant::now();
        let mut peer = peer();
        for _ in 0..256 {
            peer.send_world_event(b"fill", false, now).unwrap();
        }
        assert_eq!((peer.blob, peer.ordered_stamp), (256, 0));
        assert!(peer.send_world_event(b"ordered", true, now).is_err());
        assert!(peer.send_to_queue(1, b"selection", now).is_err());
        assert_eq!((peer.blob, peer.ordered_stamp), (256, 0));
        peer.transport.acknowledge(257).unwrap();
        let packets = peer.send_world_event(b"ordered", true, now).unwrap();
        let fragment = wire::Fragment::decode(&packets[0][wire::HEADER_LEN..]).unwrap();
        assert_eq!(fragment.blob_id, (0x0100_0001u64 << 32) | 257);
        assert_eq!(fragment.data, b"ordered");
        assert_eq!((peer.blob, peer.ordered_stamp), (257, 1));
        peer.send_to_queue(1, b"selection", now).unwrap();
        assert_eq!((peer.blob, peer.ordered_stamp), (258, 1));

        // Invalid payloads and local stamp exhaustion must also leave both
        // counters intact, including after a successful application reply.
        assert!(peer.send_world_event(b"", true, now).is_err());
        assert!(peer.send_to_queue(1, b"", now).is_err());
        assert_eq!((peer.blob, peer.ordered_stamp), (258, 1));
        peer.ordered_stamp = u16::MAX;
        assert!(peer.send_world_event(b"exhausted", true, now).is_err());
        assert_eq!((peer.blob, peer.ordered_stamp), (258, u16::MAX));
        peer.blob = u16::MAX.into();
        assert!(peer.send_to_queue(1, b"exhausted", now).is_err());
        assert_eq!(peer.blob, u32::from(u16::MAX));
    }
}
