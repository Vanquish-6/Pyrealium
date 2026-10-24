# Pyrealium Launcher

**Never played it. Already improving the commute.**

A portable Windows launcher for the experimental Pyrealium AC2 emulator.
Requires Windows and .NET Framework 4.x; no administrator access is needed.

A Legions-inspired backdrop of smoky ruins and green portal magic, bronze and
parchment accents, and the supplied **Asheron's Call 2: Legions** cover give Dereth
a new front door. **Play** holds account and server details;
**Settings** holds the game folder and recovered client. The footer's
[Support Pyrealium on Patreon](https://www.patreon.com/Vanquish90/) link opens your
browser. Supporting the project is entirely optional.

## Build

From the repository root:

```powershell
powershell -File .\launcher\build.ps1
```

This compiles the launcher, runs its validation checks, and creates:

- `launcher/dist/PyrealiumLauncher.exe`
- `launcher/PyrealiumLauncher-windows.zip` containing the executable and this guide

The executable embeds its artwork; no loose image folder is required. The package
contains no game binaries or DATs. The ZIP is generated locally, not committed.

## Connect

1. Start the emulator separately. The launcher does not start a server.
2. Enter account `local`, host `127.0.0.1`, and your server's base port.
3. Open **Settings** and choose the game-data folder and, separately, the recovered **7148** executable.
4. Return to **Play** and click **Enter Dereth**. Review any Windows Run prompt yourself.

Drag the top bar to move the launcher. Tab moves between controls; Enter launches
from Play, and Escape returns from Settings. Installation errors open Settings
with a message explaining what needs attention. The launcher does not query server
health or display an invented online player count.

For the verified world-entry experiment, follow the repository README to prepare
the converted assets. Use **19270** and the resulting `emulator/target/compat-7148`
directory as the game folder. The ordinary local selection test uses **19070**.
The installed **7230** executable is not supported by this launcher; server-side
experimental parsing is not native acceptance.

Keep the recovered client alongside its required DLLs. The selected data folder
must contain nonempty `portal.dat` and `ijl15.dll` files.

The launcher uses that folder as its working directory and passes
`--rodat --width 1024 --height 768`. Read-only DAT access is required for the
converted directory because some large assets are hardlinked. It does not replace
the installed client, patch game files, or alter firewall settings.

Settings are saved in `%LOCALAPPDATA%\Pyrealium\launcher.xml`. No passwords are
requested or stored. Current authentication is a local developer allowlist, and
the emulator accepts loopback connections only. Remote hosting remains unfinished.

“Client started” means Windows launched a process. It does not certify that a
world appeared, an account is secure, or we have finished inventing an MMO.

## Validation

The build script runs validation automatically. A standalone check is available:

```powershell
.\launcher\dist\PyrealiumLauncher.exe --self-test 'C:\Temp\launcher-tests.txt'
```

A zero exit code means input/argument checks passed; it does not launch the game.

For a rendered UI preview without starting the game:

```powershell
.\launcher\dist\PyrealiumLauncher.exe --preview 'C:\Temp\launcher-play.png'
.\launcher\dist\PyrealiumLauncher.exe --preview 'C:\Temp\launcher-settings.png' settings
```

## Artwork

`assets/legions-portal.png` was created with the built-in image generation tool,
using the supplied Legions cover as a style reference. Its exact prompt is saved
in `assets/legions-prompt.txt`. `assets/dereth.png` and `assets/legions-cover.png`
are the user-supplied gameplay and Legions cover references.
These images are presentation assets, not a promise of the
emulator's current gameplay. Asheron's Call 2 and its original artwork belong to
their respective owners; Pyrealium is an independent preservation project.

## License

The original launcher code is **AGPL-3.0-only**, copyright (C) 2026 Vanquish.
See [LICENSE](../LICENSE) and [licensing requirements](../LICENSING.md).
The license does not relicense independently owned game artwork or client assets.
Launcher packages include the license and notices; distributors must also provide
Corresponding Source through an AGPL-permitted method.
