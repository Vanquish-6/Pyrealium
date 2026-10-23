# Pyrealium

### Never legitimately played the game. Obviously qualified to rebuild the server.

Most people write an MMO emulator because they miss their childhood. I started this one without having legitimately played Asheron's Call 2. Apparently nostalgia is optional when you have Rust and an unreasonable opinion of your own chances.

**The plan: build a better AC2 emulator. The current achievement: a real client logs in, selects a character, and stands in the actual world.**

Not a mockup. Not a loading screen with aspirations. An actual character, on actual grass, contemplating the alarming amount of server code still missing.

Pyrealium contains a Rust emulator and a Windows player launcher. The server uses the standard library and **zero third-party Rust dependencies**. The dependency tree is a tasteful stump.

## Confidence is unlimited. Features are itemized.

| Feature | Current state |
| --- | --- |
| Login | Native build 7148 login and connection handshake verified locally |
| Authentication | Local developer account allowlist; no passwords or public account service |
| Transport | Checksums, bounded reliable delivery, retries, fragmentation and session expiry |
| Character selection | Real client displays the saved test character and selection scene |
| Character storage | Checksummed journal, exclusive writer lock, flush before acknowledgement |
| Native character creation | Experimental human request handling; saving through the game's UI remains unverified |
| World entry | **Verified with 7148 and converted test assets:** player and terrain render; client reports scene completion |
| Player launcher | Windows forms for account, host, port, game folder and client executable |
| Movement | Native client reports received in a live test, with malformed/stale report checks; authoritative physics remains unfinished |
| Examination UI | Targeted asset compatibility repair included; user reports the revised getter/setter repair cleared the tested crash; full examination data remains unfinished |
| Logout/re-entry | Session transition and message ordering fixed; repeated entry tested over UDP, native round trip still unverified |
| Vitals, combat, content | Unfinished. Grass is not an MMO, despite our marketing department's objections. |
| Multiplayer hosting | Not enabled; the server binds to IPv4 loopback only |

This is an early emulator, **not a complete or production-ready AC2 server**. The ambition is enormous. The status table is under oath.

Movement reports are currently telemetry: the server records the client's latest reported position after world entry, and logs a `movement_reports` count. These reports do not yet drive collision, saved positions, or other players. We can now hear the client claiming it went somewhere. Teaching the server to demand receipts comes next.

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

A music-loading error occurred during the accepted native test; choosing **Continue** allowed the world to render. Audio compatibility remains unfinished. An empty health bar is also not a minimalist design statement: vitals still need implementation.

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

Native character creation, appearance choices, movement, vitals, inventory, combat, multiplayer, and the remaining minor inconvenience of implementing an MMO.

**No nostalgia. Suspicious confidence. Somehow, already a world.**
