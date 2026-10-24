# Stationary world content

Enable the included placements with the normal server options and:

```powershell
pyrealium serve --game-data PATH --content content/lifestones.tsv
```

`lifestones.tsv` contains 89 placements imported from the supplied lifestone
database table snapshot dated 2012-08-11. Its source SHA256 is retained in the
file. These are imported placements, not positions recovered from the client
executable. Their original client/build identity has not been established.
They do not populate the human starter dungeon.

The visual chain is checked against the supplied assets: definition `470007de`,
qualities `810007de`, physical definition `47000075`, visual `1f00002d`.
The importer requires the exact qualities SHA256 recorded in the manifest before
reading its physical-definition field at byte 236. It also checks the complete
16-byte physical definition, the visual record and each backing terrain block.
It does not apply these offsets to arbitrary definitions.

The content currently supplies stationary presentation and target visibility.
Lifestone binding, death/resurrection, summon restrictions and use scripts are
not implemented. Level/hero/kingdom restrictions in the source table have not
been translated into gameplay rules. NPC AI, quests, merchants, loot and portals
require additional data and behavior; this is not a full world import.

Regenerate the included file from the supplied table and matching assets:

```powershell
python tools/import_lifestones.py --source PATH --game-data PATH --output content/lifestones.tsv
```

The converter parses numeric rows; it does not execute SQL. Unknown table
columns, definitions, rotations, duplicate IDs, missing assets or terrain records
stop the import before the output is written.

## Manifest format

The first line is `pyrealium-world-v1`. Following nonempty lines contain eight
tab-separated fields: object IID, entity DID, visual DID, cell, X, Y, Z, heading.
Lines starting with `#` are comments. IDs are hexadecimal, coordinates are
decimal and heading is a decimal byte (0..255). Object IDs use the local
`8140` namespace and must be unique. Entity/visual DIDs use `47`/`1f` prefixes.

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
