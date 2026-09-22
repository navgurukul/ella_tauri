#!/usr/bin/env bash
# Fills src-tauri/resources/engines with the Apple Silicon binaries a macOS
# bundle has to carry, fetching each from its upstream release. The macOS
# counterpart of fetch-windows-engines.ps1; weights are not staged here either,
# the app downloads them into app data on first launch.
#
# Piper is not a standalone binary here. A development Mac runs it from a venv
# whose Python is a symlink into Homebrew, which no other machine has, so the
# bundle carries a relocatable CPython (python-build-standalone) with piper-tts
# installed into it, laid out as piper-venv/ so the app finds it exactly where
# it finds a developer's venv. Because python3 then sits beside piper, the app
# runs the resident Piper daemon rather than reloading the voice every turn.
#
#   ./scripts/fetch-macos-engines.sh [destination]
#
# ELLA_PIPER_VOICE_URL   private URL for en_IN-navgurukul-medium.onnx; its
#                        .onnx.json sidecar is expected at the same URL + .json
# ELLA_LLAMA_TAG         pin a llama.cpp release (default: newest with a build)
# ELLA_PYTHON_TAG        pin a python-build-standalone release (default: latest)
# ELLA_PIPER_TTS_VERSION piper-tts to install (default: 1.8.0)
# GITHUB_TOKEN           raises the GitHub API rate limit when set
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="${1:-$ROOT/src-tauri/resources/engines}"
PIPER_TTS_VERSION="${ELLA_PIPER_TTS_VERSION:-1.8.0}"

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]]; then
  echo "This stages Apple Silicon engines and must run on an arm64 Mac: pip picks wheels for the host." >&2
  exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# curl retries 429s and 5xx with backoff and honours Retry-After, which matters
# on the shared Actions IP pool that Hugging Face and GitHub both rate-limit.
fetch() {
  curl -fsSL --retry 5 --retry-all-errors "$@"
}

github_api() {
  if [[ -n "${GITHUB_TOKEN:-}" ]]; then
    fetch -H "Authorization: Bearer $GITHUB_TOKEN" -H "User-Agent: ella-build" "https://api.github.com/$1"
  else
    fetch -H "User-Agent: ella-build" "https://api.github.com/$1"
  fi
}

# Prints the download URL of the first asset matching a regex, walking releases
# newest first. Not /releases/latest for llama.cpp: it marks every binary build
# a prerelease, so "latest" can be a release with no macOS asset at all.
release_asset_url() {
  local repo="$1" pattern="$2" tag="${3:-}" path
  if [[ -n "$tag" ]]; then path="repos/$repo/releases/tags/$tag"; else path="repos/$repo/releases?per_page=30"; fi
  github_api "$path" | /usr/bin/python3 -c '
import json, re, sys
data = json.load(sys.stdin)
for release in (data if isinstance(data, list) else [data]):
    for asset in release.get("assets", []):
        if re.search(sys.argv[1], asset["name"]):
            print(release["tag_name"], asset["name"], asset["browser_download_url"], file=sys.stderr)
            print(asset["browser_download_url"])
            sys.exit(0)
sys.exit(f"No release of {sys.argv[2]} carries an asset matching /{sys.argv[1]}/")
' "$pattern" "$repo"
}

# Extracts a tarball into a directory, flattening a single wrapper directory so
# llama-server always lands directly in bin/llama/. cp -R keeps the dylib
# symlinks as symlinks.
expand_into() {
  local archive="$1" target="$2" staging="$WORK/expand.$RANDOM"
  mkdir -p "$staging" "$target"
  tar -xzf "$archive" -C "$staging"
  local entries=("$staging"/*)
  local root="$staging"
  if [[ ${#entries[@]} -eq 1 && -d "${entries[0]}" ]]; then root="${entries[0]}"; fi
  cp -R "$root"/. "$target"/
  rm -rf "$staging"
}

mkdir -p "$DEST"

# --- llama.cpp -------------------------------------------------------------
# Its dylibs carry an @loader_path rpath, so they only need to sit beside the
# binary; the app also points DYLD_LIBRARY_PATH at this directory.
url="$(release_asset_url ggml-org/llama.cpp '^llama-.*-bin-macos-arm64\.tar\.gz$' "${ELLA_LLAMA_TAG:-}")"
fetch -o "$WORK/llama.tar.gz" "$url"
rm -rf "$DEST/bin/llama"
expand_into "$WORK/llama.tar.gz" "$DEST/bin/llama"

# --- Piper, on a relocatable Python ----------------------------------------
url="$(release_asset_url astral-sh/python-build-standalone \
  '^cpython-3\.12\.[0-9]+\+[0-9]+-aarch64-apple-darwin-install_only_stripped\.tar\.gz$' \
  "${ELLA_PYTHON_TAG:-}")"
fetch -o "$WORK/python.tar.gz" "$url"
rm -rf "$DEST/piper-venv"
expand_into "$WORK/python.tar.gz" "$DEST/piper-venv"

"$DEST/piper-venv/bin/python3" -m pip install --quiet --no-cache-dir --disable-pip-version-check \
  "piper-tts==$PIPER_TTS_VERSION"

# pip writes console scripts with an absolute shebang pointing at this build
# directory, which does not exist once the bundle is installed elsewhere. The
# app runs piper-venv/bin/piper on its one-shot path, so it has to resolve its
# own Python relative to where it actually is.
cat > "$DEST/piper-venv/bin/piper" <<'SH'
#!/bin/sh
exec "$(dirname "$0")/python3" -m piper "$@"
SH
chmod +x "$DEST/piper-venv/bin/piper"

# --- The voice that ships with the app -------------------------------------
TTS="$DEST/models/tts"
mkdir -p "$TTS"
if [[ -n "${ELLA_PIPER_VOICE_URL:-}" ]]; then
  echo "Staging the NavGurukul voice from the configured URL"
  fetch -o "$TTS/en_IN-navgurukul-medium.onnx" "$ELLA_PIPER_VOICE_URL"
  fetch -o "$TTS/en_IN-navgurukul-medium.onnx.json" "$ELLA_PIPER_VOICE_URL.json"
else
  echo "warning: ELLA_PIPER_VOICE_URL is not set - staging the public en_US voice instead. Ella will not sound Indian in this build." >&2
fi
# The stock voice is staged either way, so a bad or half-written custom voice
# still leaves the app able to speak.
base="https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx"
fetch -o "$TTS/en_US-lessac-medium.onnx" "$base"
fetch -o "$TTS/en_US-lessac-medium.onnx.json" "$base.json"

# --- What the app will look for at runtime ---------------------------------
for relative in bin/llama/llama-server piper-venv/bin/python3 piper-venv/bin/piper \
                models/tts/en_US-lessac-medium.onnx models/tts/en_US-lessac-medium.onnx.json; do
  if [[ ! -f "$DEST/$relative" ]]; then
    echo "Staging finished but $relative is missing. The upstream archive layout has probably changed." >&2
    exit 1
  fi
done

# Prove the staged copies actually run, not only that the files exist.
"$DEST/bin/llama/llama-server" --version >/dev/null 2>&1 \
  || { echo "The staged llama-server does not run." >&2; exit 1; }
"$DEST/piper-venv/bin/python3" -c "import piper" \
  || { echo "The staged Python cannot import piper." >&2; exit 1; }

echo "Staged macOS engines at $DEST ($(du -sh "$DEST" | cut -f1))"
