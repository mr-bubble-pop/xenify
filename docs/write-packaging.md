# Packaging Xenify Write

Xenify Write (`crates/xenify-write-app`) ships for Linux, Windows, macOS and
the browser. Every build has the spelling and grammar checker, the word
Look up and the mistake-spotter model (B7) built in, so nothing else needs
installing.

| Build | Command | Output (`dist-write/`, or `target/write-web/`) | Size |
|---|---|---|---|
| Linux | `scripts/package-write.sh linux` | `.deb`, `.rpm`, `.AppImage`, `.tar.gz` | ~95 MB (AppImage 194 MB) |
| Windows | `scripts/package-write.sh windows` | `xenify-write_<v>_x64-setup.exe` | ~94 MB |
| macOS | `scripts/package-write.sh macos` (on a Mac) | `.dmg` (Apple silicon and Intel) | — |
| Browser | `scripts/build-write-web.sh` | a folder for any static host | 2.2 MB before the first page |

All of them need the model staged first. `scripts/stage-spotter.sh` does
that, and both scripts call it. It takes the model from `$SPOTTER_DIR`, the
installed copy (`~/.local/share/xenify/spotter`),
`checkpoints/models/B7.tgz`, or `$SPOTTER_URL`. Packaging needs Tauri's CLI:
`cargo install tauri-cli --version '^2' --locked`.

The installed program is `xenify-write`. With no arguments, or with files,
it opens the editor. It also has the command-line tools:

```
xenify-write check report.md            # spelling, grammar, readability (uses the model)
xenify-write convert in.docx out.pdf
xenify-write merge letter.md --data people.csv --to pdf
xenify-write profiles
```

## Where each system is built

- **Linux:** built here. Packages built on this Debian 13 container need
  glibc 2.39 (Ubuntu 24.04+, Debian 13, Fedora 40+). For older systems,
  build on Ubuntu 22.04: the GitHub workflow does this.
- **Windows:** cross-built here with MinGW (`gcc-mingw-w64-x86-64`, `nsis`,
  `rustup target add x86_64-pc-windows-gnu`). On Windows itself it builds
  with MSVC. The installer brings Microsoft's WebView2 runtime if it's
  missing (Windows 10/11 normally have it), and installs for the current
  user with no admin prompt. Checked here: the command line, checker and
  model under Wine. The window itself needs a real Windows.
- **macOS:** needs a Mac (Apple's SDK can't legally be used elsewhere).
  Run the script on a Mac, or let the Release workflow's macos-14 job
  build it.
- **All at once:** the Release workflow (`.github/workflows/release.yml`,
  started by `scripts/start-build.sh release`) builds every app for Linux,
  Windows and macOS, plus the browser versions and the Hub installers, and
  publishes them as releases of this repository (see docs/hub.md). It takes the
  model `B7.tgz` from this repo's `models` branch (checked against its
  `SHA256SUMS`). Its Website workflow (`scripts/start-build.sh site`) puts
  xenify.net on Cloudflare Pages (project `xenify`; the secrets
  `CLOUDFLARE_API_TOKEN`, with only *Cloudflare Pages: Edit*, and
  `CLOUDFLARE_ACCOUNT_ID`, on this repository).

## xenify.net

`site/` is the marketing site, in the apps' light look:
- `/`: home
- `/products/write/`, `/products/draw/`, `/products/calc/` and `/products/server/`: one page per product
- `/download/`: Xenify Hub only; the apps aren't offered separately

`scripts/build-site.sh` puts it together with the browser versions (`/write/`,
`/draw/` and `/calc/`, linked as "Use it in your browser") in `target/site`, without the
`.br`/`.gz` copies, and moves each app's caching rules in `_headers` under its
folder. Deploy by hand with `npx wrangler pages deploy target/site
--project-name xenify`, or drag `target/site` into Cloudflare's *Upload
assets*.

The site's pictures and videos (`site/media`) are recordings of the real
apps; their sources are in assets/SOURCES.toml. Videos loop silently, and
pause with controls when the visitor asks for reduced motion (`site/site.js`).
While the repository is private, `<body class="soon">` hides the download
buttons and GitHub links on every page.

## Signing: none (decided 2026-10-20)

Packages are released unsigned.

- **macOS:** the app gets Apple's free "ad-hoc" signature
  (`signingIdentity: "-"`), without which Apple-silicon Macs won't run it.
  It isn't notarised, so the first time, macOS says it can't check the app.
  People open it with right-click › Open; on macOS 15 and later, they go to
  System Settings › Privacy & Security › Open Anyway. Or they run
  `xattr -dr com.apple.quarantine "/Applications/Xenify Write.app"`. Put
  this in the download page's instructions.
- **Windows:** SmartScreen says "Windows protected your PC" until enough
  people have run it; More info › Run anyway.
- **Linux:** nothing needed.

To sign later: the workflow already passes the `APPLE_*` secrets to Tauri.
For Windows, set `bundle.windows.certificateThumbprint` or `signCommand`.

## The browser version

### What loads when

1. **First page** (2.2 MB brotli): `index.html`, the screens, and the
   document engine (`pkg/xenify_write_web_bg.wasm`). The maths, CMU Serif
   and emoji fonts and the whole checker are left out of this download.
   A blank sheet shows until the engine draws the page.
2. **Fonts** (`fonts.bin`, 1.8 MB): handed to the engine right after the
   first page. Pages are drawn again with them.
3. **Checker** (`checker/xenify_write_check_web_bg.wasm`, 9.2 MB): Harper's
   rules, the word list for Look up, and the spotter code, in a Web Worker
   so typing never waits for it.
4. **Mistake spotter** (`checker/spotter/`, 62 MB in 16 MB parts): downloaded once,
   with progress shown in Settings › Spelling and grammar.

On a 20 Mbit/s line the first page shows after about 1.3 s. The rest
arrives in the background, about 30 s for everything on a first visit.

### Offline, and updates

Every file is kept in the browser's Cache Storage under its content hash
(`offline.js`). The service worker (`sw.js`) answers from that copy, so
Write opens and checks with no connection after the first visit. A later
visit downloads only files whose hash changed. A new version downloads in
the background, then the status bar shows **Update ready: restart Write**.
The new version only takes over when that is clicked, so a page never
mixes files from two versions. Settings › General › Working offline shows
how much is kept, and has **Free up space**. Write asks the browser to keep
the copy persistently (`navigator.storage.persist`). It can be installed as
an app (web app manifest).

### The checker is shared with Draw

The checker and the model are built once by `scripts/build-checker-web.sh`
into `target/checker-web`, the same folder Xenify Draw's browser version
uses. A standalone build copies it into the app as `checker/`. On
xenify.net (`scripts/build-site.sh`) it's served once at `/shared/checker/`,
and both apps' workers import `../shared/checker/kept.js` (the build sets
`CHECKER_AT`).

`kept.js` keeps those files in a cache of their own, `xenify-checker`, by
content hash, so whichever app is opened first downloads them and the
other downloads none of it. Each app's `sw.js` knows the checker's files
(`files.json`, written into it at build time) and serves them offline from
that cache. Each app writes a note into the cache listing the files its
version needs (`<app>/.xenify-checker-needs`), and a file is deleted only
when no note needs it any more, so updating one app never breaks the
other's offline copy. **Free up space** in Write removes Write's note and
its own files; the checker stays while Draw still needs it. Versions of
Write that kept the checker with their own files hand over any part that
hasn't changed when they update, so the model isn't downloaded again.

### Hosting

Any static host works. Service workers need **https** (or `localhost`).

- **Types:** `.wasm` must be served as `application/wasm` (for streaming
  compilation), `.js` as `text/javascript`, and `.webmanifest` as
  `application/manifest+json`.
- **Compression:** every file has `.br` and `.gz` copies next to it. Serve
  them with `Content-Encoding` when the browser accepts them.
  - nginx: `brotli_static on; gzip_static on;`
  - Caddy: `file_server { precompressed br gzip }`
  - Netlify and Cloudflare Pages compress by themselves.
- **Caching:** the `_headers` file (read by Netlify and Cloudflare Pages)
  marks the hashed files (`*.wasm`, `fonts.bin`, the model parts) as
  immutable, and everything else as `no-cache`. Mirror that on other hosts.
- **File sizes:** no file is over 25 MB (Cloudflare Pages' limit). The
  model is split for that reason.
- **No special headers needed:** COOP/COEP aren't used, since there are no
  threads.

To try it locally: `scripts/serve-write-web.py` serves `target/write-web`
the same way, at http://localhost:8080/. Add `?debug` to the address to
print loading steps and errors to its log.

### Differences from the desktop

- The mistake spotter takes about 0.6 s a paragraph in the browser, against
  about 25 ms natively: a browser worker has no threads. Wasm SIMD was
  tried and made no difference. So rule underlines show at once, and the
  model's findings follow paragraph by paragraph.
- AI help is hidden: a key kept in the browser needs a decision first.
- Only the bundled fonts (Inter, CMU Serif, STIX Two Math) are available,
  not the computer's own fonts.
- Kinds of writing and templates you make are not kept between visits yet
  (they are files on the desktop).

## Licences

- Everything built in is listed in `THIRD_PARTY_LICENSES.txt`, which ships
  with every package (regenerate it with `scripts/licenses.sh`).
- The model is released under MIT (decided 2026-10-20). Its credits ship
  next to it (`spotter/README.md`): DeBERTa-v3 is MIT, and the training
  data is partly derived from Wikipedia edits (CC BY-SA).
  See `docs/grammar-checker.md` and `assets/SOURCES.toml`.
- Tauri's build-time MPL-2.0 crates are not shipped (see `patches/README.md`).

## The dev container

`.devcontainer/Dockerfile` installs everything these scripts use. After
rebuilding the container, run `scripts/check-dev-tools.sh` to confirm. The
model isn't in the image or the repo. Packages take it from
`checkpoints/models/B7.tgz`, which is kept in the project folder. To have the
app you run from source use it too:

```sh
mkdir -p ~/.local/share/xenify && tar -xzf checkpoints/models/B7.tgz -C /tmp && mv /tmp/B7 ~/.local/share/xenify/spotter
```
