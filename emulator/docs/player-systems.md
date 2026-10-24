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

Dead characters cannot attack or submit movement. In this development profile,
chat command `@resurrect` restores saved vitals at the current location; it is
not lifestone binding, corpse recovery or original resurrection/penalty behavior.
Ordinary chat rate limits still apply. Encounter/world state is transient across
server restarts; player rewards and vitals are durable. Journals with `g2` require
the newer reader. Training does not yet grant starter equipment or abilities.

Compiled only. No tests, benchmarks, native client launch or deployment/restart
were performed for this addition.
