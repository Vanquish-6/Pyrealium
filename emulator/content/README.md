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

`--content observed` selects the separately bundled 277-object observation pack:
the 247 table-derived placements plus 30 actors recovered from three supplied
packet captures. Six capture inputs were examined; the other three supplied no
additional supported placed actors. It includes Banderling Ravers/Thrashers, Idol/Surrogate Spawns,
Furs Turfna, eggs, chests and other world objects at their captured positions.
It does not populate the human starter dungeon or supply AI, trading, damage,
loot, quest interactions, death or respawn behavior. The regular default pack
continues to use only the table-derived placements.

## Capture-observed actors

`observed-world.tsv` contains the observation pack; `observed-actors.json`
records each source capture SHA256, original IID, entity/visual, position,
quaternion, scale, native masks, message SHA256 and aggregate rejection reasons.
Original capture build identity and checksums have not been verified. Positions
are first materialization observations, not proven spawn origins or schedules.
These records remain distinct from table-derived and operator-authored content.

The offline importer `tools/import_observed_population.py` accepts repeated
`--capture` arguments plus `--base`, `--game-data`, `--output` and `--report`.
It reads bounded classic little-endian Ethernet/IPv4/UDP PCAPs, checks framing,
and reassembles blobs by directed endpoints, recipient, iteration, blob ID,
queue and fragment count. Conflicting duplicates and oversized tables/messages
stop conversion. IP fragments and unsupported transport packets are not used.
Reassembly is not authenticated and does not establish complete capture coverage.

The object decoder is grounded in VisualDesc `0063d7f0`, PhysicsDesc `00641530`,
SliderData `0067cfb0` and map packing `00640f40`, FXScalarAndTarget `004d5720`
and map packing `006410e0`, and gmWeenieDesc `006037c0`. It accepts the narrow
parent-visual-only profile and supported placed physics fields. It excludes
player namespaces, contained/equipped objects, missile/behavior-list profiles,
custom names and unknown description fields. It checks all remaining lengths,
finite transforms, quaternion norm and backing DAT records before placement.

Native packets are not replayed into the server or client. Runtime creates fresh
minimal stationary descriptions with local identities/stamps, observed entity,
visual, transform and optional gmWeenieDesc scale. Captured modes, effects,
sliders, velocity, look/target state and other description overrides are omitted.
This preserves a placement snapshot, not exact live behavior or full appearance.
Two-decimal position keys prevent duplicate placements; source IID repetition
is retained only at the first supported observation in each capture.
For overlapping inputs, the first accepted capture owns a cell; later captures
do not add actors to that cell. This avoids stacking snapshots from different
times. Input order and all exclusions are recorded in the report. It does not
establish a complete or simultaneous population across different cells.
New IIDs derive from capture hash and original IID in the reserved local
`81408...` range, with collisions rejected. Global and per-cell limits remain.

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

The graphical tool is `tools/content_editor.py`. On Windows, double-click
`OpenContentEditor.cmd` in the emulator directory, or run:

```powershell
python tools/content_editor.py --game-data PATH
```

Each launch writes a diagnostic log under
`%LOCALAPPDATA%\Pyrealium\content-editor\logs`, including Python callback
exceptions and native fatal tracebacks where Python can capture them. This
keeps errors available when the `pythonw` launcher has no console. Closing the
editor waits for its current DAT read to drain before destroying Tk resources.

It requires Python with Tk support and uses no third-party GUI packages. Search
the catalog by label, entity or visual identifier; select an entry; choose the
game-data folder and content file; then enter a cell, X/Y/Z, heading and optional
scale. The **3D world** tab reads actual DAT creature meshes, the selected
interior room and linked nearby rooms. Drag to orbit, right-drag to pan, and use
the wheel to zoom. **F** or **Focus creature** returns to the selected placement.
Shift-click a world surface to set X/Y/Z through a ray intersection with its
actual triangles. **Cut away ceiling** helps inspect an interior from above.
The camera stays fixed while coordinates, heading or scale change.

Outdoor cells (`0001`–`0040` in a block) load the actual height block and one
neighbor ring. Heights use the native 256-entry height table, not a linear guess;
holes, triangle cuts, surface selection, UV tiling and vertex-color variation
come from the DAT definitions and recovered client formulas. Real terrain
material-stage textures are displayed with native vertex colors. Ceiling cutaway
starts disabled outdoors. Shift-clicking another outdoor cell or block updates
the cell ID and local X/Y/Z together; interior room picks retain their room ID.
Saved neighboring outdoor placements receive the proper block offset.
Road/corner surface blending, water and procedural scenery remain unfinished
and are reported in Details. This preview does not establish traversability.

**Play animation** loops a real native clip for the selected supported creature.
The clip list shows recovered asset IDs and periods; the mapped idle clip is
selected first where available. Pause or choose another clip, and change
**Speed** from 0.1 to 3.0. Playback evaluates native bone keys, quaternion
interpolation, hierarchy, inverse bind matrices and skin weights. Time advances
without accumulating catch-up frames. Only the selected preview is animated;
this does not run AI or gameplay state. Unsupported graph branches are listed
in Details. Root-motion application, graph blending, event hooks and attachment
animation remain unfinished.

The **Position map** tab remains available, with a stable view, coordinate grid,
pan, zoom and **Fit cell**. Clicking it sets X/Y while retaining Z. Existing
placements can supply a position and entity for a new addition; their identities
are retained. Neither preview changes existing objects.

The Windows 3D view uses embedded OpenGL with actual native UV texture pixels,
linear filtering, cutout alpha, a depth buffer and preview lighting. DXT1/3/5
mips are decoded read-only from `portal.dat` and optional `highres.dat`; base
material properties are applied before ordered instance overrides. It requires
no extra Python rendering packages. If Windows cannot create its OpenGL context,
the position aid falls back to sampled triangle colors and reports the reason
in **Details**. Full native shader passes, translucency blending, collision and
native BSP visibility remain unfinished. Some attached visual groups and nearby props remain
unsupported; **Details** lists omissions instead of substituting made-up models.
Where a supported first material layer is decoded, alpha testing, its constant
reference, blend factors and texture-alpha use follow that layer. Zero-filled
skin vertex-color fields are not used as tint, and texture alpha is not treated
as transparency when the material ignores it. Full conditional/multi-pass shader
selection remains outside the preview renderer.
Loading is bounded to 32 cells and 50,000 scene triangles; the viewport draws up
to 30,000, prioritizing the selected creature and reporting truncation. DAT work
runs on one background worker so the editor remains responsive. Native in-game
appearance remains unverified. Python syntax and read-only DAT extraction were
checked; no GUI or game was launched by the agent.

The default output is `content/local-world.tsv`, a new standalone pack of your
additions. To include base placements, choose a separate copy of `world.tsv` as
the output. Serve the resulting file through `--content PATH` on the next normal
server start. The tool does not reload or restart a running server.

The command-line interface remains available:

```powershell
python tools/place_content.py --catalog content/definitions.tsv --game-data PATH --output content/world.tsv --entity ENTITY_HEX --cell CELL_HEX --position X Y Z --heading 0
```

The tool requires an unambiguous catalog entry, existing entity/visual/cell data,
valid coordinates and available per-cell/global capacity. It allocates an unused
IID in the operator range beginning `8140000000020000`, marks the placement as
operator-authored, flushes the temporary file and replaces the manifest atomically.
An exclusive `.edit.lock` prevents overlapping tool saves, and a changed-file
check rejects external edits during validation. If a process crashes while
holding that lock, close all placement tools and remove only the corresponding
stale `.edit.lock` before retrying. It does not alter a
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
The combined pack uses v2; existing v1 files remain supported by the server.
`pyrealium-world-v3` adds a twelfth field: a positive scale up to 100, or `-` to
leave scale inherited. This is local content policy. The observation pack uses
v3 and writes the scale as native gmWeenieDesc bit `0x80`; v1/v2 encodings are
unchanged. The authoring tool accepts v2 and v3. Its optional `--scale NUMBER`
upgrades a v2 file to v3 while preserving inherited scale (`-`) for every prior
object. Both interfaces allocate additions only in the operator IID range
`8140000000020000` through `814000000002ffff`.

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
