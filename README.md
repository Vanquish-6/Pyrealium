# Pyrealium

### Never legitimately played the game. Obviously qualified to rebuild the server.

Most people write an MMO emulator because they miss their childhood. I started this one without having legitimately played Asheron's Call 2. Apparently nostalgia is optional when you have Rust and an unreasonable opinion of your own chances.

**The plan: build a better AC2 emulator. The current achievement: native login, character selection, world entry and walking have been observed locally; the Rust player systems are growing, and a complete playable server is still unfinished.**

Not a mockup. Not a loading screen with aspirations. An actual character, on actual grass, contemplating the alarming amount of server code still missing.

Pyrealium contains a Rust emulator and a Windows player launcher. The server uses the standard library and **zero third-party Rust dependencies**. The dependency tree is a tasteful stump.

**Help keep the portals open:** [Support Pyrealium on Patreon](https://www.patreon.com/Vanquish90/). If rebuilding a game I never played sounds like a questionable financial decision, imagine being the person writing it. Support is optional and appreciated; it does not promise a release date or finished gameplay.

## Confidence is unlimited. Features are itemized.

**Status updated 2026-10-24. Experimental local development server; not ready for general players.**

- **Observed with the native 7148 client:** local login, character selection, world/terrain rendering, walking and saved cross-cell placement.
- **Implemented in Rust, with fresh client acceptance still pending:** the initialized HUD, supported inventory/equipment, skill training and advancement, development combat/healing, death/resurrection, binding/Recall, portal travel, a training quest, potions/emotes, command-based trading/crafting and session groups.
- **Still missing:** most original ability/effect behavior, armor bonuses, corpse/general loot, original quest content and dialogue, full merchant/crafting UI and economy, group reward sharing, authoritative collision/pathfinding and public authentication/hosting.

A compiled packet handler is not a verified player experience. Recent runtime additions passed library/server compilation and an optimized build; the local training server has been rebuilt and restarted; fresh client acceptance of the latest additions remains pending. Training rewards, combat values and instant travel are explicit development policies.


| Feature | Current state |
| --- | --- |
| Login | Native build 7148 login and connection handshake verified locally |
| Authentication | Local developer account allowlist; no passwords or public account service |
| Transport | Checksums, bounded reliable delivery, retries, fragmentation and session expiry |
| Character selection | Real client displays the saved test character and selection scene |
| Character storage | Durable character, inventory, progression, skills, quest and preference state; exclusive writer lock, atomic transactions and checked compaction |
| Native character creation | Human request fields and appearance choices are retained; default human bodies supported; broader creation/appearance acceptance pending |
| World entry | **Verified with 7148 and converted test assets:** player and terrain render; client reports scene completion |
| Player launcher | Portal-themed Windows launcher with game artwork, Play and Settings views, saved connection details and a Patreon link |
| Movement | Validated native reports drive saved placement and observer replication; authoritative collision remains unfinished |
| Examination UI | Compatibility repair cleared a previously reported crash; server descriptions cover visible players, owned items and content; fresh data/UI acceptance pending |
| Logout/re-entry | Session transition and message ordering fixed; repeated entry tested over UDP, native round trip still unverified |
| Other player objects | Scene-ready, cell-indexed spawn/move/look/remove replication; two-peer UDP tested, native two-client display unverified |
| Character recovery | Flushed deletion tombstones, reserved identities and offline restoration |
| Script actions | Local chat, tells, targeting, combat mode, supported object/skill use and eight emotes are routed; broader social systems and native acceptance remain open |
| HUD and preferences | Opt-in UI initialization restores progression, skills, inventory, quest journal, window settings and hotbar bindings; fresh client acceptance pending |
| Inventory and equipment | Transactional bags, sorting and supported hand equip/unequip; native item acquisition/removal codecs; armor composition and equipment bonuses missing |
| Skills | Catalog-based human training/untraining and XP advancement; development basic attack and learned self-heal; most original active effects/cast behavior missing |
| Combat and progression | Development creature damage/retaliation, vigor costs, XP/gold, levels, regeneration and timed respawns; original formulas, projectiles and pathfinding missing |
| Death and return | Saved vitals/Dead state, death UI, Resurrect, lifestone binding and Recall implemented; original cast presentation and death penalties missing |
| Quest journal | One authored Training Grounds quest: three kills, persistent progress, once-only 120 XP / 10 gold via a clickable training mentor or `@quest claim`; original quests/dialogue missing |
| Item rewards and potions | Training encounters grant a single health potion into a supported bag; consuming it atomically restores up to 25 health; corpse/general loot and native acceptance pending |
| Portals | 156 imported routes plus three authored training/return gateways; captured portal visual mode/slider restored; original travel presentation/access rules unfinished |
| World content | Training preset retains 247 base placements, 525 observed actors and 13 authored training objects; native interior visibility and a development outdoor view range; full world population missing |
| Content placement editor | DAT-backed mesh/texture/animation/interior/outdoor preview implemented; latest native paint/Tk crash patch awaits a fresh GUI load |
| Merchants and crafting | Native four-item shop/catalog and Buy button, inventory buyback, crafting panel with two potion recipes and atomic item/gold changes; consignment and full original economy missing |
| Groups and broader social play | Session groups up to eight: create/invite/accept/decline/leave/dismiss/disband, native member snapshots, vitals/cell refresh and @g chat; XP/loot remain individual; fresh two-client acceptance pending |
| Multiplayer hosting | Not enabled; the server binds to IPv4 loopback only |

This is an early emulator, **not a complete or production-ready AC2 server**. The ambition is enormous. The status table is under oath.

Movement reports now update durable placement and replicate to ready observers in the same cell. Re-entry restores position and correct native heading. The server checks timestamps, instance stamps, finite coordinates and available cells; authoritative collision and speed enforcement remain unfinished. We have receipts for where the client said it went. Cross-examining gravity remains on the docket.

The server tracks player presence through a cell index and publishes avatars after scene completion. One character cannot enter through two sessions at once. Movement, head look, cell transitions, logout and validated connection logoff use that same presence state. Supported hand attachments and stationary content use that scene lifecycle. Same-cell visibility is an initial development policy; full armor appearance, broad NPC behavior and recovered distance/occlusion rules remain unfinished.

## One runtime, fewer excuses

The standalone emulator now includes the integrated movement, persistence, scene,
terrain and native script-action systems, alongside the existing launcher and DAT
compatibility tools. See the [runtime guide](emulator/README.md) for current
commands, tests, recovery rules and measured performance.

The Rust runtime now has an authored training loop: receive a hand item and basic
attack skill, fight a stationary creature, earn XP/gold and potion rewards,
advance a three-kill quest, and claim its reward through the native journal path.
A learned human healing skill, potion use, death/resurrection, binding/Recall and
validated portal destinations are integrated. Use `serve-training --game-data PATH`
with the supported converted assets; see the runtime guide for account/data options.
The underlying persistence and codecs compile, but this complete loop has not
received fresh matching-client acceptance. No public-player readiness is implied.

## What's in the box

```text
emulator/           Rust server, tests and local test scripts
  tools/            Local DAT compatibility conversion and checks
launcher/           Windows launcher source and build script
```

No game executable, DLL, DAT, account database, decompilation, PDB, crash capture or research dump is included. Bring your own game installation and unreasonable expectations. This is an independent project, not an official AC2 service.

## Build the extremely early empire

Requires **Rust 1.85 or later** with Cargo. From the repository root:

```powershell
cd emulator
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
.\target\release\pyrealium.exe serve --account local
```

The default listener uses **127.0.0.1:9000 and :9001**. The second port handles connection acknowledgement. Apparently saying hello needed another front door. This default mode does not enable the experimental world bootstrap.

Use `--bind 127.0.0.1:19070` for a different base port, `--data PATH` for a separate character store, and `--seconds 600` for a ten-minute run. To stop cleanly, use the same data directory:

```powershell
.\target\release\pyrealium.exe stop --data data
```

Local authentication is an account-name allowlist, not proof of identity. An impressive Connect button does not turn it into an Internet authentication service. We checked.

## Put a character on the lawn

The verified setup uses the **recovered 7148 client** and a specific installed asset set associated with build 7230. Unconverted player settings in those assets crash 7148 during world loading. The supplied converter handles that specific layout; it is **not a universal client or DAT patcher**.

**Examination compatibility repair:** the newer scripts call native setter `0x009A0013` and getter `0x009A0014`, which build 7148 does not implement. They access an additional request field absent from the older client. The repair omits two setters and replaces one getter with its verified typed default (`0x203`), preserving script offsets and stack balance. The first setter-only repair was incomplete and still crashed on the getter; this revision upgrades that repair. The replacement instructions have been checked against the native 7148 interpreter handlers, and the user reports the revised repair cleared the tested crash. Full examination behavior and server-provided data remain unfinished. The lawn passed inspection. The inspector is on probation.

Check a local asset set without modifying it:

```powershell
python tools/check_7148_scripts.py 'C:\Games\AC2-test\portal.dat'
```

The check reports the known incompatible call sites and exits with code 2 when found. It checks this specific failure only; passing it does not certify the entire asset set. With the client closed, repair a previously prepared test directory using:

```powershell
python tools/repair_7148_examination.py 'C:\Games\AC2-test'
```

The repair requires the converter's manifest and a separate, non-hardlinked `portal.dat`. It backs up the original script record, verifies the write, and rejects unknown source data. It does not modify the executable or implement the newer examination options. The world-test script runs this repair automatically when the check finds the known calls.

On Windows, with Rust, Python 3.10+ and your own game files available:

```powershell
cd emulator
.\start-world-test.ps1 `
  -ClientExecutable 'C:\Games\AC2-7148\ac2client.exe' `
  -GameDirectory 'C:\Games\Asherons Call 2'
```

This builds the emulator, prepares `target/compat-7148`, creates the test human **Pyrealium** if the test store is new, starts a local server on **19270/19271**, and launches the client. The default test lasts 30 minutes; `-Seconds` and `-Port` override those values. Review any Windows Run prompt yourself.

The converter copies `portal.dat` and modifies two settings records whose source hashes match the supported layout. The examination repair then changes three call sites in one additional script record. Both verify their output and leave the source installation unchanged. Allow roughly **1.6 GB plus working space**. Other large DATs are hardlinked, so the client must use **`--rodat`**, as both supplied launch paths do. Never run an updater or a writable client against this test directory. An unknown source record or an existing conversion destination is rejected by the converter.

Keep the recovered client alongside its supporting DLLs. Game files are not downloaded or supplied by this project. The installed 7230 executable has not been verified with the emulator.

A music-loading error occurred during the accepted native test; choosing **Continue** allowed the world to render. Audio compatibility remains unfinished. Server vitals and an opt-in HUD initializer are now implemented; a blank or incorrect health bar still requires native-client investigation.

Close the game and stop the previous test before repeating it:

```powershell
.\target\compat-build\release\pyrealium.exe stop --data .\target\compat-test\data
```

## The player entrance has a doorbell

Build the launcher on Windows with .NET Framework 4.x, from the repository root:

```powershell
powershell -File .\launcher\build.ps1
```

The script runs launcher validation and produces `launcher/dist/PyrealiumLauncher.exe` and `launcher/PyrealiumLauncher-windows.zip`. These generated files are not tracked.

For the world test, use account **local**, host **127.0.0.1**, port **19270**, the prepared **emulator/target/compat-7148** game folder, and your recovered **7148** executable. The server must already be running. See [the launcher guide](launcher/README.md).

## Stability, because crashing confidently is still crashing

One event loop owns session mutation. Packet sizes, sequence windows, retries, fragment assemblies and session counts have explicit limits. Retransmissions reuse their original bytes. Malformed packets cannot permanently advance the checksum generator. The character journal flushes writes before acknowledging them and stops on corruption instead of silently deleting progress.

An abrupt exit can leave `data/server.lock`. Verify that no server is using that directory before removing a stale lock. Preserve a damaged journal for inspection; deleting everyone's characters is not a recovery strategy.

The ordinary Rust tests use synthetic peers and require no game files. Optional local asset checks run separately, from `emulator`:

```powershell
$env:AC2_TEST_GAME_DATA = 'C:\Games\Asherons Call 2'
$env:AC2_TEST_COMPAT_DATA = (Resolve-Path .\target\compat-7148).Path
cargo test --test udp_flow -- --include-ignored
python .\tools\test_7148_conversion.py
```

Packet tests establish protocol behavior. The native client established world rendering. Neither establishes ten thousand concurrent players. There are not even ten thousand players in front of this paragraph.

## The next boss fight

The next acceptance milestone is a fresh matching-client run through the integrated
HUD, inventory, combat, quest, potion, death and travel loop, followed by two-client
visibility. Native walking was already observed; the newer gameplay additions have
not had that same acceptance.

The remaining implementation work includes most active abilities/effects and their
animations, armor/bonuses, original loot and quest content, NPC conversations,
full merchant/crafting UI and economy, group XP/loot sharing, movement/collision/line-of-sight enforcement, NPC
pathfinding and public account/hosting support. The current authored training slice
is useful development progress, not a finished AC2 game.

**No nostalgia. Suspicious confidence. Somehow, already a world.**

## License

Original project code is **AGPL-3.0-only**, copyright (C) 2026 Vanquish. Covered
modified distributions must provide Corresponding Source, and modified network
versions must offer that source to their users as required by the AGPL. See
[LICENSE](LICENSE), [NOTICE](NOTICE), and [licensing requirements](LICENSING.md).
Third-party game/client assets retain their own rights. Previous MIT releases
retain their existing grants.
