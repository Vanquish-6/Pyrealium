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
The subsequent UI placement/opacity persistence change was compile checked only;
automated tests were skipped at the user's request.

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

An opt-in empty-player HUD experiment is available with
`serve --initialize-ui true --game-data PATH` alongside the normal bind/account/data
options. It requires the converted 7148 assets, emits UI_Load once after scene
completion, and uses typed empty story, quest, skill, effect and inventory graphs.
It grants no items, skills, gold or progress. This profile is not enabled by
default: native UI acceptance and backing gameplay systems remain unfinished.

Complete the native UI/script bootstrap, appearance
customization, world-content loading, collision and authoritative movement,
inventory/equipment, skills, combat, NPC behavior, progression and gameplay-state
persistence. Public authentication and remote hosting also remain unfinished.
Each native milestone needs a fresh matching-client run; passing packet tests
alone does not close it.
