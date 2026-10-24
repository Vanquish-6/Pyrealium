//! Bounded session groups and capture-backed native fellowship messages.
//!
//! Only the captured no-XP-sharing/personal-reward creation profile is enabled.
//! Membership is session state; it never grants shared XP or item authority.
//! Native adapters: 004682c0..00468810, 0060c1a0/0060c230. Interpreted data
//! packages 1f0/1ee/1ef are checked against the supplied WLib by the caller.
use crate::{script, world, world::WorldError};
use std::collections::{BTreeMap, BTreeSet};

pub const CREATE: u32 = 0x820c001b;
pub const INVITE: u32 = 0x820c0015;
pub const ACCEPT: u32 = 0x820c000d;
pub const DECLINE: u32 = 0x820c001a;
pub const LEAVE: u32 = 0x820c001c;
pub const DISBAND: u32 = 0x820c0003;
pub const DISMISS: u32 = 0x820c000e;
pub const SNAPSHOT: u32 = 0x820c002b;
pub const INVITATION: u32 = 0x820c002e;
pub const CLEAR: u32 = 0x820c0028;
pub const LAYOUTS: [(usize, u32); 3] = [(0x1f0, 8), (0x1ee, 5), (0x1ef, 7)];
pub const MAX_MEMBERS: usize = 8;
const MAX_GROUPS: usize = 32;
const MAX_INVITATIONS: usize = 64;
const INVITATION_MS: u64 = 30_000;
const NAMESPACE: u64 = 0x8150000000000000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Create {
        name: String,
        looting: u32,
        social: bool,
    },
    Invite(u64),
    Accept(u64),
    Decline(u64),
    Leave,
    Disband,
    Dismiss(u64),
}

pub fn handles(function: u32) -> bool {
    matches!(
        function,
        CREATE | INVITE | ACCEPT | DECLINE | LEAVE | DISBAND | DISMISS
    )
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl Reader<'_> {
    fn word(&mut self) -> Result<u32, WorldError> {
        let end = self
            .offset
            .checked_add(4)
            .ok_or(WorldError::LimitExceeded)?;
        let data = self
            .bytes
            .get(self.offset..end)
            .ok_or(WorldError::Truncated)?;
        self.offset = end;
        Ok(u32::from_le_bytes(data.try_into().unwrap()))
    }
    fn expect(&mut self, value: u32) -> Result<(), WorldError> {
        if self.word()? != value {
            return Err(WorldError::InvalidValue);
        }
        Ok(())
    }
    fn int(&mut self) -> Result<u32, WorldError> {
        self.expect(1)?;
        self.word()
    }
    fn iid(&mut self) -> Result<u64, WorldError> {
        self.expect(2)?;
        let value = u64::from(self.word()?) | u64::from(self.word()?) << 32;
        if value == 0 || value == u64::MAX {
            return Err(WorldError::InvalidValue);
        }
        Ok(value)
    }
    fn name(&mut self) -> Result<String, WorldError> {
        self.expect(5)?;
        self.expect(1)?;
        if matches!(self.word()?, 0 | u32::MAX) {
            return Err(WorldError::InvalidValue);
        }
        self.expect(0x44000100)?;
        self.expect(0)?;
        self.expect(0xffff0076)?;
        let tail = self.bytes.get(self.offset..).ok_or(WorldError::Truncated)?;
        let count = u16::from_le_bytes(
            tail.get(..2)
                .ok_or(WorldError::Truncated)?
                .try_into()
                .unwrap(),
        ) as usize;
        if !(1..=32).contains(&count) {
            return Err(WorldError::LimitExceeded);
        }
        let size = (2 + count * 2 + 3) & !3;
        let packed = tail.get(..size).ok_or(WorldError::Truncated)?;
        let mut literal = vec![0; 10];
        literal.extend(1u16.to_le_bytes());
        literal.extend(packed);
        let (name, used) = world::read_literal_name(&literal)?;
        if used != literal.len() || !valid_name(&name) {
            return Err(WorldError::InvalidValue);
        }
        self.offset += size;
        self.expect(0)?;
        Ok(name)
    }
}

pub fn decode(function: u32, bytes: &[u8]) -> Result<Request, WorldError> {
    if bytes.len() > 256 {
        return Err(WorldError::LimitExceeded);
    }
    let mut reader = Reader { bytes, offset: 0 };
    let result = match function {
        CREATE => {
            let looting = reader.int()?;
            let social = match reader.int()? {
                0 => false,
                1 => true,
                _ => return Err(WorldError::InvalidValue),
            };
            if !(1..=3).contains(&looting) {
                return Err(WorldError::InvalidValue);
            }
            Request::Create {
                name: reader.name()?,
                looting,
                social,
            }
        }
        INVITE => Request::Invite(reader.iid()?),
        ACCEPT => Request::Accept(reader.iid()?),
        DECLINE => {
            reader.expect(1)?;
            reader.expect(0x138d)?;
            Request::Decline(reader.iid()?)
        }
        LEAVE => Request::Leave,
        DISBAND => Request::Disband,
        DISMISS => Request::Dismiss(reader.iid()?),
        _ => return Err(WorldError::UnsupportedOpcode(function)),
    };
    if reader.offset != bytes.len() {
        return Err(WorldError::TrailingBytes);
    }
    Ok(result)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub id: u64,
    pub name: String,
    pub level: u32,
    pub health: u32,
    pub max_health: u32,
    pub vigor: u32,
    pub max_vigor: u32,
    pub cell: u32,
}
impl Member {
    fn valid(&self) -> bool {
        self.id != 0
            && self.id != u64::MAX
            && valid_name(&self.name)
            && (1..=150).contains(&self.level)
            && self.max_health > 0
            && self.max_health <= 1_000_000
            && self.max_vigor > 0
            && self.max_vigor <= 1_000_000
            && self.health <= self.max_health
            && self.vigor <= self.max_vigor
            && crate::dat::world_cell_record(self.cell).is_ok()
    }
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.trim() == name
        && name.encode_utf16().count() <= 32
        && !name.chars().any(char::is_control)
}

#[derive(Clone, Debug)]
struct Group {
    name: String,
    leader: u64,
    members: BTreeSet<u64>,
}
#[derive(Clone, Debug)]
struct Invitation {
    group: u64,
    inviter: u64,
    until: u64,
}
#[derive(Clone, Debug)]
pub struct Runtime {
    groups: BTreeMap<u64, Group>,
    membership: BTreeMap<u64, u64>,
    invitations: BTreeMap<u64, Invitation>,
    next: u64,
    revision: u64,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            groups: BTreeMap::new(),
            membership: BTreeMap::new(),
            invitations: BTreeMap::new(),
            next: NAMESPACE,
            revision: 0,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Delivery {
    pub recipient: u64,
    pub data: Vec<u8>,
}
pub struct Plan {
    next: Runtime,
    base_revision: u64,
    deliveries: Vec<Delivery>,
}
impl Plan {
    pub fn deliveries(&self) -> &[Delivery] {
        &self.deliveries
    }
}

#[derive(Clone, Debug)]
pub enum GroupError {
    InvalidMember,
    AlreadyGrouped,
    NotGrouped,
    NotLeader,
    Capacity,
    InvalidTarget,
    InvitationMissing,
    InvitationPending,
    UnsupportedMode,
    StalePlan,
    Wire,
}
impl GroupError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::AlreadyGrouped => "You are already in a group.",
            Self::NotGrouped => "You are not in a group.",
            Self::NotLeader => "Only the group leader can do that.",
            Self::Capacity => "The group or invitation limit has been reached.",
            Self::InvalidTarget => "That player is not available for this group action.",
            Self::InvitationMissing => "The group invitation is no longer available.",
            Self::InvitationPending => "That player already has a pending group invitation.",
            Self::UnsupportedMode => {
                "Groups currently support individual experience and personal rewards only. Select the no-sharing group profile."
            }
            _ => "The group action could not be completed.",
        }
    }
}
impl std::fmt::Display for GroupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}
impl std::error::Error for GroupError {}
impl From<WorldError> for GroupError {
    fn from(_: WorldError) -> Self {
        Self::Wire
    }
}

impl Runtime {
    pub fn contains(&self, player: u64) -> bool {
        self.membership.contains_key(&player)
    }
    pub fn members(&self, player: u64) -> Vec<u64> {
        self.membership
            .get(&player)
            .and_then(|id| self.groups.get(id))
            .map(|group| group.members.iter().copied().collect())
            .unwrap_or_default()
    }
    pub fn member_ids(&self) -> Vec<u64> {
        self.membership.keys().copied().collect()
    }
    pub fn participant_ids(&self) -> Vec<u64> {
        self.membership
            .keys()
            .chain(self.invitations.keys())
            .copied()
            .chain(self.invitations.values().map(|invite| invite.inviter))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    pub fn prepare(
        &self,
        actor: u64,
        request: &Request,
        now: u64,
        online: &BTreeMap<u64, Member>,
    ) -> Result<Plan, GroupError> {
        if !online.get(&actor).is_some_and(Member::valid) {
            return Err(GroupError::InvalidMember);
        }
        let mut next = self.clone();
        next.invitations.retain(|_, invite| invite.until > now);
        let mut deliveries = Vec::new();
        let mut clear = BTreeSet::new();
        let mut refresh = BTreeSet::new();
        match request {
            Request::Create {
                name,
                looting,
                social,
            } => {
                // The native create panel initially sends (1, false). Training
                // groups explicitly normalize that default to our supported
                // personal-reward profile, also reflected by snapshot flags 4.
                let native_default = *looting == 1 && !social;
                if !native_default && (*looting != 2 || !social) {
                    return Err(GroupError::UnsupportedMode);
                }
                if !valid_name(name) {
                    return Err(GroupError::InvalidMember);
                }
                if next.contains(actor) {
                    return Err(GroupError::AlreadyGrouped);
                }
                if next.groups.len() >= MAX_GROUPS {
                    return Err(GroupError::Capacity);
                }
                next.next = next
                    .next
                    .checked_add(1)
                    .filter(|id| id >> 48 == NAMESPACE >> 48)
                    .ok_or(GroupError::Capacity)?;
                let id = next.next;
                next.groups.insert(
                    id,
                    Group {
                        name: name.clone(),
                        leader: actor,
                        members: BTreeSet::from([actor]),
                    },
                );
                next.membership.insert(actor, id);
                next.invitations.remove(&actor);
                refresh.insert(id);
                deliveries.push(Delivery {
                    recipient: actor,
                    data: script::chat(
                        "Training groups use individual experience and personal rewards.",
                        1,
                    )?,
                });
            }
            Request::Invite(target) => {
                if *target == actor || !online.get(target).is_some_and(Member::valid) {
                    return Err(GroupError::InvalidTarget);
                }
                if next.contains(*target) {
                    return Err(GroupError::AlreadyGrouped);
                }
                if next.invitations.contains_key(target) {
                    return Err(GroupError::InvitationPending);
                }
                if next.invitations.len() >= MAX_INVITATIONS {
                    return Err(GroupError::Capacity);
                }
                let id = *next.membership.get(&actor).ok_or(GroupError::NotGrouped)?;
                let group = next.groups.get(&id).ok_or(GroupError::NotGrouped)?;
                if group.leader != actor {
                    return Err(GroupError::NotLeader);
                }
                let reserved = next
                    .invitations
                    .values()
                    .filter(|invite| invite.group == id)
                    .count();
                if group.members.len() + reserved >= MAX_MEMBERS {
                    return Err(GroupError::Capacity);
                }
                deliveries.push(Delivery {
                    recipient: *target,
                    data: invitation(&online[&actor].name, &group.name, actor)?,
                });
                next.invitations.insert(
                    *target,
                    Invitation {
                        group: id,
                        inviter: actor,
                        until: now.saturating_add(INVITATION_MS),
                    },
                );
            }
            Request::Accept(inviter) => {
                if next.contains(actor) {
                    return Err(GroupError::AlreadyGrouped);
                }
                let invite = next
                    .invitations
                    .get(&actor)
                    .filter(|invite| invite.inviter == *inviter)
                    .cloned()
                    .ok_or(GroupError::InvitationMissing)?;
                if !online.get(inviter).is_some_and(Member::valid) {
                    return Err(GroupError::InvalidTarget);
                }
                let group = next
                    .groups
                    .get_mut(&invite.group)
                    .filter(|group| group.leader == *inviter)
                    .ok_or(GroupError::InvitationMissing)?;
                if group.members.len() >= MAX_MEMBERS {
                    return Err(GroupError::Capacity);
                }
                group.members.insert(actor);
                next.membership.insert(actor, invite.group);
                next.invitations.remove(&actor);
                refresh.insert(invite.group);
            }
            Request::Decline(inviter) => {
                if !next
                    .invitations
                    .get(&actor)
                    .is_some_and(|invite| invite.inviter == *inviter)
                {
                    return Err(GroupError::InvitationMissing);
                }
                next.invitations.remove(&actor);
                if online.contains_key(inviter) {
                    deliveries.push(Delivery {
                        recipient: *inviter,
                        data: script::chat("The group invitation was declined.", 1)?,
                    });
                }
            }
            Request::Leave => next.remove_member(actor, &mut clear, &mut refresh)?,
            Request::Disband => {
                let id = *next.membership.get(&actor).ok_or(GroupError::NotGrouped)?;
                if next.groups[&id].leader != actor {
                    return Err(GroupError::NotLeader);
                }
                next.remove_group(id, &mut clear);
            }
            Request::Dismiss(target) => {
                let id = *next.membership.get(&actor).ok_or(GroupError::NotGrouped)?;
                if next.groups[&id].leader != actor {
                    return Err(GroupError::NotLeader);
                }
                if *target == actor || next.membership.get(target) != Some(&id) {
                    return Err(GroupError::InvalidTarget);
                }
                next.remove_member(*target, &mut clear, &mut refresh)?;
            }
        }
        next.finish_plan(self.revision, deliveries, clear, refresh, online)
    }
    pub fn prepare_disconnect(
        &self,
        actor: u64,
        now: u64,
        online: &BTreeMap<u64, Member>,
    ) -> Result<Plan, GroupError> {
        let mut next = self.clone();
        let mut clear = BTreeSet::new();
        let mut refresh = BTreeSet::new();
        next.invitations.retain(|target, invite| {
            *target != actor
                && invite.inviter != actor
                && invite.until > now
                && online.contains_key(target)
                && online.contains_key(&invite.inviter)
        });
        // Several sockets can fail in one poll. Remove every unavailable member
        // before encoding snapshots, otherwise each removal could be blocked by
        // a different disconnected member's missing public state.
        let unavailable = next
            .membership
            .keys()
            .copied()
            .filter(|id| *id == actor || !online.get(id).is_some_and(Member::valid))
            .collect::<Vec<_>>();
        for player in unavailable {
            if next.contains(player) {
                next.remove_member(player, &mut clear, &mut refresh)?;
            }
        }
        next.finish_plan(self.revision, Vec::new(), clear, refresh, online)
    }
    fn remove_group(&mut self, id: u64, clear: &mut BTreeSet<u64>) {
        if let Some(group) = self.groups.remove(&id) {
            for player in group.members {
                self.membership.remove(&player);
                clear.insert(player);
            }
        }
        self.invitations.retain(|_, invite| invite.group != id);
    }
    fn remove_member(
        &mut self,
        actor: u64,
        clear: &mut BTreeSet<u64>,
        refresh: &mut BTreeSet<u64>,
    ) -> Result<(), GroupError> {
        let id = *self.membership.get(&actor).ok_or(GroupError::NotGrouped)?;
        if self.groups[&id].leader == actor {
            self.remove_group(id, clear);
        } else {
            self.groups
                .get_mut(&id)
                .ok_or(GroupError::NotGrouped)?
                .members
                .remove(&actor);
            self.membership.remove(&actor);
            self.invitations.remove(&actor);
            clear.insert(actor);
            refresh.insert(id);
        }
        Ok(())
    }
    fn finish_plan(
        mut self,
        base_revision: u64,
        mut deliveries: Vec<Delivery>,
        clear: BTreeSet<u64>,
        refresh: BTreeSet<u64>,
        online: &BTreeMap<u64, Member>,
    ) -> Result<Plan, GroupError> {
        for player in clear {
            if online.contains_key(&player) {
                deliveries.push(Delivery {
                    recipient: player,
                    data: script::private_call(CLEAR, &[])?,
                });
            }
        }
        for id in refresh {
            if let Some(group) = self.groups.get(&id) {
                deliveries.extend(group_deliveries(group, online)?);
            }
        }
        self.revision = base_revision.checked_add(1).ok_or(GroupError::Capacity)?;
        Ok(Plan {
            next: self,
            base_revision,
            deliveries,
        })
    }
    pub fn commit(&mut self, plan: Plan) -> Result<Vec<Delivery>, GroupError> {
        if self.revision != plan.base_revision {
            return Err(GroupError::StalePlan);
        }
        *self = plan.next;
        Ok(plan.deliveries)
    }
    pub fn refresh(
        &self,
        player: u64,
        online: &BTreeMap<u64, Member>,
    ) -> Result<Vec<Delivery>, GroupError> {
        match self
            .membership
            .get(&player)
            .and_then(|id| self.groups.get(id))
        {
            Some(group) => group_deliveries(group, online),
            None => Ok(Vec::new()),
        }
    }
    pub fn member_updates(
        &self,
        player: u64,
        member: &Member,
    ) -> Result<Vec<Delivery>, GroupError> {
        self.member_changes(player, member, None)
    }
    /// Emit only changed native member fields. Roster/name/level changes require
    /// a full snapshot; callers must publish an admitted batch before caching it.
    pub fn member_changes(
        &self,
        player: u64,
        member: &Member,
        previous: Option<&Member>,
    ) -> Result<Vec<Delivery>, GroupError> {
        if member.id != player || !member.valid() {
            return Err(GroupError::InvalidMember);
        }
        if previous.is_some_and(|old| {
            !old.valid() || old.id != player || old.name != member.name || old.level != member.level
        }) {
            return Err(GroupError::InvalidMember);
        }
        let mut packets = Vec::new();
        // Maxima precede current values so a growing bar never temporarily
        // receives a current value larger than its old maximum.
        for (function, first, value, old_value) in [
            (
                0x820c0033,
                false,
                member.max_health,
                previous.map(|old| old.max_health),
            ),
            (
                0x820c0030,
                true,
                member.health,
                previous.map(|old| old.health),
            ),
            (
                0x820c0021,
                false,
                member.max_vigor,
                previous.map(|old| old.max_vigor),
            ),
            (
                0x820c002d,
                true,
                member.vigor,
                previous.map(|old| old.vigor),
            ),
            (0x820c003f, false, member.cell, previous.map(|old| old.cell)),
        ] {
            if old_value == Some(value) {
                continue;
            }
            let mut args = Vec::new();
            if first {
                args.extend(script::integer(0));
            }
            args.extend(script::integer(value));
            args.extend(script::identity(player));
            packets.push(script::private_call(function, &args)?);
        }
        Ok(self
            .members(player)
            .into_iter()
            .flat_map(|recipient| {
                packets
                    .iter()
                    .cloned()
                    .map(move |data| Delivery { recipient, data })
            })
            .collect())
    }
}

fn group_deliveries(
    group: &Group,
    online: &BTreeMap<u64, Member>,
) -> Result<Vec<Delivery>, GroupError> {
    let members = group
        .members
        .iter()
        .map(|id| {
            online
                .get(id)
                .filter(|member| member.valid())
                .cloned()
                .ok_or(GroupError::InvalidMember)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let data = snapshot(&group.name, group.leader, &members)?;
    Ok(group
        .members
        .iter()
        .map(|&recipient| Delivery {
            recipient,
            data: data.clone(),
        })
        .collect())
}

fn invitation(inviter: &str, name: &str, owner: u64) -> Result<Vec<u8>, WorldError> {
    let mut arguments = script::literal(inviter)?;
    arguments.extend(script::literal(name)?);
    arguments.extend(script::identity(owner));
    script::private_call(INVITATION, &arguments)
}

struct Object {
    kind: u32,
    native: Vec<u8>,
    fields: Vec<(u32, u8)>,
}
impl Object {
    fn native(kind: u16, bytes: Vec<u8>) -> Self {
        Self {
            kind: 0xffff0000 | u32::from(kind),
            native: bytes,
            fields: Vec::new(),
        }
    }
    fn interpreted(package: u16, fields: Vec<(u32, u8)>) -> Self {
        Self {
            kind: u32::from(package) << 16,
            native: Vec::new(),
            fields,
        }
    }
}
fn word(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend(value.to_le_bytes());
}
fn graph(objects: &[Object]) -> Vec<u8> {
    let mut bytes = Vec::new();
    word(&mut bytes, 5);
    word(&mut bytes, objects.len() as u32);
    for index in 1..=objects.len() {
        word(&mut bytes, index as u32);
    }
    for object in objects {
        for value in [0x44000100, 0, object.kind] {
            word(&mut bytes, value);
        }
        bytes.extend(&object.native);
        word(&mut bytes, object.fields.len() as u32);
        for &(value, _) in &object.fields {
            word(&mut bytes, value);
        }
        for &(_, tag) in &object.fields {
            bytes.push(tag);
        }
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
    }
    bytes
}
fn snapshot(name: &str, leader: u64, members: &[Member]) -> Result<Vec<u8>, WorldError> {
    if !valid_name(name)
        || members.is_empty()
        || members.len() > MAX_MEMBERS
        || !members.iter().any(|member| member.id == leader)
    {
        return Err(WorldError::InvalidValue);
    }
    let name_id = 3 + members.len() as u32 * 3;
    // Pinned 7148 WLib: 81f1003d -> 01f00002 constructs the native
    // FellowshipAdaptor from fields 7, 5/6, 2 and 4. Fields 0/1 and 3 are
    // retained at their native 01f0001e constructor defaults; their wider
    // server semantics are unknown. Flags 4 decode as social=true, looting=2
    // through 01f00012 / 01f0002e, matching the captured personal profile.
    let mut objects = vec![Object::interpreted(
        0x1f0,
        vec![
            (0, 0x82),
            (0, 0xcd),
            (4, 2),
            (0, 2),
            (2, 1),
            (leader as u32, 0x84),
            ((leader >> 32) as u32, 0xcd),
            (name_id, 1),
        ],
    )];
    let mut ordered = members.to_vec();
    ordered.sort_unstable_by_key(|member| (member.id % 11, member.id));
    let mut hash = Vec::new();
    word(&mut hash, 0x02000000 | ordered.len() as u32);
    for (index, member) in ordered.iter().enumerate() {
        word(&mut hash, member.id as u32);
        word(&mut hash, (member.id >> 32) as u32);
        word(&mut hash, 3 + index as u32 * 3);
    }
    objects.push(Object::native(0x40, hash));
    for member in ordered {
        if !member.valid() {
            return Err(WorldError::InvalidValue);
        }
        let root = objects.len() as u32 + 1;
        objects.push(Object::interpreted(
            0x1ee,
            vec![
                (0, 0x84),
                (0, 0xcd),
                (member.level, 4),
                (root + 1, 1),
                (root + 2, 1),
            ],
        ));
        objects.push(Object::interpreted(
            0x1ef,
            vec![
                (member.health, 4),
                (member.cell, 4),
                (member.max_health, 4),
                (0, 4),
                (member.vigor, 4),
                (member.max_vigor, 4),
                (0, 4),
            ],
        ));
        objects.push(Object::native(0x6a, world::literal_name(&member.name)?));
    }
    let literal = world::literal_name(name)?;
    objects.push(Object::native(0x76, literal[12..].to_vec()));
    script::private_call(SNAPSHOT, &graph(&objects))
}
