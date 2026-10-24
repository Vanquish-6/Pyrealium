# Stationary world content

Game-data mode loads the bundled snapshot automatically, independent of the
working directory. Use the normal server options and:

```powershell
pyrealium serve --game-data PATH
```

`--content content/world.tsv` instead loads an editable external manifest.
`--content none` disables world content. Selection-only mode without game data
does not load placements. The compiled bundle changes when the server is rebuilt;
operator edits to an external manifest require the explicit file option and a
server restart. There is no hot reload.

`lifestones.tsv` contains 89 placements imported from the supplied lifestone
database table snapshot dated 2012-08-11. Its source SHA256 is retained in the
file. These are imported placements, not positions recovered from the client
executable. Their original client/build identity has not been established.
They do not populate the human starter dungeon.

`world.tsv` is the combined pack: those 89 lifestones plus 158 portal placements
from the supplied portal table snapshot dated 2012-02-28. The full 160 portal
records, including target telepad IDs, string tokens and eligibility fields,
are preserved in `portals.json`. Two locations (`30780019` and `2e720032`) have
no backing terrain in the supplied assets and are marked `missing-cell` rather
than spawned. None of these placements are claimed to originate from client
decompilation, and the source snapshot's build identity remains unknown.

Portal visuals use four individually pinned qualities layouts, then verify
their physical definitions and visual records. The converter retains the
source quaternions instead of reducing them to byte headings. Source row ordinal
is part of portal IID allocation for this immutable snapshot; regenerating from
a reordered table is a content migration, not a supported hot reload.

The visual chain is checked against the supplied assets: definition `470007de`,
qualities `810007de`, physical definition `47000075`, visual `1f00002d`.
The importer requires the exact qualities SHA256 recorded in the manifest before
reading its physical-definition field at byte 236. It also checks the complete
16-byte physical definition, the visual record and each backing terrain block.
It does not apply these offsets to arbitrary definitions.

The content currently supplies stationary presentation and target visibility.
Lifestone binding, death/resurrection, summon restrictions and use scripts are
not implemented. Portal travel, triggers and destination transfer are also
unfinished. Level/hero/kingdom restrictions in the source table have not
been translated into gameplay rules. NPC AI, quests, merchants, loot and portals
require additional data and behavior; this is not a full world import.

Regenerate the included file from the supplied table and matching assets:

```powershell
python tools/import_lifestones.py --source PATH --game-data PATH --output content/lifestones.tsv
```

The converter parses numeric rows; it does not execute SQL. Unknown table
columns, definitions, rotations, duplicate IDs, missing assets or terrain records
stop the import before the output is written.

The combined importer is `tools/import_world.py`. It accepts `--portals`,
`--lifestones`, `--game-data`, `--output` and `--report`. Missing portal cells
stop conversion by default. `--allow-unresolved` explicitly permits a partial
manifest and retains every excluded portal in the report. No SQL is executed.

## Definition catalog and explicit placements

`definitions.tsv` contains 7,583 full entity/visual mappings from the supplied
workbook's `Entity` worksheet, filtered against actual DAT record presence.
Source annotations remain labels only; they are not injected as native names
or treated as NPC behavior, spawn positions, loot or balance. The catalog's
source SHA256 is embedded in its header. Another 319 rows with unsupported or
abbreviated visual mappings are retained in `unresolved-definitions.json`.
No automatic NPC placements are inferred from this catalog.

`tools/import_definitions.py` regenerates the catalog with `--source`,
`--game-data`, `--output` and `--report`. Reading legacy XLS requires `xlrd`;
the Rust server gains no dependencies. Numeric Excel cells containing full
all-digit hex identifiers are interpreted as their displayed integer strings
in base 16, not as decimal DIDs. The source workbook is never modified.

To author a stationary NPC/object at a position chosen by the operator:

```powershell
python tools/place_content.py --catalog content/definitions.tsv --game-data PATH --output content/world.tsv --entity ENTITY_HEX --cell CELL_HEX --position X Y Z --heading 0
```

The tool requires an unambiguous catalog entry, existing entity/visual/cell data,
valid coordinates and available per-cell/global capacity. It allocates an unused
IID in the operator range beginning `8140000000020000`, marks the placement as
operator-authored, and replaces the manifest atomically. It does not alter a
running server. Regenerating `world.tsv` from source tables replaces such local
edits, so retain them separately before regeneration. No made-up demonstration
NPCs were added to the shipped pack.

## Manifest format

The first line can be `pyrealium-world-v1`. Following nonempty lines contain eight
tab-separated fields: object IID, entity DID, visual DID, cell, X, Y, Z, heading.
Lines starting with `#` are comments. IDs are hexadecimal, coordinates are
decimal and heading is a decimal byte (0..255). Object IDs use the local
`8140` namespace and must be unique. Entity/visual DIDs use `47`/`1f` prefixes.
`pyrealium-world-v2` instead has eleven fields, replacing heading with quaternion
W, X, Y, Z. All quaternion components must be finite with norm within 0.01 of 1.
The combined pack and authoring tool use v2; existing v1 files remain supported.

The runtime rejects files larger than 4 MiB, placement lines over 512 bytes,
more than 16,384 objects or more than 64 objects in one cell. Positions must be
finite and bounded. All referenced definitions, visuals and backing cell data
must exist before the server opens its sockets. This checks data presence,
not arbitrary entity/visual compatibility or terrain traversability.

Outdoor cell IDs 1..64 within a block use its `ffff` terrain record. Supported
interior cells use their own record. Reserved IDs and the gap before `0100`
are rejected. The player movement data-presence check uses the same mapping,
with a bounded cache of previously accepted cells. It does not enforce collision.

Objects are created after scene readiness, destroyed when their owner-visible
cell is left, and recreated when that cell is entered again. Visibility follows
the existing same-cell policy; nearby-cell/outdoor-block visibility is unfinished.
Encoded spawn/destruction payloads are immutable and shared between recipients.
Content stays outside the player-session index and player count limit.
Object instance stamp 1 is local policy for these immutable server lifetimes;
runtime replacement/hot reload is not supported.

No tests or native-client checks were run for this addition. Compilation and
read-only asset inspection do not prove in-game appearance or interactions.
