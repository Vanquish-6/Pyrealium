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
