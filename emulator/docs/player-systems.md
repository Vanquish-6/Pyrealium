# Player systems integration

## Durable progression foundation

The `g1` journal revision adds server-owned XP, available XP, gold, health and
vigor. Commands are validated on a candidate; the store flushes the event before
publishing state. Replay rejects missing/deleted owners and invalid transitions.
Older characters begin with the prior local level-one defaults. Deletion and
restoration retain progression. Journals containing `g1` require this reader.

The imported 150-level development profile comes from the supplied numeric level
table. Its SHA256 is recorded in `src/progression/levels.rs`; the importer never
executes SQL. This is imported balance data, not verified 7148 server balance.
The table is immutable for journal revision `g1`: rebalancing requires migration.
Level is derived from total XP, using monotonic thresholds. XP/currency overflow,
overspending and spending vigor while dead are rejected. Local level-up policy
preserves missing vital amounts and does not resurrect dead characters. Healing
does not resurrect; explicit resurrection restores vitals. These policies are
development rules rather than claims about original death penalties or healing.

World entry now restores saved vitals, level and gold in the integer quality
table and XP in the long-int quality table. CBaseQualities::GetPackHeader
`00516d10` assigns long-int tables bit `0x800`; Pack `00516f80` places them last.
The two XP keys retain native bucket order. HUD initialization receives saved
gold and available skill credits after deducting the saved skill book. Native
training/untraining, encounter rewards and resurrection use the store transitions
described below. Quest and active-effect payloads remain at their earlier scope.

Private int (`0x10069`) and long-int (`0x100d4`) update builders follow dispatch
`00494010` and receivers `004981b0` / `00498350`. Live mutation consumers are
being connected separately; a codec alone does not establish UI acceptance.
The stat IDs also agree with the supplied protocol/quality declarations.

The offline `player-state --character-id N [--event COMMAND] --data PATH` command
inspects or changes state under the normal exclusive store lock. Event forms:
`x,XP`, `g,GOLD`, `s,GOLD`, `d,DAMAGE`, `h,HEAL`, `v,COST`, `r,RECOVERY`, `z`.
No network request is permitted to award XP or currency directly. This operator
tool is not an implemented quest, combat or resurrection interaction.

Compiled only. No automated tests or native game checks were run, as requested.

## Populated-scene backpressure

Scene deliveries now enter bounded per-session outgoing queues instead of
disconnecting a peer solely because its reliable-send window is temporarily
full. Queues share immutable payloads, admit at most 1,024 events / 1 MiB per
peer, and flush at most 32 events per event-loop pass when complete-message
fragment capacity is available. Limits, sequence exhaustion and send failures
still retire the affected peer. Transport retry/ISAAC state is not advanced while
waiting for capacity. Create/destroy and visual quality updates use ordered
delivery; ordinary movement retains its existing delivery policy. Queued scene
state is cleared on avatar exit and before another avatar lifetime starts.
This is compiled only; populated native multiplayer acceptance remains open.

Inventory-object and HUD restoration now enter the same bounded queue as one
prevalidated group. Admission checks every message and the combined event/byte
limits before publishing any part of the group; flushing waits for transport
capacity. Inventory objects precede UI_Load, which precedes scene activation
deliveries. Restoring an inventory therefore does not require the complete group
to fit in the currently free fragment window. The previous temporary transport
clone/rollback is removed. Initial empty HUD restoration follows the same queue.
These changes are compile checked only, without native login acceptance.

## Explicit development encounter loop

`--encounters content/development-encounters.tsv` enables bounded, deterministic
server combat using an explicit local rule profile. No combat rules are enabled
by default. It supports native combat-start target requests, range and visibility
checks, player vigor costs, repeated attacks, stationary creature retaliation,
health updates, death/despawn, timed respawn and XP/gold rewards. It is not an
implementation of original weapon/skill formulas, armor, resistances, animations,
projectiles, pathfinding, collision/line of sight or corpse/item looting.

Use the existing game-data/account/bind/data options with:

```powershell
--content training --initialize-ui true --encounters content/development-encounters.tsv
```

The training pack places one explicitly authored development Idol near the
supported human starting position. This position is not an original recovered
spawn and has not been checked in the native client. `--content observed` can
instead enable the same rules for matching captured creature definitions. Other
objects, friendly NPCs and players are not attackable through this profile.

Profiles specify player damage/cost/range/cadence, creature health/damage/range/
cadence/respawn/rewards, and out-of-combat health/vigor recovery. These values are
development balance, not reconstructed original creature statistics. Same-cell
Euclidean range is enforced; collision and visibility geometry remain unfinished.
Creatures retaliate in place. They switch to another eligible attacker or reset
health when disengaged; no movement/pathfinding is implied.

Ticks run at most ten times per second, with one attack per due participant and
no catch-up bursts. Player cooldowns survive attack cancellation/reselection.
Only the first killing transition grants rewards. Vigor cost and XP/gold rewards
share one flushed `g2` journal transaction before creature state changes. Batch
replay validates every transition before publishing it. All attackers targeting
a killed lifetime stop; respawn cannot retain stale automatic attacks.

Creature health/visibility and fresh instance stamps are reflected in scene
create/destroy/update messages, including current health for late observers.
Player public health/level is cached for late observers and updated on changes;
XP, gold and other private player state are sent only to the owning connection.
The initialized HUD receives skill-credit updates as levels change. Native
animation and interface acceptance remain unverified.

Dead characters cannot attack or submit movement. The native Resurrect button
now restores saved vitals at the current location. Death sends the private and
public boolean Dead quality (`117`), the native death notice (`8207002b`) and
UI lock (`82070033`). Reconnecting a dead character restores that state and
notice after scene initialization. Resurrection persists the recovery before
clearing combat, publishing live vitals/Dead and unlocking the UI. The same
recovery remains available if the encounter profile has subsequently changed;
`@resurrect` is a development chat fallback when encounters are enabled.
This is local recovery policy. The optional development binding/return path is
described below; corpse recovery, death animations and original penalties remain
unfinished.
Ordinary chat rate limits still apply. Encounter/world state is transient across
server restarts; player rewards and vitals are durable. Journals with `g2` require
the newer reader. Training does not yet grant starter equipment or abilities.

Compiled only. No tests, benchmarks, native client launch or deployment/restart
were performed for this addition.

## Skill catalog and training interface

The DAT importer resolves `700016e2`'s native skill hash and external references,
then reads bounded common fields and supported dependency objects from each
definition. It exports 1,952 definitions; 192 unsupported common-field layouts
remain in `content/unresolved-skills.json`. Species, level, credit cost, rank cap,
advancement-record reference, prerequisites and incompatibilities are retained.
This metadata importer does not evaluate advancement tables or active effects;
the separate table importer and advancement transaction are described below.
SHA256 provenance and FNV compatibility fingerprints are recorded. The opt-in
HUD startup validates the index and every exported definition against the supplied
DAT before using the imported rules. Regenerating this catalog after `k1` journals
exist requires an explicit skill migration.

Native train/untrain/raise requests use the captured integer argument profile:
functions `82140002`, `82140007`, `8214000c`, exactly eight argument bytes. Training
and immediate rank-zero untraining now mutate a durable skill book, flush the
`k1` journal event before acknowledgment, and send native skill and credit updates.
Replay repeats ownership, eligibility, credit, dependency and incompatibility
validation. Reconnect initializes the complete saved skill hash. Credits used by
trained skills remain deducted when level-up updates the available balance.

The supported training profile is human, ordinary skills through minimum level
50; faction/common, hero, recall, portal and mob grants require further services.
Requests must satisfy imported species/level/prerequisite/credit rules, including
incompatibility in both directions. Duplicate training, dead-player changes,
removing prerequisite skills and removing locked grants are rejected. A skill
book is bounded to 128 entries. Rank zero is the captured newly-trained leaf
profile. Locked grants are enforced server-side; their full native arrow/status
presentation has not been established.

SkillData uses interpreted package `231`, fifteen fields; initialization checks
that layout in the supplied WLib. Train updates use `8214000d`; removals use
`82140014`; credit changes use `82140011`. Saved skills initialize the skill map
within UI_Load. The retained skilling captures support these envelope/object
layouts, but their original build identity and native acceptance remain unverified.

`skill-state --character-id N [--event t,HEX|g,HEX|u,HEX] --data PATH` provides
offline inspection, training, explicit zero-credit locked grants and untraining
under the existing exclusive store lock. No starter skills are automatically
inferred from the catalog. Native requests cannot invoke the operator grant path.

Native advancement now uses the referenced cumulative XP tables described below.
Active ability use, costs/cooldowns/effects, timed original untraining,
hero/crafting/faction systems and full original training eligibility remain
separate work. No automated tests or native client checks were run.

## Live bag sorting

The initialized HUD now accepts the bounded `820e0049` sorting request profile
and emits `820e0055` acknowledgments after flushing the existing `i1` inventory
journal record. The request must identify the active owner and an item at its
saved source position; dead characters cannot move items. Candidate validation
and reply construction complete before persistence. Invalid requests cannot
grant an item or change its definition, quantity, icon or appearance.

The supported operation is insertion in a dense bag list for quantity-one items.
The reply preserves the requested insertion boundary and reports the resulting
post-removal slot. Forward and backward moves use the observed integer tags.
Stacks, sparse lists, equipment and optional per-item property graphs require
separate profiles. The existing 78-slot local capacity policy still applies.

Icons come from DID quality `103` and physical definitions from DID quality `1`
in each item's DAT qualities record. The bounded prefix reader follows recovered
`gmWeenieDesc::StreamPack` (`006037c0`) and `CBaseQualities::UnPack` (`005192f0`),
rejects unsupported description/name layouts, bounds quality tables and rejects
duplicate DID keys. It does not treat the description icon selector as a texture
DID. Startup caches metadata for saved inventory definitions (maximum 4,096),
validates backing assets and checks package `1f8` has 51 interpreted fields.
Unsupported metadata disables sorting for that definition with a readable error.

The retained `sorting.items.elog` contains 17 single-object move replies and
seven replies with additional property graphs. Its SHA256 is
`17f1593014477b0ea08347d1c4cbe1decc2905f59df464d7ba99dfbdd9166173`.
Only the single-object, single-item profile is generated here. Capture build
identity and native UI acceptance remain unverified. This change was compiled;
no tests, native client launch, deployment or server restart were performed.

The retained player equip captures send
general object-use function `82160016` with package `245`, rather than direct
`820e004f`/`820e0015` requests. Replies use packages `1f6` and `1f5` and are
accompanied by item/player appearance or parenting updates.

## Hand equipment integration

The bounded two-object, twenty-field object-use profile now accepts an owned
item and the active owner. Both objects, reference links, field tags, reserved
values and the localized display-hint shape are checked; no supplied graph is
instantiated or executed. WLib initialization validates packages `245` (20),
`1f6` (19) and `1f5` (7) before enabling the stored inventory profile.

Out of combat, a living character can toggle a quantity-one hand item with a
supported DAT primary location and saved equipment permission. The slot is
selected from DAT preferred/valid locations (`11a`/`113`), with primary/secondary
parenting (`114`/`115`) retained in the native acknowledgment. This initial local
profile supports main/off hand (`40`/`80`) with primary locations 1/2, empty target
slots and dense bags. It does not implement original skill/level/race equipment
eligibility, bonuses, dual-wield policy, armor, or automatic equipment swaps.

Native equipment replies use `820e0051` and `820e0056`, with fresh reference IDs
and authoritative identities/metadata. Dense-list equip uses the new `w` form in
the existing `i1` journal, flushes before acknowledging and compacts the remaining
bag list. Historical `e` journal semantics retain their original behavior.
Unequip appends to the dense bag list and rejects a full bag. Successful sorting
and equipment actions share a 100 ms local cooldown to bound journal churn.

The equipped object's CreateObject includes PhysicsDesc parent bit `20`, owner,
primary location and parent lifetime, following `00641530`. Each live toggle
uses a checked fresh item instance, retires the owner's old object, creates its
replacement, then acknowledges. Nearby players receive only the visible hand
object. Scene activation, cell transitions and disconnect also create/retire
those children, with the player created before its children and children retired
before the player. Reconnect restores supported equipped hand objects from the
saved inventory; no starter items are inferred or granted.

Retained capture evidence includes `equip.item.elog`, `equip.unequip.item.elog`
and `unequip.item.elog`. The middle capture's SHA256 is
`e08b702dc927dcd9432500899b7f10735128ef661446e0047a1eba27a3d7dfa7`.
This is static packet/layout integration, not native gameplay acceptance. Held
items currently use their primary attachment in both stances; stowing, animations,
armor appearance composition, equipment effects and original eligibility remain
unfinished. Compilation only; no automated tests or native game checks were run.

## Native object examination

The initialized 7148 HUD now handles `820a0007` with a single native
ExaminationRequest (`9a`), non-admin object type `40000002`, and zero interpreted
fields. The exact argument length is 44 bytes. The matching serializer
`00609660` does not contain the newer options word in the retained captures;
those newer request layouts are rejected instead of accepted by offset.

Replies use `820a0009`, native ExaminationProfile `99`, native request fields,
and a bounded list of string/break nodes. `00608380`, `00608490`, `00609140` and
`006dbb00` define the matching native profile, nodes, list and heap field count.
No newer native-profile trailing fields are replayed. Text uses supported
StringInfo literal serialization and retained header/body appearance selectors.

The handler allows the current player, owned items, visible equipped hand items,
other active visible players and active same-cell content. Player replies include
public level/health/death state; vigor, XP, spendable XP and gold are included only
when examining oneself. Owned items show quantity and stored location. Content
replies include encounter health when available. Other players' inventories and
private progression are not exposed. Unavailable/out-of-view targets are rejected.

Default definition names are cached from bounded DAT qualities prefixes, using
localized StringInfo references without substitutions. Unsupported names use a
generic fallback. The cache holds at most 16,384 unique definitions, covers saved
inventory and loaded content, and performs no disk reads per request. A 100 ms
request cooldown bounds repeated examination work. Full original descriptions,
item effects, quest facts and examination of skills/recipes are separate work.

Retained `ac2_npc_target.elog` and
`ac2_player_target_npc_pets_objects_buff_pets.elog` demonstrate the newer envelope
and string/break node pattern; their build identity remains unverified. Matching
7148 recovered serializers govern the emitted layout. This was compiled only;
no tests, native game launch or running-server replacement were performed.

## Explicit development training preset

`serve-training --game-data PATH` enables the initialized HUD, bundled `training`
world and encounter profiles, and `--training-kit true`. The normal `serve`
command keeps that grant disabled. `--encounters training` also selects the
embedded development rules without a working-directory dependency; an explicit
encounter file remains supported. The preset accepts the normal account/bind/data
options and optional content/encounter overrides.

The kit is a single quantity-one hand item, definition `47000f1e`, physical
`4700035b`, visual `1f000056`, under an explicit development grant policy. DAT
validation requires those backing records, their supported physical-definition
prefix, and the imported valid/preferred/primary hand locations. No original
starter entitlement, ability grant, economy or item balance is inferred.

At world entry, the kit uses one free bag slot for an eligible human character.
One checksummed/fsynced `t1` journal record records the grant and the persistent
once-per-character marker atomically. Replay repeats profile, global item-ID,
space and inventory validation. Reconnect, deletion/restoration and consuming
the item cannot repeat the grant. A full bag defers a first grant without blocking
world entry. Older journals default the marker to false; journals containing
`t1` require this reader and its immutable kit definition.

The kit definition is included in startup metadata and examination-name caches
before it can be granted. The owned item enters the normal inventory bootstrap,
sorting, hand-equipment and examination paths. This change was compile checked
only; no grants were executed against existing player data and no tests or native
client checks were run.

## Development lifestone binding and return

`--lifestones true` requires the initialized 7148 HUD and enables explicit local
binding/return rules. `serve-training` enables it and includes an authored nearby
lifestone using the existing DAT-backed definition/visual. Ordinary `serve` keeps
it disabled. This is not original level/hero/kingdom eligibility or cast timing.

The captured object-use profile can bind to an active, visible same-cell lifestone
definition `470007de` within four meters. The character must be alive and out of
combat. The saved return point is the player's current accepted position, avoiding
an invented offset at the center of the lifestone. The bind is fsynced before a
native `82160021` success reply and chat confirmation. The reply uses package
`245`, a localized use hint, and native Position `5a`, following retained binding
captures. A one-second interaction cooldown bounds repeated binding writes.

`@recall` provides development instant return while alive and out of combat, with
a five-second cooldown. The native Resurrect button restores vitals and returns
to a saved binding when this mode is enabled. Unbound characters retain in-place
recovery. Destination cells must exist in the configured world. World/cell checks,
outgoing queue admission and native position encoding complete before persistence.
The native Recall request now uses the same instant development path, as
described below. Portal-space presentation, animations, original cast timing
and death penalties are not implemented by this command.

Forced movement increments a checked native force-position stamp, preserves the
avatar lifetime, and uses the neutral `7f7f7f` motion vector. Old-stamp movement is
rejected after return; subsequent client updates must echo the new force stamp.
The owner receives an ordered correction, while normal movement retains its prior
delivery policy. Scene relocation updates nearby player/hand-object lifecycles
and old/new cell content. Resurrected vitals/Dead state precede the correction and
UI unlock in the owner's ordered queue.

Legacy binding uses journal event `l1`; live recall uses `p1`; bound resurrection uses a
single `r2` event for restored vitals and destination. Replay validates ownership,
life state and the saved bind point. New position snapshots use `AC2POS02`, with
the character-journal revision included in their checksum. Startup reads both
`AC2POS01` and `AC2POS02`, rejects snapshots newer than the journal, and overlays
only journal relocations newer than the snapshot. Later ordinary movement in a
flushed snapshot therefore supersedes an earlier recall, while an acknowledged
relocation survives a crash before the next movement checkpoint. Old readers
cannot read these new journal events/snapshots.

Retained evidence includes `binding.to.lifestone.1.elog`,
`binding.to.lifestone.2.elog` and `recalling.to.lifestone.elog`. Native position
packing follows `0069be20`; position reception is recovered at `004ed470`.
The new gameplay/restore path is compile checked only. No automated tests, native
client checks, existing player-data mutations or running-server replacement were
performed.

## Table-backed skill advancement

The earlier investigation inspected low-ID `6f0000f*` records instead of the
`6f001ef*` records actually referenced by the imported skill catalog. The former
are unrelated graphs; that did not establish that XP tables were absent.
`tools/import_skill_advancement.py` follows every catalog reference and extracts
30 cumulative native `ffff0037` u64 vectors from package `1e5` roots. Reference
tables, field tags, vector counts, level bounds, monotonic costs and heap/footer
prefixes are validated; unsupported shapes stop conversion. SHA256 provenance
and FNV compatibility fingerprints cover each complete source record.

HUD startup verifies all 30 advancement fingerprints against the supplied DAT.
The runtime performs bounded binary searches in the embedded integer tables.
The next base level costs its cumulative threshold minus previously invested
XP. Level-one zero thresholds do not produce an extra free Raise request. The
skill's imported cap and table length bound advancement. Race/profile eligibility,
trained state, life state, arithmetic and sufficient available XP are checked
before mutation. Untraining an advanced skill remains unavailable; no invented
refund or timed untraining is applied.

One fsynced `k2` event stores the skill and payment and publishes the skill book
and XP balance together. Replay recomputes the immutable table cost, repeats
eligibility and verifies the recorded payment. Total earned XP and character
level are unchanged; only available XP decreases. Native replies update long-int
XP qualities and SkillData's invested-XP fields, including restoration on entry.
The base rank is retained; equipment/descendant adjusted-level bonuses remain
separate work. A 100 ms local advancement cooldown bounds repeated writes.

`skill-state --event r,SKILL_HEX` offers the same offline transaction under the
normal store lock. `player-state --event a,AMOUNT` is the explicit operator-only
available-XP debit. Network requests cannot supply their own price or award XP.
Existing `k1` books begin with zero investment. Changing advancement tables after
`k2` records exist requires a migration.

The retained skilling capture's cumulative values `8,19,35,57,89` match the
referenced `6f001ef5` table prefix. That is static format/data evidence; native
advancement UI and full original semantics remain unverified. No tests or native
client checks were run, and no existing character XP was changed.

## Native development Recall skill

The captured `8204000b` skill-use profile consists of tagged use flags `8`, a
skill ID and a target IID (28 argument bytes, no trailing data). The dispatcher
now routes Recall `343` to the bound-return service when development lifestones
are enabled and the caster owns that skill. The selected target is ignored for
this self-only service. Unsupported active skills return an unavailable message
without spending XP or vigor or scheduling an effect.

Development binding grants Recall as a locked, zero-credit level-one skill after
checking its imported species/common-kind/cap/eligibility shape. A single `l2`
event commits the binding and grant together. `l1` retains its prior replay
semantics; a living previously-bound character receives the missing grant on
entry through `k3`, provided its skill book has capacity. New binding requires
space in the bounded skill book. Native training and generic operator grants
cannot bypass the separate Recall grant path.

The binder receives a native skill update before the use-success reply. UI_Load
restores the grant with the skill hash; the existing hotbar accepts a skill
shortcut. Successful native Recall and `@recall` share the persisted destination,
life/combat/cooldown checks, forced-position correction and skill/combat UI reset.
The server cooldown remains five seconds; original cast/cooldown presentation,
portal-space transitions and animation are not reproduced. The original starter
grant rules are not inferred from this explicit development policy.

`recalling.to.lifestone.elog` contains the tagged Recall request and native skill
update. This integration is compile checked only, with no automated tests, native
client checks or changes to existing player data.

## Bounded progression-journal compaction

Repeated combat and regeneration append progression events; the bounded journal
previously stopped all mutations once it reached 16 MiB. Before a new write would
cross 8 MiB, the store now attempts checked compaction. It coalesces uninterrupted
`g1`/`g2`/`g3` runs into final per-character progression checkpoints only when
that run becomes smaller. Every skill, inventory, binding, creation/deletion,
appearance/preference and avatar-lifetime event remains a replay barrier and is
retained. Non-progression history is not discarded. New `g3` states validate
vitals, currency, monotonic earned XP and available-XP growth bounds.

The store replays the old journal and verifies it matches active character state,
then replays the compacted result and compares all characters, logical revision
and final relocation destinations before publication. A `j1` revision base keeps
future append revisions monotonic despite fewer physical records. The current
position snapshot is flushed at that revision before replacement, so rebased old
relocation records cannot override later checkpointed ordinary movement.

The replacement is written and fsynced separately, the prior journal is copied
and fsynced to `characters.journal.previous`, then the canonical journal is
replaced and reopened for append. The exclusive store lock remains held.
Integrity/replay mismatches poison further writes; a failed post-replacement
reopen also stops writes. The preceding backup is bounded and replaced at the
next successful compaction; it is not a complete long-term combat audit archive.

Unproductive attempts back off by at least 1 MiB instead of rewriting on every
mutation. The 16 MiB input limit remains: a journal dominated by retained
non-progression transactions can still exhaust that cap. This change addresses
repeated progression history rather than claiming unbounded storage.

`compact-data --data PATH` runs the same checked operation offline under the
normal exclusive lock. No compaction command was executed against player data
for this change. Compilation only; no tests, benchmarks, native game checks or
running-server replacement were performed.

## Imported development portal routes

`tools/import_portal_routes.py` joins the retained placed-portal metadata to the
numeric telepad table without executing SQL. It parses bounded tuples, requires
active destinations, validates finite positions and planar normalized rotations,
and checks backing cells. Source/target omissions and unsupported hero/kingdom
eligibility are reported rather than bypassed. The output contains 156 imported
routes and one explicitly authored training-gateway source, using telepad 1473's
human bootcamp-exit destination. Four original entries remain unresolved.
Source snapshot and telepad SHA256 provenance are retained with the route data.

`--portals true` requires the initialized 7148 HUD and opts into instant
development portal travel. `serve-training` enables it, merges the 247 base world
placements with its three authored starter objects, and includes the gateway.
Only routes whose source object is loaded are enabled. Startup checks source
identity, definition, cell and coordinates against the loaded content and reads
every used destination's backing cell. The source cannot be replaced with an
arbitrary client-supplied destination or a phantom object.

Native object use requires an owned active caster, a visible same-cell portal
within four meters, sufficient imported minimum level, living/out-of-combat state
and the five-second world-use cooldown. Unresolved placed portals are rejected
with a readable destination-unavailable message. The captured portal-success
profile uses `82160021`, package `245`, authoritative destination Position `5a`
and server-computed interaction distance. The destination correction and combat
reset follow in the owner's ordered queue after prevalidated admission.

A single `p2` journal transaction durably relocates the character without changing
its binding, XP, inventory or vitals. Replay validates a living owner and bounded
placement; revised snapshots and compaction retain the relocation. Native
position stamps reject stale pre-travel movement. Scene relocation retires old
cell/player/hand objects and creates those visible at the destination.

Heading is quantized to the supported native movement byte. Collision/LOS,
original portal-space transitions, cast/animation timing, hero/kingdom access,
portal linking/recall and native travel acceptance remain separate work. The
retained `porting.into.linvak.weapons.chamber.elog` supports the success graph;
its original build identity is unverified. Destination extraction used the
original installed cell DAT after the earlier converted asset directory became
unavailable. This is static data/code integration only; no tests, native game
checks, existing-character travel or running-server replacement were performed.

## Training item services and session groups (2026-10-24)

The training profile now exposes four fixed-price stock items through `@shop`,
`@buy potion|weapon|mushroom|tallow`, and `@sell SLOT` (1-based dense bag index).
Quartermasters are authored placements using DAT entity `47003724`, visual
`1f001941`. Every trade rechecks a ready living owner, out-of-combat state and a
four-metre same-cell range. The captured native sale `820f0002` has no merchant
identity or nonce, so it additionally requires a nearby shop context opened in
the preceding 30 seconds. Client quantities other than one are rejected. Prices
and item descriptions are never accepted from the request. Native catalog graphs
are unresolved; no fabricated shop graph is sent.

`crafting.rs` decodes the native ingredient check `82060006` and execution
`82060012`, returning `82060016` / `8206001c`. Exact DAT recipe graphs
`6f001f17` / `6f001f18` identify health/vigor potion outputs and Mucor Mushroom
`47001744` plus Tallow `470027a7`. One item of each, guaranteed success, no skill
requirement and no craft XP are explicitly authored training rules. Original
ingredient field 12 differs (1 versus 100); its complete quantity semantics are
not reconstructed. `@craft health|vigor` makes this supported profile usable
without claiming recipe discovery UI is complete.

Purchases, sales and crafting use the existing `a1` journal transaction, coupling
all removals/grants with gold changes in one flush. The entire native replication
batch is encoded and admitted before persistence. Removed items receive native
inventory removal before DestroyObject, then grants use fresh globally allocated
IIDs, CreateObject and acquisition. Native acquisition field 6 is AppearanceFile,
not the texture icon; a null DID is capture-backed. Tallow's pinned descriptor has
no AppearanceFile, while its actual texture icon is `410082d6`. No texture DID is
substituted for the absent appearance animation.

Health/vigor potion use removes one owned bag item and restores at most 25 of its
vital atomically, with a shared five-second cooldown. Full vitals and rejected
requests do not consume the item.

`groups.rs` implements create, invite, accept, decline, dismiss, leave and disband
using native `820c` requests and fresh member/invitation graphs. Package sizes
`1f0=8`, `1ee=5`, `1ef=7` are validated against the WLib. Limits are eight members,
32 groups, 64 invitations and a 30-second invitation expiry. Immutable plans are
admitted for every connected recipient before publishing membership. Offline
members/invitees are removed; leader departure disbands the session group.
Native snapshots refresh changed vitals/levels/cells. `@group` provides command
access and `@g MESSAGE` provides explicit group chat. Only the captured individual
XP/personal-reward creation mode is enabled; shared XP, round-robin loot, original
native group-chat routing and persistent membership are unfinished.

Library/binary compilation and an optimized build succeeded. No test suites or
automated native-client runs were performed. The local server was restarted with
the training profile; complete native shop/crafting/group acceptance remains open.

## Native training interfaces (2026-10-24)

Quartermaster interaction now builds the native four-item store catalog through
`b83a0013`; purchases use strict `b83a000b` requests and the catalog refresh
`b83a002a`. Graph layouts `385d/382e/3812`, a pinned store template and the actual
gold definition are validated before enabling it. Offer IIDs are fresh, scoped to
the active shop and separate from owned item identities. Leaving range, death,
combat, logout or 30 seconds of inactivity retires the shop context. Buy requests
cannot choose prices or grant an unissued offer. Inventory selling remains the
fixed buyback surface; the newer shop's consignment tab is a separate, unfinished
escrow/listing feature and must not be interpreted as an immediate sale.

The native crafting panel receives default skills `82060034`, then tree state
`82060019`, after UI_Load. Only the training skill and two actual potion recipes
are advertised, with fresh graphs and zero earned counters. Original craft-rule
parity and native UI eligibility/assembly still require client acceptance.

Native group creation accepts the panel's default settings and explicitly
normalizes them to the supported individual-XP/personal-reward group. The owner
is told this policy; snapshots report the actual settings. The pinned client's
FellowshipAdaptor path consumes the existing name, leader, member map and flags.

Three authored Pectar Training Master placements accept/update/claim the training
quest on interaction. Two extra corridor Idol Spawn allow the three-kill objective
without waiting for one creature to respawn repeatedly. Native SelectionInfo
`8212002c` now populates the target panel and follows creature health changes,
separately from public object qualities. Its exact package `ab` packing follows
`006097c0` and `ac2_npc_target.elog`. Native 36-byte type-zero examination clear
requests are accepted as no-ops, following `00609530`/`00609660`.

These interfaces compile; graph construction alone does not establish successful
display or interaction in the original client. No test suites or automated game
runs were performed for these changes.

## Native ingredient quantities and safe stack removal (2026-10-24)

Further bytecode tracing supersedes the earlier one-of-each training policy.
Ingredient function `01d4000c` at `c9558` reads ingredient field 12, and
`01d40006` compares it with quantity getter `01720010`, backed by native integer
quality `0106` (`gmWeenieDesc::InqInt`, `006021b0`). The actual potion recipes
therefore require one Mucor and 100 Tallow. Shop stock now grants an exact
100-unit Tallow bundle, with a bounded runtime stack limit of 100; partial craft
consumption remains unsupported. The client and server use the same quantities.

Full-stack removal uses a distinct `b,IID,quantity` inventory journal command.
It validates the complete expected quantity before removing the item and closing
the dense bag gap; the older `d` command retains its quantity-one replay behavior.
The existing atomic player transaction couples both ingredient removals and the
fresh potion grant. Native acquisition fields 26 and 48 carry the Tallow count,
and CreateObject supplies the contained object's quantity. Actual stacked-Tallow
captures support those quantity fields; direct-grant native acceptance remains
pending. Full-bundle sales verify the request quantity equals the owned stack;
partial requests cannot destroy a larger stack.

The newer store's Sell confirmation is a paid consignment listing, not immediate
fixed buyback (`gmShopNewSaleUI::AddSale`, `0041ad40`). Those native requests now
receive an explicit unsupported-feature message without inventory/gold changes.
The supported inventory Sell route remains separate; no fictional fee/escrow
transaction is acknowledged.

Standing initialization is now explicit for the training actors. Idol Spawn
and quartermaster captures supply mode `40000001` plus the scale slider
`4000000a=1.0`. The mentor's visual points to behavior `44000018`, whose native
initial mode is also `40000001`. PhysicsDesc defaults mode to zero, and
ApplyPhysicsDesc/ModeTracker apply that zero rather than preserving the table's
initial mode (`00641c70`, `004ec830`, `0068a9f0`, `0067d000`). The mentor therefore
receives the DAT-derived initial mode. This repairs behavior initialization;
parts assembly is separate, so it does not prove a visibility report fixed.

The expanded manifest contains 772 base/observed placements, including 495 newly
validated observations, and the training pack adds 13 authored objects. Combined
with the operator's preserved local placement, the deployed manifest contains
786 objects. The capture scan found no grounded actors in region `0299`, so the
training placements remain authored rather than being described as recovered
original spawns. New summoned-pet observations were excluded. Full source hashes
are retained in the manifest; detailed extraction output is local build evidence.

Native merchant affordability is aligned with the server's existing gold state.
Sale field 6 stores raw float price bits with heap tag 4; native SetCostQuantity
`0046d9e0` copies those bits into `m_fCostQuantity`. Store affordability `b8320005`
resolves the gold currency through enum master `28000000` and table `2800001d`:
key `41000009` maps to DID `47001108`. That currency selects function `821a0004`,
which reads private integer quality `0c1c`, matching Progression's gold updates.
No separate physical gold stack is required for this native catalog. This is
static evidence, not an observed successful purchase in the client.

## Equipment-window and recovery repair (2026-10-24)

A player reported reduced interaction and failure to unequip through the equipment
window/drag-to-bag path. The running executable was confirmed to be the latest
deployed build. The session recorded 312 incoming packets, 257 rejections and only
21 dispatched events; that counter alone does not identify the rejection cause.
Packet and rejected-channel diagnostics now have independent bounded reason
budgets, so pre-ready application messages cannot hide later transport failures.
Diagnostics expose sequence/order counts, never ISAAC keys.

The equipment-window path was not routed. It uses native `820e0015`, a strict
180-byte `1f6`/`1f5` action constructed by `b4000e` through `1f6002b`, then
`9ab4000a` / `820e005b`. The source equipment slot and repeated identities are
validated; fields 6 and 18 are the requested backpack destination. Both recovered
native callers supply concrete positions, not a negative sentinel. The server
rechecks ownership and the currently equipped hand, then inserts at the bounded
dense-list destination. Empty positions beyond the dense list append at its end.
The distinct `n,IID,slot,destination` journal command preserves old `u` replay.

Generic item use and explicit unequip now share hand-change preparation and
ownership rules. Scene shape/capacity and the owner's full destroy/create/ack
batch are admitted before persistence. Replies describe the actual final bag
position. Captured `820e0056` replies retain tag 4 on equipment-slot fields 7/15;
the previous reply changed those tags to 2. Destination fields retain their
constructor-zero representation at zero and the native scalar representation
when nonzero.

Separately, native optional-header processing stages `HandleNak` before
`HandlePak` (`004c5d60`, `004c5d20`). The old receiver retired ACKed packets before
the server collected requested retries. A combined NAK/ACK could therefore erase
the missing world/UI packet without retransmitting it. Retries are now staged
before retirement and published outside fallible application handling. ACK ranges,
checksums, queue acceptance, sequence windows and ISAAC semantics are unchanged.
No late-NAK history cache is introduced by this repair.

These are source-grounded repairs, not proof of the reported session's full cause.
No test suites or automated native-client runs were performed; fresh player
unequip and interaction acceptance remains required after deployment.

### Delayed missing-packet requests

The subsequent recovery repair also handles a missing-packet request arriving
after a separate highest-observed acknowledgment. Up to 256 acknowledged packets
remain eligible for explicit retransmission for 30 seconds after acknowledgment.
They do not occupy the active send window or receive automatic timed retries.
Exact encrypted bytes are reused without advancing ISAAC or message identities.
The existing 100 ms requested-retry rate limit and eight-retry budget still apply;
repeated acknowledgments do not renew the retention deadline. Storage is bounded
and shared across transactional transport clones. These cache limits are emulator
policy, not a claim about the original server's retention policy.

This closes a source-visible recovery gap; there has not yet been a new native
client session establishing whether it explains the player's interaction report.

### Player-action ordering stall (2026-10-24)

The subsequent native session did reproduce a broad interaction failure:
600 received packets, 510 rejected, and 20 dispatched events. The first logged
failure was Capacity at packet 30, with 16 buffered ordered messages and expected
application stamp 19. Later state reached stamp 20 but remained full. Item use,
NPC use and combat all pass through this ordered path.

Packet completion now handles ordered messages by stamp before admitting future
messages. Otherwise a future completion appearing first in a datagram could
reject the same datagram's later gap-closing completion. Buffered duplicates are
checked before capacity. Retained native captures establish unsorted completion
order, including login packets with stamps [11,7,6] and [10,4]. Native stamp
extraction and the shared queue-3/7 counter are unchanged. The count bound now
matches the existing 512-stamp window, retaining the 256 KiB buffered-byte bound.

Incomplete fragments no longer expire silently at 30 seconds: their packet
receipts were already acknowledged, so deleting them could permanently lose an
ordered message. They remain bounded by 16 assemblies, 256 fragments per blob
and 64 KiB per blob, and are discarded when the session is removed. Capacity
errors now identify the expected/received application stamp and buffer use.
This repairs confirmed source-level failure paths; fresh player acceptance of
the reported interaction failure remains pending. No test or client runs were
performed by the agent.

The authored corridor training hub now also has a quartermaster and lifestone,
using the same validated asset profiles as the start area. The corridor merchant
is routed through native use and the existing shop/range/context validation.
Players saved in cell 0299011d can buy ingredients and bind locally alongside the
existing mentor, practice creatures and outdoor portal. These placements are
development content on the previously observed corridor plane; collision and
fresh client visibility remain unverified.
