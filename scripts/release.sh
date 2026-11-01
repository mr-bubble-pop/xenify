#!/usr/bin/env bash
# Adds built packages to Xenify's release list and signs it, for Xenify Hub.
#
#   scripts/release.sh <channel> <base url> <packages folder> [list.json]
#
# Each package in the folder is named <product>-<version>-<os>-<arch>.<ext>
# (e.g. write-1.2.0-linux-x86_64.tar.gz, server-0.3.0-windows-x86_64.zip),
# uploaded beside each other at <base url> (a GitHub release's download
# address). The list (default target/releases/list.json) keeps earlier
# releases; the signed copy goes to manifest.json beside it, which is
# published at https://xenify.net/releases/manifest.json.
#
# The release key: XENIFY_RELEASE_KEY = a file holding it (make one with
# `xenify-release keygen`; keep it only in a CI secret or offline, never in
# the project folder; its public half goes in xenify_hub_core::TRUSTED).
#
# The signed list never runs out. XENIFY_MIN_SAFE="server=0.1.2 write=0.1.1" names each
# product's oldest safe version: Hubs never install an older one, whatever
# a server's policy says. Set it again on every run (it isn't kept).
set -euo pipefail
cd "$(dirname "$0")/.."

channel=${1:?channel: release, preview or nightly}
base=${2:?base url of the uploaded packages}
dir=${3:?folder of packages}
list=${4:-target/releases/list.json}
key=${XENIFY_RELEASE_KEY:?set XENIFY_RELEASE_KEY to the release key file}

cargo build --quiet -p xenify-hub-core --bin xenify-release
tool=target/debug/xenify-release
mkdir -p "$(dirname "$list")"

declare -A names=([write]=Write [draw]=Draw [calc]=Calc [chat]=Chat [plan]=Plan [server]=Server [hub]=Hub [web]="Apps for the browser")
declare -A runs=([write]=xenify-write [draw]=xenify-draw [calc]=xenify-calc [chat]=xenify-chat [plan]=xenify-plan [server]=xenify-server [hub]=xenify-hub)
# What the Hub shows for a product, and the parts it installs with it
# (Plan keeps its plans on a Xenify server, so the Hub sets one up too).
declare -A abouts=([plan]="Issues, boards and timelines, kept on a Xenify server" [server]="Your own server for files, chat and plans")
declare -A requires=([plan]="server,postgres" [server]="postgres")

added=0
for f in "$dir"/*; do
  name=$(basename "$f")
  [[ $name == *.run ]] && continue
  # product-version-os-arch.ext (the version may hold dashes: 1.3.0-beta.1).
  # (os and arch "any": the same for every computer, like the browser apps.)
  if [[ $name =~ ^([a-z]+)-(.+)-(linux|windows|macos|any)-(x86_64|aarch64|any)\.(tar\.gz|tgz|zip|exe|AppImage)$ ]]; then
    product=${BASH_REMATCH[1]} version=${BASH_REMATCH[2]} os=${BASH_REMATCH[3]} arch=${BASH_REMATCH[4]} ext=${BASH_REMATCH[5]}
    run=${runs[$product]:-$product}
    [[ $os == windows ]] && run="$run.exe"
    # A single program (not an archive) is installed under its own name.
    [[ $ext == exe || $ext == AppImage ]] && run=$name
    # scripts/hub-packages.sh says which program inside to start.
    [[ -f $f.run ]] && run=$(head -n1 "$f.run")
    "$tool" add "$list" "$product" "${names[$product]:-$product}" "$version" "$channel" "$os" "$arch" "$f" "${base%/}/$name" "$run"
    echo "added $name"
    added=$((added + 1))
  else
    echo "skipped $name (not <product>-<version>-<os>-<arch>.<ext>)" >&2
  fi
done
[[ $added -gt 0 ]] || { echo "no packages found in $dir" >&2; exit 1; }
in_list() { grep -q "\"id\": *\"$1\"" "$list"; }
for product in "${!requires[@]}"; do
  in_list "$product" || continue
  # Only parts the list has (the Hub can't install one it can't find).
  have=()
  IFS=, read -ra want <<< "${requires[$product]}"
  for r in "${want[@]}"; do in_list "$r" && have+=("$r"); done
  "$tool" describe "$list" "$product" "${names[$product]}" "${abouts[$product]}" "$(IFS=,; echo "${have[*]}")"
done
sign_opts=()
for floor in ${XENIFY_MIN_SAFE:-}; do sign_opts+=(--min-safe "$floor"); done
"$tool" sign "$list" "$key" "$(dirname "$list")/manifest.json" ${sign_opts[@]+"${sign_opts[@]}"}
echo "signed: $(dirname "$list")/manifest.json"
