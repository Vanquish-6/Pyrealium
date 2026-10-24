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
    collections::{BTreeMap, BTreeSet, VecDeque, hash_map::RandomState},
    hash::BuildHasher,
    io,
    net::{SocketAddr, UdpSocket},
    path::PathBuf,
    sync::Arc,
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
    pub training_kit: bool,
    pub lifestones: bool,
    pub portals: bool,
    pub content: Option<PathBuf>,
    pub encounters: Option<PathBuf>,
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
    emotes: crate::emotes::State,
    last_chat_ms: Option<u64>,
    last_inventory_ms: Option<u64>,
    last_examine_ms: Option<u64>,
    last_world_use_ms: Option<u64>,
    merchant_open_ms: Option<u64>,
    native_shop: Option<crate::merchants::Shop>,
    unhandled_scripts: BTreeSet<u32>,
    scene_outbox: VecDeque<(Arc<[u8]>, bool)>,
    scene_bytes: usize,
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
    fn clear_scene_outbox(&mut self) {
        self.scene_outbox.clear();
        self.scene_bytes = 0;
    }
    fn queue_scene(&mut self, data: Arc<[u8]>) -> io::Result<()> {
        let ordered = Self::scene_ordered(&data);
        self.queue_scene_as(data, ordered)
    }
    fn scene_ordered(data: &[u8]) -> bool {
        data.get(..4).is_some_and(|opcode| {
            matches!(
                u32::from_le_bytes(opcode.try_into().unwrap()),
                0x10058 | 0x100c2 | 0x10069
                    ..=0x10076 | 0x100d4 | 0x100d5 | 0x10066 | crate::emotes::EVENT
            )
        })
    }
    fn queue_scene_as(&mut self, data: Arc<[u8]>, ordered: bool) -> io::Result<()> {
        if data.len() < 4
            || data.len() > 65536
            || self.scene_outbox.len() >= 1024
            || self.scene_bytes.saturating_add(data.len()) > 1024 * 1024
        {
            return Err(invalid("scene outbox limit"));
        }
        self.scene_bytes += data.len();
        self.scene_outbox.push_back((data, ordered));
        Ok(())
    }
    fn flush_scene(
        &mut self,
        socket: &UdpSocket,
        addr: &SocketAddr,
        now: Instant,
    ) -> io::Result<()> {
        if !matches!(
            self.session.phase,
            Phase::Entering { .. } | Phase::InWorld { .. }
        ) {
            self.clear_scene_outbox();
            return Ok(());
        }
        for _ in 0..32 {
            let Some((data, ordered)) = self.scene_outbox.front().cloned() else {
                break;
            };
            if !self.transport.send_available(data.len()).map_err(other)? {
                break;
            }
            let packets = self.send_world_event(&data, ordered, now)?;
            self.scene_outbox.pop_front();
            self.scene_bytes -= data.len();
            for packet in packets {
                socket.send_to(&packet, addr)?;
            }
        }
        Ok(())
    }
    /// Admit a dependent group completely before changing the outgoing queue.
    /// Fragment-window pressure is handled by flush_scene, without cloning
    /// transport/ISAAC history or requiring every event to fit in one send window.
    fn validate_scene_batch(&self, events: &[Vec<u8>]) -> io::Result<usize> {
        let mut total = self.scene_bytes;
        if self.scene_outbox.len().saturating_add(events.len()) > 1024 {
            return Err(invalid("scene batch event limit"));
        }
        for event in events {
            if !(4..=65536).contains(&event.len()) {
                return Err(invalid("scene batch message limit"));
            }
            total = total
                .checked_add(event.len())
                .filter(|size| *size <= 1024 * 1024)
                .ok_or_else(|| invalid("scene batch byte limit"))?;
        }
        Ok(total)
    }
    fn queue_event_batch(&mut self, events: Vec<Vec<u8>>) -> io::Result<()> {
        let total = self.validate_scene_batch(&events)?;
        self.scene_outbox.extend(events.into_iter().map(|data| {
            let ordered = Self::scene_ordered(&data);
            (Arc::from(data), ordered)
        }));
        self.scene_bytes = total;
        Ok(())
    }
    fn queue_ordered_batch(&mut self, events: Vec<Vec<u8>>) -> io::Result<()> {
        let count = events.len();
        self.queue_event_batch(events)?;
        for (_, ordered) in self.scene_outbox.iter_mut().rev().take(count) {
            *ordered = true;
        }
        Ok(())
    }
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

struct ForcedMove {
    motion: Motion,
    update: crate::movement::ClientPosition,
    wire: Vec<u8>,
}
fn prepare_forced_move(
    peer: &Peer,
    character: u64,
    destination: Placement,
    scene: &mut crate::scene::Scene,
    bootstrap: &mut Option<crate::bootstrap::Bootstrap>,
) -> io::Result<ForcedMove> {
    if scene.player_placement(character).is_none()
        || !bootstrap
            .as_mut()
            .is_some_and(|profile| profile.contains_cell(destination.cell))
    {
        return Err(invalid("Saved destination is not available in this world."));
    }
    scene.prepare_cell(destination.cell)?;
    let (motion, update) = peer
        .motion
        .as_ref()
        .ok_or_else(|| invalid("Missing player motion state."))?
        .forced(destination)
        .map_err(other)?;
    let wire = crate::movement::server_position(
        character,
        motion.instance,
        motion.position_stamp,
        &update,
    )
    .map_err(other)?;
    Ok(ForcedMove {
        motion,
        update,
        wire,
    })
}
fn use_portal(
    peer: &mut Peer,
    character: u64,
    target: u64,
    route: crate::portals::Route,
    store: &mut Store,
    scene: &mut crate::scene::Scene,
    encounters: &mut Option<crate::encounters::Encounters>,
    bootstrap: &mut Option<crate::bootstrap::Bootstrap>,
    now_ms: u64,
) -> io::Result<Vec<crate::scene::Delivery>> {
    let saved = &store.characters[&character];
    if peer.combat || !saved.progression.alive() {
        return Err(invalid("Use a portal while alive and out of combat."));
    }
    if saved.progression.level().number < route.level {
        return Err(io::Error::other(format!(
            "This portal requires level {}.",
            route.level
        )));
    }
    if peer
        .last_world_use_ms
        .is_some_and(|last| now_ms.saturating_sub(last) < 5000)
    {
        return Err(invalid("Wait a few seconds before using a portal."));
    }
    let player = scene
        .player_placement(character)
        .ok_or_else(|| invalid("Player is not ready."))?;
    let portal = scene
        .examination_content(character, target)
        .ok_or_else(|| invalid("Portal is not visible."))?;
    let distance: f64 = player
        .coordinates
        .into_iter()
        .zip(portal.placement.coordinates)
        .map(|(a, b)| (f64::from(a) - f64::from(b)).powi(2))
        .sum();
    if player.cell != portal.placement.cell || distance > 16.0 {
        return Err(invalid("Move within four meters of the portal."));
    }
    let prepared = prepare_forced_move(peer, character, route.destination, scene, bootstrap)?;
    let events = vec![
        crate::ui::portal_used(character, target, route.destination, distance.sqrt() as f32)
            .map_err(other)?,
        prepared.wire,
        script::private_call(script::COMBAT_STATE, &script::integer(0)).map_err(other)?,
    ];
    peer.validate_scene_batch(&events)?;
    store.relocate(&peer.session.account, character, route.destination)?;
    if let Some(runtime) = encounters {
        runtime.stop(character);
    }
    let updates = scene
        .update(character, prepared.motion.position_stamp, &prepared.update)
        .map_err(other)?;
    peer.motion = Some(prepared.motion);
    peer.target = None;
    peer.last_world_use_ms = Some(now_ms);
    peer.queue_ordered_batch(events)?;
    Ok(updates)
}
fn recall_character(
    peer: &mut Peer,
    character: u64,
    store: &mut Store,
    scene: &mut crate::scene::Scene,
    encounters: &mut Option<crate::encounters::Encounters>,
    bootstrap: &mut Option<crate::bootstrap::Bootstrap>,
    now_ms: u64,
) -> io::Result<Vec<crate::scene::Delivery>> {
    let saved = &store.characters[&character];
    if peer.combat || !saved.progression.alive() {
        return Err(invalid("Recall while alive and out of combat."));
    }
    if peer
        .last_world_use_ms
        .is_some_and(|last| now_ms.saturating_sub(last) < 5000)
    {
        return Err(invalid("Wait a few seconds before recalling."));
    }
    let destination = saved
        .binding
        .ok_or_else(|| invalid("Bind to a lifestone before recalling."))?;
    let prepared = prepare_forced_move(peer, character, destination, scene, bootstrap)?;
    let mut events = vec![prepared.wire];
    if let Some(entry) = saved.skills.get(crate::skills::LIFESTONE_RECALL) {
        events
            .push(crate::ui::skill_update(crate::skills::LIFESTONE_RECALL, entry).map_err(other)?);
    }
    events.push(script::private_call(script::COMBAT_STATE, &script::integer(0)).map_err(other)?);
    peer.validate_scene_batch(&events)?;
    store.return_to_binding(&peer.session.account, character, false)?;
    if let Some(runtime) = encounters {
        runtime.stop(character);
    }
    let updates = scene
        .update(character, prepared.motion.position_stamp, &prepared.update)
        .map_err(other)?;
    peer.motion = Some(prepared.motion);
    peer.target = None;
    peer.last_world_use_ms = Some(now_ms);
    peer.queue_ordered_batch(events)?;
    Ok(updates)
}
fn resurrect_character(
    peer: &mut Peer,
    character: u64,
    store: &mut Store,
    scene: &mut crate::scene::Scene,
    encounters: &mut Option<crate::encounters::Encounters>,
    lifestones: bool,
    bootstrap: &mut Option<crate::bootstrap::Bootstrap>,
) -> io::Result<Vec<crate::scene::Delivery>> {
    let saved = store
        .characters
        .get(&character)
        .filter(|c| !c.deleted && c.account == peer.session.account)
        .ok_or_else(|| invalid("Character unavailable."))?;
    if saved.progression.alive() {
        return Err(invalid("You are already alive."));
    }
    let next = saved
        .progression
        .apply(&crate::progression::Command::Resurrect)
        .map_err(other)?;
    let forced = if lifestones {
        saved
            .binding
            .map(|destination| prepare_forced_move(peer, character, destination, scene, bootstrap))
            .transpose()?
    } else {
        None
    };
    let mut owner_events = next.private_updates();
    if let Some(prepared) = &forced {
        owner_events.push(prepared.wire.clone());
    }
    owner_events.push(crate::ui::lock_player(false).map_err(other)?);
    owner_events
        .push(script::private_call(script::COMBAT_STATE, &script::integer(0)).map_err(other)?);
    peer.validate_scene_batch(&owner_events)?;
    if forced.is_some() {
        store.return_to_binding(&peer.session.account, character, true)?;
    } else {
        store.progression_command(
            &peer.session.account,
            character,
            crate::progression::Command::Resurrect,
        )?;
    }
    if let Some(runtime) = encounters {
        runtime.stop(character);
    }
    peer.combat = false;
    peer.target = None;
    let state = &store.characters[&character].progression;
    let mut updates = scene.player_stats(character, state);
    if let Some(prepared) = forced {
        updates.extend(
            scene
                .update(character, prepared.motion.position_stamp, &prepared.update)
                .map_err(other)?,
        );
        peer.motion = Some(prepared.motion);
    }
    peer.queue_ordered_batch(owner_events)?;
    Ok(updates)
}

fn inventory_spawn(
    item: &crate::inventory::Item,
    owner: u64,
    instance: u16,
    parent_instance: u16,
    metadata: &BTreeMap<u32, Option<crate::inventory_request::Metadata>>,
) -> io::Result<Vec<u8>> {
    if matches!(item.location, crate::inventory::Location::Equipped(_)) {
        if let Some(hand) = metadata
            .get(&item.definition)
            .and_then(|entry| *entry)
            .and_then(|entry| entry.equipment)
        {
            if hand.hand_slot(item).is_some() {
                return world::create_hand_item(
                    item,
                    owner,
                    instance,
                    parent_instance,
                    hand.primary,
                )
                .map_err(other);
            }
        }
    }
    world::create_inventory_item(item, owner, instance).map_err(other)
}

fn examine_target(
    character: u64,
    target: u64,
    store: &Store,
    scene: &crate::scene::Scene,
    names: &crate::examination::Names,
) -> io::Result<Vec<u8>> {
    use crate::{
        examination::{self, Name},
        inventory::Location,
    };
    if let Some(player) = store
        .characters
        .get(&target)
        .filter(|player| !player.deleted && scene.can_observe(character, player.id))
    {
        let state = &player.progression;
        let mut lines = vec![
            format!("Level {}", state.level().number),
            format!("Health: {} / {}", state.health, state.level().health),
        ];
        if !state.alive() {
            lines.push("Dead".to_owned());
        }
        if character == target {
            lines.extend([
                format!("Vigor: {} / {}", state.vigor, state.level().vigor),
                format!("Experience: {}", state.xp),
                format!("Available experience: {}", state.available_xp),
                format!("Gold: {}", state.gold),
            ]);
            if player.binding.is_some() {
                lines.push("Bound to a lifestone.".to_owned());
            }
        }
        return examination::response(target, Name::Literal(player.name.clone()), &lines)
            .map_err(other);
    }
    if let Some(item) = store.characters[&character]
        .inventory
        .items()
        .find(|item| item.id == target)
    {
        let location = match item.location {
            Location::Equipped(0x40) => "Equipped in main hand.",
            Location::Equipped(0x80) => "Equipped in off hand.",
            Location::Equipped(_) => "Equipped.",
            Location::Bag(_) => "In your inventory.",
        };
        return examination::response(
            target,
            names.get(item.definition, "Item"),
            &[format!("Quantity: {}", item.quantity), location.to_owned()],
        )
        .map_err(other);
    }
    if let Some(definition) = scene.visible_hand_definition(character, target) {
        return examination::response(target, names.get(definition, "Item"), &[]).map_err(other);
    }
    if let Some(object) = scene.examination_content(character, target) {
        let mut lines = Vec::new();
        if let Some((health, maximum)) = object.stat(0x102).zip(object.stat(0x107)) {
            lines.push(format!("Health: {health} / {maximum}"));
        }
        return examination::response(target, names.get(object.entity, "Object"), &lines)
            .map_err(other);
    }
    Err(invalid("That object is not available to examine."))
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
    _socket: &UdpSocket,
    _now: Instant,
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
        if peer.queue_scene(delivery.data).is_err() {
            failed.insert(*addr);
        }
    }
    Ok(())
}

fn use_training_potion(
    peer: &mut Peer,
    character: u64,
    item: u64,
    store: &mut Store,
    scene: &mut crate::scene::Scene,
    now: u64,
) -> io::Result<Vec<crate::scene::Delivery>> {
    let saved = &store.characters[&character];
    if !scene.can_observe(character, character) || !saved.progression.alive() {
        return Err(invalid("Potion use requires a ready living character."));
    }
    if peer
        .last_world_use_ms
        .is_some_and(|last| now.saturating_sub(last) < crate::item_services::POTION_COOLDOWN_MS)
    {
        return Err(invalid("Wait five seconds before using another potion."));
    }
    let owned = saved
        .inventory
        .items()
        .find(|owned| owned.id == item)
        .filter(|owned| {
            (owned.definition == crate::item_services::POTION_DEFINITION
                && owned.visual == crate::item_services::POTION_VISUAL
                || owned.definition == crate::crafting::VIGOR_OUTPUT && owned.visual == 0x1f000e40)
                && owned.quantity == 1
                && owned.stack_limit == 1
                && owned.equipment_mask == 0
        })
        .ok_or_else(|| {
            invalid("This training potion is not owned or has unsupported properties.")
        })?;
    let vigor = owned.definition == crate::crafting::VIGOR_OUTPUT;
    if !matches!(owned.location, crate::inventory::Location::Bag(_))
        || if vigor {
            saved.progression.vigor == saved.progression.level().vigor
        } else {
            saved.progression.health == saved.progression.level().health
        }
    {
        return Err(invalid(
            "The potion must be in your bag and its vital must be below the maximum.",
        ));
    }
    let instance = scene.inventory_instance(character, item).map_err(other)?;
    let command = crate::inventory::Command::ConsumeDense { item };
    let changes = [if vigor {
        crate::progression::Command::RecoverVigor(25)
    } else {
        crate::progression::Command::Heal(crate::item_services::POTION_HEALTH)
    }];
    let next = saved
        .progression
        .apply(&changes[0])
        .map_err(io::Error::other)?;
    let mut events = vec![
        crate::item_services::removed(item).map_err(other)?,
        world::destroy_inventory_item(item, instance),
    ];
    events.extend(next.private_changes(&saved.progression));
    events.push(
        script::chat(
            &format!(
                "You recover {} {}.",
                if vigor {
                    next.vigor - saved.progression.vigor
                } else {
                    next.health - saved.progression.health
                },
                if vigor { "vigor" } else { "health" }
            ),
            1,
        )
        .map_err(other)?,
    );
    peer.validate_scene_batch(&events)?;
    store.player_transaction(&peer.session.account, character, &[command], &changes)?;
    scene.inventory_removed(character, item).map_err(other)?;
    peer.last_world_use_ms = Some(now);
    let updates = scene.player_stats(character, &next);
    peer.queue_ordered_batch(events)?;
    Ok(updates)
}

fn quest_action(
    peer: &mut Peer,
    character: u64,
    action: &str,
    store: &mut Store,
    scene: &mut crate::scene::Scene,
) -> io::Result<Vec<crate::scene::Delivery>> {
    if !scene.can_observe(character, character) {
        return Err(invalid("Player is not ready."));
    }
    let saved = &store.characters[&character];
    let command = match action.trim().to_ascii_lowercase().as_str() {
        "" | "status" => None,
        "start" => Some(crate::quests::Command::Accept),
        "claim" => Some(crate::quests::Command::Claim),
        _ => return Err(invalid("Use @quest, @quest start, or @quest claim.")),
    };
    if command.is_some() && (!saved.progression.alive() || peer.combat) {
        return Err(invalid(
            "Quest acceptance and rewards require a living player out of combat.",
        ));
    }
    let quests = match command.as_ref() {
        Some(command) => saved.quests.apply(command).map_err(io::Error::other)?,
        None => saved.quests.clone(),
    };
    let mut state = saved.progression.clone();
    if let Some(command) = &command {
        for reward in crate::quests::rewards(command) {
            state = state.apply(&reward).map_err(io::Error::other)?;
        }
    }
    let text = match quests.get() {
        None => "Type @quest start to accept the training quest.".into(),
        Some(progress) if progress.claimed => {
            "Training complete. The 120 XP and 10 gold reward has been claimed.".into()
        }
        Some(progress) if progress.killed == crate::quests::GOAL => {
            "Training objective complete. Leave combat and type @quest claim for your reward."
                .into()
        }
        Some(progress) => format!(
            "Training creatures defeated: {}/{}. Progress is shown in your quest journal.",
            progress.killed,
            crate::quests::GOAL
        ),
    };
    let mut events = vec![script::chat(&text, 1).map_err(other)?];
    if quests.get().is_some() {
        events.push(quests.update().map_err(other)?);
    }
    events.extend(state.private_changes(&saved.progression));
    if saved.skills.available_credits(&state) != saved.skills.available_credits(&saved.progression)
    {
        events
            .push(crate::ui::skill_credits(saved.skills.available_credits(&state)).map_err(other)?);
    }
    peer.validate_scene_batch(&events)?;
    if let Some(command) = command {
        store.quest_command(&peer.session.account, character, command)?;
    }
    let updates = scene.player_stats(character, &state);
    peer.queue_ordered_batch(events)?;
    Ok(updates)
}

fn change_hand_item(
    peer: &mut Peer,
    character: u64,
    request: &crate::equipment::UseRequest,
    destination: Option<(u16, u16)>,
    store: &mut Store,
    scene: &mut crate::scene::Scene,
    metadata: &BTreeMap<u32, Option<crate::inventory_request::Metadata>>,
    now: u64,
) -> io::Result<Vec<crate::scene::Delivery>> {
    if request.owner != character {
        return Err(invalid("Item owner does not match this character."));
    }
    if peer
        .last_inventory_ms
        .is_some_and(|last| now.saturating_sub(last) < 100)
    {
        return Err(invalid("Wait briefly before changing equipment again."));
    }
    let saved = &store.characters[&character];
    if !saved.progression.alive() || peer.combat {
        return Err(invalid("Change equipment while alive and out of combat."));
    }
    if !saved.inventory.dense_bags() {
        return Err(invalid("This equipment profile requires a dense bag list."));
    }
    let item = saved
        .inventory
        .items()
        .find(|item| item.id == request.item)
        .ok_or_else(|| invalid("Item is not owned."))?;
    let hand = metadata
        .get(&item.definition)
        .and_then(|entry| *entry)
        .and_then(|entry| entry.equipment)
        .ok_or_else(|| invalid("This item-use profile is not available."))?;
    let slot = hand
        .hand_slot(item)
        .ok_or_else(|| invalid("Only supported hand-held items can be changed here."))?;
    let equipped =
        destination.is_none() && matches!(item.location, crate::inventory::Location::Bag(_));
    let command = if let Some((expected_slot, requested)) = destination {
        if item.location != crate::inventory::Location::Equipped(expected_slot)
            || slot != expected_slot
        {
            return Err(invalid("The item is no longer equipped in that slot."));
        }
        let count = saved
            .inventory
            .items()
            .filter(|item| matches!(item.location, crate::inventory::Location::Bag(_)))
            .count() as u16;
        if requested >= crate::inventory::BAG_SLOTS {
            return Err(invalid("That backpack position is unavailable."));
        }
        crate::inventory::Command::UnequipAt {
            item: item.id,
            slot,
            destination: requested.min(count),
        }
    } else if equipped {
        crate::inventory::Command::Wear {
            item: item.id,
            slot,
        }
    } else {
        crate::inventory::Command::Unequip { item: item.id }
    };
    let candidate = saved.inventory.apply(&command).map_err(other)?;
    let changed = candidate
        .items()
        .find(|item| item.id == request.item)
        .unwrap();
    let bag_slot = match changed.location {
        crate::inventory::Location::Bag(index) => index,
        _ => 0,
    };
    let other_hands = saved
        .inventory
        .items()
        .filter(|other| other.id != item.id)
        .fold(0, |mask, other| match other.location {
            crate::inventory::Location::Equipped(slot) => mask | (slot & 0xc0),
            _ => mask,
        });
    let acknowledgment = crate::ui::hand_equipment_at(
        character,
        item.id,
        slot,
        other_hands,
        hand,
        equipped,
        bag_slot,
    )
    .map_err(other)?;
    let instance = scene
        .next_item_instance(character, item.id)
        .map_err(other)?;
    let parent_instance = peer
        .motion
        .as_ref()
        .ok_or_else(|| invalid("Equipment has no avatar lifetime."))?
        .instance;
    let spawn = inventory_spawn(changed, character, instance, parent_instance, metadata)?;
    scene
        .validate_hand_change(character, item.id, &spawn, equipped)
        .map_err(other)?;
    let old_instance = scene
        .inventory_instance(character, item.id)
        .map_err(other)?;
    let events = vec![
        world::destroy_inventory_item(item.id, old_instance),
        spawn.clone(),
        acknowledgment,
    ];
    peer.validate_scene_batch(&events)?;
    store.inventory_command(&peer.session.account, character, command)?;
    let updates = scene
        .hand_change(character, request.item, spawn, equipped)
        .map_err(other)?;
    peer.last_inventory_ms = Some(now);
    peer.queue_ordered_batch(events)?;
    Ok(updates
        .into_iter()
        .filter(|delivery| delivery.recipient != character)
        .collect())
}

fn selection_status(
    observer: u64,
    target: u64,
    scene: &crate::scene::Scene,
    store: &Store,
) -> io::Result<Vec<u8>> {
    let vitals = store
        .characters
        .get(&target)
        .filter(|character| !character.deleted)
        .map(|character| {
            let state = &character.progression;
            [
                state.health,
                state.level().health,
                state.vigor,
                state.level().vigor,
            ]
        })
        .or_else(|| {
            let object = scene.examination_content(observer, target)?;
            Some([object.stat(0x102)?, object.stat(0x107)?, 0, 0])
        });
    crate::ui::selection(target, vitals).map_err(other)
}

/// Admit the complete native item/vital update before committing a service's
/// ownership and currency changes. Only the server's bounded grant/removal
/// plans are supported here; no client object description enters this path.
fn item_service_transaction(
    peer: &mut Peer,
    character: u64,
    items: &[crate::inventory::Command],
    changes: &[crate::progression::Command],
    mut completion: Vec<Vec<u8>>,
    store: &mut Store,
    scene: &mut crate::scene::Scene,
    metadata: &BTreeMap<u32, Option<crate::inventory_request::Metadata>>,
) -> io::Result<Vec<crate::scene::Delivery>> {
    let saved = &store.characters[&character];
    if !scene.can_observe(character, character) || !saved.progression.alive() || peer.combat {
        return Err(invalid(
            "Item services require a ready living player out of combat.",
        ));
    }
    if items
        .iter()
        .filter(|item| matches!(item, crate::inventory::Command::Grant(_)))
        .count()
        > 1
    {
        return Err(invalid("Item services support one output per transaction."));
    }
    let first = items
        .iter()
        .any(|item| item.allocated_id().is_some())
        .then(|| store.next_item_id())
        .transpose()?;
    let (bag, state) = crate::player_transaction::candidate(
        &saved.inventory,
        &saved.progression,
        items,
        changes,
        first,
    )?;
    let mut events = Vec::new();
    let mut removed = Vec::new();
    let mut granted = Vec::new();
    for command in items {
        match command {
            crate::inventory::Command::ConsumeDense { item }
            | crate::inventory::Command::ConsumeStackDense { item, .. } => {
                let owned = saved
                    .inventory
                    .items()
                    .find(|owned| owned.id == *item)
                    .ok_or_else(|| invalid("Service item is not owned."))?;
                if !matches!(owned.location, crate::inventory::Location::Bag(_)) {
                    return Err(invalid("Service items must be in your bag."));
                }
                let instance = scene.inventory_instance(character, *item).map_err(other)?;
                events.push(crate::item_services::removed(*item).map_err(other)?);
                events.push(world::destroy_inventory_item(*item, instance));
                removed.push(*item);
            }
            crate::inventory::Command::Grant(item) => {
                let item = bag
                    .items()
                    .find(|owned| owned.id == item.id)
                    .ok_or_else(|| invalid("Service grant absent from final inventory."))?;
                let info = metadata
                    .get(&item.definition)
                    .and_then(|info| *info)
                    .ok_or_else(|| invalid("Service item metadata unavailable."))?;
                let instance = scene
                    .admit_inventory_grant(character, item.id)
                    .map_err(other)?;
                events
                    .push(world::create_inventory_item(item, character, instance).map_err(other)?);
                events.push(crate::item_services::acquired(character, item, info).map_err(other)?);
                granted.push((item.id, instance));
            }
            _ => return Err(invalid("Unsupported item service transition.")),
        }
    }
    events.extend(state.private_changes(&saved.progression));
    events.append(&mut completion);
    peer.validate_scene_batch(&events)?;
    store.player_transaction(&peer.session.account, character, items, changes)?;
    for item in removed {
        scene.inventory_removed(character, item).map_err(other)?;
    }
    for (item, instance) in granted {
        scene
            .inventory_granted(character, item, instance)
            .map_err(other)?;
    }
    let updates = scene.player_stats(character, &state);
    peer.queue_ordered_batch(events)?;
    Ok(updates)
}

fn group_online(
    active: &BTreeMap<u64, SocketAddr>,
    store: &Store,
    scene: &crate::scene::Scene,
) -> BTreeMap<u64, crate::groups::Member> {
    active
        .keys()
        .filter_map(|id| {
            let saved = store.characters.get(id).filter(|saved| !saved.deleted)?;
            let point = scene.player_placement(*id)?;
            let level = saved.progression.level();
            Some((
                *id,
                crate::groups::Member {
                    id: *id,
                    name: saved.name.clone(),
                    level: u32::from(level.number),
                    health: saved.progression.health,
                    max_health: level.health,
                    vigor: saved.progression.vigor,
                    max_vigor: level.vigor,
                    cell: point.cell,
                },
            ))
        })
        .collect()
}

fn group_admit(
    deliveries: &[crate::groups::Delivery],
    active: &BTreeMap<u64, SocketAddr>,
    peers: &BTreeMap<SocketAddr, Peer>,
) -> io::Result<BTreeMap<SocketAddr, Vec<Vec<u8>>>> {
    let mut packets: BTreeMap<SocketAddr, Vec<Vec<u8>>> = BTreeMap::new();
    for delivery in deliveries {
        // A disconnect plan also clears the departed player's UI. There is no
        // transport to send that clear to, and reconnect starts with empty UI.
        if let Some(addr) = active.get(&delivery.recipient) {
            packets
                .entry(*addr)
                .or_default()
                .push(delivery.data.clone());
        }
    }
    for (addr, events) in &packets {
        peers
            .get(addr)
            .ok_or_else(|| invalid("Group recipient disconnected."))?
            .validate_scene_batch(events)?;
    }
    Ok(packets)
}

fn group_publish(
    packets: BTreeMap<SocketAddr, Vec<Vec<u8>>>,
    peers: &mut BTreeMap<SocketAddr, Peer>,
) -> io::Result<()> {
    for (addr, events) in packets {
        peers
            .get_mut(&addr)
            .ok_or_else(|| invalid("Group recipient disconnected."))?
            .queue_ordered_batch(events)?;
    }
    Ok(())
}

const TRAINING_MERCHANT: u64 = 0x8140000000030003;
const TRAINING_MERCHANT_OUTDOOR: u64 = 0x8140000000030006;
const TRAINING_MENTORS: [u64; 3] = [0x8140000000030008, 0x8140000000030009, 0x814000000003000a];

fn use_training_mentor(
    peer: &mut Peer,
    character: u64,
    target: u64,
    store: &mut Store,
    scene: &mut crate::scene::Scene,
) -> io::Result<Vec<crate::scene::Delivery>> {
    let player = scene
        .player_placement(character)
        .ok_or_else(|| invalid("Player is not ready."))?;
    let mentor = scene
        .examination_content(character, target)
        .filter(|object| object.entity == 0x47001699)
        .ok_or_else(|| invalid("That training mentor is not visible."))?;
    if player.cell != mentor.placement.cell
        || player
            .coordinates
            .iter()
            .zip(mentor.placement.coordinates)
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f32>()
            > 16.0
    {
        return Err(invalid("Move within four metres of the training mentor."));
    }
    if peer.combat || !store.characters[&character].progression.alive() {
        return Err(invalid(
            "Speak to the training mentor while alive and out of combat.",
        ));
    }
    let book = &store.characters[&character].quests;
    let action = if book.get().is_none() {
        "start"
    } else if book.ready() {
        "claim"
    } else {
        "status"
    };
    quest_action(peer, character, action, store, scene)
}

fn merchant_ready(character: u64, scene: &crate::scene::Scene) -> io::Result<u64> {
    let player = scene
        .player_placement(character)
        .ok_or_else(|| invalid("Player is not ready."))?;
    let nearby = [TRAINING_MERCHANT, TRAINING_MERCHANT_OUTDOOR]
        .into_iter()
        .find(|id| {
            let Some(merchant) = scene
                .examination_content(character, *id)
                .filter(|object| object.entity == crate::merchants::MERCHANT_ENTITY)
            else {
                return false;
            };
            player.cell == merchant.placement.cell
                && player
                    .coordinates
                    .iter()
                    .zip(merchant.placement.coordinates)
                    .map(|(a, b)| (a - b).powi(2))
                    .sum::<f32>()
                    <= 16.0
        });
    nearby.ok_or_else(|| invalid("Move within four metres of the training quartermaster."))
}

fn merchant_action(
    peer: &mut Peer,
    character: u64,
    action: crate::merchants::Action,
    store: &mut Store,
    scene: &mut crate::scene::Scene,
    metadata: &BTreeMap<u32, Option<crate::inventory_request::Metadata>>,
    catalog: &crate::merchants::Catalog,
    next_offer: &mut u64,
    now: u64,
) -> io::Result<Vec<crate::scene::Delivery>> {
    let merchant = merchant_ready(character, scene)?;
    let saved = &store.characters[&character];
    if !saved.progression.alive() || peer.combat {
        return Err(invalid("Trading requires a living player out of combat."));
    }
    if peer
        .last_inventory_ms
        .is_some_and(|last| now.saturating_sub(last) < 250)
    {
        return Err(invalid("Wait briefly before another inventory action."));
    }
    if action == crate::merchants::Action::List {
        let last_offer = next_offer
            .checked_add(4)
            .filter(|id| id >> 48 == 0x8160)
            .ok_or_else(|| invalid("Shop offer identities exhausted; restart required."))?;
        let offers = std::array::from_fn(|index| *next_offer + 1 + index as u64);
        let shop = crate::merchants::Shop::new(merchant, offers).map_err(other)?;
        peer.queue_ordered_batch(vec![
            shop.open(catalog).map_err(other)?,
            script::chat(&crate::merchants::listing(), 1).map_err(other)?,
        ])?;
        *next_offer = last_offer;
        peer.native_shop = Some(shop);
        peer.merchant_open_ms = Some(now);
        peer.last_inventory_ms = Some(now);
        return Ok(Vec::new());
    }
    let plan = match action {
        crate::merchants::Action::Buy(stock) => crate::merchants::plan_buy(
            stock,
            &saved.inventory,
            &saved.progression,
            store.next_item_id()?,
        ),
        crate::merchants::Action::SellSlot(slot) => {
            crate::merchants::plan_sell(slot, &saved.inventory, &saved.progression)
        }
        crate::merchants::Action::List => unreachable!(),
    }
    .map_err(io::Error::other)?;
    let mut completion = plan.events;
    completion.push(script::chat(&plan.message, 1).map_err(other)?);
    let updates = item_service_transaction(
        peer,
        character,
        &plan.items,
        &plan.changes,
        completion,
        store,
        scene,
        metadata,
    )?;
    peer.merchant_open_ms = Some(now);
    peer.last_inventory_ms = Some(now);
    Ok(updates)
}

fn craft_item(
    peer: &mut Peer,
    character: u64,
    request: &crate::crafting::ExecuteRequest,
    store: &mut Store,
    scene: &mut crate::scene::Scene,
    metadata: &BTreeMap<u32, Option<crate::inventory_request::Metadata>>,
    now: u64,
) -> io::Result<Vec<crate::scene::Delivery>> {
    if peer
        .last_inventory_ms
        .is_some_and(|last| now.saturating_sub(last) < crate::crafting::COOLDOWN_MS)
    {
        return Err(invalid("Wait one second before crafting again."));
    }
    let saved = &store.characters[&character];
    let ingredients = request.require_inputs(&saved.inventory, true)
        .map_err(|_| invalid("Crafting needs one Mucor Mushroom and a full 100-unit Tallow bundle in your bag. Use @shop near the training quartermaster."))?;
    let mut items: Vec<_> = ingredients
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            if index == 1 {
                crate::inventory::Command::ConsumeStackDense {
                    item,
                    quantity: crate::crafting::TALLOW_QUANTITY,
                }
            } else {
                crate::inventory::Command::ConsumeDense { item }
            }
        })
        .collect();
    let mut bag = saved.inventory.clone();
    for item in &items {
        bag = bag.apply(item).map_err(io::Error::other)?;
    }
    let slot = bag
        .free_slot()
        .ok_or_else(|| invalid("There is no bag space for the crafted item."))?;
    let output = crate::crafting::output(request.recipe, store.next_item_id()?, slot)
        .ok_or_else(|| invalid("That recipe is not enabled."))?;
    let name = if request.recipe == crate::crafting::HEALTH_RECIPE {
        "health"
    } else {
        "vigor"
    };
    items.push(crate::inventory::Command::Grant(output));
    let events = vec![
        crate::crafting::completed(request.recipe, true).map_err(other)?,
        script::chat(&format!("Crafted one {name} potion. Training recipe: one mushroom and 100 Tallow; no crafting XP or chance roll."), 1).map_err(other)?,
    ];
    let updates =
        item_service_transaction(peer, character, &items, &[], events, store, scene, metadata)?;
    peer.last_inventory_ms = Some(now);
    Ok(updates)
}

pub fn run(config: Config) -> io::Result<()> {
    println!("{}", crate::licensing::notice());
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
    let mut inventory_metadata = BTreeMap::new();
    let mut merchant_catalog = None;
    if (config.training_kit || config.lifestones || config.portals) && !config.initialize_ui {
        return Err(invalid(
            "training kits and world interactions require --initialize-ui true",
        ));
    }
    let initial_ui = if config.initialize_ui {
        let directory = config.game_data.as_ref().unwrap();
        let mut portal = crate::dat::Dat::open(directory.join("portal.dat"))?;
        let wlib = portal.read(0x56000005)?;
        crate::ui::validate_wlib(&wlib).map_err(other)?;
        crate::ui::validate_skills_wlib(&wlib).map_err(other)?;
        crate::skills::validate_assets(&mut portal)?;
        crate::emotes::validate_assets(&mut portal)?;
        crate::quests::validate_assets(&mut portal)?;
        crate::crafting::validate_wlib(&wlib).map_err(other)?;
        crate::ui::validate_layouts(&wlib, &crate::merchants::LAYOUTS).map_err(other)?;
        crate::ui::validate_layouts(&wlib, &crate::groups::LAYOUTS).map_err(other)?;
        if config.training_kit
            || config.lifestones
            || config.portals
            || store
                .characters
                .values()
                .any(|c| !c.deleted && c.inventory.items().next().is_some())
        {
            crate::ui::validate_inventory_wlib(&wlib).map_err(other)?;
            if config.lifestones || config.portals {
                // String tables live in the locale archive, not portal.dat.
                let mut locale = crate::dat::Dat::open(directory.join("local_English.dat"))?;
                locale.read(0x25000014)?;
            }
            if config.training_kit {
                let catalog = crate::merchants::Catalog::validate(&mut portal)?;
                for (definition, metadata) in catalog.entries() {
                    inventory_metadata.insert(definition, Some(metadata));
                }
                merchant_catalog = Some(catalog);
                crate::crafting::validate_assets(&mut portal)?;
                let vigor = crate::crafting::validate_vigor(&mut portal)?;
                inventory_metadata.insert(crate::crafting::VIGOR_OUTPUT, Some(vigor));
            }
            for character in store.characters.values().filter(|c| !c.deleted) {
                for item in character.inventory.items() {
                    portal.read(item.definition)?;
                    portal.read(item.visual)?;
                    if !inventory_metadata.contains_key(&item.definition) {
                        if inventory_metadata.len() >= 4096 {
                            return Err(invalid("inventory metadata capacity exceeded"));
                        }
                        let metadata =
                            crate::inventory_request::Metadata::load(&mut portal, item.definition);
                        if let Err(error) = &metadata {
                            eprintln!(
                                "Bag sorting unavailable for definition {:#010x}: {error}",
                                item.definition
                            );
                        }
                        inventory_metadata.insert(item.definition, metadata.ok());
                    }
                }
            }
        }
        println!(
            "Experimental HUD and owned-inventory initialization enabled; native acceptance pending."
        );
        Some(crate::ui::empty_player().map_err(other)?)
    } else {
        None
    };
    let content = match config.content.as_deref() {
        Some(path) if path == std::path::Path::new("none") => crate::content::Content::default(),
        Some(path) if path == std::path::Path::new("training") => {
            let directory = config
                .game_data
                .as_deref()
                .ok_or_else(|| invalid("--content training requires --game-data"))?;
            let content = crate::content::Content::training(directory)?;
            println!(
                "Loaded {} world objects: base placements, retained actors and authored training objects.",
                content.len()
            );
            content
        }
        Some(path) if path == std::path::Path::new("observed") => {
            let directory = config
                .game_data
                .as_deref()
                .ok_or_else(|| invalid("--content observed requires --game-data"))?;
            let content = crate::content::Content::observed(directory)?;
            println!(
                "Loaded {} objects from the observed actor snapshot; stationary presentation only.",
                content.len()
            );
            content
        }
        Some(path) => {
            let directory = config
                .game_data
                .as_deref()
                .ok_or_else(|| invalid("--content requires --game-data"))?;
            let content = crate::content::Content::load(path, directory)?;
            println!(
                "Loaded {} stationary content objects; interactions and AI remain unfinished.",
                content.len()
            );
            content
        }
        None => match config.game_data.as_deref() {
            Some(directory) => {
                let content = crate::content::Content::bundled(directory)?;
                println!(
                    "Loaded {} bundled stationary content objects; interactions and AI remain unfinished.",
                    content.len()
                );
                content
            }
            None => crate::content::Content::default(),
        },
    };
    let mut encounters = config
        .encounters
        .as_deref()
        .map(|path| crate::encounters::Encounters::load(path, &content))
        .transpose()?;
    if config.training_kit {
        if let Some(runtime) = &mut encounters {
            runtime.enable_training_loot();
        }
    }
    let portal_routes = if config.portals {
        let routes =
            crate::portals::Routes::bundled(&content, config.game_data.as_deref().unwrap())?;
        println!(
            "Loaded {} development portal routes for the selected content pack.",
            routes.len()
        );
        routes
    } else {
        crate::portals::Routes::default()
    };
    let examination_names = if config.initialize_ui {
        crate::examination::Names::load(
            config.game_data.as_deref().unwrap(),
            content.objects().map(|object| object.entity).chain(
                store
                    .characters
                    .values()
                    .filter(|character| !character.deleted)
                    .flat_map(|character| character.inventory.items().map(|item| item.definition))
                    .chain(inventory_metadata.keys().copied()),
            ),
        )?
    } else {
        crate::examination::Names::default()
    };
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
    let mut scene = crate::scene::Scene::with_content(content);
    if let Some(directory) = config.game_data.as_deref() {
        scene = scene.with_visibility(crate::visibility::Visibility::open(directory)?);
    }
    if let Some(runtime) = &encounters {
        for (id, health) in runtime.initial_health() {
            scene.content_stat(id, 0x107, health)?;
            scene.content_stat(id, 0x102, health)?;
        }
        println!(
            "Development encounters enabled with explicit authored rules; original combat parity is unfinished."
        );
    }
    let training_quest_enabled = config.training_kit
        && encounters
            .as_ref()
            .is_some_and(|runtime| runtime.includes_entity(crate::quests::TARGET_ENTITY));
    let mut abilities = if config.initialize_ui && config.training_kit {
        Some(crate::abilities::Runtime::development().map_err(io::Error::other)?)
    } else {
        None
    };
    let mut failed_peers = BTreeSet::new();
    let mut next_offer = 0x8160000000000000u64;
    let mut groups = crate::groups::Runtime::default();
    let mut group_cache = BTreeMap::new();
    let mut next_group_refresh = 0u64;
    let random = RandomState::new();
    let mut generation = 0u64;
    let mut counters = Counters::default();
    let mut buffer = [0u8; 65536];
    let mut next_report = 10_000;
    let mut next_stop_check = 0;
    let mut next_checkpoint = 5000;
    let mut diagnosed_events = BTreeSet::new();
    let mut diagnosed_application_errors = BTreeSet::new();
    let mut diagnosed_packet_errors = BTreeSet::new();
    let mut diagnosed_queues = BTreeSet::new();
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
                let mut group_requests = Vec::new();
                // Keep transport recovery outside fallible application handling.
                // A later world/UI rejection must not discard ACK-covered NAKs.
                let mut recovery_packets = Vec::new();
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
                                emotes: crate::emotes::State::default(),
                                last_inventory_ms: None,
                                last_examine_ms: None,
                                last_world_use_ms: None,
                                merchant_open_ms: None,
                                native_shop: None,
                                unhandled_scripts: BTreeSet::new(),
                                scene_outbox: VecDeque::new(),
                                scene_bytes: 0,
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
                    recovery_packets.extend(received.retransmissions);
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
                    for message in received.messages {
                        if !matches!(message.queue, 3 | 4) {
                            counters.rejected += 1;
                            let opcode = message
                                .data
                                .get(..4)
                                .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
                                .unwrap_or(0);
                            if diagnosed_queues.len() < 16
                                && diagnosed_queues.insert((message.queue, opcode))
                            {
                                eprintln!(
                                    "client queue rejected: queue={} opcode={opcode:#x} bytes={} blob={:#x}",
                                    message.queue,
                                    message.data.len(),
                                    message.blob_id
                                );
                            }
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
                                        script::Request::Group(request) => {
                                            if !config.initialize_ui
                                                || !scene.can_observe(character, character)
                                            {
                                                return Err(invalid(
                                                    "Groups require a ready player with the HUD enabled.",
                                                ));
                                            }
                                            if group_requests.len() >= 4 {
                                                return Err(invalid(
                                                    "Too many group requests in one packet.",
                                                ));
                                            }
                                            group_requests.push((character, request));
                                            return Ok(replies);
                                        }
                                        script::Request::MerchantSell(request) => {
                                            let result =
                                                (|| -> io::Result<Vec<crate::scene::Delivery>> {
                                                    if !config.training_kit
                                                        || !peer.merchant_open_ms.is_some_and(
                                                            |last| {
                                                                now_ms.saturating_sub(last)
                                                                    <= 30_000
                                                            },
                                                        )
                                                    {
                                                        return Err(invalid(
                                                            "Open the nearby training quartermaster before selling.",
                                                        ));
                                                    }
                                                    merchant_ready(character, &scene)?;
                                                    if peer.last_inventory_ms.is_some_and(|last| {
                                                        now_ms.saturating_sub(last) < 250
                                                    }) {
                                                        return Err(invalid(
                                                            "Wait briefly before another inventory action.",
                                                        ));
                                                    }
                                                    let saved = &store.characters[&character];
                                                    let plan = crate::merchants::plan_native_sell(
                                                        character,
                                                        &request,
                                                        &saved.inventory,
                                                        &saved.progression,
                                                    )
                                                    .map_err(io::Error::other)?;
                                                    let mut events = plan.events;
                                                    events.push(
                                                        script::chat(&plan.message, 1)
                                                            .map_err(other)?,
                                                    );
                                                    let updates = item_service_transaction(
                                                        peer,
                                                        character,
                                                        &plan.items,
                                                        &plan.changes,
                                                        events,
                                                        &mut store,
                                                        &mut scene,
                                                        &inventory_metadata,
                                                    )?;
                                                    peer.last_inventory_ms = Some(now_ms);
                                                    peer.merchant_open_ms = Some(now_ms);
                                                    Ok(updates)
                                                })(
                                                );
                                            match result {
                                                Ok(updates) => deliveries.extend(updates),
                                                Err(error) => replies.extend(
                                                    peer.send_event(
                                                        &script::chat(&error.to_string(), 1)
                                                            .map_err(other)?,
                                                        now,
                                                    )?,
                                                ),
                                            }
                                            return Ok(replies);
                                        }
                                        script::Request::MerchantBuy(request) => {
                                            let result =
                                                (|| -> io::Result<Vec<crate::scene::Delivery>> {
                                                    if !config.training_kit
                                                        || !peer.merchant_open_ms.is_some_and(
                                                            |last| {
                                                                now_ms.saturating_sub(last)
                                                                    <= 30_000
                                                            },
                                                        )
                                                    {
                                                        return Err(invalid(
                                                            "Open the nearby quartermaster before buying.",
                                                        ));
                                                    }
                                                    let merchant =
                                                        merchant_ready(character, &scene)?;
                                                    let shop = peer.native_shop.as_ref().filter(|shop| shop.merchant() == merchant)
                                                    .ok_or_else(|| invalid("The shop has changed; speak to the quartermaster again."))?;
                                                    if peer.last_inventory_ms.is_some_and(|last| {
                                                        now_ms.saturating_sub(last) < 250
                                                    }) {
                                                        return Err(invalid(
                                                            "Wait briefly before buying again.",
                                                        ));
                                                    }
                                                    let saved = &store.characters[&character];
                                                    let plan = shop
                                                        .plan_buy(
                                                            character,
                                                            &request,
                                                            &saved.inventory,
                                                            &saved.progression,
                                                            store.next_item_id()?,
                                                        )
                                                        .map_err(io::Error::other)?;
                                                    let mut events = plan.events;
                                                    events.push(shop.refresh(merchant_catalog.as_ref().ok_or_else(|| invalid("Merchant catalog is not enabled."))?).map_err(other)?);
                                                    events.push(
                                                        script::chat(&plan.message, 1)
                                                            .map_err(other)?,
                                                    );
                                                    let updates = item_service_transaction(
                                                        peer,
                                                        character,
                                                        &plan.items,
                                                        &plan.changes,
                                                        events,
                                                        &mut store,
                                                        &mut scene,
                                                        &inventory_metadata,
                                                    )?;
                                                    peer.last_inventory_ms = Some(now_ms);
                                                    peer.merchant_open_ms = Some(now_ms);
                                                    Ok(updates)
                                                })(
                                                );
                                            match result {
                                                Ok(updates) => deliveries.extend(updates),
                                                Err(error) => replies.extend(
                                                    peer.send_event(
                                                        &script::chat(&error.to_string(), 1)
                                                            .map_err(other)?,
                                                        now,
                                                    )?,
                                                ),
                                            }
                                            return Ok(replies);
                                        }
                                        script::Request::MerchantConsignment(merchant) => {
                                            let text = if peer
                                                .native_shop
                                                .as_ref()
                                                .is_some_and(|shop| shop.merchant() == merchant)
                                                && merchant_ready(character, &scene).ok()
                                                    == Some(merchant)
                                            {
                                                "Consignment listings are not available at this training shop. Use the inventory Sell action for fixed buyback. No items or fees were taken."
                                            } else {
                                                "Speak to a nearby quartermaster to open a shop."
                                            };
                                            peer.queue_ordered_batch(vec![
                                                script::chat(text, 1).map_err(other)?,
                                            ])?;
                                            return Ok(replies);
                                        }
                                        script::Request::ExamineClear => return Ok(replies),
                                        script::Request::CraftValidate(request) => {
                                            let saved = &store.characters[&character];
                                            let allowed = config.training_kit
                                                && config.initialize_ui
                                                && scene.can_observe(character, character)
                                                && saved.progression.alive()
                                                && !peer.combat
                                                && request.allowed(&saved.inventory, true);
                                            peer.queue_ordered_batch(vec![
                                                crate::crafting::validation_reply(
                                                    &request, allowed,
                                                )
                                                .map_err(other)?,
                                            ])?;
                                            return Ok(replies);
                                        }
                                        script::Request::CraftExecute(request) => {
                                            let result = if config.training_kit
                                                && config.initialize_ui
                                            {
                                                craft_item(
                                                    peer,
                                                    character,
                                                    &request,
                                                    &mut store,
                                                    &mut scene,
                                                    &inventory_metadata,
                                                    now_ms,
                                                )
                                            } else {
                                                Err(invalid("Training crafting is not enabled."))
                                            };
                                            match result {
                                                Ok(updates) => deliveries.extend(updates),
                                                Err(error) => peer.queue_ordered_batch(vec![
                                                    crate::crafting::completed(
                                                        request.recipe,
                                                        false,
                                                    )
                                                    .map_err(other)?,
                                                    script::chat(&error.to_string(), 1)
                                                        .map_err(other)?,
                                                ])?,
                                            }
                                            return Ok(replies);
                                        }
                                        script::Request::Emote(token) => {
                                            let result = (|| -> io::Result<()> {
                                                let saved = &store.characters[&character];
                                                if !config.initialize_ui
                                                    || !scene.can_observe(character, character)
                                                    || !saved.progression.alive()
                                                    || peer.combat
                                                {
                                                    return Err(invalid(
                                                        "Emotes require a ready, living character out of combat.",
                                                    ));
                                                }
                                                let (next, wire) = peer
                                                    .emotes
                                                    .prepare(
                                                        character,
                                                        saved.instance,
                                                        now_ms,
                                                        token,
                                                    )
                                                    .map_err(other)?;
                                                peer.validate_scene_batch(&[wire.clone()])?;
                                                let updates = scene
                                                    .broadcast(character, wire)
                                                    .map_err(other)?;
                                                peer.emotes = next;
                                                deliveries.extend(updates);
                                                Ok(())
                                            })(
                                            );
                                            if let Err(error) = result {
                                                replies.extend(
                                                    peer.send_event(
                                                        &script::chat(&error.to_string(), 1)
                                                            .map_err(other)?,
                                                        now,
                                                    )?,
                                                );
                                            }
                                        }
                                        script::Request::UseSkill { skill, target } => {
                                            if abilities
                                                .as_ref()
                                                .is_some_and(|runtime| runtime.supports(skill))
                                            {
                                                let result = (|| -> io::Result<()> {
                                                    let saved = &store.characters[&character];
                                                    let cast = abilities
                                                        .as_mut()
                                                        .unwrap()
                                                        .prepare(
                                                            character,
                                                            skill,
                                                            target,
                                                            saved
                                                                .creation
                                                                .as_ref()
                                                                .map_or(1, |request| request.race),
                                                            &saved.skills,
                                                            &saved.progression,
                                                            now_ms,
                                                        )
                                                        .map_err(io::Error::other)?;
                                                    let next = cast.next().clone();
                                                    let mut events =
                                                        next.private_changes(cast.previous());
                                                    events.push(
                                                        crate::ui::skill_update(
                                                            skill,
                                                            saved.skills.get(skill).unwrap(),
                                                        )
                                                        .map_err(other)?,
                                                    );
                                                    events.push(
                                                        script::chat(
                                                            &format!(
                                                                "You recover {} health.",
                                                                cast.healed()
                                                            ),
                                                            1,
                                                        )
                                                        .map_err(other)?,
                                                    );
                                                    peer.validate_scene_batch(&events)?;
                                                    store.progression_batch(
                                                        &peer.session.account,
                                                        character,
                                                        cast.commands(),
                                                    )?;
                                                    cast.commit();
                                                    deliveries.extend(
                                                        scene.player_stats(character, &next),
                                                    );
                                                    peer.queue_ordered_batch(events)?;
                                                    Ok(())
                                                })(
                                                );
                                                if let Err(error) = result {
                                                    replies.extend(
                                                        peer.send_event(
                                                            &script::chat(&error.to_string(), 1)
                                                                .map_err(other)?,
                                                            now,
                                                        )?,
                                                    );
                                                }
                                                return Ok(replies);
                                            }
                                            if config.initialize_ui
                                                && config.training_kit
                                                && skill == crate::skills::TRAINING_ATTACK
                                            {
                                                let saved = &store.characters[&character];
                                                let attack = (|| -> io::Result<()> {
                                                    if saved.skills.get(skill).is_none() {
                                                        return Err(invalid(
                                                            "The development training attack is not learned.",
                                                        ));
                                                    }
                                                    if !saved.inventory.items().any(|item| {
                                                        matches!(
                                                            item.location,
                                                            crate::inventory::Location::Equipped(
                                                                0x40
                                                            )
                                                        )
                                                    }) {
                                                        return Err(invalid(
                                                            "Equip a main-hand item before using the training attack.",
                                                        ));
                                                    }
                                                    let events = vec![
                                                        crate::ui::skill_update(
                                                            skill,
                                                            saved.skills.get(skill).unwrap(),
                                                        )
                                                        .map_err(other)?,
                                                        script::private_call(
                                                            script::COMBAT_STATE,
                                                            &script::integer(1),
                                                        )
                                                        .map_err(other)?,
                                                    ];
                                                    peer.validate_scene_batch(&events)?;
                                                    encounters
                                                        .as_mut()
                                                        .ok_or_else(|| invalid("Combat rules are not enabled on this server."))?
                                                        .once(character, target, now_ms, &scene, &store)?;
                                                    peer.queue_ordered_batch(events)?;
                                                    peer.combat = true;
                                                    peer.target = Some(target);
                                                    Ok(())
                                                })(
                                                );
                                                if let Err(error) = attack {
                                                    replies.extend(
                                                        peer.send_event(
                                                            &script::chat(&error.to_string(), 1)
                                                                .map_err(other)?,
                                                            now,
                                                        )?,
                                                    );
                                                }
                                                return Ok(replies);
                                            }
                                            if !config.initialize_ui
                                                || !config.lifestones
                                                || skill != crate::skills::LIFESTONE_RECALL
                                            {
                                                replies.extend(peer.send_event(&script::chat("This active skill is not available in the current server profile.",1).map_err(other)?,now)?);
                                                return Ok(replies);
                                            }
                                            if store.characters[&character]
                                                .skills
                                                .get(skill)
                                                .is_none()
                                            {
                                                replies.extend(peer.send_event(&script::chat("Bind to a lifestone to unlock development Recall.",1).map_err(other)?,now)?);
                                                return Ok(replies);
                                            }
                                            match recall_character(
                                                peer,
                                                character,
                                                &mut store,
                                                &mut scene,
                                                &mut encounters,
                                                &mut bootstrap,
                                                now_ms,
                                            ) {
                                                Ok(updates) => deliveries.extend(updates),
                                                Err(error) => replies.extend(
                                                    peer.send_event(
                                                        &script::chat(&error.to_string(), 1)
                                                            .map_err(other)?,
                                                        now,
                                                    )?,
                                                ),
                                            }
                                        }
                                        script::Request::Examine(target) => {
                                            if !config.initialize_ui {
                                                return Err(invalid(
                                                    "examination requires initialized player UI",
                                                ));
                                            }
                                            if peer.last_examine_ms.is_some_and(|last| {
                                                now_ms.saturating_sub(last) < 100
                                            }) {
                                                return Err(invalid(
                                                    "examination request cooldown",
                                                ));
                                            }
                                            peer.last_examine_ms = Some(now_ms);
                                            match examine_target(
                                                character,
                                                target,
                                                &store,
                                                &scene,
                                                &examination_names,
                                            ) {
                                                Ok(packet) => {
                                                    deliveries.push(crate::scene::Delivery {
                                                        recipient: character,
                                                        data: packet.into(),
                                                    })
                                                }
                                                Err(error) => replies.extend(
                                                    peer.send_event(
                                                        &script::chat(&error.to_string(), 1)
                                                            .map_err(other)?,
                                                        now,
                                                    )?,
                                                ),
                                            }
                                        }
                                        script::Request::UseObject(request) => {
                                            if TRAINING_MENTORS.contains(&request.item)
                                                && config.training_kit
                                                && training_quest_enabled
                                            {
                                                if request.owner != character {
                                                    return Err(invalid(
                                                        "Mentor use owner mismatch.",
                                                    ));
                                                }
                                                match use_training_mentor(
                                                    peer,
                                                    character,
                                                    request.item,
                                                    &mut store,
                                                    &mut scene,
                                                ) {
                                                    Ok(updates) => deliveries.extend(updates),
                                                    Err(error) => replies.extend(
                                                        peer.send_event(
                                                            &script::chat(&error.to_string(), 1)
                                                                .map_err(other)?,
                                                            now,
                                                        )?,
                                                    ),
                                                }
                                                return Ok(replies);
                                            }
                                            if matches!(
                                                request.item,
                                                TRAINING_MERCHANT | TRAINING_MERCHANT_OUTDOOR
                                            ) && config.training_kit
                                            {
                                                if request.owner != character {
                                                    return Err(invalid(
                                                        "Merchant use owner mismatch.",
                                                    ));
                                                }
                                                match merchant_action(
                                                    peer,
                                                    character,
                                                    crate::merchants::Action::List,
                                                    &mut store,
                                                    &mut scene,
                                                    &inventory_metadata,
                                                    merchant_catalog.as_ref().ok_or_else(|| {
                                                        invalid("Merchant catalog is not enabled.")
                                                    })?,
                                                    &mut next_offer,
                                                    now_ms,
                                                ) {
                                                    Ok(updates) => deliveries.extend(updates),
                                                    Err(error) => replies.extend(
                                                        peer.send_event(
                                                            &script::chat(&error.to_string(), 1)
                                                                .map_err(other)?,
                                                            now,
                                                        )?,
                                                    ),
                                                }
                                                return Ok(replies);
                                            }
                                            if store.characters[&character].inventory.items().any(
                                                |item| {
                                                    item.id == request.item
                                                        && matches!(
                                                            item.definition,
                                                            crate::item_services::POTION_DEFINITION
                                                                | crate::crafting::VIGOR_OUTPUT
                                                        )
                                                },
                                            ) {
                                                if !config.initialize_ui
                                                    || !config.training_kit
                                                    || request.owner != character
                                                {
                                                    return Err(invalid(
                                                        "Training potion use is not enabled or not owned by this character.",
                                                    ));
                                                }
                                                match use_training_potion(
                                                    peer,
                                                    character,
                                                    request.item,
                                                    &mut store,
                                                    &mut scene,
                                                    now_ms,
                                                ) {
                                                    Ok(updates) => deliveries.extend(updates),
                                                    Err(error) => replies.extend(
                                                        peer.send_event(
                                                            &script::chat(&error.to_string(), 1)
                                                                .map_err(other)?,
                                                            now,
                                                        )?,
                                                    ),
                                                }
                                                return Ok(replies);
                                            }
                                            if let Some(route) = portal_routes.get(request.item) {
                                                if request.owner != character {
                                                    return Err(invalid(
                                                        "portal owner differs from active character",
                                                    ));
                                                }
                                                match use_portal(
                                                    peer,
                                                    character,
                                                    request.item,
                                                    route,
                                                    &mut store,
                                                    &mut scene,
                                                    &mut encounters,
                                                    &mut bootstrap,
                                                    now_ms,
                                                ) {
                                                    Ok(updates) => deliveries.extend(updates),
                                                    Err(error) => replies.extend(
                                                        peer.send_event(
                                                            &script::chat(&error.to_string(), 1)
                                                                .map_err(other)?,
                                                            now,
                                                        )?,
                                                    ),
                                                }
                                                return Ok(replies);
                                            }
                                            if config.portals
                                                && scene
                                                    .examination_content(character, request.item)
                                                    .is_some_and(|object| {
                                                        matches!(
                                                            object.entity,
                                                            0x4700084d
                                                                | 0x47001451
                                                                | 0x47001454
                                                                | 0x4700369a
                                                        )
                                                    })
                                            {
                                                replies.extend(peer.send_event(&script::chat("This portal's destination is unavailable.",1).map_err(other)?,now)?);
                                                return Ok(replies);
                                            }
                                            if config.lifestones
                                                && scene
                                                    .examination_content(character, request.item)
                                                    .is_some_and(|object| {
                                                        object.entity == 0x470007de
                                                    })
                                            {
                                                let result = (|| -> io::Result<Vec<Vec<u8>>> {
                                                    if request.owner != character
                                                        || peer.combat
                                                        || !store.characters[&character]
                                                            .progression
                                                            .alive()
                                                    {
                                                        return Err(invalid(
                                                            "Bind while alive and out of combat.",
                                                        ));
                                                    }
                                                    if peer.last_world_use_ms.is_some_and(|last| {
                                                        now_ms.saturating_sub(last) < 1000
                                                    }) {
                                                        return Err(invalid(
                                                            "Wait before using the lifestone again.",
                                                        ));
                                                    }
                                                    let player = scene
                                                        .player_placement(character)
                                                        .ok_or_else(|| {
                                                            invalid("Player is not ready.")
                                                        })?;
                                                    let stone = scene
                                                        .content_placement(request.item)
                                                        .ok_or_else(|| {
                                                            invalid("Lifestone is not available.")
                                                        })?;
                                                    let distance: f64 = player
                                                        .coordinates
                                                        .into_iter()
                                                        .zip(stone.coordinates)
                                                        .map(|(a, b)| {
                                                            (f64::from(a) - f64::from(b)).powi(2)
                                                        })
                                                        .sum();
                                                    if player.cell != stone.cell || distance > 16.0
                                                    {
                                                        return Err(invalid(
                                                            "Move within four meters of the lifestone.",
                                                        ));
                                                    }
                                                    let saved = &store.characters[&character];
                                                    let skills = saved
                                                        .skills
                                                        .with_lifestone_recall(
                                                            &saved.progression,
                                                            saved
                                                                .creation
                                                                .as_ref()
                                                                .map_or(1, |request| request.race),
                                                        )
                                                        .map_err(other)?;
                                                    let events = vec![crate::ui::skill_update(crate::skills::LIFESTONE_RECALL,skills.get(crate::skills::LIFESTONE_RECALL).unwrap()).map_err(other)?,crate::ui::lifestone_bound(character,request.item,player).map_err(other)?,script::chat("You are bound to this lifestone. Recall is available in your skill list; @recall also returns under development travel rules.",1).map_err(other)?];
                                                    peer.validate_scene_batch(&events)?;
                                                    store.bind_lifestone(
                                                        &peer.session.account,
                                                        character,
                                                        player,
                                                    )?;
                                                    peer.last_world_use_ms = Some(now_ms);
                                                    Ok(events)
                                                })(
                                                );
                                                match result {
                                                    Ok(events) => {
                                                        peer.queue_ordered_batch(events)?
                                                    }
                                                    Err(error) => replies.extend(
                                                        peer.send_event(
                                                            &script::chat(&error.to_string(), 1)
                                                                .map_err(other)?,
                                                            now,
                                                        )?,
                                                    ),
                                                }
                                                return Ok(replies);
                                            }
                                            let result = if config.initialize_ui {
                                                change_hand_item(
                                                    peer,
                                                    character,
                                                    &request,
                                                    None,
                                                    &mut store,
                                                    &mut scene,
                                                    &inventory_metadata,
                                                    now_ms,
                                                )
                                            } else {
                                                Err(invalid("Inventory is not initialized."))
                                            };
                                            match result {
                                                Ok(updates) => deliveries.extend(updates),
                                                Err(error) => replies.extend(
                                                    peer.send_event(
                                                        &script::chat(
                                                            &format!("Item use rejected: {error}"),
                                                            1,
                                                        )
                                                        .map_err(other)?,
                                                        now,
                                                    )?,
                                                ),
                                            }
                                        }
                                        script::Request::UnEquip(request) => {
                                            let result =
                                                (|| -> io::Result<Vec<crate::scene::Delivery>> {
                                                    if !config.initialize_ui {
                                                        return Err(invalid(
                                                            "Inventory is not initialized.",
                                                        ));
                                                    }
                                                    let slot = u16::try_from(request.slot)
                                                        .map_err(|_| {
                                                            invalid("Invalid equipment slot.")
                                                        })?;
                                                    let destination =
                                                        u16::try_from(request.destination)
                                                            .map_err(|_| {
                                                                invalid(
                                                                    "Invalid backpack position.",
                                                                )
                                                            })?;
                                                    let item = crate::equipment::UseRequest {
                                                        owner: request.owner,
                                                        item: request.item,
                                                    };
                                                    change_hand_item(
                                                        peer,
                                                        character,
                                                        &item,
                                                        Some((slot, destination)),
                                                        &mut store,
                                                        &mut scene,
                                                        &inventory_metadata,
                                                        now_ms,
                                                    )
                                                })(
                                                );
                                            match result {
                                                Ok(updates) => deliveries.extend(updates),
                                                Err(error) => replies.extend(
                                                    peer.send_event(
                                                        &script::chat(
                                                            &format!("Unequip rejected: {error}"),
                                                            1,
                                                        )
                                                        .map_err(other)?,
                                                        now,
                                                    )?,
                                                ),
                                            }
                                        }
                                        script::Request::InventoryMove(request) => {
                                            if peer.last_inventory_ms.is_some_and(|last| {
                                                now_ms.saturating_sub(last) < 100
                                            }) {
                                                return Err(invalid("inventory action cooldown"));
                                            }
                                            let result = (|| -> io::Result<(crate::inventory::Command, Vec<u8>)> {
                                                if !config.initialize_ui { return Err(invalid("Inventory is not initialized.")); }
                                                let saved = &store.characters[&character];
                                                if !saved.progression.alive() { return Err(invalid("You cannot move items while dead.")); }
                                                let command = request.command(character).map_err(other)?;
                                                let candidate = saved.inventory.apply(&command).map_err(other)?;
                                                let item = candidate.items().find(|item| item.id == request.item).ok_or_else(||invalid("Item is not owned."))?;
                                                let metadata = inventory_metadata.get(&item.definition).and_then(|entry| *entry).ok_or_else(||invalid("Bag sorting is unavailable for this item definition."))?;
                                                let acknowledgment = request.acknowledgment(item, metadata).map_err(other)?;
                                                Ok((command, acknowledgment))
                                            })();
                                            match result {
                                                Ok((command, acknowledgment)) => {
                                                    store.inventory_command(
                                                        &peer.session.account,
                                                        character,
                                                        command,
                                                    )?;
                                                    peer.last_inventory_ms = Some(now_ms);
                                                    deliveries.push(crate::scene::Delivery {
                                                        recipient: character,
                                                        data: acknowledgment.into(),
                                                    });
                                                }
                                                Err(error) => replies.extend(
                                                    peer.send_event(
                                                        &script::chat(
                                                            &format!("Item move rejected: {error}"),
                                                            1,
                                                        )
                                                        .map_err(other)?,
                                                        now,
                                                    )?,
                                                ),
                                            }
                                        }
                                        script::Request::Resurrect => {
                                            match resurrect_character(
                                                peer,
                                                character,
                                                &mut store,
                                                &mut scene,
                                                &mut encounters,
                                                config.lifestones,
                                                &mut bootstrap,
                                            ) {
                                                Ok(updates) => deliveries.extend(updates),
                                                Err(error) => replies.extend(
                                                    peer.send_event(
                                                        &script::chat(&error.to_string(), 1)
                                                            .map_err(other)?,
                                                        now,
                                                    )?,
                                                ),
                                            }
                                        }
                                        script::Request::Skill { action, skill } => {
                                            if !config.initialize_ui {
                                                replies.extend(peer.send_event(&script::chat("Skills are not available in this session.",1).map_err(other)?,now)?);
                                                return Ok(replies);
                                            }
                                            if action == crate::skills::Action::Raise {
                                                if peer.last_inventory_ms.is_some_and(|last| {
                                                    now_ms.saturating_sub(last) < 100
                                                }) {
                                                    return Err(invalid(
                                                        "skill advancement cooldown",
                                                    ));
                                                }
                                                let saved = &store.characters[&character];
                                                let race = saved
                                                    .creation
                                                    .as_ref()
                                                    .map_or(1, |request| request.race);
                                                match saved.skills.advance(
                                                    skill,
                                                    &saved.progression,
                                                    race,
                                                ) {
                                                    Ok((skills, progression)) => {
                                                        let mut events = progression
                                                            .private_updates()
                                                            .into_iter()
                                                            .filter(|wire| {
                                                                wire[..4]
                                                                    == 0x100d4u32.to_le_bytes()
                                                            })
                                                            .collect::<Vec<_>>();
                                                        events.push(
                                                            crate::ui::skill_update(
                                                                skill,
                                                                skills.get(skill).unwrap(),
                                                            )
                                                            .map_err(other)?,
                                                        );
                                                        peer.validate_scene_batch(&events)?;
                                                        store.raise_skill(
                                                            &peer.session.account,
                                                            character,
                                                            skill,
                                                        )?;
                                                        peer.last_inventory_ms = Some(now_ms);
                                                        peer.queue_ordered_batch(events)?;
                                                    }
                                                    Err(error) => replies.extend(
                                                        peer.send_event(
                                                            &script::chat(error.message(), 1)
                                                                .map_err(other)?,
                                                            now,
                                                        )?,
                                                    ),
                                                }
                                                return Ok(replies);
                                            }
                                            let command = match action {
                                                crate::skills::Action::Train => {
                                                    crate::skills::Command::Train(skill)
                                                }
                                                crate::skills::Action::Untrain => {
                                                    crate::skills::Command::Untrain(skill)
                                                }
                                                crate::skills::Action::Raise => {
                                                    return Err(invalid(
                                                        "advancement dispatch invariant",
                                                    ));
                                                }
                                            };
                                            let saved = &store.characters[&character];
                                            let race = saved
                                                .creation
                                                .as_ref()
                                                .map_or(1, |request| request.race);
                                            if let Err(error) = saved.skills.apply(
                                                command,
                                                &saved.progression,
                                                race,
                                            ) {
                                                replies.extend(
                                                    peer.send_event(
                                                        &script::chat(error.message(), 1)
                                                            .map_err(other)?,
                                                        now,
                                                    )?,
                                                );
                                                return Ok(replies);
                                            }
                                            store.skill_command(
                                                &peer.session.account,
                                                character,
                                                command,
                                            )?;
                                            let saved = &store.characters[&character];
                                            let wire = match action {
                                                crate::skills::Action::Train => {
                                                    crate::ui::skill_update(
                                                        skill,
                                                        saved.skills.get(skill).unwrap(),
                                                    )
                                                }
                                                _ => crate::ui::skill_remove(skill),
                                            }
                                            .map_err(other)?;
                                            deliveries.push(crate::scene::Delivery {
                                                recipient: character,
                                                data: wire.into(),
                                            });
                                            deliveries.push(crate::scene::Delivery {
                                                recipient: character,
                                                data: crate::ui::skill_credits(
                                                    saved
                                                        .skills
                                                        .available_credits(&saved.progression),
                                                )
                                                .map_err(other)?
                                                .into(),
                                            });
                                        }
                                        script::Request::UiSettings(settings) => {
                                            store.update_ui(
                                                &peer.session.account,
                                                character,
                                                settings,
                                            )?;
                                        }
                                        script::Request::Hotbar(update) => {
                                            store.update_hotbar(
                                                &peer.session.account,
                                                character,
                                                update,
                                            )?;
                                        }
                                        script::Request::Target(target) => {
                                            if target.is_some_and(|target| {
                                                !scene.can_observe(character, target)
                                            }) {
                                                return Err(invalid("target is not visible"));
                                            }
                                            let packet = match target {
                                                Some(target) => selection_status(
                                                    character, target, &scene, &store,
                                                )?,
                                                None => crate::ui::selection(u64::MAX, None)
                                                    .map_err(other)?,
                                            };
                                            peer.queue_ordered_batch(vec![packet])?;
                                            peer.target = target;
                                        }
                                        request @ (script::Request::Combat(_)
                                        | script::Request::Attack(_)) => {
                                            let enabled = match request {
                                                script::Request::Combat(enabled) => {
                                                    if enabled
                                                        && !store.characters[&character]
                                                            .progression
                                                            .alive()
                                                    {
                                                        replies.extend(
                                                            peer.send_event(
                                                                &script::chat(
                                                                    "You cannot fight while dead.",
                                                                    1,
                                                                )
                                                                .map_err(other)?,
                                                                now,
                                                            )?,
                                                        );
                                                        return Ok(replies);
                                                    }
                                                    if !enabled {
                                                        if let Some(runtime) = &mut encounters {
                                                            runtime.stop(character);
                                                        }
                                                    }
                                                    enabled
                                                }
                                                script::Request::Attack(target) => {
                                                    if target.is_some_and(|target| {
                                                        target == character
                                                            || !scene.can_observe(character, target)
                                                    }) {
                                                        return Err(invalid(
                                                            "attack target is not a visible other object",
                                                        ));
                                                    }
                                                    if let Some(target) = target {
                                                        let started=encounters.as_mut().ok_or_else(||invalid("Combat rules are not enabled on this server."))
                                                            .and_then(|runtime|runtime.start(character,target,now_ms,&scene,&store));
                                                        if let Err(error) = started {
                                                            if let Some(runtime) = &mut encounters {
                                                                runtime.stop(character);
                                                            }
                                                            peer.combat = false;
                                                            peer.target = None;
                                                            replies.extend(
                                                                peer.send_event(
                                                                    &script::chat(
                                                                        &error.to_string(),
                                                                        1,
                                                                    )
                                                                    .map_err(other)?,
                                                                    now,
                                                                )?,
                                                            );
                                                            replies.extend(
                                                                peer.send_event(
                                                                    &script::private_call(
                                                                        script::COMBAT_STATE,
                                                                        &script::integer(
                                                                            u32::from(peer.combat),
                                                                        ),
                                                                    )
                                                                    .map_err(other)?,
                                                                    now,
                                                                )?,
                                                            );
                                                            return Ok(replies);
                                                        }
                                                    } else if let Some(runtime) = &mut encounters {
                                                        runtime.stop(character);
                                                    }
                                                    peer.target = target;
                                                    true
                                                }
                                                _ => unreachable!(),
                                            };
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
                                                    if matches!(
                                                        text.trim().to_ascii_lowercase().as_str(),
                                                        "@source" | "@license"
                                                    ) {
                                                        peer.queue_ordered_batch(vec![
                                                            script::chat(
                                                                &crate::licensing::notice(),
                                                                1,
                                                            )
                                                            .map_err(other)?,
                                                        ])?;
                                                        return Ok(replies);
                                                    }
                                                    if let Some(action) =
                                                        crate::merchants::parse_command(&text)
                                                    {
                                                        peer.last_chat_ms = Some(now_ms);
                                                        let result = action.map_err(io::Error::other).and_then(|action| {
                                                            if !config.training_kit { return Err(invalid("The training quartermaster is not enabled.")); }
                                                            merchant_action(peer, character, action, &mut store, &mut scene, &inventory_metadata, merchant_catalog.as_ref().ok_or_else(|| invalid("Merchant catalog is not enabled."))?, &mut next_offer, now_ms)
                                                        });
                                                        match result {
                                                            Ok(updates) => {
                                                                deliveries.extend(updates)
                                                            }
                                                            Err(error) => replies.extend(
                                                                peer.send_event(
                                                                    &script::chat(
                                                                        &error.to_string(),
                                                                        1,
                                                                    )
                                                                    .map_err(other)?,
                                                                    now,
                                                                )?,
                                                            ),
                                                        }
                                                        return Ok(replies);
                                                    }
                                                    let words =
                                                        text.split_whitespace().collect::<Vec<_>>();
                                                    if words.first().is_some_and(|word| {
                                                        word.eq_ignore_ascii_case("@craft")
                                                    }) {
                                                        peer.last_chat_ms = Some(now_ms);
                                                        let result = (|| -> io::Result<Vec<crate::scene::Delivery>> {
                                                            if !config.initialize_ui || !config.training_kit {
                                                                return Err(invalid("Training crafting is not enabled."));
                                                            }
                                                            let recipe = match words.as_slice() {
                                                                [_, name] if name.eq_ignore_ascii_case("health") => crate::crafting::HEALTH_RECIPE,
                                                                [_, name] if name.eq_ignore_ascii_case("vigor") => crate::crafting::VIGOR_RECIPE,
                                                                _ => return Err(invalid("Use @craft health or @craft vigor. Training recipes consume one Mucor Mushroom and 100 Tallow; no craft skill or chance roll is required.")),
                                                            };
                                                            let request = crate::crafting::select_inventory(recipe, &store.characters[&character].inventory, true)
                                                                .map_err(|_| invalid("You need one Mucor Mushroom and a full 100-unit Tallow bundle in your bag."))?;
                                                            craft_item(peer, character, &request, &mut store, &mut scene, &inventory_metadata, now_ms)
                                                        })();
                                                        match result {
                                                            Ok(updates) => {
                                                                deliveries.extend(updates)
                                                            }
                                                            Err(error) => replies.extend(
                                                                peer.send_event(
                                                                    &script::chat(
                                                                        &error.to_string(),
                                                                        1,
                                                                    )
                                                                    .map_err(other)?,
                                                                    now,
                                                                )?,
                                                            ),
                                                        }
                                                        return Ok(replies);
                                                    }
                                                    if words.first().is_some_and(|word| {
                                                        word.eq_ignore_ascii_case("@group")
                                                    }) && config.initialize_ui
                                                    {
                                                        peer.last_chat_ms = Some(now_ms);
                                                        let online = group_online(
                                                            &active_characters,
                                                            &store,
                                                            &scene,
                                                        );
                                                        let verb =
                                                            words.get(1).copied().unwrap_or("");
                                                        let name =
                                                            words.get(2..).unwrap_or(&[]).join(" ");
                                                        let target = online
                                                            .values()
                                                            .find(|member| {
                                                                member
                                                                    .name
                                                                    .eq_ignore_ascii_case(&name)
                                                            })
                                                            .map(|member| member.id);
                                                        let request = match verb
                                                            .to_ascii_lowercase()
                                                            .as_str()
                                                        {
                                                            "create" if !name.is_empty() => Some(
                                                                crate::groups::Request::Create {
                                                                    name,
                                                                    looting: 2,
                                                                    social: true,
                                                                },
                                                            ),
                                                            "invite" => target.map(
                                                                crate::groups::Request::Invite,
                                                            ),
                                                            "accept" => target.map(
                                                                crate::groups::Request::Accept,
                                                            ),
                                                            "decline" => target.map(
                                                                crate::groups::Request::Decline,
                                                            ),
                                                            "dismiss" => target.map(
                                                                crate::groups::Request::Dismiss,
                                                            ),
                                                            "leave" if name.is_empty() => {
                                                                Some(crate::groups::Request::Leave)
                                                            }
                                                            "disband" if name.is_empty() => Some(
                                                                crate::groups::Request::Disband,
                                                            ),
                                                            _ => None,
                                                        };
                                                        if let Some(request) = request {
                                                            if group_requests.len() >= 4 {
                                                                return Err(invalid(
                                                                    "Too many group requests in one packet.",
                                                                ));
                                                            }
                                                            group_requests
                                                                .push((character, request));
                                                        } else {
                                                            replies.extend(peer.send_event(&script::chat("Groups use individual XP and personal rewards. Commands: @group create NAME; invite/accept/decline/dismiss PLAYER; leave; disband. Use @g MESSAGE for group chat. Named players must be online.", 1).map_err(other)?, now)?);
                                                        }
                                                        return Ok(replies);
                                                    }
                                                    if words.first().is_some_and(|word| {
                                                        word.eq_ignore_ascii_case("@g")
                                                    }) && config.initialize_ui
                                                    {
                                                        peer.last_chat_ms = Some(now_ms);
                                                        let members = groups.members(character);
                                                        if members.is_empty() || words.len() < 2 {
                                                            replies.extend(peer.send_event(&script::chat("Join a group and use @g MESSAGE.", 1).map_err(other)?, now)?);
                                                        } else {
                                                            let packet: Arc<[u8]> = script::chat(
                                                                &format!(
                                                                    "[Group] {name}: {}",
                                                                    words[1..].join(" ")
                                                                ),
                                                                0x10,
                                                            )
                                                            .map_err(other)?
                                                            .into();
                                                            for recipient in members {
                                                                deliveries.push(
                                                                    crate::scene::Delivery {
                                                                        recipient,
                                                                        data: packet.clone(),
                                                                    },
                                                                );
                                                            }
                                                        }
                                                        return Ok(replies);
                                                    }
                                                    if words.first().is_some_and(|word| {
                                                        word.eq_ignore_ascii_case("@quest")
                                                    }) && config.initialize_ui
                                                    {
                                                        peer.last_chat_ms = Some(now_ms);
                                                        if !training_quest_enabled
                                                            && store.characters[&character]
                                                                .quests
                                                                .get()
                                                                .is_none()
                                                        {
                                                            replies.extend(peer.send_event(&script::chat("The training quest is not enabled on this server.",1).map_err(other)?,now)?);
                                                            return Ok(replies);
                                                        }
                                                        let action = words[1..].join(" ");
                                                        match quest_action(
                                                            peer, character, &action, &mut store,
                                                            &mut scene,
                                                        ) {
                                                            Ok(updates) => {
                                                                deliveries.extend(updates)
                                                            }
                                                            Err(error) => replies.extend(
                                                                peer.send_event(
                                                                    &script::chat(
                                                                        &error.to_string(),
                                                                        1,
                                                                    )
                                                                    .map_err(other)?,
                                                                    now,
                                                                )?,
                                                            ),
                                                        }
                                                        return Ok(replies);
                                                    }
                                                    if text.trim().eq_ignore_ascii_case("@recall")
                                                        && config.lifestones
                                                    {
                                                        peer.last_chat_ms = Some(now_ms);
                                                        match recall_character(
                                                            peer,
                                                            character,
                                                            &mut store,
                                                            &mut scene,
                                                            &mut encounters,
                                                            &mut bootstrap,
                                                            now_ms,
                                                        ) {
                                                            Ok(updates) => {
                                                                deliveries.extend(updates)
                                                            }
                                                            Err(error) => replies.extend(
                                                                peer.send_event(
                                                                    &script::chat(
                                                                        &error.to_string(),
                                                                        1,
                                                                    )
                                                                    .map_err(other)?,
                                                                    now,
                                                                )?,
                                                            ),
                                                        }
                                                        return Ok(replies);
                                                    }
                                                    if text
                                                        .trim()
                                                        .eq_ignore_ascii_case("@resurrect")
                                                        && encounters.is_some()
                                                    {
                                                        peer.last_chat_ms = Some(now_ms);
                                                        match resurrect_character(
                                                            peer,
                                                            character,
                                                            &mut store,
                                                            &mut scene,
                                                            &mut encounters,
                                                            config.lifestones,
                                                            &mut bootstrap,
                                                        ) {
                                                            Ok(updates) => {
                                                                deliveries.extend(updates)
                                                            }
                                                            Err(error) => replies.extend(
                                                                peer.send_event(
                                                                    &script::chat(
                                                                        &error.to_string(),
                                                                        1,
                                                                    )
                                                                    .map_err(other)?,
                                                                    now,
                                                                )?,
                                                            ),
                                                        }
                                                        return Ok(replies);
                                                    }
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
                                    if let Phase::InWorld { character }
                                    | Phase::Entering { character } = peer.session.phase
                                    {
                                        if !store.characters[&character].progression.alive() {
                                            return Err(invalid(
                                                "dead character movement rejected",
                                            ));
                                        }
                                    }
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
                                    scene.prepare_cell(update.placement.cell)?;
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
                                                if config.training_kit {
                                                    let saved = &store.characters[&character_id];
                                                    let can_grant = saved.progression.alive()
                                                        && (saved.skills.entries().count() < 128
                                                            || saved
                                                                .skills
                                                                .get(crate::skills::TRAINING_ATTACK)
                                                                .is_some());
                                                    if can_grant
                                                        && (saved.training_kit
                                                            || saved
                                                                .inventory
                                                                .free_slot()
                                                                .is_some())
                                                    {
                                                        if store.grant_training_kit(
                                                            &peer.session.account,
                                                            character_id,
                                                        )? {
                                                            eprintln!(
                                                                "Development training kit saved for character {character_id}"
                                                            );
                                                        }
                                                        store.ensure_training_attack(
                                                            &peer.session.account,
                                                            character_id,
                                                        )?;
                                                    } else {
                                                        eprintln!(
                                                            "Development training kit deferred for character {character_id}: dead, bag full or skill capacity"
                                                        );
                                                    }
                                                }
                                                if config.lifestones {
                                                    let saved = &store.characters[&character_id];
                                                    if saved.binding.is_some()
                                                        && saved.progression.alive()
                                                        && saved
                                                            .skills
                                                            .get(crate::skills::LIFESTONE_RECALL)
                                                            .is_none()
                                                        && saved.skills.entries().count() < 128
                                                    {
                                                        store.ensure_lifestone_recall(
                                                            &peer.session.account,
                                                            character_id,
                                                        )?;
                                                    }
                                                }
                                                if training_quest_enabled
                                                    && store.characters[&character_id]
                                                        .quests
                                                        .get()
                                                        .is_none()
                                                    && store.characters[&character_id]
                                                        .progression
                                                        .alive()
                                                {
                                                    store.quest_command(
                                                        &peer.session.account,
                                                        character_id,
                                                        crate::quests::Command::Accept,
                                                    )?;
                                                }
                                                let instance = store.begin_instance(
                                                    &peer.session.account,
                                                    character_id,
                                                )?;
                                                let events = profile
                                                    .events_at_state(
                                                        character_id,
                                                        &request,
                                                        placement,
                                                        instance,
                                                        &store.characters[&character_id]
                                                            .progression,
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
                                                peer.clear_scene_outbox();
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
                                                        .map_err(other)?
                                                        .with_progression(
                                                            &store.characters[&character_id]
                                                                .progression,
                                                        )
                                                        .with_hands(store.characters[&character_id].inventory.items()
                                                            .filter(|item| matches!(item.location, crate::inventory::Location::Equipped(_)))
                                                            .filter(|item| inventory_metadata.get(&item.definition).and_then(|entry| *entry).and_then(|entry| entry.equipment).is_some_and(|hand| hand.hand_slot(item).is_some()))
                                                            .map(|item| inventory_spawn(item, character_id, instance, instance, &inventory_metadata))
                                                            .collect::<io::Result<Vec<_>>>()?)
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
                                    peer.clear_scene_outbox();
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
                                    if first {
                                        if let Some(packet) = initial_ui.as_deref() {
                                            let saved = match peer.session.phase {
                                                Phase::Entering { character } => {
                                                    store.characters.get(&character).filter(|c| {
                                                        c.ui.is_some()
                                                            || !c.hotbar.is_empty()
                                                            || c.inventory.items().next().is_some()
                                                            || c.progression != crate::progression::Progression::default()
                                                            || !c.skills.is_empty()
                                                            || c.quests.get().is_some()
                                                    })
                                                }
                                                _ => None,
                                            };
                                            if let Some(saved) = saved {
                                                let restored = crate::ui::player_all_quests(
                                                    saved.ui.as_ref(),
                                                    Some(&saved.hotbar),
                                                    Some(&saved.inventory),
                                                    Some(&saved.progression),
                                                    Some(&saved.skills),
                                                    Some(&saved.quests),
                                                )
                                                .map_err(other)?;
                                                let instance=peer.motion.as_ref().ok_or_else(||invalid("inventory bootstrap without avatar lifetime"))?.instance;
                                                let mut events = saved
                                                    .inventory
                                                    .items()
                                                    .map(|item| {
                                                        inventory_spawn(
                                                            item,
                                                            saved.id,
                                                            instance,
                                                            instance,
                                                            &inventory_metadata,
                                                        )
                                                    })
                                                    .collect::<io::Result<Vec<_>>>()?;
                                                events.push(restored);
                                                peer.queue_event_batch(events)?;
                                            } else {
                                                peer.queue_scene(Arc::from(packet))?;
                                            }
                                        }
                                    }
                                    peer.session.scene_complete().map_err(other)?;
                                    if let Phase::InWorld { character } = peer.session.phase {
                                        deliveries
                                            .extend(scene.activate(character).map_err(other)?);
                                        if first {
                                            deliveries.push(crate::scene::Delivery {
                                                recipient: character,
                                                data: script::chat(&crate::licensing::notice(), 1)
                                                    .map_err(other)?
                                                    .into(),
                                            });
                                        }
                                        if first && config.initialize_ui {
                                            for packet in
                                                crate::crafting::initialization(config.training_kit)
                                                    .map_err(other)?
                                            {
                                                deliveries.push(crate::scene::Delivery {
                                                    recipient: character,
                                                    data: packet.into(),
                                                });
                                            }
                                        }
                                        if first && config.training_kit {
                                            deliveries.push(crate::scene::Delivery { recipient: character, data: script::chat(
                                                "Training: equip the weapon from your bag. Defeat three Idol Spawn, then speak to Pectar, Training Master, for your reward. Speak to Tebor the Armorer to open the shop. Open Crafting to make potions with 1 Mucor Mushroom and 100 Tallow. The portals lead outside and back. Training uses authored rules.", 1).map_err(other)?.into() });
                                        }
                                        eprintln!(
                                            "Scene ready for character {character}: cell={:08x}, visible content={}",
                                            scene
                                                .player_placement(character)
                                                .map_or(0, |point| point.cell),
                                            scene.visible_content_count(character).unwrap_or(0)
                                        );
                                        if first
                                            && !store.characters[&character].progression.alive()
                                        {
                                            for wire in [
                                                crate::ui::death_notice("You have fallen.")
                                                    .map_err(other)?,
                                                crate::ui::lock_player(true).map_err(other)?,
                                            ] {
                                                deliveries.push(crate::scene::Delivery {
                                                    recipient: character,
                                                    data: wire.into(),
                                                });
                                            }
                                        }
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
                                let reason: String = error.to_string().chars().take(512).collect();
                                if diagnosed_application_errors.len() < 32
                                    && diagnosed_application_errors.insert(reason)
                                {
                                    eprintln!("application event rejected: {error}");
                                }
                            }
                        }
                    }
                    Ok(replies)
                })();
                for packet in recovery_packets {
                    primary.send_to(&packet, addr)?;
                }
                match result {
                    Ok(replies) => {
                        for reply in replies {
                            primary.send_to(&reply, addr)?;
                        }
                    }
                    Err(error) => {
                        counters.rejected += 1;
                        let reason: String = error.to_string().chars().take(512).collect();
                        if diagnosed_packet_errors.len() < 32
                            && diagnosed_packet_errors.insert(reason)
                        {
                            eprintln!(
                                "packet rejected: flags={:#x} sequence={} bytes={} reason={error} state={}",
                                h.flags,
                                h.sequence,
                                len,
                                peers
                                    .get(&addr)
                                    .map(|peer| peer.transport.diagnostic())
                                    .unwrap_or_else(|| "no peer".into())
                            );
                        }
                    }
                }
                for (character, request) in group_requests {
                    let online = group_online(&active_characters, &store, &scene);
                    let result = (|| -> io::Result<()> {
                        let plan = groups
                            .prepare(character, &request, now_ms, &online)
                            .map_err(io::Error::other)?;
                        let packets = group_admit(plan.deliveries(), &active_characters, &peers)?;
                        groups.commit(plan).map_err(io::Error::other)?;
                        group_publish(packets, &mut peers)
                    })();
                    if let Err(error) = result {
                        if let Some(peer) = active_characters
                            .get(&character)
                            .and_then(|addr| peers.get_mut(addr))
                        {
                            if peer
                                .queue_ordered_batch(vec![
                                    script::chat(&error.to_string(), 1).map_err(other)?,
                                ])
                                .is_err()
                            {
                                failed_peers.insert(addr);
                            }
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
        if let Some(runtime) = &mut encounters {
            let events = runtime.tick(
                now_ms,
                &scene,
                &mut store,
                active_characters.keys().copied(),
            )?;
            let mut updates = Vec::new();
            for event in events {
                match event {
                    crate::encounters::Event::Item {
                        player,
                        item,
                        instance,
                    } => {
                        let metadata = inventory_metadata
                            .get(&item.definition)
                            .and_then(|entry| *entry)
                            .ok_or_else(|| {
                                invalid("Encounter item metadata was not initialized")
                            })?;
                        let spawn =
                            world::create_inventory_item(&item, player, instance).map_err(other)?;
                        let acquired = crate::item_services::acquired(player, &item, metadata)
                            .map_err(other)?;
                        scene
                            .inventory_granted(player, item.id, instance)
                            .map_err(other)?;
                        updates.push(crate::scene::Delivery {
                            recipient: player,
                            data: spawn.into(),
                        });
                        updates.push(crate::scene::Delivery {
                            recipient: player,
                            data: acquired.into(),
                        });
                        updates.push(crate::scene::Delivery {
                            recipient: player,
                            data: script::chat(
                                "Training reward: a health potion was placed in your bag.",
                                1,
                            )
                            .map_err(other)?
                            .into(),
                        });
                    }
                    crate::encounters::Event::Quest { player, book } => {
                        updates.push(crate::scene::Delivery {
                            recipient: player,
                            data: book.update().map_err(other)?.into(),
                        });
                        if book.ready() {
                            updates.push(crate::scene::Delivery {recipient:player,data:script::chat("Training objective complete. Leave combat and type @quest claim for your reward.",1).map_err(other)?.into()});
                        }
                    }
                    crate::encounters::Event::Health { actor, health } => {
                        updates.extend(scene.content_stat(actor, 0x102, health)?);
                        for (&recipient, addr) in &active_characters {
                            if peers
                                .get(addr)
                                .is_some_and(|peer| peer.target == Some(actor))
                                && scene.can_observe(recipient, actor)
                            {
                                updates.push(crate::scene::Delivery {
                                    recipient,
                                    data: selection_status(recipient, actor, &scene, &store)?
                                        .into(),
                                });
                            }
                        }
                    }
                    crate::encounters::Event::Despawn(actor) => {
                        updates.extend(scene.despawn_content(actor)?)
                    }
                    crate::encounters::Event::Respawn { actor, health } => {
                        scene.content_stat(actor, 0x102, health)?;
                        updates.extend(scene.respawn_content(actor)?);
                    }
                    crate::encounters::Event::Owner {
                        player,
                        previous,
                        state,
                    } => {
                        updates.extend(scene.player_stats(player, &state));
                        let skills = &store.characters[&player].skills;
                        if config.initialize_ui
                            && skills.available_credits(&previous)
                                != skills.available_credits(&state)
                        {
                            updates.push(crate::scene::Delivery {
                                recipient: player,
                                data: crate::ui::skill_credits(skills.available_credits(&state))
                                    .map_err(other)?
                                    .into(),
                            });
                        }
                        for wire in state.private_changes(&previous) {
                            updates.push(crate::scene::Delivery {
                                recipient: player,
                                data: wire.into(),
                            });
                        }
                        if previous.alive() && !state.alive() {
                            for wire in [
                                crate::ui::death_notice("You were defeated by a creature.")
                                    .map_err(other)?,
                                crate::ui::lock_player(true).map_err(other)?,
                            ] {
                                updates.push(crate::scene::Delivery {
                                    recipient: player,
                                    data: wire.into(),
                                });
                            }
                        }
                    }
                    crate::encounters::Event::Note { player, text } => {
                        updates.push(crate::scene::Delivery {
                            recipient: player,
                            data: script::chat(&text, 1).map_err(other)?.into(),
                        })
                    }
                    crate::encounters::Event::Stop(player) => {
                        if let Some(peer) = active_characters
                            .get(&player)
                            .and_then(|addr| peers.get_mut(addr))
                        {
                            peer.combat = false;
                            peer.target = None;
                        }
                        updates.push(crate::scene::Delivery {
                            recipient: player,
                            data: script::private_call(script::COMBAT_STATE, &script::integer(0))
                                .map_err(other)?
                                .into(),
                        });
                    }
                }
            }
            deliver_scene(
                updates,
                &active_characters,
                &mut peers,
                &primary,
                now,
                &mut failed_peers,
            )?;
        }
        let mut remove = std::mem::take(&mut failed_peers);
        for (addr, peer) in &mut peers {
            if peer.native_shop.is_some() {
                let valid = match peer.session.phase {
                    Phase::InWorld { character } => {
                        store
                            .characters
                            .get(&character)
                            .is_some_and(|saved| saved.progression.alive())
                            && !peer.combat
                            && peer
                                .merchant_open_ms
                                .is_some_and(|last| now_ms.saturating_sub(last) <= 30_000)
                            && merchant_ready(character, &scene).ok()
                                == peer.native_shop.as_ref().map(|shop| shop.merchant())
                    }
                    _ => false,
                };
                if !valid {
                    peer.native_shop = None;
                    peer.merchant_open_ms = None;
                }
            }
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
            if !remove.contains(addr) && peer.flush_scene(&primary, addr, now).is_err() {
                remove.insert(*addr);
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
        let online = group_online(&active_characters, &store, &scene);
        for player in groups
            .participant_ids()
            .into_iter()
            .filter(|id| !online.contains_key(id))
        {
            let result = (|| -> io::Result<()> {
                let plan = groups
                    .prepare_disconnect(player, now_ms, &online)
                    .map_err(io::Error::other)?;
                let packets = group_admit(plan.deliveries(), &active_characters, &peers)?;
                groups.commit(plan).map_err(io::Error::other)?;
                group_publish(packets, &mut peers)
            })();
            if let Err(error) = result {
                eprintln!("Group departure deferred: {error}");
            }
        }
        if now_ms >= next_group_refresh {
            let mut refreshed = BTreeSet::new();
            for player in groups.member_ids() {
                let Some(member) = online.get(&player) else {
                    continue;
                };
                let stamp = (
                    member.level,
                    member.health,
                    member.max_health,
                    member.vigor,
                    member.max_vigor,
                    member.cell,
                );
                if group_cache.get(&player) == Some(&stamp) || refreshed.contains(&player) {
                    continue;
                }
                let result = (|| -> io::Result<()> {
                    let updates = groups.refresh(player, &online).map_err(io::Error::other)?;
                    let packets = group_admit(&updates, &active_characters, &peers)?;
                    group_publish(packets, &mut peers)
                })();
                if result.is_ok() {
                    for id in groups.members(player) {
                        refreshed.insert(id);
                        if let Some(m) = online.get(&id) {
                            group_cache.insert(
                                id,
                                (
                                    m.level,
                                    m.health,
                                    m.max_health,
                                    m.vigor,
                                    m.max_vigor,
                                    m.cell,
                                ),
                            );
                        }
                    }
                }
            }
            group_cache.retain(|id, _| groups.contains(*id));
            next_group_refresh = now_ms.saturating_add(1000);
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
            emotes: crate::emotes::State::default(),
            last_inventory_ms: None,
            last_examine_ms: None,
            last_world_use_ms: None,
            merchant_open_ms: None,
            native_shop: None,
            unhandled_scripts: BTreeSet::new(),
            scene_outbox: VecDeque::new(),
            scene_bytes: 0,
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
