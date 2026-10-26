# Build 7148 character and world event evidence

## Durable development item rewards and consumable healing (2026-10-24)

The initialized training profile validates potion definition `47001745`, visual
`1f000e3f`, physical `47001773` and icon `20000268` against the DAT. Its physical
record is exactly `[0b,47001773,40000001,1f000e3f]`. Grant quantity/stack limit 1,
zero equipment mask, 25 health restoration and five-second world-use cooldown
are authored development policy, not inferred original loot or balance.

Qualifying development encounters grant directly into a free supported bag.
`a2` records the new globally sequential item, normal combat cost/XP/gold and
quest objective progress together before creature death is published. Missing
bag capacity skips the item with a readable message; it does not block the
normal payout. Native contained-object creation precedes acquisition `820e005e`
(package `1f8`, 51 fields, 296 bytes). Retained pickup captures cover slot zero;
nonzero slot encoding follows the same package's unsigned index convention,
but fresh matching-client acquisition acceptance remains open.

Owned potion use prepares native removal `820e0058` before DestroyObject, then
vital changes. Sales/crafting captures establish the tagged-IID removal argument;
matching 7148 WLib resolves the removal handler to `1aa60002`. The `a1` player
transaction commits consumption and healing together after outgoing batch
admission. New inventory command `d` removes one unstacked item and closes its
bag-list gap; old `c` consumption keeps its historical semantics. Current item
lifetimes are tracked and removed without leaving stale equipped-child state.
Full-health/dead/unsupported items and cooldown failures consume nothing.

This is a direct-grant training slice, not corpse ownership, ground pickup,
original loot tables, stacking, merchants or crafting. Object-use request shape,
nonzero-slot acquisition and resulting native inventory presentation still need
matching-client acceptance. Formatted, library/server compile checked and an
optimized executable built; no tests or native client were run.

## Durable native training quest and journal restoration (2026-10-24)

The authored Training Grounds quest `4000ff01` counts three authoritative kills
of entity `470007d1`. The active training profile must contain encounters for
that entity before new characters are automatically offered the quest. Living
players can use `@quest`, `@quest start` and `@quest claim`; acceptance/claiming
requires being out of combat. Existing saved quests remain queryable/claimable
when the training profile is later disabled. This is explicit development
content, not an imported original quest or recovered grant rule.

`q1` stores acceptance or claiming; the fixed 120 XP and 10 gold payout is
validated and fsynced with the claimed flag. Repeated claims fail before payout.
`q2` stores qualifying objective progress with the normal encounter cost/payout,
before creature death is published. Both records remain compaction barriers and
replay validates the same transitions. Earlier journals initialize an empty book.

Native `gmQuestInfo` (`ffff00a0`) follows build-7148 `0060af00`; list packing
(`ffff00a1`) follows `0060b500`. Status meanings follow `0060aca0`: underway
`10000000`, completed `30000000`, failed `20000000`. Update `82130024` uses
the capture-established argument order and matching WLib function at `1022f8`.
The later capture's extra maxed-out word is absent in 7148 and is not emitted.
Quest entries restore through UI_Load and update on qualifying kills/claiming.
Icon `4100034b` is checked against the supplied DAT.

Broader quest definitions, NPC conversations, branching/abandon/repeat rules and
fresh matching-client journal acceptance remain incomplete. Formatted and
compile checked only; no tests or native client were run.

## Native player emote routing (2026-10-24)

Strict `8205000b` decoding accepts a tagged token from an eight-emote allowlist.
Wave, Yawn, Moose, Dance 1–4 and AFK token/behavior pairings are corroborated by
retained capture messages; capture build identity remains unverified. Native
behavior encoding follows `004946d0`, `00679220`, `006795b0`, `00497a00` and
`004ec680`, including stop-on-move, supported child fading and AFK visual effect.
Fresh `1007f` messages use the active owned avatar, current instance and a checked
behavior counter. Captured context counters and the reference implementation's
guessed `0xAA` are not replayed.

Initialized UI startup pins enum `23000027` and default-human behavior table
`44000001`. The runtime requires a ready living player out of combat, reserves
owner queue capacity before publishing emote state, then broadcasts through
same-cell scene delivery. Behavior messages join the ordered lifecycle queue.
The one-second cooldown is explicit server policy. Full emote coverage, social
AFK state and ongoing-behavior restoration for late arrivals remain incomplete.
Formatted and compile checked; no tests or native client were run.

## Learned active healing with durable payment (2026-10-24)

The explicit initialized training profile routes native UseSkill `8204000b`
for skill `55` to a prepared self-heal. The pinned catalog entry is human,
minimum level 3, one credit, cap 50, prerequisite `59:0`; the character must
already have learned it. The heal amount 30, vigor cost 10, skill cooldown
8 seconds and ability cooldown 1 second are authored development policy.
Other targets, dead/full-health players, insufficient vigor and exhausted
service capacity are rejected without cost. Native no-target sentinels are
accepted as self selection.

The runtime borrows its bounded cooldown state while a cast is prepared. The
server constructs/admit-checks skill and vital updates, persists SpendVigor and
Heal in one progression batch, then commits cooldown state and queues updates.
Failed preparation/admission/persistence does not spend a cooldown. Unexpired
cooldowns survive logout/relogin within the same server lifetime. Original
cast timing, animation, effects, ally healing, native cooldown UI timers and
restart-persistent cooldowns remain incomplete. Formatted and compile checked;
no tests or native client were run.

## Atomic player inventory/progression transactions (2026-10-24)

`Store::player_transaction` validates up to eight inventory and eight progression
commands as one candidate and fsyncs one `a1` record before publishing either
state. Global sequential item identities are enforced across the complete
batch, including when other characters already own higher IDs. Replay uses the
same candidate validator. Transaction records remain compaction barriers, so
item grants/consumption and their payments or vital changes cannot be separated
by progression checkpoint coalescing. Existing `i1`/`g1`/`g2` meanings are intact.
This is the storage foundation for server-authorized rewards and item services;
it does not grant client packets authority to mint items or choose prices.
Formatted and compile checked; no tests or native client were run.

## Editor load crash: native paint callback re-entry (2026-10-24)

Windows recorded two `pythonw.exe` crashes (`0xc0000409`, fatal-app-exit code 7).
Retained local dumps `pythonw.exe.2424.dmp` and `pythonw.exe.31316.dmp` match the
installed Python 3.14 DLL and `_tkinter.pyd` module timestamps. The first dump's
stack contains `_tkinter.pyd+30f9`, returning from its imported
`PyEval_RestoreThread`, and the Python fatal path at `python314.dll+2063f7`.
The matching Python image identifies the fatal function as
`PyEval_RestoreThread`; the surrounding Tcl stack is servicing an idle callback.

The OpenGL Win32 window procedure previously invoked `Preview3D.redraw`, which
calls Tk `after_idle`, directly from a ctypes native callback. That re-entry can
overwrite `_tkinter`'s saved Tcl/Python thread state before the outer idle
callback restores it. The window procedure now only validates paint and sets a
Python boolean. A regular Tk-owned timer consumes that flag and schedules
redraw. The native renderer no longer accepts or retains a Tk scheduling
callback, keeping this boundary explicit.

Shutdown also keeps the Tk owner alive until the serialized DAT worker has
finished and released its bound methods; only then is the worker joined and
the window destroyed. Otherwise Tk variable finalization could occur on a worker.
The console-free entry point now retains stdout/stderr, callback exception logs,
and Python fatal tracebacks in the user's local application-data log directory.

Syntax and scoped whitespace checks completed; existing dumps were inspected
read-only. No test suite, GUI or game was launched after this patch, so a fresh
user load is still required to establish crash resolution.

## Material alpha and unused skin color fields (2026-10-24)

Read-only extraction showed all 5,185 Gurog vertex colors are zero, while the
body diffuse texture's alpha is mostly zero. Those fields do not imply an
invisible creature. The first native material layer (`005cc0b0`) and its stage
operations (`005be300`, `005c4f20`, recovered TextureOp/BlendMode enums) show
that material `31000407` -> `2b000081` ignores texture alpha and disables
alpha testing. Applying a universal texture-alpha cutoff or vertex-color tint
would therefore erase valid geometry.

The preview now uses explicit terrain near colors and leaves skin/room color
fields unused until their shader use is established. The bounded first-layer
reader retains blend factors, depth-write policy, alpha-test mode and constant
Waveform NONE references. Stage SELECTARG/MODULATE dependencies determine
whether texture alpha participates; ignored alpha is uploaded opaque. Supported
per-material alpha/depth/blend state replaces the universal cutoff. Conditional
shader selection, later passes and animated UV/alpha modifiers remain incomplete
and are reported rather than guessed. Syntax and read-only DAT extraction only;
no GUI, test suite or native client was launched.

## Native outdoor height blocks and terrain materials (2026-10-24)

`preview_terrain.py` reads region, landscape, terrain and surface descriptors,
height blocks and outdoor static-entity prefixes. Evidence includes `00629d60`,
`0048f310`, `0048f980`, `0048ca50`, `0048cca0`, `00522000`, `006c6460`,
`00522380`, `006c5910`, `006c45a0`, `006c44d0`, `006c4970` and `006c36d0`.
The installed landscape uses 160 m blocks, 10 m vertex spacing and the actual
256-float height lookup. The real lookup includes a nonmonotonic entry; no
monotonic or linear substitute is imposed. Hole code 30, native hash-based
diagonals, slope/surface selection, material variants and HSV variation are
preserved within the supported profile.

Ground diffuse textures are read from the actual first material-layer stage
(`005b9e00` -> `005cc0b0` -> `005be300`), after base properties and instance
overrides. Examples: `2b000033` -> `40000241`, `2b000037` -> `40000244`,
and detail material `2b000007` -> `4000003c`. Alpha mask `4000023e` is not
misrepresented as diffuse ground texture. The OpenGL backend now modulates
textures with native packed vertex colors. Detail UVs are retained; complete
native detail/corner/road passes, water and procedural scenery remain incomplete.

At `74ba0008`, read-only extraction produced nine blocks, 4,608 triangles,
25 material-image group references and seven static placement records. Unmapped
static visuals are explicitly omitted. Neighbor geometry and saved placements
share the selected block's coordinate frame. Outdoor ray picks canonicalize the
block/cell and local coordinates together; interior picks preserve room IDs.
No GUI, test suite or native client was launched. These are static/data checks,
not proof of live renderer acceptance or collision semantics.

## Native clip playback and bind-correct skinning (2026-10-24)

`preview_animation.py` follows AnimMap `006796f0`, DBAnimator `0067a5f0`,
AnimAsset `0069ab70`/`00699400`, key records `007366b0`/`007366f0`/`006e5e80`,
setup `006105a0`, inverse bind `006124f0`, and deformation `004ee0c0`.
The Gurog profile resolves 37 bones, 22 stored clips and 15,573 key records.
Idle token `80000001` maps to clip `050007d3`. Read-only extraction sampled
both native meshes at 0.25 s, producing 441 and 4,744 finite animated positions
and matching normals. This is asset evaluation, not matching-client acceptance.

The editor provides native clip selection, play/pause and bounded speed control.
Hierarchy transforms, key interpolation, quaternion slerp and inverse-bind
weighted deformation use the actual stored data. Prepared nonzero influences
and per-time bone matrices are cached; instance transforms are evaluated once
per matrix instead of composing quaternion helper allocations for every vertex.
Wall-clock playback skips overdue frames instead of building a Tk work backlog.

AMHuman, AnimCube and AMSwitchME graphs supply actual leaf clips. AMLook is
reported unsupported. Original behavior-state selection, graph blending, root
motion, event hooks and attachments are not executed. No procedural substitute
motion is generated. Syntax and read-only asset extraction completed; no test
suite, GUI or game was launched.

## Per-pixel texture viewport (2026-10-24)

`preview_gl.py` provides an embedded Win32 child window with its own device
context and OpenGL context. The renderer uses real mesh UVs, decoded RGBA texture
pixels, linear filtering, perspective interpolation, depth testing and normal
lighting. It does not change Tk's pixel format. Input hit testing passes to the
Tk parent so the existing camera and placement controls remain usable. Geometry
arrays and uploaded textures are reused and bounded; old scene resources are
released. The view reports the active driver or the reason for software fallback.
Cutout alpha uses an explicit editor threshold; complete native translucency,
multi-pass shaders and material lighting parity are not claimed.

Python syntax and source/interface inspection completed. No GUI, native client
or test suite was launched, so hardware-driver and interactive acceptance remain
unverified.

## Material defaults and zero-category surfaces (2026-10-24)

`preview_material.py` now reads the base RenderMaterial's embedded modifier
properties before ordered material-instance overrides (`005b9e00`, `005b91b0`).
It also accepts a zero data-category word for RenderSurface records:
`DBObj::UnPack_Type` (`006d7830`) reads that value verbatim; categorization is
determined by the object's family, not a high bit in the category value.
Room material `3100034f` now resolves its inherited slot-3000 texture `40000771`
to real 256x256 DXT1 pixels. Base material `2b` records can be resolved directly.
Decoded texture images are retained on mesh material groups for per-pixel
rendering. Complete shader layers and effects are still outside this reader.
Checked through syntax compilation and read-only DAT extraction; no test suite
or game was launched.

## Native geometry for the content editor (2026-10-24)

The former position-dot canvas is supplemented with an actual DAT geometry
viewport. Visual description and part-group traversal follow `0063d7f0` and
`006443c0`; setup mesh arrays follow `006105a0`. Render-mesh prefixes follow
`004d2ab0`, `004d3bf0`, `004cf3a0`, `00730160`, `0062b380`, `0052c260`, and
`005bdea0`. Vertex positions, skin weights, normals, UVs and real triangle
indices are bounded; physics/BSP tails remain opaque. The renderer displays
stored bind-pose positions rather than evaluating animation deformation.

Interior cells follow `00482da0`, `006200b0`, `0049c980`, `00617370` and
`006161b0`. Room meshes receive the cell frame transform; static entity positions
are already in block space. Connected/visible cell references supply nearby
room geometry under explicit bounds. Static prop visuals require supported
catalog mappings and part-group graphs; unsupported data is reported.

Material reads follow `005badf0`, `005b91b0`, `005daf10`, `005bd850`,
`005b1c40` and `006d7830`: material instance `31` to modifier `30`, texture
slot `3000` to texture `40`, then surface `41` mips in portal/highres archives.
The stdlib decoder supports DXT1/3/5. UV-centroid samples color triangles; this
does not constitute per-pixel textured rendering or complete native shaders.

Read-only extraction at cell `02990109`, visual `1f0002f7` recovered the room
mesh `01000084` (814 triangles) and Gurog meshes `06000563`/`06000564`
(278/2,328 triangles). Combined supported surrounding geometry produced 17 mesh
instances and 20,502 triangles at the user's placement; unsupported room meshes,
props and one material inheritance case were reported rather than fabricated.
Python syntax and DAT extraction were checked only. No test suites, GUI or
native client were launched; no running server was replaced.

## Combat update traffic and regeneration work (2026-10-24)

Encounter owner events now retain their preceding committed progression state.
Private updates compare integer qualities, earned/available XP, gold and Dead
against that state, emitting only changed values in their established order.
No network cache or acknowledged-state assumption is introduced: existing
ordered reliable delivery remains responsible for transmitting every admitted
delta. Login and resurrection retain full-state restoration.

From the encoder shape, a health-only hit now creates one private quality
message rather than eleven progression messages; health-and-vigor regeneration
creates two. This is a source-level message-count reduction, not a throughput
or latency benchmark. Skill-credit updates are emitted only when the available
credit count changes, and death presentation requires a living-to-dead transition.
Public nearby-player state still passes through the existing scene cache.

Each regeneration pass builds a deterministic set from pending attacks and
creature targets. Players use membership checks instead of rescanning the full
creature map. Combat formulas, journal durability and regeneration eligibility
are unchanged. Formatted and compile checked only; no tests, benchmarks or
native client checks were run.

## Development training attack integration (2026-10-24)

The explicit training profile grants skill `b0` with locked rank 1 and zero
invested XP. Its pinned DAT catalog definition is human, minimum level 1,
zero credits, cap 1, kind `44`, without prerequisites or exclusions. This is
an authored development entitlement, not proof of original starter grants.
New `t2` transactions save the hand item, skill and once-per-character marker
together; `t1` retains its historical item-only replay semantics. A separate
`k4` transaction upgrades previously granted training characters. Dead players
and full skill books defer grants without blocking world entry.

The bounded native `8204000b` request now routes this learned skill to one
queued encounter strike. It requires initialized UI, training-kit and encounter
profiles, a main-hand item, and an eligible visible creature within the encounter
range. It shares existing cooldown and transactional vigor/reward handling.
Repeated requests replace one pending strike instead of accumulating attacks;
canceling or changing targets does not reset a paid strike's cooldown. A strike
does not enable automatic repetition. The existing Attack request still does.
Skill data and combat-state updates are admitted before scheduling the strike.
The creature can retaliate after a surviving strike.

Original attack damage, weapon bonuses, cast animations, projectiles, collision,
line of sight and full skill-use completion presentation remain unresolved.
Only formatting, source inspection and compilation were performed. No tests or
native client checks were run, and no running server was replaced or restarted.

All addresses below refer to canonical recovered files under
`../../recovery/exports/ac2client.exe/functions/<address>.c`. These are recovered
pseudocode plus checked executable instructions, not live packet captures. Code
in `src/world.rs` produces **application NetBlob bodies**, not complete datagrams.

## Server-to-client selection routing

`0071fc50` constructs the client's CEventQueue on receive queue **1**. Queue 4
is processed by `0071fb10` as a logon/chat queue, which ignores CharacterSet.
Client-to-server event queues must not be reused as server-to-client routing.
`00494890` filters events while NOT_IN_WORLD unless the blob has out-of-world
bit 61 set (`0063a580`). `00494a00`/`00494b10` dispatch unordered type 3 without
requiring a world player's CBookKeep; native `006da360` independently uses
high-word namespace `0x23000000` for pre-world blobs. Selection responses use
that namespace and queue 1. The UDP integration test checks both properties.

The native test on 2026-10-22 passed login/connection but faulted while moving
past the connection UI after this correction. The first log entry reports an
access to address `0x18`; its later `005e3e25` cleanup stack is not the original
fault location. Character-selection display remains **unverified**.
`00410f00` assumes several roots exist in layout enum `0x41000006`, resolved
through FileArray group `0x40000003`; `0040f590` and `00410c00` use those roots
without complete null checks. Installed build-7230 assets with the recovered
7148 executable are a possible compatibility issue, not an established cause.
`tools/capture_client_fault.py` can attach to the explicitly selected local
client after the user's normal launch and record its first access violation.

### First-fault diagnosis and local data-ready initialization

The next trace captured first-chance EIP `006dce3b` in
`VM::GetSingletonPackage`, with a null resolver. The stack includes
`006dd2e3` (CreateSingletonProcess), `0046bf2f` (InqRaceSexData), and
`0040f1c8` (avatar-selection construction). This identifies a missing VM
initialization step, rather than establishing an incompatible UI asset.

`CLCache::UseTime` (`004c0f10`) handles opcode `0x1003b` on queue 5 by
calling `OnEndDDD` (`004c0610`). Its PatchtimeEnd callback reaches
`gmClient::OnDDDEvent` (`00401dc0`), whose vtable entries +0x84 and +0xa4
at `00a4a178` point to InitWeenie (`00401da0`) and InitKeymap (`00403d30`).
InitWeenie reaches `CWAllocator::Init_Internal` (`0052d1e0`) and the VM
constructor (`006dd220`), which assigns the resolver.

The local emulator now sends this four-byte no-download completion on queue 5
before selection, and holds CharacterSet until the reliable notification is
acknowledged. `00722f20` processes the cache before the application event queue
on each client tick. This is an explicit local preinstalled-asset assumption,
not a full DAT patch/version-negotiation service or proof of asset compatibility.
The UDP test checks the completion opcode/queue and acknowledges it before
expecting the character list. Fresh native acceptance after the fix reached
the rendered avatar-selection scene and then the customization UI. The fresh
server session logged one login/connection with zero rejected packets, and
portal.log remained empty. The screenshot is retained locally at
`target/connect-test/native-avatar-selection.jpg`. This verifies this local
login/selection path; character saving, world entry, and full asset parity
are not established.

| Event | Direction | Body following little-endian u32 opcode | Evidence |
|---|---|---|---|
| `0x1002b` EnterWorld | client -> server | u64 character IID, packed narrow account string | `004c9e70`, group 4 |
| `0x1002a` CreateCharacter | client -> server | account, packed gmCharGenResult | `004c9d40`, `00600f90`, group 4 or 8 |
| `0x10027` LogOffCharacter | client -> server | u64 character IID | `004c9d10`, group 4 |
| `0x100d1` SceneRenderingComplete | client -> server | empty | `00493500`, group 3 |
| `0x10055` CharacterSet | server -> client | active identity array, deleted identity array, u32 status, i32 allowed slots, account, u32 chat flag | `00494010`, `006db7f0` |
| `0x100b8` MinimalCharacterSet | server -> client | i32 allowed slots, account, array of UTF-16 names, array of u64 IIDs | `00494010`, `00493c50` |
| `0x10060` CreatePlayer | server -> client | u64 character IID, u32 region | `00494010`, `00495060` |
| `0x10056` CharacterError | server -> client | u32 charError enum | `00494010`, `00493410` |
| `0x10057` CharGenVerification | server -> client | u32 response, CharacterIdentity, u32 trailing argument | `00493520` |
| `0x10058` CreateObject | server -> client | u64 IID, VisualDesc, PhysicsDesc, gmWeenieDesc | `00723280`, `007269d0` |
| `0x10068` PlayerDesc | server -> client | CBaseQualities | `00493460` |
| `0x10027` CharExitGame | server -> client | empty | `00494010`, `004933b0` |

Array count is u32: `00493de0` (names), `006db450` (identities).
CharacterIdentity (`006db0f0`) serializes IID, UTF-16 name, u32 seconds greyed
out, and VisualDesc. `character_set` emits a narrow but useful VisualDesc variant:
flags `2`, followed by a parent VisualDesc DID (`0063d7f0`). The caller must supply
a DID present in matching client data. No synthetic asset ID is claimed valid.
An empty full set uses status 0, five slots by default, and disabled chat, matching
the constructor `006db3e0`. The selection UI's `004108e0` resolves the total visual
description; if unsuccessful it attempts its configured `token_nologininfo` asset.

## String wire format

`004042f0` / `00406cb0` (narrow), `00409990` (UTF-16), and the StreamPack wrappers
`00493cb0` / `004098e0` establish:

- u16 code-unit count, or 0xffff followed by a u32 count for extended lengths.
- No serialized terminator; UTF-16 code units are little-endian.
- Nibble swap every payload byte, then XOR each complete 4-byte word with the
  repeating 16-word key table in `00401fc0` / `00409ab0`.
- Do not XOR the incomplete final word. The executable instructions at
  `00401fc8` through `00402078` confirm the end pointer and 4-byte loop condition,
  resolving misleading typed pointer arithmetic in the pseudocode.
- Align the pointer to four bytes. Encoder padding is deterministically zero;
  decoder accepts arbitrary padding because the recovered client skips it.
- Decode reverses XOR first, then nibble swap (`00406cb0`). This is obfuscation,
  not credential protection.

The implementation deliberately limits strings to 4096 bytes and lists/slots to
64; these are emulator resource limits, not recovered game limits. It preserves
narrow account bytes without assuming an unproved code page. Parsing bounds are
checked before copying, and trailing bytes are rejected.

## Where real world entry remains incomplete

`CreatePlayer` is a misleadingly broad name: `00492cb0` only publishes player IID
and region and calls `00494b00` to mark the event queue IN_WORLD. It does not
instantiate the player's physics object or populate qualities.

The actual object event is `0x10058` in **gmCEventQueue** (`00723280`), not the base
event dispatcher. `007269d0` decodes u64 IID, VisualDesc, PhysicsDesc, and
gmWeenieDesc. `004d99c0` then begins entity creation and handles object-parent and
instance-stamp dependencies. PhysicsDesc (`00641530`) includes a presence bitmap,
instance/visual ordering stamps, optional position and physics state, and four
mandatory timestamp values followed by alignment. gmWeenieDesc (`006037c0`) is
another mask-driven descriptor. Valid world placement and asset data are required;
zeroing these structures is not evidence of a playable player.

PlayerDesc `0x10068` (`00493460`) unpacks CBaseQualities and then dereferences the
existing player object. `0052e6a0` applies qualities and publishes LoginFinished.
CBaseQualities (`005192f0`, `00516f80`) carries a presence mask, the DBObj type
header, and optional qualities tables. Its DBObj type is `0x40000019` (`00516a00`);
DBObj header serialization (`006d77b0`) depends on the runtime category map.
Further recovery of `006d4640` lines 1674-1694 confirms DB_TYPE_QUALITIES is
uncategorized, so this base qualities header contains a DID and no category word.
`empty_player_description(qualities_did)` now serializes a zero presence mask and
that explicit DID. It carries no stats, abilities or inventory. Sending it before
object creation risks client failure; the codec's existence does not authorize
the server to bypass bootstrap readiness.

`create_stationary_object` now supports a recovered, explicit configuration:
VisualDesc parent DID; PhysicsDesc position-only mask `0x8000`, two u16 stamps,
Position (u32 cell, three f32 coordinates, quaternion w/x/y/z), four u16 timestamps;
gmWeenieDesc entity-DID-only mask `0x02000000` and u32 entity DID. Position is
`00620070`, Frame forwards to AFrame (`0061a290` -> `0062ac90`), quaternion order
is `0061c640`, and packed u16 width is `00453560`. It rejects zero identities/cell,
nonfinite numbers and nonnormalized orientation. These checks are structural:
the supplied cell/coordinates, DIDs and stamps still require actual world evidence.
No arbitrary spawn, asset or character stats are selected by the codec. The subset
is an unparented stationary entity with no movement/behavior overrides, not a
claim of complete player setup.

Therefore these codecs support real recovered selection and entry-request bytes,
but cannot establish a rendered/controllable world. SceneRenderingComplete must
not be fabricated as proof. A complete client distribution, matching asset DATs,
transport compatibility, object descriptors, valid spawn placement, and fresh
matching-client execution remain necessary.

## Validation

Eight module tests pass: independently calculated EnterWorld bytes, every truncation
of that request, oversize extended-string rejection, partial-word obfuscation,
full/minimal list layouts, supported controls, unknown opcode and trailing-byte
rejection, native creation fields and every truncation, appearance-table resource
limits and nonfinite values, stationary object offsets and placement-number checks.
Tested with `rustc --edition 2024 --test src/world.rs` independently of
the rest of the in-progress emulator. These are local codec tests, not client or
live-server acceptance.

## Native character creation and rejection

`00600f90` / `006010d0` serialize gmCharGenResult as UTF-16 name, u32 entity DID,
start area, race, sex, class, implement, then PHashTable<u32,float> appearance
values. `00409e00` establishes a u32 table header: lower 24 bits entry count,
upper 8 bits bucket-size index, followed by u32 key/f32 value pairs. The decoder
caps entries at 256, rejects duplicate keys and nonfinite floats, and retains
exact float bits for future authoritative validation. No client-supplied entity,
race, area or appearance value becomes authoritative merely by parsing.

`CharGenResponse` in recovered-types.h lines 12245-12255 defines 0 undefined,
1 OK, 2 pending, 3 name in use, 4 name banned, 5 corrupt, 6 database down,
7 admin privilege denied, 8 no more slots. The rejection encoder excludes OK
and pending, carries an unallocated zero IID, empty name/VisualDesc, and a zero
trailing argument. The creation UI handles the response separately from the
identity (`0040fca0`); no success or character allocation is implied by rejection.
Use Undefined for an unsupported creation policy instead of falsely claiming a
database outage, banned name or corrupt request. Successfully accepting native
creation still requires authoritative entity/start-area/appearance validation and
persisted character setup.

## Current native world-entry failure (2026-10-22)

The optional local human profile now persists creation requests and sends
CreatePlayer, CreateObject and ordered PlayerDesc. Native creation acceptance,
appearance conversion, movement and gameplay are not verified.

CreatePlayer's region argument is the enum value **1**, not asset DID 0F000000.
WorldManager::SetRegion (0057c570) calls CRegionDesc::SetRegion (0051e7a0),
which resolves it through group 40000007 before SetRegionByGID. Installed
EnumIDMap 28000008 maps 1 to 0F000000. Correcting this removed the incorrect
region argument but did not resolve the script-state crash.

The latest two captured first-chance faults are both EIP 006DD7AE in
ProcessState::GetPackageID. The read targets DE5EE6C2 and 2934760E differ;
the call chain does not: WState::GetSubDataIDs (006b7c60), asset preload,
CLCache processing, then the client tick. The focused second capture confirms
ProcessState.m_instance_state is FFFFFFFF. AllocatePackage (006ddd50) returns
that sentinel after UnPackReference fails; GetSubDataIDs does not check it.
Read-only process inspection identified the WState DID as 7100110F and the
resolver library as 56000005. Offline inspection shows the root package 037F
has 60 fields in both the state and library, so a root size mismatch alone is
not the explanation. The exact rejected child record remains under investigation.

Evidence is retained under target/world-test/first-fault-script-state.jsonl.
The debugger helper optionally captures one HeapObject::UnPack failure using
a temporary, restored breakpoint; it never saves changes to the client file.
The installed 7230 executable remains untested because Windows canceled its
launch; Authenticode reported a revoked signing certificate. No OS security
setting, signature or game DAT was altered to work around that block.

### Captured loader failure: GameplayOptionsProfile

The one-shot failure breakpoint at 006DBD26 caught native type 009B, which
gmCNativeFactory::NewNativeObject (00466c10) identifies as GameplayOptionsProfile.
HeapObject.m_size was 02000003 (33,554,435), with only 192 bytes left in the
package payload. The source bytes match WState 7100110F offset 062C. This is
before the invalid-reference dereference at 006DD7AE, whose read address was
08D87D19 in this run. Evidence: target/world-test/unpack-failure-wow64.jsonl.

The options object starts at record offset 0304; its payload mask at 0310 is
00000000000027EA. The recovered 7148 GameplayOptionsProfile::StreamPack
(006000b0) handles flags through 1000 but has no 2000 field handler. It leaves
newer options bytes unconsumed, which HeapObject interprets as its own size.
This establishes a concrete client/data layout incompatibility in the current
player bootstrap, rather than a guessed malformed UDP field.

The server now remembers the login build and rejects world entry into this
installed-DAT profile for 7148 before sending any world-bootstrap events.
Selection remains available. Only experimental 7230 entry is eligible for this
profile; that eligibility is not proof of matching-client acceptance or gameplay.

Validation: `installed_dat_profile_rejects_7148_before_world_bootstrap` passed
against the actual installed DAT directory. It creates a persisted native human,
performs a real UDP 7148 login, verifies the populated character list, requests
entry, and asserts a placement-error response instead of a bootstrap event.
Run this opt-in test with AC2_TEST_GAME_DATA set and `cargo test --test udp_flow
installed_dat_profile -- --ignored`. This remains synthetic client verification;
the guarded binary was started on 19170/19171 but no fresh native rejection
screen was claimed. Standard tests and clippy also passed.

### Isolated 7148 conversion experiment

`tools/make_7148_test_assets.py` creates a new test directory, copies portal.dat,
and converts only SHA256-pinned records 7100110F and 7000037F. It clears options
mask bit 2000 and removes its three-entry u32/u32 table at original offset 062C
(28 bytes). The remaining script fields are preserved, PackageState size is
reduced, and the DAT directory length is updated without reallocating chains.
The archive is reopened and checked, and source records are rechecked unchanged.
Other large DATs are hardlinked for read-only use; **--rodat is mandatory**.
This directory must never be used by an updater or writable game session.

The output records are 1752 bytes. SHA256 values are recorded in
target/compat-7148/pyrealium-compatibility.json. Bootstrap eligibility now checks
both player/package record fingerprints, recognizing original 7230 and converted
7148 profiles separately and rejecting unknown/mixed layouts. These fingerprints
are compatibility identifiers, not a security authenticity guarantee.

The local conversion test verifies preserved fields and rejects changed sources
or repeated conversion. Both installed-asset rejection and converted-asset
bootstrap UDP tests passed. The converted test server uses 19270/19271 with a
separate character store. Native execution remains the acceptance requirement;
these tests do not establish that the world renders or is playable.

### Native acceptance of the converted profile

The recovered 7148 client was launched with working directory target/compat-7148,
--rodat, and server 127.0.0.1:19270. Character selection rendered the saved CLI-seeded
human Pyrealium. After Enter World, the previous script-state failure did not
recur. The game displayed a MusicLoader E_FAIL dialog; Continue dismissed it.
The native window then rendered the human at the authored spawn among grass,
trees and stone arches, with the in-game HUD. The server logged both
`world bootstrap sent for character 1` and `native client reported SceneRenderingComplete`.
Screenshot: target/compat-test/native-world-entry.jpg. This is actual client
world-entry evidence, not a synthetic packet result. Native UI character saving,
appearance overrides, gameplay vitals, multiplayer and movement remain unverified.
The music-loading error remains an explicit compatibility issue.

Use start-world-test.ps1 for repeat runs. Its syntax was checked; the equivalent
build/start/launch actions were executed directly during this accepted test.

## Extracted terrain record format (2026-10-23)

The current `ac2client.exe` SHA-256 was checked again as
`9126b463f8dbe5814c011ffb64bd430ade8219d8963cfaca74db483675579796`.
The following references are recovered build-7148 pseudocode in
`recovery/exports/ac2client.exe/functions/`, not original C++ source:

- `0048f310.c`, `CLandBlockData::StreamPack`: calls the DBObj serializer first,
  followed by `m_lbi_id`, `m_pathmap_id`, `m_height`, `m_cellinfo`, and alignment.
- `006d7830.c`, `DBObj::UnPack_Type`: reads a 32-bit DID, with an additional
  category only for categorized record types. The supported extracted terrain
  profile has the single-word identity header.
- `0048faa0.c`, `PSmartArray<unsigned_char>::StreamPack`: serializes a 32-bit
  element count followed by that many bytes.
- `006388c0.c`, `PSmartArray<unsigned_long>::StreamPack`: serializes a 32-bit
  element count followed by that many 32-bit words.
- `0047f1c0.c`, `STREAMPACK_ALIGN`: aligns the stream pointer at the end.
- `0048ef60.c`, `CLandBlockData::GetSubDataIDs`: registers the info and pathmap
  IDs as dependent data, with type IDs `0x40000051` and `0x40000035` respectively.

For the supported extracted profile, the byte offsets are: DID at 0, info ID at 4,
pathmap ID at 8, height count at 12, 289 byte heights at 16, terrain count at 305,
and 289 four-byte terrain words at 309. Payload length is 1,465 bytes before final
alignment, or 1,468 bytes with padding. The parser also permits the unpadded
payload as an extraction format. It requires zero padding; the recovered alignment
routine alone does not prove that the client checks padding contents.

The 17-by-17 grid, numeric file paths, `FFFF` terrain suffix, same-block `FFFE`
info link (or zero for no link), and restricted lengths are corpus-profile checks.
They should not be confused with universal restrictions in the native serializer.
The field at offset 8 is now named `pathmap_id` in Rust based on the recovered
member name. Pathmap contents and links are not validated by the terrain command.

The release validator checked 65,025 terrain records and found zero missing or
empty nonzero info links. This is static format/corpus evidence. No new terrain
packet capture or native-client terrain-download test was performed. The packet
and native-session evidence elsewhere in this document retains its own scope.

## Movement and saved placement (2026-10-23)

Recovered build-7148 functions `00496a90` (SendEvt_CPosition), `0069c060`
(CPositionPack::StreamPack), and `00625930` (PositionOffset::StreamPack) establish
opcode `0x1008e` on queue 3 and a 40-byte body, or 52 bytes with jump velocity.
The body is double time, cell ID and three floats, a packed motion/heading word,
flags, then four ushort stamps: instance, teleport, forced-position and move-to.
Flag 2 includes three jump-velocity floats; flag 4 cancels move-to. They are not
one movement counter. `004969f0` establishes the two-float `0x100c8` look request.
The server-position serializer `0069be20` has three stamps in position/force/
teleport order, and its bit 4 means contact. `004946d0` supplies the prefixed IID
and two ushort stamps. `0061a940` quantizes heading into 255 units per revolution.

The local runtime accepts finite, increasing-time movement for its active instance,
checks changed cell IDs against its configured cell DAT, and stores latest placement
in memory. Finite coordinate/velocity bounds, expected zero force/teleport stamps,
and one active session per character are explicit development policies. This is
client-reported movement, not a server collision simulation or speed validation.
Look messages are decoded and phase-checked; remote head-look replication is pending.

Placement snapshots are bounded, checksummed, written to a temporary file and flushed
before replacement. A five-second checkpoint, logout, and clean shutdown save dirty
placement; an abrupt crash can lose movement since the last checkpoint. Character
creation retains its existing independently flushed journal. Re-entry restores saved
placement and increments the instance stamp. In-world event replies now share the
native ordered world counter, separately from pre-world messages.

The real-UDP converted-DAT test exercises movement, replay rejection, logout, saved
placement on re-entry, the new instance stamp, and persistence after reopening the
store. The standard tests cover malformed packets, snapshot corruption/truncation,
replacement of an existing snapshot, and invalid logout transitions. These are
synthetic-peer tests; native-client movement acceptance is tracked separately.

## Player presence and replication (2026-10-23)

The runtime maintains one scene owner, an IID-to-peer map and a cell-to-IID index.
Players sharing a cell exchange current CreateObject descriptions when entering.
Accepted movement sends `0x10086` PositionPack to observers; `0x100c8` client look
produces observer `0x100c9`. Departures send `0x100c2` with InstanceIDWithStamp.
The opcodes are mapped by recovered `CEventQueue::DispatchEvent` (`00494010`), with
receivers `00495da0`, `00495b80` and `004951f0`. Same-cell visibility is an initial
policy, not recovered distance, occlusion, indoor or adjacent-cell visibility.

Movement inside a cell does not rebuild the entire interest set or serialize a
spawn description. When there are no observers, no movement packet is allocated.
Observers share an immutable application payload; each peer still gets its own
checksummed, reliable transport bytes. Cell changes exchange creates/destroys as
visibility changes. Logout, expiry and transport backpressure remove presence.
Failed application messages preserve earlier replies and transport ACKs; failed
login replacement also releases an expired character's prior presence.

The opt-in `tests/multiplayer.rs` converted-DAT test runs two independent UDP peers
through login, spawn exchange, rejected character takeover, movement, head look,
logout destruction, saved re-entry and snapshot reopening. Scene tests cover cell
transitions, 64-player capacity, duplicate identities and shared delivery payloads.
This establishes server behavior under synthetic peers; rendering by two actual
clients remains unverified. The native test launch is waiting on a Windows security
prompt that must be handled by the user.

## Character deletion and recovery (2026-10-23)

`Proto_UI::SendDeleteCharacter` (`004c9e00`) emits `0x10029`, the packed account
string, then the character IID on queue 4. `ECM_Login::RecvEvt_CharacterDeletion`
(`004933c0`) receives the same opcode followed by the IID alone. The runtime
requires selection phase, the configured account, an owned nondeleted character,
and no active session for that character. It flushes the journal before sending
the deletion acknowledgment. Deleted identities cannot enter the world or receive
new placement, and do not consume active character slots.

Deletion is implemented as a checksummed `d1` tombstone, not removal of journal,
appearance or saved placement. IDs and names remain reserved. An operator can
restore with the offline `character-restore --account ACCOUNT --character-id ID`
command, which writes an `r1` journal event after checking ownership and slots.
`character-list` shows active and deleted characters. These retention/recovery
rules are explicit local policies; native admin restoration and timed deletion
have not been implemented. The old v1/v2 creation records remain readable.

Tests exercise unauthorized/duplicate deletion, slot release, ID nonreuse, restore
slot limits, preserved appearance/placement, and replay after reopening the store.
The two-peer UDP test also rejects deletion while in-world and with a mismatched
account, acknowledges valid deletion, rejects world entry for the tombstone, and
restores it offline. Native deletion UI acceptance remains unverified.

## Native movement probe and heading correction (2026-10-23)

The actual 7148 client connected to the new movement runtime on 19370/19371,
rendered Explorer, and sent SceneRenderingComplete and accepted CPosition updates.
The five-second snapshot recorded cell `0x02990109`, X `59.060577392578125`,
Y `240.19900512695312`, Z `-44.887088775634766`, heading 191. X/Y did not change
during automated W/Up taps, so walking is not established. The known MusicLoader
E_FAIL still needed Continue. Capture: `target/play-native/native-world-before.png`.
The read-only `tools/probe_native_movement.py --pid PID` verifies executable identity
and inspects PDB-bound movement fields. It observed player IID 1 and locked_actions
0. The probe neither attaches a debugger nor modifies process memory.

This exposed an orientation conversion error: `Frame::set_heading` (`0061a810`)
uses a clockwise heading from +Y, whereas a positive quaternion Z rotation runs
the other way. The saved native heading must map to a negative quaternion angle.
`tools/movement_binary_vectors.py` executes original `Frame::cache_matrix`
instructions (`00619f70`) under offline x86 emulation at eight headings. The native
forward axes match the corrected conversion; their values are checked in Rust.
The authored human start uses heading 191, corresponding to the original positive
quarter-turn quaternion up to native byte quantization. Actual native logout/re-entry
and walking remain acceptance work; synthetic re-entry and snapshot tests pass.

## Local player qualities and diagnostics (2026-10-23)

The authored `0x8100110f` qualities record has five integer properties but no
initial health, vigor or level. PlayerDesc now preserves those authored entries
and supplies a development level-one profile with raw, current and cached maximum
health/vigor of 100. IDs `0x100`, `0x101`, `0x102`, `0x103`, `0x107`, `0x108` and
`0x12e` are mapped by the matching recovered `gmintstats` enum (`2300002D`). The
values are local test defaults, not recovered progression formulas. The boolean,
float and DID tables retain their original layout and selected human body DID.

The converted-DAT UDP test walks the variable-length name and integer table,
checks every authored and added integer, then checks the remaining tables against
the DAT record. Both DAT-profile tests and the two-peer test pass. A new native
session is awaiting the user's Windows launch warning; HUD rendering of these
properties remains unverified. Missing UI script initialization is still a
separate open issue. Unsupported application events now log bounded opcode,
queue, length and decoding reason once per opcode, for at most 32 opcodes;
packet contents and account data are not logged by this diagnostic.

## Same-cell replication benchmark (2026-10-23)

The common movement and head-look paths now iterate the existing cell index
directly instead of allocating and populating a temporary BTreeSet. Head look
with no observers also skips packet construction. Cell transitions retain set
comparison for their create/destroy behavior. A regression test moves an isolated
player, then verifies that a later arrival receives the updated position and
orientation. The complete unit suite and two-peer UDP integration pass.

`cargo bench --bench scene --target-dir target/scene-build` runs five warmed
samples of 10,000 operations, retaining returned deliveries through black_box.
Before/after median nanoseconds, on the same development host:

- One player: movement 38 / 25; look 83 / 12.
- Sixteen players: movement 701 / 470; look 587 / 413.
- Sixty-four players: movement 1,832 / 1,358; look 1,701 / 1,298.

This is same-cell application replication only. It excludes encryption, UDP,
storage, collision and rendering, and does not establish player latency or
production capacity. Raw samples are local ignored artifacts at
`target/performance/scene-before.txt` and `scene-after.txt`.

## Clean connection logoff (2026-10-23)

`ClientNet::LogOffServer` (`004c7590`) sends `CEmptyHeader<32768,3>` to each
receiver. `SharedNet::SendOptionalHeader` (`004c66e0`) emits sequence zero,
current recipient/iteration and an empty, unmasked body. The matching header
factory (`0063bf90`) confirms mask `0x8000` and zero payload bytes. This is
connection logoff, separate from `0x10027` returning to character selection.

The server validates the entire packet, checksum and session identity before
flushing dirty placement and scheduling immediate peer removal. Subsequent
packets from that peer in the same drain are ignored. Scene departure broadcasts
and active-character release use the existing removal path. Only the recovered
standalone form is supported; combined flags and sequenced/encrypted forms are
rejected. No new acknowledgment is invented for this unsequenced control.

Regression tests reject all truncations, stale recipient/iteration, corrupt
checksums, payload bytes and unsupported combinations without consuming ISAAC
state. The two-peer UDP test rejects a stale logoff, verifies continued head-look
delivery, then accepts the valid logoff, observes a destroy packet and enters the
released character through the other connection without an expiry delay.
Native-client connection-logoff acceptance remains unverified.

## PlayerDesc hash-order crash repair (2026-10-23)

A fresh native run faulted at `0065bb1f` during `PlayerDesc` cleanup. The captured
stack passed through `HashTable<unsigned_long,long>` destruction (`00517790`),
`CBaseQualities::Clear` and `ECM_Login::RecvEvt_PlayerDesc` (`00493460`).
`PHashTable::StreamPack` (`00517fd0`) uses a 24-bit element count and high-byte
bucket-size index. Index zero selects 11 buckets from the binary table at
`00a7b140` (pointer `00ad703c`). Its `add_for_unpacking` (`004fc9f0`) requires each
bucket's entries to be contiguous: returning to a previous bucket overwrites
the bucket head without reducing the stored element count. The earlier numeric
ordering of the added vital properties violated this requirement.

The writer now orders entries by `(key % 11, key)`. The UDP test checks the actual
wire bucket order in addition to counts, values and trailing tables. The offline
`tools/qualities_binary_vectors.py` runs the original insertion instructions:
numeric ordering stores count 12 but leaves only 8 entries reachable; corrected
ordering retains all 12. The corrected release builds and the converted-DAT UDP
test passes. Native-client re-entry after this repair remains unverified.

## Script action integration and scene readiness (2026-10-23)

The runtime now accepts the bounded `0x10053` envelope: function ID, PackBuffer
byte length and exact argument bytes. This follows native sender `00498f30` and
PackBuffer serialization `006ed9c0`. Primitive type tags are validated against
`006edde0`; InstanceID uses tag 2, integer uses tag 1. The supported text dialect
is a single standalone native StringInfo reference, without arbitrary reference
graphs, interpreted fields, tokens or substitutions. Its framing follows
ReferenceTable (`00457330`, `00457120`), HeapObject (`006dbbd0`) and StringInfo
(`00652db0`). Unsupported formats are rejected rather than passed to a VM.

The explicit local action profile handles combat-mode start/stop, target changes,
local chat and online tells. Private replies use `0x10066` and the recovered
PackBuffer receiver (`00498d60`). Combat-mode requests emit the supported
`0x8204001b` integer response; this is a mode/UI transition, not attack simulation.
Targets must exist in the current ready scene. Chat names come from the owned
character record; local messages go to ready players in the same cell, and tells
go only to the named active character with a sender confirmation. Text is bounded
to 512 characters, rejects control characters, and has a 500 ms per-peer rate
limit. These are development policies. Native UI/function acceptance remains
unverified; no complete skill, damage, inventory or script VM implementation is
claimed by the router.

World entry now registers a hidden scene body. Only SceneRenderingComplete
publishes creates to the other ready players, and repeated activation emits no
duplicates. Position reports received during loading update the hidden spawn
without exposing it; aborting a load emits no destroy for an unseen player.
Chat, target and combat requests require the ready InWorld phase. This preserves
the existing separate-checkout readiness behavior with the newer cell index.

Tests cover exact envelopes, every truncation, oversized lengths, wrong tags,
invalid text, hidden-body updates, idempotent activation and unready recipients.
The real-UDP two-player test now crosses script and movement queues, rejects
gameplay during scene loading, rejects an invisible target and malformed combat
arguments, delivers mode replies, broadcasts chat and routes a private reply,
then continues the existing movement/logout/deletion/logoff flow.

The integrated scene stores loading bodies outside the ready-cell index. This
keeps the readiness gate without adding per-observer body lookups to movement,
look or chat iteration. Tests move a hidden body between cells and enter a ready
observer in its temporary cell, verifying no premature create; later activation
publishes its latest placement in the correct cell. A repeat of the scene
benchmark on the integrated code measured median movement/look costs of 25/12 ns
with one player, 466/451 ns with sixteen, and 1,373/1,278 ns with sixty-four.
The samples are at `target/performance/scene-integrated.txt`; these remain isolated
application costs, not native player latency measurements.

## Durable avatar lifetimes (2026-10-23)

Instance stamps now belong to the character rather than its UDP peer. The store
flushes an `e1` journal event before world-entry packets can be published. A
different connection, server restart, or deletion/restoration therefore cannot
reset the character's next lifetime. Existing creation/lifecycle records start
at stamp zero when no lifetime has yet been recorded. Older binaries that do not
understand `e1` cannot read a journal after this upgrade writes such an event.

This preserves the native 16-bit InstanceIDWithStamp wire layout. Refusing entry
after exhausting the nonzero 16-bit range is an explicit local policy; wrapping
and reusing an old lifetime is not silently assumed safe. Failed delivery may
consume a lifetime, which is harmless: stamps must be unique, not gap-free on
the wire. The journal itself requires exact increasing events and rejects
unknown/deleted characters, duplicate/backward/skipped stamps and overflow.

Store tests cover ownership, deletion/restoration, restart, checksum-valid invalid
events and exhaustion without another write. The two-peer UDP flow now verifies
that a character released by one connection receives stamp two when entered by
the other, then verifies the next stamp after reopening the store.

## Reproducible UI initializer resolution (2026-10-23)

`tools/wsl_function.py` reads WLib record `0x56000005` from the supplied portal
archive and resolves numeric functions using `VTableSection::StreamPack`
(`006e89d0`) and `Resolver::GetFunctionOffset` (`00455460`). It honors absolute
function IDs and concrete object-package mappings for virtual calls. Optional
linear instruction boundaries follow `OpcodeStream::WalkStream` (`006ec490`),
including switch tables; immediate words are not mistaken for instructions.
All reads and counts are checked against section bounds. No archive is changed
and no script is executed.

On the current converted/repaired native-test archive, record SHA-256
`b402b04a6ef587e83145524ad8612c66e6a16d3a710fc7addeb78e8df22ba21d`,
UI_Load `0x82120027` resolves to opcode offset `0x101654`. Its forwarding stub
loads fourteen argument slots followed by the receiver, calls `0x1aa40065`, and
cleans fifteen stack slots. Resolving that method for object package `0x1aa4`
produces offset `0x363ac`. These are static identities and stack observations;
they do not establish the fourteen argument schemas or their safe defaults.
The runtime does not yet send UI_Load. A bounded linear window is not a complete
function extent or a control-flow graph.

Example (supply the actual archive path):

```text
python tools/wsl_function.py --portal PATH/portal.dat --function 0x82120027 --instructions 30
python tools/wsl_function.py --portal PATH/portal.dat --function 0x1aa40065 --object-package 0x1aa4
```

Parser tests cover absolute/virtual dispatch, missing implementations, every
truncation of the resolver tables, invalid offsets and overflowing instruction
tables. The supplied archive was also resolved directly; native UI acceptance
remains a separate check.

## Reply counters under backpressure (2026-10-23)

Per-peer blob IDs and ordered world stamps now commit after the transport accepts
the complete message. Previously a rejected send (for example a full reliable
queue) consumed an ordered stamp even though no packet carried it, leaving later
ordered messages behind an impossible gap. Pre-world sends also preserve their
counter on rejection. A saturation test fills all 256 pending packet slots,
rejects both send paths, acknowledges the queue, and checks the next serialized
fragment uses world stamp one. Empty payload rejection and stamp exhaustion
also leave the counters unchanged. This does not implement application retries
or a complete gameplay transaction system.

## Native movement observation (2026-10-24)

The original build-7148 client entered the world against release `087904c` on
the local native-test realm (UDP 19370/19371). The matching executable hash was
verified by the read-only movement probe. Explorer rendered at the start and
later at another position with the inventory panel open. The probe reported
player ID 1, no locked actions, and actively_moving=1. The server's persisted
placement subsequently contained cell `0x02990108`, coordinates approximately
`[-8.711257, 234.96587, -43.98427]`, heading 171, compared with the configured
start cell `0x02990109` at `[59.060577, 240.199, -44.606205]`. This establishes
native movement and saved placement across a cell boundary in this session.

The user was providing input during the observation; the automation's attempted
W tap was refused because user input had occurred. Do not attribute the walking
to an automated key hold. A screenshot is retained outside version control at
the recovery checkout's `emulator/target/play-native/native-world-lifetimes.png`.
The native client later exited; no cause was captured, so this is not an
assertion of crash-free extended play. HUD labels/vitals remained incomplete,
and opening the skills panel showed an empty list. Initial loading emitted
rejected pre-ready script calls; unsupported `0x100c7` and `0x82030005` requests
were also logged. These remain integration work, not completed gameplay.

## Target-look replication (2026-10-24)

The previously unsupported `0x100c7` now decodes exactly one 64-bit target ID,
matching `ECM_Physics::SendEvt_CLookAt` (`00496980`). A ready player can look at
a ready same-cell player, or clear using zero/all-ones sentinel IDs. Unknown,
loading and cross-cell targets are rejected under the current visibility policy.
Observers receive `0x10096`, the source InstanceIDWithStamp and target InstanceID,
matching dispatch `00494010` and receiver `00495ac0`. This visual head-look path
does not select a combat target or authorize an attack. It has parser, scene and
two-peer real-UDP coverage; native two-client animation acceptance remains open.

## Opt-in empty-player HUD profile (2026-10-24)

`--initialize-ui true` enables a newly encoded fourteen-argument UI_Load profile
after the first SceneRenderingComplete of each avatar lifetime. Repeated scene
completion does not duplicate it. The switch is off by default and requires the
converted build-7148 bootstrap profile; installed 7230 data is rejected.

The retained classic PCAP `Packets/login.logout/login.from.char.overview.screen.elog`
(SHA-256 `7f0d175d9ed5f972da5573a9203dc445495fcbff41afac9b610397e86a05ec70`)
contains a reassembled 11,164-byte UI_Load with a 11,152-byte argument buffer.
Its object layouts were compared with the matching client decoder, rather than
replaying the captured character's items, trained skills, gold or progression.
The capture's original client build is unconfirmed. Matching WLib package info
independently reports interpreted sizes 2, 8, 14 and 2 for packages `0x1a4`,
`0x233`, `0x14f` and `0x24a7`, respectively. The new writer uses those exact sizes.
Native kinds are checked against factories `004b3980`/`00466c10`; HeapObject
unpacking (`006dbbd0`) defines the field-word/tag/alignment layout. The options
profile uses a zero u64 mask, following `GameplayOptionsProfile::StreamPack`
(`006000b0`) and Clear (`005ffc60`), which clears m_bitField before decoding.

The profile contains empty story/quest/skill/effect/equipment/item collections,
zero gold/XP/credits and four empty development bag descriptors (42/12/12/12
capacity, a local policy). It is presentation initialization, not an inventory,
quest, combat or progression implementation. Fields not semantically recovered
remain experimental, including trailing status flags. No captured packet or
character-specific state is distributed as a runtime fixture.

Tests independently walk all fourteen arguments, verify closed reference graphs,
check alignment and field tags, and exercise ordered delivery over real UDP after
scene completion plus duplicate completion. This does not establish native HUD
acceptance; keep the switch opt-in until a matching-client run validates it.

Enabling the switch now also performs a bounded read of WLib's VTable package
metadata before binding the server ports. Changed/missing interpreted sizes,
truncated sections, duplicate VTable sections and oversized counts fail startup.
This gate checks the four serialized class layouts, not script behavior. The
constant HUD packet is built once at startup and reused for each new avatar
lifetime. Tests cover the gate's valid/malformed layouts, and the supplied
converted archive passes the enabled real-UDP bootstrap test.

## Native combat-start target argument (2026-10-24)

The HUD-enabled native session completed scene loading and produced a 12-byte
`0x82040008` argument buffer that the old development-only empty/int parser
rejected. The matching WLib outbound stub loads a long argument and calls
PackLInt (`0x00500003`) before sending this event; native PackLInt (`0053f830`)
writes tag 2 followed by a 64-bit value. Combat start now accepts this exact
target encoding, normalizes zero/all-ones as no target, and checks that other
targets are visible ready objects. Self-targeting is rejected as local policy.
The empty/int development forms remain for compatibility. Combat stop remains
the no-argument/reserved-int form; the matching outbound stub does not pack a
long target. This repairs the request dialect, not combat damage simulation.

Tests check exact types, sentinel IDs, every native-buffer truncation and trailing
bytes. The two-peer real-UDP test rejects unknown/self targets before accepting
the other ready player, then continues stop/chat/movement/logout. The native
session later exited with no access violation recorded; HUD presentation and
extended stability are still unverified without a fresh visual observation.

## Durable UI placement and opacity (2026-10-24)

`0x82120031` is now decoded as two bounded, standalone native graphs: a
BaseHash<unsigned long,unsigned long,2> opacity table (kind 1) followed by
UISaveLocations (kind `0x6f`). The latter is a hash of layout IDs to window hashes;
each UILocationData contains four float coordinates and a 32-bit shown value.
Factory `004b3980`, hash packing `004093a0`, and the retained window-update
packet layout establish this structure. The decoder rejects duplicate keys,
nonfinite coordinates, out-of-range opacity, trailing bytes, arbitrary graphs,
oversized counts and interpreted fields on these native objects. Local bounds
are 64 opacity entries, eight layouts, 64 windows per layout, 256 windows total,
and coordinate magnitude at most 1024.

Ready characters' updates merge with existing settings rather than erasing
windows omitted by a partial upload. Identical state causes no journal write.
Changed settings append a checksummed, flushed `u1` record before in-memory
publication. Replay decodes and validates the canonical payload again, rejects
unknown/deleted characters, and preserves preferences through deletion/restore.
Older server binaries cannot read journals after their first `u1` record.

When the opt-in HUD initializer is enabled, the next avatar lifetime restores
saved locations and opacities through GameplayOptionsProfile's `0x890` mask,
bit-field marker and version 4, in the order specified by `006000b0`. Every hash
is re-encoded in bucket-contiguous order with the native 11-bucket size index.
This reuses semantic values, not client-provided pointers or arbitrary serialized
script state. Aliases and channel fonts remain unfinished; hotbars are covered below.

Per the user's instruction, no automated tests were added or run for this
change. Compilation succeeded; native persistence/restoration remains unverified.

## Durable hotbar edits (2026-10-24)

`0x82120018` now decodes one standalone native ShortcutInfo graph (native kind
`0xac`, factory `00466c10`), followed by an integer slot. ShortcutInfo packing
`00608150` specifies empty, skill, item instance, text, enum and craft-DID forms;
PSmartArray packing `00436a00` supplies the restored array. The decoder bounds
slots to 0..99 and text to 256 non-control characters, rejects unknown kinds,
additional graph objects/fields and trailing data, and preserves UTF-16 text
through the existing recovered string codec. Local aggregate hotbar data is
limited to 16 KiB.

Only the ready character owned by the sending session can edit its hotbar.
Changes append flushed, checksummed `h1` journal events; identical edits avoid
another write. Replay validates each edit and cumulative size before publishing
it. Empty shortcuts remove the slot, and other slots are preserved. Tombstones
and restoration retain the bindings. Journals containing `h1` require this newer
reader. The opt-in HUD initializer combines shortcut-array mask bit 2 with saved
window settings, following native GameplayOptionsProfile's field order.

Bindings are UI preferences: referencing a skill/item does not train, grant,
equip or authorize its use. Those gameplay handlers are separate unfinished
work. No automated tests were added or run, per the user's request. Compilation
succeeded; native hotbar editing and restoration have not been verified.

## Transactional inventory storage (2026-10-24)

The Rust inventory now owns item identity, definition/visual IDs, count, stack
limit, permitted equipment slots and exactly one location per item. Local bag
capacity remains 78 (42/12/12/12). Equipment uses the eleven single-bit slot
values documented by the supplied implementation against DAT table `0x23000074`.
Stack and equipment eligibility are trusted operator inputs for now, not values
accepted from native clients or recovered balance rules.

Moves require the expected source slot. Occupied destinations swap items or
merge compatible stacks only when the entire count fits. Splits require a fresh
global identity, a nonempty proper subset of the source and a free bag slot.
Equipping requires one item, a permitted single equipment slot and a bag source;
replaced equipment goes into that vacated bag slot atomically. Unequipping
requires free bag space. Consumption cannot silently remove equipped objects.
Candidate transitions are validated before the journal is changed.

Store checks active ownership and allocates monotonically increasing IDs in the
`0x8130` namespace across every character, including deleted characters. Each
inventory remembers its highest-ever allocated identity even after stacks merge
or items are consumed. Checksummed `i1` events are flushed before state changes
are published. Replay applies the same transition rules and enforces global
identity progression, rejecting unknown/deleted owners and invalid operations.
The parser bounds each local inventory event to 256 bytes. Journals containing
`i1` require the newer reader.

Offline CLI commands expose list, development grant, move, equip, unequip, split
and consume. The normal exclusive store lock requires the runtime to be stopped.
Grant checks that definition and visual records exist in the supplied portal
archive; it does not infer an original starter grant, stack rule or equipment
rule from existence alone. No items were granted to the native test character.

Native inventory request/reply graphs, contained item CreateObject descriptions,
HUD item maps and appearance replication are not connected to this stored state
yet. Two-handed occupancy, cross-character trade, loot and economy rules also
remain separate work. No automated tests were added or run at the user's request;
the code was formatted and compiled only.

## Owned-item native initialization (2026-10-24)

The opt-in HUD path now emits contained CreateObject records for every owned item
before its populated inventory UI_Load. VisualDesc uses its parent-DID field
(`0063d7f0`); PhysicsDesc has no world position/parent and includes mandatory
instance/visual stamps and four timestamps (`00641530`). gmWeenieDesc carries
the stored definition, container or wielder owner, and quantity (`006037c0`).
Items use the owning avatar's durable lifetime stamp and remain private to that
connection; they are not inserted into the world-cell scene index.

The HUD encoder emits equipment ARHash and item LRHash graphs with ItemData
(`0x1f9`, six fields), appearance (`0x1a2`, three fields), unit scale, empty
appearance overrides and parent VisualDesc references. The matching factory
maps native kinds `0x0f`, `0x70` and `0x71` to AppearanceInfo hash, Vector and
VisualDesc. LRHash packing `00456c20` and insertion `00456eb0` use full-u64 modulo
bucket selection. Both map types are sorted by their actual native hash bucket.
Bag descriptors contain current per-bag counts; the bag IID list preserves flat
slot positions with zero empty entries. Sparse-slot presentation is unverified.

Startup checks the two additional interpreted class sizes when stored items are
present, and verifies all active owners' item definition/visual records exist.
The owned item CreateObjects and HUD packet queue as one transaction: a failed
send restores transport, blob and ordering counters before any packet reaches
the socket. This avoids publishing only part of an inventory initialization.

Native equip/move request/reply handling, server-driven worn appearance updates,
and complete item metadata/tooltip presentation remain unfinished. No items were
automatically granted. The code was compiled only; no automated tests or native
client checks were run, following the user's instruction.

## Inventory location indexes (2026-10-24)

Inventory candidates now carry fixed, bounded bag and equipment indexes derived
from their owned item map. Occupancy checks read one index entry, and finding a
free bag slot scans at most 78 entries instead of repeatedly searching every
item. Every changed candidate rebuilds its indexes and rejects duplicate or
out-of-range locations before the store can journal or publish it. Failed
commands leave the original map and indexes intact. The derived indexes do not
change the journal format, slot policies, or ownership rules.

Formatted and compiled with `cargo check --all-targets`; no tests, benchmarks,
or native client checks were run. This is an algorithmic improvement, not a
measured performance claim.

## Native sorting request import (2026-10-24)

The retained `sorting.items.elog` capture contains `0x10053 / 0x820e0049`
requests with one PackBuffer object: package `0x1f8`, 51 interpreted fields,
51 type bytes and one padding byte. The decoder accepts this exact 296-byte
application-message profile, including its declared 284-byte argument length.
It checks repeated owner/item identities, source/destination fields, type tags,
request discriminants, absent metadata and padding. It does not deserialize a
general script graph. Capture build identity remains unknown; this is a narrow
capture-derived profile, not proof of matching-client acceptance.

`inventory-reorder --character-id N --request PATH` imports that message into
the locked offline store. Input reads stop after 297 bytes, allowing rejection
of oversized files without loading them in full. The request owner must match
the active account-owned character; the item must already occupy the requested
source. The `r` inventory journal operation removes the item and inserts it at
the requested pre-removal boundary, shifting intervening items and adjusting a
forward destination by one. It never merges stacks or changes equipment.
Sparse bags are rejected because their compaction semantics are not established.
The existing `m` slot-swap/merge operation retains its local policy.

This importer uses normal candidate validation and flush-before-publish store
handling. Journals containing `r` inventory commands require this newer reader.
Live native move replies remain gated on authoritative icon/physical-definition
metadata and matching-client verification; this change does not route sorting
requests from a connected client or claim a complete inventory implementation.
Formatted and compiled only; no tests or native game checks were run.

## Missing tutorial content and visibility (2026-10-24)

The user reported being unable to leave and seeing no portals or doors. The
persisted position had moved from start cell `02990109` into `0299011d`, at
approximately `[96.63357, 241.79227, -44.962006]`. The previous Scene implementation
sent content only for an exact cell match. Actual native DAT visible-cell arrays
include each of those rooms from the other; removing start-room objects on that
transition contradicted their native visibility data.

`visibility.rs` reads the bounded first visible-cell array of `CEnvCell`, following
`00482da0`, `00481630`, `00486f10` and `00485800`. Shared cells and recursive portal
reachability are not promoted to PVS. Scene publication now treats observer and
target direction separately, sends only entered/left interest differences and
retains overlapping visible objects across room changes. Outdoors, a 3x3
landblock view is an explicit development policy; it is not a reconstructed
landscape nearWidth setting or collision/line-of-sight authority.

The stationary portal visual `1f00031e` also lacked a captured mode and slider.
The exact retained portal profile uses flags `8082`, mode `40000001`, slider
`4000000a` value 1, velocity 0, before cell/frame data. It occurs in
`ac2_omishan_allyhall.elog` and four other captures; `00641530` confirms packing
order. The visual itself supplies setup/mesh and startup effects. Other object
visuals retain their existing packet layout.

The training preset now retains the 30 observed actors in addition to the 247
base placements. Eight authored objects include the start/corridor gateways,
an outdoor return, quartermasters and training creatures. A bounded scan of 79
tutorial cells found no geometry exit reachable from the starting component;
this does not prove the absence of an original scripted exit. The authored
gateways remain explicitly development travel, not original tutorial progression.

The optimized runtime was rebuilt and restarted on loopback UDP 19370/19371;
startup loaded 285 built-in objects and 159 routes. The operator's untracked
local placement is preserved separately and can be combined in the runtime
manifest without replacing the base content. No test suite or automated client
run was performed. The source/data causes are established; actual portal display,
travel and broader population still require fresh matching-client observation.

## Native merchant refresh (2026-10-26)

The client request `b83a0066` contains exactly one tagged merchant IID (12
argument bytes). Matching build-7148 WLib `56000005`, SHA-256
`b402b04a6ef587e83145524ad8612c66e6a16d3a710fc7addeb78e8df22ba21d`,
resolves outbound stub `b83a0067` at opcode offset `1030e8` and its active-store
caller `b832000c` at `11ca70`. The caller marks the store pending before sending.
The cleaned capture `Npc interaction/händelr_gilden_schatzmeister_quitungs_kauf.elog`
corroborates that request between `b83a002a` refresh responses carrying the same
merchant IID. Capture client identity is unknown. Native endpoint corroboration
is `gCM_Store::CallEFun_SendNotice_UpdateStore` at `00467c20` and
`gmShopUI::RecvNotice_UpdateStore` at `0041e8d0`.

The server reuses its existing catalog/offer identities and `Shop::refresh`; no
item or currency transaction occurs. It rechecks a matching unexpired shop,
visibility/trade distance, living/out-of-combat state and a 100 ms action limit.
Only successful queue admission extends the shop deadline. Compilation is
separate from native UI acceptance; no gameplay tests were run.

## Gameplay option persistence (2026-10-26)

`8212000c` is `UploadGameOptions`, not a quest request. Native `00451ac0` and
`00463b70` route to WLib `9aa4008f` (`437ac`) and `8212001d` (`102078`).
The matching WLib is the same `56000005` SHA-256 recorded above. Arguments are
exactly 32 bytes: tagged integer bitfield, tagged integer radar mask, tagged
integer shortcut height and tagged float damage-text range. Float tag 3 is
corroborated by `gPackBuffer::CallEFun_PackFloat` at `0053f690`. The cleaned
`ac2_player_die.elog` capture (SHA-256
`f14f793a2f75d5461f0f6561f21119fee8d3ada60cf731cfb414fc65479da6dc`)
corroborates that shape; its client build identity remains unknown.

Native `00437e50` clamps shortcut height to 1–5; this server rejects other
values. `GameplayOptionsProfile::Clear` at `005ffc60` uses range -1.0 as its
default. The bounded profile accepts that sentinel and finite nonnegative
ranges; no unsupported original upper bound is inferred. Preference bitfields
remain opaque and grant no server gameplay authority. Duplicate uploads do not
append another record. Owner-checked `o1` journal records persist exact float
bits and replay strictly, including through the existing checked compaction.
Older binaries lacking `o1` cannot read a journal containing these records.

On reconnect, `GameplayOptionsProfile::StreamPack` at `006000b0` establishes
mask `10a8` for these four fields. The serializer combines this with saved
window/hotbar masks and emits height, damage range, locations, radar, bitfield,
version and opacity in native order. The version marker is retained when
opacity needs it. No upload acknowledgement is invented. Compilation of the
isolated staged source passed; persistence/UI runtime acceptance remains open
because gameplay tests were not requested.
