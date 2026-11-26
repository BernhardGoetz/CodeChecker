#!/usr/bin/env bash
# Lädt das KI-Modell der App (Qwen2.5-Coder-7B-Instruct, Q4_K_M) zur im
# Katalog gepinnten Revision von Hugging Face und prüft die SHA-256 gegen
# src-tauri/src/ai/catalog.json.
#
#   scripts/ki-bench/fetch-model.sh <Zielordner>
#
# Der Download ist fortsetzbar (curl -C -): ein abgebrochener Lauf wird beim
# nächsten Aufruf an derselben Stelle weitergeführt.
set -euo pipefail

dest="${1:-}"
if [ -z "$dest" ]; then
  echo "Aufruf: $0 <Zielordner>" >&2
  exit 2
fi
command -v jq >/dev/null || {
  echo "jq wird benötigt." >&2
  exit 1
}

root="$(cd "$(dirname "$0")/../.." && pwd)"
catalog="$root/src-tauri/src/ai/catalog.json"
file="qwen2.5-coder-7b-instruct-q4_k_m.gguf"
repo="Qwen/Qwen2.5-Coder-7B-Instruct-GGUF"

entry="$(jq -e --arg repo "$repo" --arg file "$file" \
  '.models[] | select(.repo == $repo) | {revision, f: (.files[] | select(.name == $file))}' \
  "$catalog")"
revision="$(jq -r '.revision' <<<"$entry")"
sha256="$(jq -r '.f.sha256' <<<"$entry")"
size="$(jq -r '.f.size' <<<"$entry")"
if ! [[ "$revision" =~ ^[0-9a-f]{40}$ ]] || ! [[ "$sha256" =~ ^[0-9a-f]{64}$ ]]; then
  echo "Katalogeintrag für $file ist nicht gepinnt." >&2
  exit 1
fi

sha256_of() {
  if command -v sha256sum >/dev/null; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

mkdir -p "$dest"
target="$dest/$file"
part="$target.part"
if [ -f "$target" ]; then
  echo "$target existiert, prüfe SHA-256 …"
else
  url="https://huggingface.co/$repo/resolve/$revision/$file"
  echo "Lade $file (Revision $revision, $size Bytes) …"
  curl -fL --proto '=https' --tlsv1.2 --retry 10 --retry-delay 5 --retry-all-errors \
    -C - -o "$part" "$url"
  mv "$part" "$target"
fi

actual="$(sha256_of "$target")"
if [ "$actual" != "$sha256" ]; then
  echo "SHA-256 stimmt nicht: erwartet $sha256, erhalten $actual" >&2
  mv "$target" "$target.bad"
  exit 1
fi
echo "OK: $target ($sha256)"
