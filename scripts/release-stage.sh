#!/usr/bin/env bash
# Folgeschritte nach der Abnahme (docs/RELEASE_QAS.md), ohne Build.
# Benötigt gh (mit GH_TOKEN), jq und GH_REPO; liest die Version aus dem
# ausgecheckten src-tauri/Cargo.toml.
#
#   scripts/release-stage.sh qas <Commit von qas>
#       (release-qas.yml, Push auf qas; Rechte contents: read,
#       pull-requests: write) Ist der abgenommene Commit (Tag v<version>)
#       in qas enthalten und v<version> noch nicht veröffentlicht: PR
#       qas -> main anlegen, falls keiner offen ist. Wurde der abgenommene
#       PR per Squash/Rebase gemergt (Tag-Commit fehlt in qas): Fehler mit
#       Erklärung, auch als Kommentar im PR.
#   scripts/release-stage.sh main <Commit von main>
#       (release-main.yml, Push auf main; Recht contents: write) Ist der
#       Tag-Commit in main enthalten und das Release noch ein Entwurf:
#       veröffentlichen und als "latest" markieren (App-Updater).
# Passt nichts, tut das Skript nichts (Exit 0).
set -euo pipefail

stage="${1:-}"
tip="${2:-}"
case "$stage" in
  qas | main) ;;
  *)
    echo "Aufruf: $0 <qas|main> <Commit>" >&2
    exit 2
    ;;
esac
if ! [[ "$tip" =~ ^[0-9a-f]{40}$ ]]; then
  echo "::error::Ungültiger Commit: $tip" >&2
  exit 2
fi
: "${GH_REPO:?GH_REPO fehlt}"
summary="${GITHUB_STEP_SUMMARY:-/dev/stdout}"

version="$(sed -n 's/^version = "\(.*\)"$/\1/p' src-tauri/Cargo.toml | head -n 1)"
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "::error::Version in src-tauri/Cargo.toml nicht gefunden ($version)." >&2
  exit 1
fi
tag="v$version"

# Commit, auf den der Tag zeigt (leer: kein Tag = keine Abnahme).
tag_commit=""
if out="$(gh api "repos/$GH_REPO/git/ref/tags/$tag" 2>&1)"; then
  tag_commit="$(jq -r .object.sha <<<"$out")"
  if [ "$(jq -r .object.type <<<"$out")" = "tag" ]; then
    tag_commit="$(gh api "repos/$GH_REPO/git/tags/$tag_commit" --jq .object.sha)"
  fi
elif ! grep -q "HTTP 404" <<<"$out"; then
  echo "::error::Tag $tag konnte nicht geprüft werden: $out" >&2
  exit 1
fi
if [ -z "$tag_commit" ]; then
  echo "Kein Tag $tag: Version $version ist (noch) nicht abgenommen, nichts zu tun." >>"$summary"
  exit 0
fi

# Ist <tag_commit> Vorfahre von <tip> (oder gleich)?
contained() {
  local status
  status="$(gh api "repos/$GH_REPO/compare/$tag_commit...$tip" --jq .status)"
  [ "$status" = "ahead" ] || [ "$status" = "identical" ]
}

# Veröffentlichtes Release (sichtbar auch ohne Schreibrecht).
published() {
  gh api "repos/$GH_REPO/releases/tags/$tag" >/dev/null 2>&1
}

if [ "$stage" = qas ]; then
  if published; then
    echo "$tag ist bereits veröffentlicht; neue Änderungen brauchen eine neue Version." >>"$summary"
    exit 0
  fi
  if ! contained; then
    # Zugehöriger PR des abgenommenen Commits: gemergt, aber Commit fehlt
    # in qas -> Squash oder Rebase. Noch offen -> kein Fehler.
    pr_info="$(gh api "repos/$GH_REPO/commits/$tag_commit/pulls" \
      --jq '[.[] | select(.base.ref == "qas")] | first // empty')"
    if [ -n "$pr_info" ] && [ "$(jq -r '.merged_at // ""' <<<"$pr_info")" != "" ]; then
      pr="$(jq -r .number <<<"$pr_info")"
      msg="Der abgenommene Commit $tag_commit (Tag $tag) ist in qas nicht enthalten, obwohl PR #$pr gemergt ist – vermutlich per Squash oder Rebase. Dann zeigt der Tag auf einen Commit außerhalb von qas/main, und das Release passt nicht zum Stand in main. Abhilfe: PRs nach qas immer mit \"Create a merge commit\" mergen. Für diesen Stand: Version erhöhen und erneut über einen PR nach qas abnehmen (der Tag $tag und sein Entwurfs-Release werden dann nicht verwendet und können gelöscht werden)."
      gh pr comment "$pr" --body "$msg" || true
      echo "::error::$msg" >&2
      exit 1
    fi
    echo "Der abgenommene Commit $tag_commit ($tag) ist noch nicht in qas enthalten (PR noch nicht gemergt); nichts zu tun." >>"$summary"
    exit 0
  fi
  existing="$(gh pr list --base main --head qas --state open --json url -q '.[0].url')"
  if [ -n "$existing" ]; then
    echo "Pull Request qas -> main existiert bereits: $existing" >>"$summary"
    exit 0
  fi
  url="$(gh pr create --base main --head qas \
    --title "Release $tag nach main" \
    --body "$(printf '%s\n' \
      "Abgenommener Stand: Tag \`$tag\` auf \`$tag_commit\`, in qas enthalten (qas: \`$tip\`)." \
      "" \
      "Entwurfs-Release: https://github.com/$GH_REPO/releases (Entwurf $tag; nur für Personen mit Schreibrecht sichtbar)." \
      "" \
      "Mit \"Create a merge commit\" mergen. Der Merge nach main veröffentlicht das Release automatisch (release-main.yml). Automatisch angelegt von release-qas.yml.")")"
  echo "Pull Request qas -> main: $url" >>"$summary"
  exit 0
fi

# stage = main
if ! contained; then
  echo "Tag-Commit $tag_commit ($tag) ist nicht in main enthalten; nichts zu veröffentlichen." >>"$summary"
  exit 0
fi
draft="$(gh release view "$tag" --json isDraft -q .isDraft 2>/dev/null || true)"
if [ "$draft" != "true" ]; then
  echo "Kein Entwurfs-Release $tag (bereits veröffentlicht oder nicht vorhanden); nichts zu tun." >>"$summary"
  exit 0
fi
gh release edit "$tag" --draft=false --latest
echo "Release $tag veröffentlicht (latest): $(gh release view "$tag" --json url -q .url)" >>"$summary"
