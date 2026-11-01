# Xenify Hub

Xenify Hub installs and updates the Xenify apps: Write, Draw, Chat and the
Xenify server. It works on its own. When it's signed in to a Xenify server,
that server's admin decides which apps and versions the computer gets.

- App: `crates/xenify-hub` (Tauri; screens in `ui/`)
- Logic without a window: `crates/xenify-hub-core`, covering the signed release
  list, channels, the server's policy, and installing and rolling back
- Server side: `crates/xenify-server/src/hub.rs` and the admin page Apps & updates
- Release tooling: `scripts/release.sh` and the `xenify-release` program

## For people using it

- **Installed** lists what's on this computer. **More from Xenify** lists what
  isn't installed yet. Downloads run in the background, with progress shown.
- **⋯ on an app** offers:
  - **Get updates from**:
    - **Release**: tested versions.
    - **Preview**: the next version early.
    - **Nightly**: every day's build.
    - **Fixed version…**: stays on one version.
  - Each channel also gets newer releases from the steadier channels. For
    example, a Release newer than the last Preview reaches Preview users.
  - **Update automatically**: off means the app shows an **Update** button instead.
  - **Go back to x.y**: the version before is always kept. Going back keeps
    the app on that version until you pick a channel again.
  - **Show in folder** and **Remove**.
- **Settings** (the gear or the status pill):
  - Sign in to a Xenify server.
  - Change where updates come from.
  - **Check for updates now**.
- The Hub checks every 30 minutes. Automatic updates install in the
  background. An app that's already open uses the new version from its next start
  ("Restart Write to use it").
- **The Hub updates itself** (product `hub` in the release list: the Windows
  installer, the Linux AppImage, the Mac app as a .tar.gz, made by
  `scripts/package-hub.sh <system> <version>`). A newer one is downloaded and
  checked like any app, then put in place: by itself while the window is
  closed and nothing is installing, otherwise from the window's
  **Restart now**. Windows runs the new installer quietly; the AppImage and
  the Mac app are swapped. A copy that can't replace itself (the .deb, an app
  in a folder it can't write) shows **Download it** instead. Its version is
  `XENIFY_VERSION` at build time (so a nightly knows it's a nightly).
- **Settings › Run AI models on this computer for the server** (shown when
  the server is installed and there's an AI runner for this computer; there
  is none for Windows): installs LocalAI, starts it with the Hub, and points
  the server this Hub is signed in to on this computer at it (Admin › AI).

## Managed by a server

The admin sets this on **Admin › Apps & updates** in the server's admin pages.

- **Which versions**, per app:
  - each person chooses
  - Release, Preview or Nightly
  - a fixed version
  - not allowed
- **When**: automatically, automatically at night (22:00 to 06:00), or people click Update.
- Anything the admin left as "each person chooses" stays the person's choice.
  Choices the server makes appear in the Hub with "set by …" and can't be changed there.
- **Keep copies of updates on this server**: computers download each file
  from the server, which fetches it once. The Hub still checks every file
  against Xenify's signature, so a server can't change what gets installed.
- **On computers** shows how many computers have each version. Each Hub
  reports what it has installed.
- **Update server now** asks the Hub on the server's own computer to install the
  new server version and hand over without downtime:
  1. The new server starts on the same port.
  2. The old one finishes its requests and stops.

## How downloads are trusted

- The release list is JSON signed with Xenify's **release key** (ed25519).
  The Hub knows the key's public half, in `TRUSTED` in
  `crates/xenify-hub/src/hub.rs`.
- The Hub refuses:
  - a list signed with an unknown key
  - a list changed after signing
  - a list older than one it has already seen (protection against updates
    being held back)
- Each download's size and sha256 are in the signed list. A file that
  doesn't match is refused before anything is unpacked.
- Archives (`.tar.gz` and `.zip`) can't write outside the app's folder.
- Builds of your own: put your key's public half in
  `<data>/xenify/hub/trusted-keys.json` as a JSON array of 32-byte arrays.
  `<data>` is `~/.local/share` on Linux, `%LOCALAPPDATA%` on Windows and
  `~/Library/Application Support` on a Mac, or `$XENIFY_DATA_DIR`.

Apps go into `<data>/xenify/hub/apps/<app>/<version>/`. `current.json` beside
them names the version in use and the one kept for going back. Older
versions are removed.

## Making a release

**From GitHub (the usual way).** Push to `main`, then run
`scripts/start-build.sh release [nightly|preview|release]` (or start
**Release** from the Actions tab of mr-bubble-pop/xenify). The workflows
are in `.github/workflows`; `scripts/start-build.sh site` deploys
xenify.net the same way. Linux, Windows and macOS are built.

The workflow does the rest:

1. It builds Write, Draw, Calc, Chat, Plan and the server for Linux,
   Windows and macOS.
   `scripts/hub-packages.sh` turns each build into a Hub package named
   `<app>-<version>-<system>-<arch>.<ext>`, with a `.run` note naming the
   program inside.
2. It uploads them to a new GitHub release, `hub-v<version>`.
3. It adds them to the release list, kept as `list.json` on the
   `hub-releases` GitHub release. It also adds PostgreSQL and LocalAI from
   their own releases.
4. It signs the list with the release key and publishes it as
   `manifest.json` on `hub-releases`. The Hub reads it from
   `https://github.com/mr-bubble-pop/xenify/releases/download/hub-releases/manifest.json`.

Preview and Nightly versions get a suffix, for example
`1.2.0-nightly.202610241230` or `1.2.0-preview.17`.

**The release key** is a file made once with
`cargo run -p xenify-hub-core --bin xenify-release -- keygen <file>`.

- The numbers it prints are its public half. They're in `TRUSTED` in
  `crates/xenify-hub/src/hub.rs`, so every Hub checks lists against them.
- The file is the secret half:
  - Keep a backup offline. If it's lost, Hubs can't take new lists until a
    Hub update trusts a new key.
  - Never commit it.
  - GitHub gets it as the secret **XENIFY_RELEASE_KEY** on
    mr-bubble-pop/xenify. Run `base64 -w0 <file>`, then on GitHub
    open Settings › Secrets and variables › Actions › New repository secret.

**By hand**, with the key file on your own computer:

```sh
XENIFY_RELEASE_KEY=~/xenify-release.key \
  scripts/release.sh release https://github.com/mr-bubble-pop/xenify/releases/download/v1.2.0 dist-hub/
```

The `xenify-release` commands:

- `keygen <key file>`
- `public <key file>`: prints the public half again.
- `add <list> <app> <name> <version> <channel> <os> <arch> <file> <url> <program>`
- `describe <list> <app> <name> <about> [part] [requires,…]`
- `sign <list> <key> <signed list>`

## Tray, starting with the computer, apps

- **Tray:** where the desktop has a tray (on Linux this needs the
  appindicator library), the Hub keeps running there when its window closes.
  The tray menu has Open, Check for updates and Quit. Without a tray it's an
  ordinary window.
- **Settings › This computer › Start Xenify Hub when I sign in:** adds an
  autostart entry (Linux), a Run key (Windows) or a launch agent (Mac). It
  starts with `--hidden`, so only the tray icon shows. The Hub turns it on
  the first time it runs, so updates arrive in the background; an app uses
  its new version from its next start.
- **Start the Xenify server when this computer starts:** appears once the
  server is installed. The Hub writes `server.toml` (the database, the data
  folder, its own PostgreSQL, the AI runner when that starts with the Hub)
  and runs `xenify-server service install` as administrator (pkexec, the
  macOS administrator prompt, or UAC; without pkexec it shows the `sudo`
  command to run). From then on the service starts the server before anyone
  signs in and updates it (`docs/server.md`, As a service); the Hub only
  shows it. Unticked, the Hub starts the server while the Hub runs.
- **No release list address to set:** the Hub and the server read Xenify's
  own signed list. For builds of your own, put the list's address in
  `hub.json` (`source`) and its key in `trusted-keys.json`.
- **The apps:**
  - Write and Draw check whether the Hub has put a newer version of them in
    place (`current.json` beside their version folder).
  - If it has, Help offers **Restart to use version x.y**. Unsaved work is
    asked about first.
  - Apps installed some other way (a package, the web) never show it.

## Spelling and grammar, shared

- Write and Draw both use the mistake-spotter model (67 MB). The Hub
  installs it **once**, as the `checker` part, which both apps `require`.
  Their Hub packages leave it out (`scripts/hub-packages.sh`).
- It goes to `<data>/xenify/hub/apps/checker/<version>/spotter`. The apps
  look for a model in this order (`SpotterPrefs::model_dir` in
  xenify-wp-check):
  1. a folder chosen in Settings
  2. `<data>/xenify/spotter`
  3. the Hub's part
  4. the copy in their own package (the web build and the separate
     `.deb`/`.exe` packages still carry one)
- The release workflow packs it with `scripts/hub-packages.sh checker`.
  It's uploaded once per `CHECKER_VERSION` (7.0.0 = model B7) to the
  `hub-parts` release, and listed for every system. A new model means a new
  `CHECKER_VERSION`; until then the apps' updates don't download it again.
- Parts (the shared model, PostgreSQL) get no Start menu entry or command.

## The server, its database and the AI runner

- **Installing the Server** also installs **PostgreSQL**. The release list
  says the server `requires` it; it's marked as a `part`. Portable builds
  come from [theseus-rs/postgresql-binaries](https://github.com/theseus-rs/postgresql-binaries)
  under the PostgreSQL-style licence, downloaded from there and not shipped by Xenify.
- **Start** (the server's button), the first time:
  1. The Hub makes the database folder with a random password (`initdb`,
     scram-sha-256).
  2. It starts PostgreSQL on 127.0.0.1:55433 with no socket files, and makes
     the `xenify` database.
  3. It starts the server with that database and a data folder beside it.
  4. The server's messages go to `<data>/xenify/hub/server.log`. The Hub
     reads the first-run setup link from there, so **Admin pages** opens setup
     directly.
- **Using a database of your own:** set `server_args` in `hub.json` to the
  server's arguments (e.g. `["--database", "postgres://…", "serve"]`). The
  Hub then doesn't start PostgreSQL.
- **AI runner (LocalAI, MIT):** a single program from LocalAI's releases,
  for Linux (x86-64, ARM) and Mac (ARM).
  - **Start** runs it on 127.0.0.1:8080 with its models in the Hub's folder.
  - **How to use it** shows what to enter on the server's Admin › AI page.
- Settings › This computer can start the server and the AI runner with the Hub.
- PostgreSQL can't be removed while the server needs it.
- Downloads go to `<data>/xenify/hub/downloads/` first. An interrupted
  download carries on where it stopped. Each file is checked in pieces, never
  held in memory whole; the server's copies are made the same way.

## Still to do

- Run the release workflow for the first time (Actions › Xenify Hub release).
- Check the Windows and Mac installers and shortcuts on real machines.
