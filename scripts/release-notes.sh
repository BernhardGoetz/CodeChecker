#!/usr/bin/env bash
# Schreibt die Release-Notizen (Markdown) nach stdout (Release-Workflow).
#
#   scripts/release-notes.sh <qas|release> <katalog.json>
#
# Umgebung: VERSION, REGISTRY, GH_REPO, IMAGE_TAG (Tag der Runner-Images),
# WINDOWS_NOTE (Signaturstatus Windows), QAS_TAG (nur release: getesteter
# QAS-Stand). Steht in CHANGELOG.md ein Abschnitt "## [<VERSION>]", wird er
# unter "Änderungen" übernommen (sonst entfällt der Teil).
set -euo pipefail

kind="${1:?Aufruf: $0 <qas|release> <katalog.json>}"
catalog="${2:?Aufruf: $0 <qas|release> <katalog.json>}"
: "${VERSION:?}" "${REGISTRY:?}" "${GH_REPO:?}" "${IMAGE_TAG:?}" "${WINDOWS_NOTE:?}"

if [ "$kind" = "qas" ]; then
  cat <<NOTES
**QAS – nicht für Produktion.** Testfassung von Code-Prüfer ${VERSION} zur Abnahme
(Pre-Release). Nach der Abnahme entsteht daraus das Release v${VERSION}.

NOTES
else
  cat <<NOTES
Code-Prüfer ${VERSION}

Abgenommen als QAS-Stand \`${QAS_TAG:-?}\`; Windows-Programmdateien und macOS-DMGs
stammen aus genau diesem Build (Windows nur neu signiert, siehe docs/RELEASE_QAS.md).

NOTES
fi
cat <<NOTES
- Windows x64: \`*_x64-setup.exe\` (${WINDOWS_NOTE})
- macOS Apple Silicon: \`*_aarch64.dmg\`, macOS Intel: \`*_x64.dmg\`
- Prüfsummen: \`SHA256SUMS.txt\` (\`sha256sum -c SHA256SUMS.txt\`)
- Code signing policy: https://github.com/${GH_REPO}/blob/main/docs/CODE_SIGNING_POLICY.md
- Der Code-Prüfer prüft Programmieraufgaben (Python, Java) lokal gegen Testfälle, in abgeschotteten Docker-/Podman-Containern ohne Netzwerk. Anleitung: README.
NOTES
if [ "$kind" = "release" ]; then
  echo "- Herkunft: \`gh attestation verify <Datei> --repo ${GH_REPO}\`"
fi
# Abschnitt der Version aus CHANGELOG.md (ohne Überschrift, ohne Leerzeilen
# am Anfang/Ende); index() statt Regex, damit Punkte in der Version wörtlich gelten.
changelog="$(dirname "$0")/../CHANGELOG.md"
changes=""
if [ -f "$changelog" ]; then
  changes="$(awk -v head="## [${VERSION}]" '
    index($0, head) == 1 { found = 1; next }
    found && /^## / { exit }
    found { lines[++n] = $0 }
    END {
      first = 1; while (first <= n && lines[first] ~ /^[[:space:]]*$/) first++
      last = n; while (last >= first && lines[last] ~ /^[[:space:]]*$/) last--
      for (i = first; i <= last; i++) print lines[i]
    }' "$changelog")"
fi
if [ -n "$changes" ]; then
  printf '\n## Änderungen in %s\n\n%s\n' "$VERSION" "$changes"
fi
cat <<NOTES

Die Runner-Images (Python, Java) sind in den Installern enthalten und werden
beim ersten Start automatisch in Docker bzw. Podman geladen. Die KI-Laufzeit
(llama.cpp) ist enthalten, das KI-Modell nicht: es wird in der App einmalig
von huggingface.co geladen oder als .gguf-Datei importiert
(Lizenzen: THIRD_PARTY_NOTICES.txt). Online-Update:
\`${REGISTRY}/code-pruefer-python:${IMAGE_TAG}\`
und \`${REGISTRY}/code-pruefer-java:${IMAGE_TAG}\`.

KI-Modelle zum Download in der App (Prüfsumme wird nach dem Download geprüft):

| Modell | Repository | Revision | Datei | Größe (Bytes) | SHA-256 |
|---|---|---|---|---|---|
NOTES
jq -r '.models[] as $m | $m.files[]
  | "| \($m.id) | \($m.repo) | `\($m.revision)` | \(.name) | \(.size) | `\(.sha256)` |"' \
  "$catalog"
