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
This does not evaluate advancement curves or implement active skill effects.
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

Advancement requests currently return an unavailable message and do not spend XP.
The supplied client advancement records are not the precomputed table shape
expected by the older reference reader; using its assumptions would debit the
wrong amounts. Active ability use, costs/cooldowns/effects, timed original
untraining, hero/crafting/faction systems and full original training eligibility
remain separate work. No automated tests or native client checks were run.

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
Native recall skill casts, portal-space presentation, animations and original
death penalties are not implemented by this command.

Forced movement increments a checked native force-position stamp, preserves the
avatar lifetime, and uses the neutral `7f7f7f` motion vector. Old-stamp movement is
rejected after return; subsequent client updates must echo the new force stamp.
The owner receives an ordered correction, while normal movement retains its prior
delivery policy. Scene relocation updates nearby player/hand-object lifecycles
and old/new cell content. Resurrected vitals/Dead state precede the correction and
UI unlock in the owner's ordered queue.

Binding uses journal event `l1`; live recall uses `p1`; bound resurrection uses a
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
