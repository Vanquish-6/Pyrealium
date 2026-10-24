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
gold and the level profile's skill credits; skill training/spending is not yet
connected. Other quest/effect/skill payloads remain at their earlier scope.

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
This is local recovery policy; lifestone binding, corpse recovery, death
animations, teleportation and original penalties remain unfinished.
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
