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

This compiles and packages the launcher. Validation checks are opt-in with `-RunChecks`. It creates:

- `launcher/dist/PyrealiumLauncher.exe`
- `launcher/PyrealiumLauncher-windows.zip` containing the executable and this guide

The executable embeds its artwork; no loose image folder is required. The package
contains no game binaries or DATs. The ZIP is generated locally, not committed.

## Connect

1. Start the emulator separately. The launcher does not start a server.
2. For your existing characters, use `local` with a blank password. For a new identity, enter a new account name and a password of 12-128 characters, then click **Create Account**. Re-enter the password to sign in.
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

Settings are saved in `%LOCALAPPDATA%\Pyrealium\launcher.xml`. Passwords and
sign-in tickets are never saved there or passed on the client command line.
The account-service URL is editable in **Settings**; HTTPS is required except
for a service on this PC. The default is `http://127.0.0.1:19400/`.
Passwords use the server's existing Argon2id account store. The native client
receives a short-lived, single-use ticket through its Windows registry handoff.

`emulator/Start-Server.ps1` starts the local game and account service together.
The explicit `--local-accounts PATH` mode only permits an IPv4 loopback bind:
the configured existing `local` profile retains its blank-password development
login, while other identities require tickets. Public `--auth-data PATH` mode
still rejects every no-ticket login. Registration reserves existing character-account
names, including `local`; it does not change ownership of old saves.

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
