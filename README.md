# Xenify

Free office apps that work together: documents, spreadsheets, plans,
drawings, chat and email. Use them on your own computer, or run your own
Xenify server so a whole team shares everything in one place. No
subscription, no account, and nothing is sent to us.

Website and downloads: <https://xenify.net> ·
Release files: <https://github.com/mr-bubble-pop/xenify/releases>

## The apps

| App | What it does |
|---|---|
| **Write** | Documents with spelling and grammar checked on your own computer. Opens and saves .docx and Markdown, exports PDF. |
| **Calc** | Spreadsheets that can stay linked to a database or a CSV file. Opens and saves .xlsx. |
| **Plan** | Tasks on a board, who's doing what, and a timeline that works out the dates. |
| **Draw** | Pictures, posters, slides and animations; export a video, a GIF or a picture. |
| **Chat** | Chat, email and calendars in one window, with calls, meetings and room booking. |
| **Hub** | Installs the apps you pick and keeps them up to date. |
| **Server** | Self-hosted: a shared, searchable wiki of everyone's work, editing together, version history, chat, email, calendars, and the apps in any browser. |

AI is optional and off until you turn it on. You choose where it runs: on
your own computer or server, or with an online provider you pick.

## Getting started

Most people should install **Xenify Hub** from <https://xenify.net> and pick
the apps they want there. To run a server for a team, see the server page on
the site, or `docs/server.md`.

## Building from source

Xenify is written in Rust, with HTML/JavaScript screens shown through
[Tauri](https://tauri.app). The dev container in `.devcontainer/` has every
tool the builds use; `scripts/check-dev-tools.sh` lists anything missing.

```sh
scripts/check.sh                  # formatting, lints, tests, browser builds
scripts/package-write.sh linux    # one app's Linux packages (also -calc, -plan, -draw, -chat, -hub)
scripts/build-site.sh             # the website and the browser versions of the apps
```

The repository is a Cargo workspace: each app and library is a crate in
`crates/`. Write's mistake-spotter model (67 MB) is not kept in the
repository; `scripts/stage-spotter.sh` says where it is looked for.
`patches/` holds small replacements and changed copies of dependencies;
`patches/README.md` explains each one.

## How Xenify was made

Xenify was written entirely by AI: Anthropic's Claude, working through
Claude Code from the maintainer's directions, feedback and testing. The
maintainer has not reviewed the source code line by line, and there is far
more of it than one person could check well enough to vouch for its
security or correctness.

The code has been through several AI-led security reviews, and the
problems they found have been fixed. That lowers the risk, but it does not
remove it. As with software written by people, there may be bugs or
security weaknesses that no review has found.

What this means for you:

- Xenify is provided **"as is", without warranty of any kind**, as set out
  in the [licence](LICENSE). To the fullest extent the law allows, the
  maintainer and contributors accept no liability for any loss or damage
  arising from its use.
- Before you trust it with sensitive or important data, or run a Xenify
  server that other people can reach, make your own assessment: review and
  test the code that matters to you, keep backups, and limit what the
  server can access.
- If you find a security problem, please report it privately as described
  in [SECURITY.md](SECURITY.md).

## Licence

Xenify is free software under the [MIT licence](LICENSE). To the extent any
part of it is protected by copyright, it is licensed under those terms.

Xenify uses only permissively licensed dependencies and assets. Their
licences and credits are in [THIRD_PARTY_LICENSES.txt](THIRD_PARTY_LICENSES.txt),
which ships with every app. The source
and licence of every bundled or downloaded asset (fonts, icons, emoji,
dictionary data, models, website media) is recorded in
[assets/SOURCES.toml](assets/SOURCES.toml); `scripts/licenses.sh`
regenerates the licences list.

One exception: four sample clips in `site/media/` were made with the
MiniMax H3 model and come under its licence, which excludes the US, the EU,
the UK and South Korea. See
[site/media/MINIMAX-H3-NOTICE.md](site/media/MINIMAX-H3-NOTICE.md) before
using or sharing them.

Optional AI models that you download yourself (from Hugging Face, Civitai
or elsewhere) come under their own licences, which are yours to check and
follow.

## Trademarks

Xenify is an independent project, not affiliated with or endorsed by any
company named in this repository. Product and company names belong to their
owners and are used only to describe what Xenify works with or how it was
made.
