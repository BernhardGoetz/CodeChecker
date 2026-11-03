#!/usr/bin/env bash
# Prüft, dass ein Pull Request nach qas noch genau den getesteten Stand
# hat (Release-Workflow, docs/RELEASE_QAS.md). Benötigt gh (mit GH_TOKEN,
# Recht pull-requests: read), jq und GH_REPO.
#
#   scripts/pr-head-check.sh <PR-Nummer> <Commit-SHA> [--up-to-date]
#       Fehler, wenn der PR nicht offen ist, nicht nach qas geht, aus einem
#       anderen Repository (Fork) kommt oder sein Head nicht mehr <Commit>
#       ist (neuer Push nach dem Start des Laufs).
#       Mit --up-to-date zusätzlich: Fehler, wenn der aktuelle Stand von
#       qas nicht in <Commit> enthalten ist (Branch zuerst mit qas
#       aktualisieren; sonst enthielte der Merge ungetesteten Code).
set -euo pipefail

pr="${1:?Aufruf: $0 <PR-Nummer> <Commit-SHA> [--up-to-date]}"
sha="${2:?Aufruf: $0 <PR-Nummer> <Commit-SHA> [--up-to-date]}"
mode="${3:-}"
: "${GH_REPO:?GH_REPO fehlt}"

if ! [[ "$pr" =~ ^[1-9][0-9]{0,9}$ ]]; then
  echo "::error::Ungültige PR-Nummer: $pr" >&2
  exit 1
fi
if ! [[ "$sha" =~ ^[0-9a-f]{40}$ ]]; then
  echo "::error::Ungültiger Commit-SHA: $sha" >&2
  exit 1
fi

info="$(gh api "repos/$GH_REPO/pulls/$pr")"
state="$(jq -r .state <<<"$info")"
base="$(jq -r .base.ref <<<"$info")"
head_repo="$(jq -r '.head.repo.full_name // ""' <<<"$info")"
head_sha="$(jq -r .head.sha <<<"$info")"

if [ "$state" != "open" ]; then
  echo "::error::PR #$pr ist nicht offen ($state)." >&2
  exit 1
fi
if [ "$base" != "qas" ]; then
  echo "::error::PR #$pr geht nach $base, Releases laufen nur über PRs nach qas." >&2
  exit 1
fi
# Groß-/Kleinschreibung von owner/repo ist bei GitHub nicht signifikant.
if [ "${head_repo,,}" != "${GH_REPO,,}" ]; then
  echo "::error::PR #$pr kommt aus ${head_repo:-einem gelöschten Repository}; PRs aus Forks bauen keine Releases." >&2
  exit 1
fi
if [ "$head_sha" != "$sha" ]; then
  echo "::error::PR #$pr steht inzwischen auf $head_sha, getestet wurde $sha. Der Lauf zum neuen Stand gilt; dieser wird nicht befördert." >&2
  exit 1
fi

if [ "$mode" = "--up-to-date" ]; then
  behind="$(gh api "repos/$GH_REPO/compare/qas...$sha" --jq .behind_by)"
  if [ "$behind" != "0" ]; then
    echo "::error::Der Branch von PR #$pr enthält den aktuellen Stand von qas nicht ($behind Commit(s) fehlen). Zuerst qas in den Branch mergen (\"Update branch\"), dann startet der Lauf neu." >&2
    exit 1
  fi
fi
echo "PR #$pr: offen, Basis qas, Head $sha."
