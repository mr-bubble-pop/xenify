# Xenify Server

A server you run yourself, on a home PC, an office machine or a VPS. Xenify
doesn't host one for anyone. It gives the Xenify apps:

- sign-in
- shared files, searched and read in a browser (the Files page) and public
  read-only links
- groups and permissions
- editing together in Write and Draw
- templates
- Draw canvases and Calc tables and charts that update inside Write
  documents
- chat
- AI through a model runner on your own computer
- update policy for Xenify Hub

Without a server, the apps keep working on their own exactly as before.

- Program: `crates/xenify-server` (`xenify-server`). It has no window: setup
  and admin are web pages it serves.
- Database: PostgreSQL. Files go in a data folder (`blobs/`, `tls/`, `hub/`).
- Shared app code: `crates/xenify-server/client/`, copied into Write and Draw
  by `scripts/sync-server-client.sh`.

## Installing it

- **A computer with a screen** (home or office): Xenify Hub installs the
  server, its PostgreSQL and the AI runner. In the Hub's settings, **Start the
  Xenify server when this computer starts** makes it a system service (it
  asks for the computer's password once); without it, the Hub starts the
  server while the Hub runs.
- **A VPS, or any Linux server without a screen:**

  ```sh
  curl -fsSL https://xenify.net/install-server.sh | sudo sh
  # with a domain name pointing at it:
  curl -fsSL https://xenify.net/install-server.sh | sudo sh -s -- --domain files.example.com --email you@example.com
  ```

  It installs nothing on the system. It makes the user `xenify` and
  `/etc/xenify-server/config.toml`, puts the server in `/opt/xenify-server`
  as a systemd service, and prints the one-time setup link
  (`site/install-server.sh`). The database is either a private PostgreSQL
  just for Xenify (the default: `xenify-server database setup` fetches the
  portable build from the signed release list into
  `/var/lib/xenify-server/postgres`, makes its database with a random
  password, listening on 127.0.0.1:55433 only, and the service starts and
  stops it) or one you already run (`--database postgres://…`; from a
  terminal, the script asks).

### Setting it up safely

Until it's set up, the server only takes its setup with the one-time link
it printed (a random code, also in `<data>/address.json`; `xenify-server
setup-link` shows it again), and only from this computer, its network, a VPN
(100.64.0.0/10, fc00::/7) or over HTTPS. The code never crosses the internet
unencrypted.

- **With a domain name:** HTTPS on port 443 with a certificate from Let's
  Encrypt; the link is `https://<domain>/admin/#/setup/…`. Admin pages are
  then open from anywhere, over HTTPS only.
- **Without one:** plain HTTP listens on 127.0.0.1 only. Set it up through an
  SSH tunnel (`ssh -L 7700:localhost:7700 you@server`, then
  `http://localhost:7700/admin/#/setup/…`) or a VPN. Admin pages stay closed to
  the internet afterwards too.

## Running it

```sh
xenify-server --config /etc/xenify-server/config.toml serve
```

The settings file (`--config`; default `/etc/xenify-server/config.toml`, or
`%ProgramData%\Xenify\server\config.toml` on Windows, when it's there) holds
only what the server needs before it can read its database:

```toml
database = "postgres://xenify:…@127.0.0.1/xenify"
data = "/var/lib/xenify-server/data"
# Optional: fixed listening addresses (otherwise Admin › Network).
listen = "127.0.0.1:7700"
# The service starts these first (the Hub writes them for its own PostgreSQL
# and the AI runner).
[postgres]
program = ".../bin/postgres"
data = ".../postgres-data"
[[also]]
program = ".../local-ai"
args = ["run", "--address", "127.0.0.1:8080"]
```

Everything else is on **Admin › Network** and applied while it runs: the
ports (HTTP 7700 and HTTPS 7443 by default, or HTTPS off behind a reverse
proxy), announcing the server to apps on the network (mDNS, `_xenify._tcp`)
and
the domain name for Let's Encrypt. The server always offers the apps in the
browser: it fetches them from the signed release list by itself (retrying
every few hours until it has them; Network › Try again now), or serves
`--apps <dir>`. A port is shared only with the same server
(an update taking over): another program or Xenify server there is refused.

Flags (and the same `XENIFY_*` environment variables) still win, and the
admin pages show them as fixed: `--listen`, `--https <address|off>`,
`--apps <dir>`, `--hidden`, `--domain` / `--email` / `--acme-staging`,
`--allow-local-webhooks`. `xenify-server configure` saves the Network
settings before the first start (what `install-server.sh` uses):
`--http-port`, `--https-port <port|off>`, `--domain`, `--email`,
`--admin-access local|lan|anywhere`.

Let's Encrypt checks the domain on port 443 (TLS-ALPN-01): set the HTTPS
port to 443 (the systemd service may bind it) or forward 443 to it. Other
names and addresses (the network's) keep the server's own certificate
(Admin › Network › Download this server's certificate).

On the first start, open the setup link and set it up:

- The admin's name, email address (what they sign in with) and password.
- An admin who can't sign in: `xenify-server reset-password <email>` on
  the server's computer prints a code: sign in with the email address,
  type the code, and pick a new password (it signs them out everywhere).

### As a service

`xenify-server service install [--dir D] [--user U]` (as root or
administrator) registers `service run`: a systemd unit on Linux (as user U,
allowed to bind port 443), a launch daemon on a Mac, a service on Windows.
`service uninstall` removes it; `service status` says whether it's there.

`service run` is a small supervisor: it starts the server's own PostgreSQL
and `[[also]]` programs, runs the newest version in `<dir>/versions` (else
`<dir>/xenify-server`), and restarts it if it stops. The running server
updates itself into `<dir>/versions` from the signed release list, following
the **Server** row of Admin › Apps & updates (Release and automatic by
default; **Update server now** too). When a new version appears, the
supervisor starts it beside the old one; once it listens, the old one is
told to finish (its input closes) and stops. A version that doesn't start is
left alone and the old one keeps running.

## Admin pages

The admin pages are reachable from this computer only, from computers on the
network or a VPN (the default), or from anywhere (from outside the network,
over HTTPS only). Set this under **Network**.

- **People & groups**:
  - Add people: only admins (or someone with People in Roles › Advanced)
    make accounts; nobody joins by themselves, and there are no starting
    passwords. With an email address (and outgoing mail set up) the new
    person gets an invitation by email; otherwise the admin gets a one-time
    code to pass on (the dialog links to Network › Outgoing mail). A
    person is a name and an email address: no user names to pick or
    remember. Signing in the first time: the email address, a code
    (emailed then, or the admin's), then their own password, and they're
    in. A forgotten password goes the same way ("Forgot password?", or ⋯ ›
    Reset password).
  - Each person's roles are picked when adding or changing them; Admin is
    one of them (Roles › Admin lists who has it).
  - **Two-step sign-in**: who must use it (nobody, admins, everyone);
    ⋯ › Clear two-step sign-in for a lost phone.
  - The only admin can't lose the Admin role or be turned off.
- **Roles**: what people may do on the server (Everyone, and roles given to
  people or groups), and document roles for sharing (for Plan, exactly
  which changes). **Advanced**: parts of running the server (people and
  groups, roles, AI, network, updates, …) for a role, so someone can look
  after some of it without being a full admin. Only full admins make
  admins or hand out these powers.
- **Spaces**: shared folders, and who can view, comment, edit or manage
  them. Everyone has "My files". Templates are in the shared "Templates"
  space, which only admins can change.
- **AI**:
  - The program for writing and questions: Ollama (recommended), LocalAI or
    any OpenAI-style service; ComfyUI for pictures and clips.
  - Search Hugging Face and Civitai and install models. Only SafeTensors and
    GGUF files are installed, never pickled files.
  - Choose which models people can pick, per kind (text, vision, pictures,
    video), with a default for each.
  - **Picture styles**: choose a picture model, an installed LoRA and a
    strength, and **Make a style**. The server writes a LocalAI model file
    for it (the picture model's settings plus `lora_adapters` and
    `lora_scales`) and offers it as a picture model of its own.
    - LocalAI's diffusers runner takes LoRAs only from a model's settings,
      not per request, so Write and Draw just pick the style like any other
      picture model.
- **Data sources** (databases for Calc: PostgreSQL, MySQL, SQL Server, a
  SQLite file or CSV files on the server):
  - Set up by whoever may set up database connections (a role permission,
    Roles › Advanced; admins too). They see and change every source, and
    write queries on each.
  - Each source lists who else writes queries on it (people and groups):
    in Calc they write SQL or pick a saved query, and save queries by name
    for each other.
  - Once a query (or an editable table) is in a file, anyone signed in who
    has the file loads new values for it, with values from their own cells,
    but can't change the query. The server gives whoever put it there a
    ticket for exactly that query on that source, which the file keeps; it
    fits no other SQL. Removing a saved query or the source stops it.
  - An editable table's rows are changed only by people who may edit the
    server file it's in (its ticket is for that file, so a copy can't).
- **Apps & updates**: what Xenify Hub does on the computers signed in here
  (see `docs/hub.md`). Also the server's own update, with no downtime.
- **Backups**: where the daily backup goes, its passphrase, when, how long
  backups are kept, and how long bins keep deleted things (see Backups
  below).
- **Network**: who can open the admin pages, ports, announcing, the browser
  apps, the domain name, HTTPS, the server's name, **Outgoing mail** (the
  mail server it sends from, with a test email), **Virus scanning** (see
  below) and **Chat** (largest file, recording calls, how long messages are
  kept).

## What's stored and who can read it

- The server is trusted: the apps send passwords, names and contents over
  HTTPS, and the server reads them. Whoever runs the server can read
  everything on it.
- Passwords are kept hashed (Argon2id); sessions are kept as hashes of
  their tokens.
- Permissions (spaces, groups, roles) are checked by the server.
- Pictures and clips in Draw canvases are kept apart from the canvas, once
  each, named by their SHA-256: versions and changes only name them, so a
  picture put in costs its size once, not once per change and version.
  The apps send them in 4 MB parts and carry on after a break; they're
  read back whole or by range (video plays while it loads), and only by
  people who may open a canvas that has them. Downloading a canvas puts
  the parts together as one `.xdraw` file; uploading one takes it apart.
- Apps and server speak one protocol version (`xenify-server-proto`'s
  `VERSION`, in `/v1/info`): an app that finds another asks for an update.

## Virus scanning (optional)

Admin › Network › Virus scanning sends every upload to ClamAV's scanner
(clamd) before it's kept: file contents saved from the Files page or an
app, pictures put in documents, previews, and chat files (scanned whole
when the sender finishes). A file with a virus is refused with the name of
what was found, and the server's log says who sent it.

- clamd is a separate program the server talks to over TCP or its unix
  socket (its INSTREAM command): nothing of ClamAV is built into or shipped
  with the server.
- Install it on Ubuntu: `sudo apt install clamav-daemon` (freshclam, which
  keeps its signatures up to date, comes with it). It listens on its unix
  socket, `/run/clamav/clamd.ctl`: give that as the address and add the
  `xenify` user to the `clamav` group, or set `TCPSocket 3310` and
  `TCPAddr 127.0.0.1` in `/etc/clamav/clamd.conf` and give
  `127.0.0.1:3310`. Raise `StreamMaxLength` (25M by default) to the largest
  file people send, or bigger files are refused.
- Test sends the harmless EICAR test file, which a working scanner finds.
  Turning scanning on tests first.
- While it's on and the scanner doesn't answer, nothing can be uploaded
  (nothing gets in unscanned); the setting shows the last problem.
- Limits: it finds known viruses by their signatures, not new ones; files
  encrypted by whoever sends them (a zip with a password, one-time secrets)
  can't be looked into. Documents people only open in the Xenify apps are
  lower risk than files people download and run.

## Encryption at rest (optional, Linux)

The database and people's files can live in a folder the computer encrypts
(fscrypt on ext4): a copy of the disk, a snapshot or a backup of that
folder can't be read. A running, unlocked server reads everything as
usual, and so could whoever controls the running machine. Its own backups
(Admin › Backups) read the files decrypted, so keep their encryption on
(the page warns when it's off).

- Install with it: `install-server.sh --encrypt`. It turns on the disk's
  encryption feature, makes `/var/lib/xenify-server/secure` (the database
  and `blobs/`, `chat-files/` go in it) and prints the passphrase once.
- After every restart the server answers only "This server is locked" until
  someone runs, on its computer:
  `sudo -u xenify /opt/xenify-server/xenify-server --config /etc/xenify-server/config.toml unlock`
  (`--key-file` reads the passphrase from a file). `[at_rest] key_file` in
  the settings file unlocks it at every start instead, which only helps if
  the key file isn't on the same disk.
- An update (Hub or service) doesn't lock it: the key stays in the kernel
  until the computer restarts.
- Later, on a server set up without it: stop the service, then
  `xenify-server at-rest setup --dir <new folder> --move` moves the database
  and files in and prints the settings file's lines. Moving leaves the old
  plain copies' blocks on the disk until they're overwritten.
- Linux 7.0 (Ubuntu 26.04) can crash writing encrypted files with ext4's
  default `dioread_nolock`: the installer mounts the disk with
  `nodioread_nolock` (and adds it to `/etc/fstab`), and the server warns at
  start-up when it's missing.
- `xenify-server at-rest status`: unlocked, locked or off.

## Signing in

- The email address first, then the password (older accounts' user names
  still work). An account
  an admin added that has no password yet asks instead for the code from
  the admin (or "Email me a link", when it has an address and the server
  sends email), then a new password. Names nobody has also go on to the
  password, so this doesn't tell who has an account. An emailed code
  replaces only an earlier emailed one: a code an admin gave out still
  works until it runs out or a code is used (using one ends the others).
- Passwords go to the server over HTTPS and are kept hashed (Argon2id).
- **Two-step sign-in** (each person in their account settings, or required
  by an admin): a code from an authenticator app (QR code to scan), codes
  by email (not for admins: email is also how passwords are reset), and ten
  one-time backup codes. "Remember this computer" skips it there for 30
  days.
- Every app, the Hub, Chat, the admin pages and the Files page handle the
  second step and setting it up.
- In the browser, every page a server serves shares one sign-in (Files,
  the admin pages, Write, Draw, Calc, Plan and Chat there): signing in on
  one signs in on all, and signing out signs out everywhere. The apps
  there don't ask themselves: they send you to Files to sign in, which
  brings you back. The desktop apps follow the Hub's sign-in.

## Email from the server

With Admin › Network › Outgoing mail set (any SMTP server or sending
service), the server sends, in one Xenify-styled template (light and dark,
with a plain-text part):

- invitations, "your password was reset" notes, and codes to pick a
  password;
- codes for two-step sign-in by email;
- a digest of missed chat messages (each person: off, direct messages and
  mentions, or everything; never during Do not disturb or quiet hours).

## Search

The server searches file names and the words in files (each app sends a
plain-text copy when it saves; the server reads Write documents itself,
uploaded ones too, and follows what people type together within seconds)
and chat messages, with permissions applied; results come best first with
the matching words marked. Only a file's name counts from its details. The
apps, the Files page and Chat search through it.

## Files page and public links

Everyone with an account has a Files page at the server's address
(`/files/`): places, spaces and folders, search, upload and download. A
click opens a file (in its app) or a folder; each file and folder has a
menu (⋯, right-click, a long press or the menu key) with Open, Rename,
Move to…, Download, Share…, Version history, Details and Delete. A search
result opens the page with the words found marked.

**My files** and Templates are plain files and folders. A shared space is
a wiki: its **Pages** are Write documents marked as pages, each able to
hold pages of its own (only pages go under a page; deleting one deletes
the pages under it), shown read-only under their name with Edit, Add page
and Copy link; its **Files** (the last entry of its page tree) are
everything else in it. A document in a space's Files can be made a page.
A page's menu can make it the space's **home page** (shown on the space's
own page, with its pages and what changed in it lately) and move it up or
down among the pages beside it; two pages beside each other can't have
the same name. In Write, **Link** on a server document offers the pages
of its space by name. **Deleting** puts things in the space's **Bin**
(Undo right after, or Put back from the Bin later, with what went with
it). Search has its own screen (`#/search/…`, everywhere or in one space),
each result with where it is. Text files and pictures open in a preview.
Write documents
are read as pages the server makes itself (no browser apps needed), showing
what's in the document now, changes made together included; pictures of
Draw canvases on the same server are the canvas's latest picture when the
reader may see the canvas. Calc books are read as their tables and charts,
Draw canvases as their picture. **Edit in Write** starts desktop Write
(the `xenify-write://` link Xenify Hub registers), or Write in the browser
when the server offers the apps.

**Share…** makes a read-only public link to a file or a whole folder (no
account needed). Each link can be stopped or replaced on its own; admins
see every link and can turn them all off on **Admin › Public links**.

## Chat on the server

- **Keeping messages**: forever by default, or for a set time (Admin ›
  Network › Chat); owners of a group or channel may choose a shorter time.
  Older messages and their files go every hour. Calendars, meetings and
  contacts are kept.
- **Settings** (notifications, quiet hours, Do not disturb, previews and the
  like) follow each person between devices; sound devices, start at sign-in
  and email accounts stay on each computer.
- **Reminders** come from the server to every device, with Snooze, Open and
  Dismiss; browsers get them by Web Push even with Chat closed.
- **Secrets** (Share a secret, in a chat or an email): a password or the
  like as a link (`/secret/#…`) that opens once, within 1 to 7 days. It's
  encrypted by the app; the key is only in the link after `#`, which
  browsers never send, so the server keeps what it can't read. Opening the
  page uses nothing up (link previews are safe); **Show secret** does, and
  whoever asks second finds it gone. An optional **passphrase**, given
  another way, is needed as well: the key is made from the link's key and
  the passphrase, so the link alone opens nothing. A wrong passphrase
  doesn't use it up (the server checks a value made from both keys before
  handing it over); five wrong ones delete it.

## Editing together

- Opening a server document joins it live. Changes are ordered by the
  server and rebased in the app.
- People see each other's carets and selections (Write) or selected layers
  (Draw), in each person's colour. Undo only undoes your own changes.
- What's typed is saved as it goes (a few seconds after a pause): readers,
  search and the history see it without Save. The history keeps one
  version for each stretch of someone's editing (a new one after ten
  minutes, or when someone else saves); **Save** always keeps a new
  version. **File › Version history** opens any version as a copy.
- In the browser, the address names the document open (`/write/#open=…`),
  so reloading reopens it and the link can be passed on; **File › Quit**
  goes back to where Files opened it from.
- Offline:
  - Changes wait and are sent on reconnecting.
  - If the app closes first, they're kept on the computer, and the app
    offers to send them after the next sign-in.
  - If someone saved meanwhile, the kept changes open as a copy to compare.

## Linking

Without a server: **Insert › Picture…** in Write accepts a Draw canvas
(`.xdraw`).
- It's shown from its file and follows it. Draw keeps a picture of the canvas
  in the file each time it saves (an `.xdraw` file is a zip: the canvas,
  each picture and clip once, and that picture).
- An animated canvas gets a play button when selected. Write asks Xenify
  Draw to make a short clip (`xenify-draw --export-video`, no window), keeps
  it until the canvas changes, and plays it in place.
- Print, PDF and Word show the still picture.

With a server:

- **Insert › From a server…** in Write (or `/server`) takes a Draw canvas, a
  Calc book's table, summary or chart, or a plan's issues, milestones or
  timeline. The picker shows only those files. A canvas goes in as a picture.
- Draw puts a picture of the canvas (its first frame) on the server each time it saves.
- Write refreshes the picture when the document is focused and every 20
  seconds. The last picture is saved in the document, so it shows offline too.
- Right-click the picture and choose **Keep this copy** to stop it updating.
- The other way round, **Insert › Picture from a server…** in Draw shows the
  first page of a Write document. Write puts a picture of page 1 on
  the server each time it saves. Draw refreshes it the same way; the
  picture's layers keep their place and size.

## AI

- The apps send AI requests to `/v1/ai/*` with the person's session. They need no
  key of their own, and Settings in each app chooses between this server's AI
  and a personal OpenRouter key.
- Write's Ask AI and Draw's assistant, illustrations and **AI › New video
  clip** use the models the admin offers.
- When the runner answers with a link to a made picture or clip on itself
  (as LocalAI does), the server fetches the file and sends it to the app.
- Requests can be read by whoever runs the server and the runner.

## Updating and backups

- **Updates:** the service (above), or Xenify Hub on the server's computer
  when it runs the server, installs new versions:
  1. The new server starts beside the old one on the same port (`SO_REUSEPORT`).
  2. The old one finishes its requests, tells live documents to reconnect, and stops.
  3. Database changes only add, so both versions can run at once (the
     protocol 2 release, which removed encryption in the apps, is the
     exception: it drops old columns).
- **Backups:** Admin › Backups (below).

## Backups

Admin › Backups backs up the server every day at a set hour (02:00 unless
changed). Each backup is the database (a `pg_dump`) and the data folder
(and the encrypted folder, with encryption at rest), taken together so they
match. Code: `src/backup/` (the run, admin API, CLI helpers) and
`src/backup/store.rs` (the places).

- **Where:** a folder on this computer (a second disk, a USB disk, a share
  the system mounted), S3-compatible storage (AWS, Backblaze B2, Cloudflare
  R2, Wasabi, MinIO…: address, bucket[/folder], access key, secret key; the
  region is read from the address), or an SFTP server (`user@host[:port]`
  and a folder; signs in with the server's own ed25519 key, shown on the
  page for `authorized_keys`, or a password). Check tries the place; for
  SFTP it trusts the server's key the first time and refuses a different
  one later.
- **Format:** restic's repository format (rustic_core), so `restic` can list
  and restore them too. Every backup is complete; files are cut into
  pieces named by their hash, and pieces already there aren't sent again.
  The dump is uncompressed (`--compress=0`) so unchanged tables aren't
  stored again. Left out: the browser apps (`data/web`), unfinished Draw
  uploads, and any PostgreSQL data folder.
- **Encryption:** on unless turned off, with a passphrase (typed, or Make
  one: six words). It can't be changed for backups that exist. Losing it
  loses the backups. Off means restic's empty passphrase.
- **Keeping:** 7 daily, 4 weekly and 6 monthly unless changed; unused pieces
  are removed about once a week (prune). An optional upload speed limit
  (Mbit/s).
- **Big files:** read in place and sent in packs of a few MB, never whole.
  The index is written every 5 minutes, and after a backup that stopped
  partway the next one first lists what already arrived (`repair index`),
  so little is sent twice.
- **Failures:** shown on the page and emailed to admins with an address;
  tried again the next day (or Back up now).
- **Restoring** (the server stopped):

      xenify-server backup list
      xenify-server backup restore latest

  On a new computer, install the server (and its database: `database
  setup` or an empty PostgreSQL database), then add `--from PLACE` (a
  folder, `s3:https://host/bucket[/folder]` or
  `sftp:user@host[:port]/folder`). The passphrase comes from
  `XENIFY_BACKUP_PASSPHRASE` or is asked for; S3 keys from
  `AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`; SFTP signs in with
  `--ssh-key FILE` or `XENIFY_SFTP_PASSWORD`. Files go back into the data
  folder (files not in the backup are left; the clean-up removes unused
  ones), then `pg_restore --clean` replaces the database. `backup now`
  backs up from the command line.
- **pg_dump/pg_restore:** beside the server's own PostgreSQL, else the
  newest under /usr/lib/postgresql, else PATH.

## Deleted files

A space's bin keeps deleted things for 30 days (Admin › Backups › Deleted
files: 7 days to a year, or until someone empties it); then they're deleted
for good. Once an hour (`src/cleanup.rs`) stored bytes no version, picture,
Draw media or preview uses any more are removed, a day after they were last
written or used, and Draw uploads nobody carried on with for a week.

## Development

- Tests: `cargo test -p xenify-server`.
- The tests need a PostgreSQL to use. They skip themselves without one; see
  `tests/common/mod.rs` for the address.
- `cargo test -p xenify-server --test server web::screens -- --ignored --nocapture`
  keeps two servers running (one to set up, one with people and links) for
  looking at the admin pages.
