# Pyrealium

A Rust server for the AC2 admin client, build **7148**. Protocol and data-format
claims are tied to matching recovered client routines, original-binary test
vectors and recorded native-client observations. Development policies are labeled
separately in the evidence notes.

**Local development is working; a complete playable AC2 server is still unfinished.**

The real 7148 client has connected, rendered character selection, entered the
authored human starting area and sent scene-complete and position messages. Native
walking and saved cross-cell placement were observed in the matching client.
The complete HUD, two-client rendering, combat and populated gameplay still need
implementation or acceptance. A MusicLoader error can require Continue.

## Implemented

- Native login framing, connection handshake and two-port UDP transport, with a
  loopback-only developer account allowlist.
- Original-binary-checked ISAAC checksums, bounded reliable delivery, reassembly,
  duplicate suppression, exact-byte retries and shared ordered queues.
- Human character creation fields retained in a checksummed journal, selection,
  ownership checks, deletion tombstones and offline restoration. Appearance uses
  the supported default human bodies; arbitrary customization is not rendered yet.
- Experimental DAT-backed world entry, saved placement, increasing instance
  stamps and return to character selection.
- Movement and head-look decoding, replay/stamp rejection, cell-indexed player
  presence and create/move/look/destroy replication. Same-cell visibility is a
  development policy; authoritative collision and movement-speed enforcement are
  unfinished.
- Validated native connection logoff releases player ownership and scene presence
  promptly. This has synthetic UDP coverage; native acceptance is pending.
- Initial health, vigor and level properties for a local level-one test profile.
  Values of 100 for vitals are development defaults, not recovered game balance.
- Durable per-character window placement, opacity and hotbar bindings, restored
  by the opt-in HUD initializer. These changes are compile checked only; native
  acceptance remains pending. Bindings do not grant skills or items.
- Transactional inventory storage with bag moves/swaps, stack merge/split,
  equipment swaps, unequip and consumption, plus offline management commands.
  The opt-in HUD path now initializes owned item objects and inventory maps;
  native presentation is unverified and inventory request/reply integration
  remains unfinished. No automatic starter grants.
- Bounded read-only DAT and extracted terrain readers. The supplied corpus of
  **65,025 terrain records** passed full validation with no missing/empty linked
  info records. Terrain decoding is not yet server physics.

## Build and run

Requires Rust with edition 2024 support (minimum 1.85; development checks used
1.94). The runtime uses the standard library, no third-party Rust dependencies,
and forbids unsafe Rust.

From this directory:

```powershell
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release --target-dir target/local-build
.\target\local-build\release\pyrealium.exe serve --account local --data data
```

The default ports are **127.0.0.1:9000/9001**. Use `--bind 127.0.0.1:19070` for a
different pair. This is local developer authentication, not a password or public
account service. Default mode does not supply a world bootstrap.

Use `--seconds 30` for a timed run. To stop gracefully from another terminal:

```powershell
.\target\local-build\release\pyrealium.exe stop --data data
```

Use the same data directory for both commands. Separate build directories let you
compile without replacing an executable already in use by a test server.

## Native world experiment

Close the previous test client and stop its server before running:

```powershell
.\start-world-test.ps1 -GameDirectory "C:\Asheron's Call 2" -ClientExecutable "C:\path\to\ac2client.exe"
```

This starts a bounded local experiment on **19270/19271**, account **local**, with
a persisted human test character. It prepares a separate `target/compat-7148`
asset directory when needed. The installed build-7230 player-state records are
incompatible with 7148; conversion accepts only pinned known records and refuses
an existing destination. The original installation stays untouched.

Preparation requires about 1.6 GB for a portal.dat copy. Other large DATs are
hardlinked for **read-only** use: `--rodat` is mandatory. Never open this test asset
directory with an updater or writable client session. Windows executable-launch
warnings must be reviewed directly by the user.

The portable [launcher](../launcher/README.md) accepts the server, account, port,
assets folder and recovered executable separately. For this experiment select
`target/compat-7148`, the original recovered 7148 executable, and port 19270. Game
files are not included in the launcher distribution.

## Character recovery and durability

Run offline against the same data directory:

```powershell
.\target\local-build\release\pyrealium.exe character-create-human --account local --name Explorer --data data
.\target\local-build\release\pyrealium.exe character-list --account local --data data
.\target\local-build\release\pyrealium.exe character-restore --account local --character-id 1 --data data
```

Creation and deletion/restoration events are flushed before acknowledgment.
Deletion preserves identity, appearance and saved position; IDs and names remain
reserved. This retention policy is local, not a claim of original timed deletion.
Placement checkpoints occur every five seconds, on character logout, connection
logoff and clean shutdown. A crash can lose movement since the latest checkpoint.

The store uses an exclusive writer lock. Prefer graceful shutdown. If a process
is killed, verify that no server using that directory remains before removing a
stale `server.lock`. Preserve a damaged journal for inspection; startup fails on
corruption rather than silently discarding progress.

## Validation and measured performance

The last executed standard suite passed **57 unit tests and seven integration checks**,
including one real-UDP test. Three additional opt-in UDP tests cover the converted bootstrap, rejection
of incompatible installed assets, and two-player movement/look/logout/re-entry,
deletion and connection-logoff behavior. They passed on the supplied assets.
These peers are synthetic; they do not establish native gameplay acceptance.
The subsequent window-preference and hotbar persistence changes were compile checked only;
automated tests were skipped at the user's request.
The later inventory-state implementation is also compile checked only.

```powershell
$env:AC2_TEST_COMPAT_DATA = 'C:\path\to\emulator\target\compat-7148'
$env:AC2_TEST_GAME_DATA = "C:\Asheron's Call 2"
cargo test --test udp_flow -- --ignored
cargo test --test multiplayer -- --ignored
cargo bench --bench transport --target-dir target/bench-build
cargo bench --bench scene --target-dir target/bench-build
```

A separate opt-in unit test uses `AC2_TEST_LANDBLOCKS` for the extracted terrain
corpus. The `legacy-inspect` and `legacy-validate` commands inspect that corpus
without changing it. The latter checks terrain records and linked info-file
presence; it does not decode every info record or envcell.

The ACK fast path avoids copying outstanding payloads. In the recorded warmed
microbenchmark with 64 outstanding 1 KiB messages, median duplicate-ACK processing
fell from 10,230 ns to 31 ns. Direct iteration of the cell index reduced 64-player
same-cell movement replication from 1,832 ns to 1,358 ns (about 26%) and head look
from 1,701 ns to 1,298 ns (about 24%). These are isolated processing costs, not
player latency, full-server throughput or capacity claims.

Sharing immutable retry bytes also reduced fresh encrypted-message processing
with 64 outstanding messages from 16,205 ns to 10,482 ns (about 35%) while retaining
transactional validation. The zero-pending case showed no improvement. See the
checksum evidence for the full measurements and benchmark boundaries.

Optional verification scripts execute selected original x86 routines offline:
`tools/crypto_binary_vectors.py` and `tools/movement_binary_vectors.py`. They need
Python's pefile and unicorn packages plus the matching original executable; these
are verification dependencies, not server runtime dependencies.

Pass `--client C:\path\to\ac2client.exe` when the original is outside this
checkout. `tools/qualities_binary_vectors.py` also accepts `--game-data PATH` for
the converted DAT directory. Original game binaries and DATs are not included.

Exact addresses, identities, policies, test boundaries and open work:

- [Authentication and wire format](docs/auth-evidence.md)
- [Checksum generation and reliability](docs/crypto-evidence.md)
- [World entry, movement, persistence and native observations](docs/world-evidence.md)

## Remaining gameplay work

An explicit development encounter loop is available with `--content training
--initialize-ui true --encounters content/development-encounters.tsv` alongside
the normal game-data/account/bind/data options. It connects native attack targets
to server health/vigor changes, stationary retaliation, XP/gold rewards, levels,
regeneration and timed creature respawn. Saved player state is restored on entry.
The rules and starter placement are development content; original combat formulas,
skills, animations, collision, item looting and native acceptance are unfinished.
See [player systems integration](docs/player-systems.md).

Game-data mode (`serve --game-data PATH`) now loads the bundled stationary world
pack automatically. Use `--content content/world.tsv` for an editable external
pack, or `--content none` to disable content, alongside normal bind/account/data options.
The combined pack contains 89 lifestones and 158 portals with DAT-backed visuals.
The server creates/removes them as players enter/leave their cell. Two additional
portal locations remain in the unresolved report because their terrain is missing.
A 7,583-entry entity/visual catalog and offline placement tool support explicit
NPC/object authoring. Binding, portal travel, NPC behavior and other interactions
remain unfinished, and native appearance has not been checked.
See [content format and provenance](content/README.md).

An additional snapshot is available with `--content observed`: it adds 30
capture-observed stationary actors to the 247-object pack, including Banderlings,
Idol/Surrogate Spawns, a named NPC, eggs and chests. It retains observed positions,
rotations and scale overrides. This snapshot does not implement AI, merchants,
combat, loot, quest interactions or respawn schedules; original capture build
identity and native appearance remain unverified.

Offline inventory management is available through the `inventory-*` commands in
`--help`; use the normal `--account`, `--character-id` and `--data` arguments.
The server must be stopped. Development grants require existing definition and
visual records in `--game-data`, with operator-supplied stack/equipment rules.
`inventory-reorder --character-id N --request PATH` imports one reassembled
296-byte native sorting request while offline. It requires matching owner/item
identities and a dense bag list; it does not enable live client sorting.
Existence of a DAT definition does not establish starter entitlement or balance.
See [inventory storage boundaries](docs/world-evidence.md#transactional-inventory-storage-2026-10-24).

An opt-in empty-player HUD experiment is available with
`serve --initialize-ui true --game-data PATH` alongside the normal bind/account/data
options. It requires the converted 7148 assets, emits UI_Load once after scene
completion, and uses typed empty story, quest, skill, effect and inventory graphs.
It restores saved window settings and hotbar bindings when present, and grants
no items, skills, gold or progress. This profile is not enabled by
default: native UI acceptance and backing gameplay systems remain unfinished.

Complete the native UI/script bootstrap, appearance
customization, world-content loading, collision and authoritative movement,
inventory/equipment, skills, combat, NPC behavior, progression and gameplay-state
persistence. Public authentication and remote hosting also remain unfinished.
Each native milestone needs a fresh matching-client run; passing packet tests
alone does not close it.
