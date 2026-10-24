# Online accounts and hosting

The repository now contains an account-registration page, password backend,
native GLS-ticket login, per-account character selection and VPS service files.
These are implementation/build results. No public VPS, DNS name or certificate
has been provisioned, and native login across the internet remains unverified.
The normal local server remains loopback-only unless `--auth-data` is supplied.

## Starting size

For a small alpha, start with a Linux VPS, two vCPUs, 4 GB RAM, 40 GB SSD,
public IPv4 and off-host backups. Prefer a region close to the players and good
single-core performance. The gameplay loop is currently one thread; extra cores
do not automatically multiply world capacity. The account backend is a separate
process so password hashing does not block gameplay.

This is an engineering starting estimate, not a player-count benchmark. The
current game caps concurrent sessions at 64; that cap is not a demonstrated
playable capacity. A larger trial can start with four vCPUs and 8 GB, but should
be sized from measured live CPU, tick delay, memory, journal latency and traffic.

The server needs the matching `portal.dat`, `cell_1.dat`, `cell_2.dat` and
`local_English.dat` assets, not a GPU or a running Windows game client. Preserve
their compatibility identity. These proprietary assets are not part of this
project's AGPL grant and must not be served by the public website.

DigitalOcean's listed 2-vCPU/4-GiB Basic plan is $24/month with an 80-GiB SSD.
Hetzner's CX23 is the 2-vCPU/4-GB/40-GB alternative, subject to region and stock.
Provider prices and availability were checked on 2026-10-24; see
[DigitalOcean](https://www.digitalocean.com/pricing/droplets) and
[Hetzner](https://www.hetzner.com/cloud/cost-optimized/).
Allow a separate budget for backups and a domain; no paid resource was ordered.

## Sharing the existing Oracle host

A read-only inspection on 2026-10-24 at approximately 14:06 UTC found an
ARM64 Ubuntu 22.04 host with two logical CPUs and 11,932 MiB RAM. Available
memory was 7,917 MiB, the load averages were 0.08/0.22/0.18, and a short CPU
sample was 87–89% idle. Both Aetherium worlds were active, using approximately
1.5 GiB each. These are quiet-period observations, not peak-load guarantees.

The root volume had only 3.9 GiB free and was 92% used. Do not install the
matching AC2 assets or compile on that volume in this state. Expand the volume
and filesystem, or attach storage mounted specifically for AC2. The read-only
`deploy/shared-host-preflight.sh` requires at least 12 GiB free before copying
assets and 4 GiB available RAM. It also checks existing service health and AC2
port conflicts. It never cleans files or changes an existing service.

Build an immutable published revision on a separate workstation. For Linux/WSL,
install the Rust musl target with `rustup target add aarch64-unknown-linux-musl`,
set `PYREALIUM_SOURCE_URL` to the published revision URL, and run
`deploy/build-linux-arm64.sh`. The host needs its usual native linker for Rust
build scripts and procedural macros. Rust LLD links the ARM64 musl executable;
this avoids requiring a newer glibc than Oracle's Ubuntu installation provides.
Compilation and ELF inspection do not establish runtime or WAN compatibility.

On a Windows Rust/MSVC build workstation, install the same musl target and run
`deploy/build-linux-arm64.ps1 -SourceRoot PATH -SourceUrl REVISION_URL`, with an
optional `-TargetDirectory PATH`. Use an immutable source archive for `PATH` so
another editor's uncommitted changes cannot enter the release. This build uses
the Windows host toolchain for procedural macros and Rust LLD for ARM linking.

Install `deploy/pyrealium.slice` along with both AC2 service units. The slice
caps both AC2 processes together at 75% of one CPU and 2 GiB RAM, with a 1.5 GiB
memory-pressure threshold, lower CPU/I/O weights and no swap allowance. Each
service has additional memory/task/log limits. These are conservative starting
limits for a shared two-core host; they may throttle AC2 under load. Shared
kernel, storage and network contention remain possible. Do not change
Aetherium's units, limits, executable, ports or data when installing AC2.

Nginx already owns ports 80/443 on this host. Use the separate virtual-host
template `deploy/nginx-accounts.conf.example`; do not install Caddy or replace
the existing Nginx configuration. A real account hostname, matching certificate
and canonical account origin are required before enabling that virtual host.
Validate the complete Nginx configuration before a graceful reload. Keep the
account backend on 127.0.0.1:19400 and use only UDP 19370/19371 for AC2, leaving
Aetherium's UDP 9000/9001 and 9100/9101 untouched. Add only the AC2-specific
firewall rules after the isolated service is ready; never flush either firewall.

The operator selected separate AC2 storage. OCI block volumes have a minimum
size of **50 GB**, so the earlier 40 GB suggestion does not fit that service.
Create the volume in the instance's availability domain, review the tenancy's
storage quota and billing, and attach it to this instance. There was no OCI CLI
or cloud-management configuration available through the inspected SSH account;
creating/attaching the volume requires Oracle Console or separately configured
cloud API access. No volume has been created or formatted by this preparation.
See [Oracle's volume creation guide](https://docs.oracle.com/en-us/iaas/Content/Block/Tasks/creatingavolume.htm).

Mount the verified new volume at `/srv/pyrealium` using its filesystem UUID.
Identify the attached device explicitly and inspect its signatures before
formatting anything; `/dev/sda` is the existing Aetherium boot disk. Never infer
the new device from an arbitrary disk ordering. Back up `/etc/fstab` before
adding the new UUID entry. Use `nofail` so an unavailable AC2 disk does not
prevent the existing host from booting.

Create `/srv/pyrealium/state/{auth,game}` on that volume. On a fresh AC2 install,
make `/var/lib/pyrealium` a symlink to `/srv/pyrealium/state`; if that path already
exists, stop and inspect it instead of replacing or moving unknown state. Keep
the service user as owner and use mode 0700 for private state. Install
`deploy/oracle-volume.conf` as a drop-in for **both** AC2 services. The mount
condition and mount dependency stop AC2 from falling back to the root disk if
its volume is unavailable. Assets and the content manifest also live on the
mounted volume; the small executable and legal notices can remain under `/opt`.

Stage small release bundles under the operator's home directory first. Check
their SHA-256 before installation. Provision a separate `pyrealium` user and
directories using the layout below; leave the services disabled until storage,
matching assets, the content manifest, account setup and HTTPS are ready. Record
Aetherium's PIDs and activation timestamps before/after installation. Rollback
stops only the AC2 services and removes only its new web virtual host; it does
not restart Aetherium. Never delete another application's files to make room.

Resource-control semantics are documented in
[systemd.resource-control](https://www.freedesktop.org/software/systemd/man/249/systemd.resource-control.html);
proxy forwarding directives are documented by
[Nginx](https://nginx.org/en/docs/http/ngx_http_proxy_module.html).

## Authentication flow

1. The player creates an account on the HTTPS page. Passwords are stored as
   salted Argon2id v19 hashes (19 MiB, two passes, one lane), never plaintext.
2. `launcher/Start-Online.ps1` prompts for credentials and posts them over HTTPS.
3. The backend issues a random 256-bit ticket, valid for 60 seconds and bound to
   the source IPv4 address. Only its SHA-256-indexed record is saved.
4. The launcher writes the ticket to `HKCU\Software\Turbine\ac2`, value
   `GLSTicket`, then launches the native client with `/glsticket`. The client
   consumes and deletes that value. Passwords/tickets are absent from the game
   command line and launcher settings.
5. Native authentication type `40000002` carries a packed ticket string. Public
   mode atomically consumes the matching ticket before allocating a game session.
   It rejects developer/no-ticket authentication and account overrides. Exact
   retries from an established endpoint reuse its handshake without consuming
   the ticket twice.
6. Character lists, world entry and mutations use the authenticated session's
   account, not a shared configured developer name.

A fresh valid login replaces that account's prior endpoint. Replaying an old
ticket at a new endpoint cannot replace it. This keeps one account from using
all world session slots while allowing an authenticated reconnect.

Native authority: `gmClient::InitAuth` 00404e10, command-line evaluation 007218f0,
ticket packing 004042f0 and NetAuthenticator serialization 004c5200. No client
patch is needed to carry the ticket. Its native obfuscation is reversible:
**game UDP is not made confidential by this change**. Single-use, short expiry
and IP binding reduce ticket exposure but are not a substitute for encrypted
game transport or a hostile-network security assessment.

## Prepare the server

Build on the Linux host with the saved lockfile:

```sh
cargo build --locked --release --manifest-path emulator/Cargo.toml
```

Use a dedicated `pyrealium` system user. Install the binary and legal notices
(`LICENSE`, `NOTICE`, `LICENSING.md` and the entire `LICENSES/` directory) under
`/opt/pyrealium`. Put the game store under `/var/lib/pyrealium/game`, account data
under `/var/lib/pyrealium/auth`, matching DATs under `/srv/pyrealium/game-data`
and the operator's combined content manifest at `/srv/pyrealium/live-world.tsv`.
Keep the state directories writable only by that service user. Preserve the
complete game store, including its position snapshot and journal backups.

Before starting the account backend, provision the owner's account interactively:

```sh
/opt/pyrealium/pyrealium account-create --auth-data /var/lib/pyrealium/auth --account local
```

The hidden password prompts avoid shell-history exposure. The name `local` and
other existing game-account names are reserved from public registration to
prevent someone claiming older characters. Use the same game-store directory
for the account backend and game process. Never put account files or ticket
directories under a web document root or commit them to Git.

Copy the service templates from `deploy/` to the system's service directory,
and `deploy/online.env.example` to `/etc/pyrealium/online.env`. Set real account
and game DNS names. Replace the example account hostname in `deploy/Caddyfile`
and install it as Caddy's configuration. Caddy terminates HTTPS and forwards to
the loopback backend. Its configuration replaces `X-Forwarded-For`; do not expose
the backend directly or trust client-supplied forwarding headers.

Publish direct IPv4 A records for both names. At this stage, do not put the
account API behind an additional CDN/proxy or publish an IPv6-only account
endpoint: its observed source must match the native game's IPv4 source for
ticket binding. Do not change networks between signing in and launching.

Allow inbound TCP 80/443 and UDP 19370/19371. Keep TCP 19400 private to the host.
Use the VPS firewall as well as the host firewall. The two game ports must be
reachable under their configured numbers. Cross-internet UDP/NAT behavior still
needs real native-client acceptance; a local startup does not establish it.
The secondary-port connection ACK can be matched back to the original endpoint
only with its current cookie, net ID, iteration and source IP. This allows NAT
to choose a different source port for that ACK without trusting port arithmetic
or unauthenticated endpoint rebinding.

The service templates wait for the normal stop request before terminating a
process. After a crash, inspect a leftover lock and its owning process before
removing it; restart intentionally fails closed on uncertain ownership. Back up
game and account journals off-host with restrictive permissions. Tickets expire
and are not durable player state.

## Local account-page development

The web backend permits an explicit loopback HTTP origin for local development:

```text
pyrealium account-web --auth-data emulator/data/auth --data GAME_STORE --web-bind 127.0.0.1:19400 --public-origin http://127.0.0.1:19400 --public-host 127.0.0.1 --public-port 19370
```

The online launcher intentionally requires HTTPS. A local page preview is not
an internet deployment or proof of successful native ticket login. The existing
GUI launcher remains the local developer launcher; the new PowerShell entry is
the separate authenticated online path pending GUI integration.

Stop the account backend with:

```text
pyrealium account-stop --auth-data PATH --seconds 20
```

## Boundaries before opening registration widely

Account recovery, password reset, email verification, administrative moderation,
distributed account storage and production abuse controls remain incomplete.
The backend bounds request bodies, password work, account/ticket counts and
login attempts; these limits do not provide upstream DDoS protection. No tests,
browser automation or native client run was performed for this change at the
user's request. Build and startup validation are reported separately from
account creation, authentication and gameplay acceptance.

The account page links `/source` to the binary's compiled source offer. Operators
of modified builds must publish their complete corresponding source and set
`PYREALIUM_SOURCE_URL` to that source when building.
