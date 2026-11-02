# Release mit QAS-Stufe und Abnahme

Jedes Release durchläuft zuerst eine **QAS-Stufe** (Qualitätssicherung). Dort wird es
genau so gebaut wie ein Release und als GitHub-**Pre-Release** zum Testen
bereitgestellt. Erst nach der **manuellen Abnahme** („QAS-Abnahme“) wird **genau dieser
Build** zum Release befördert. Neu gebaut wird dabei nichts, nur neu signiert.
Workflows: [`release.yml`](../.github/workflows/release.yml) (QAS-Stufe, Abnahme,
Beförderung), [`release-qas.yml`](../.github/workflows/release-qas.yml) (nach dem Merge
in `qas`: PR `qas` → `main`), [`release-main.yml`](../.github/workflows/release-main.yml)
(nach dem Merge in `main`: Release veröffentlichen).

## Ablauf

```
 Entwicklungsbranch (in diesem Repository, kein Fork)
        │
        ▼
 Pull Request ──► qas    jeder Push auf den PR (opened / synchronize / reopened)
        │                startet release.yml; ein laufender Lauf desselben PRs
        │                wird abgebrochen (auch eine wartende Abnahme)
        ▼                (Fallback: „Run workflow“ mit der PR-Nummer)
 version ── PR offen, Basis qas, kein Fork, enthält den aktuellen Stand von qas?
        │   Versionen gleich? v<version> noch frei? Modell-Pins gegen Hugging Face
        │   gebaut wird der PR-Head-Commit <sha> (nicht der Test-Merge)
        ├─► images ─────────── Images bauen, push :<version>-qas.<n> und :sha-<sha>
        │                      (nie :<version>), Archive + image-digests.json
        ├─► build-windows ──── code-pruefer.exe + KI-Laufzeit (unsigniert, SHA256SUMS)
        ├─► bundle-windows ─── Installer, SignPath Test-Signing (Env. „qas“, falls eingerichtet)
        ├─► bundle-macos ───── DMGs (Apple-Signatur, falls eingerichtet, Env. „qas“)
        ▼
 qas-release ──────── Pre-Release v<version>-qas.<n> „QAS – nicht für Produktion“
        │             (Ziel-Commit <sha>), Installer und SHA256SUMS.txt
        ├─► qas-kommentar ─ Kommentar im PR: Links zu Pre-Release und Lauf
        ▼
 abnahme ──────────── wartet auf „Approve and deploy“ (Environment „production“);
        │             danach: PR-Head noch <sha>? sonst Abbruch ohne Beförderung
        ▼
        ├─► promote-images ── :<version> per Digest auf das getestete Image setzen
        ├─► promote-windows ─ dieselben unsignierten Dateien mit Release-Signing
        │                     neu bündeln (Env. „release“), Inhalt mit QAS vergleichen
        ▼
 updater-sign ─────── Updater-Signaturen über den finalen Installer und die
        │             macOS-Archive (Env. „release“, docs/UPDATER.md)
        ▼
 release ──────────── PR-Head noch <sha>? Tag v<version> auf <sha>, ENTWURFS-Release
        │             mit latest.json, SHA256SUMS.txt + Build-Provenance-Attestierung
        ▼
 abnahme-kommentar ── Kommentar im PR: „Abgenommen – bitte in qas mergen
                      (Merge-Commit, kein Squash)“. Der Workflow mergt nie.

 Bernhard mergt den PR in qas („Create a merge commit“)
        ▼
 release-qas.yml ──── (Push auf qas) <sha> in qas enthalten? → PR „Release v<version>
                      nach main“ (qas → main). Squash/Rebase erkannt → Fehler + Hinweis im PR
 Bernhard mergt qas → main („Create a merge commit“)
        ▼
 release-main.yml ─── (Push auf main) <sha> in main enthalten, Release noch Entwurf?
                      → veröffentlichen, „latest“ → installierte Apps bieten das Update an
```

`<n>` ist die Laufnummer von `release.yml` (`github.run_number`), z. B. `v0.2.0-qas.17`.
Jeder Lauf, der bis `qas-release` kommt, legt ein eigenes Pre-Release an (auch mehrere je
PR). Gewählt ist das gegenüber PR-bezogenen Artefakten, weil Pre-Releases nicht nach 30
Tagen verfallen, einen festen Link haben und denselben Ablauf wie bisher nutzen. Alte
QAS-Pre-Releases kann man von Hand löschen.

**Direkte Pushes auf `qas` starten keinen Build mehr.** Sie lösen nur `release-qas.yml`
aus, und das legt höchstens den PR nach `main` an, wenn der abgenommene Commit enthalten
ist. `main` bekommt Änderungen nur noch über abgenommene Releases (PR `qas` → `main`).

## QAS-Lauf starten

1. Auf einem Branch in diesem Repository arbeiten. Die Version in `src-tauri/Cargo.toml`,
   `src-tauri/tauri.conf.json` und `package.json` erhöhen (alle drei gleich). Ist die
   Version schon als Tag oder Release vorhanden, bricht der Job `version` ab.
2. Den Branch mit dem aktuellen Stand von `qas` zusammenführen (sonst bricht `version` mit
   „enthält den aktuellen Stand von qas nicht“ ab, Knopf „Update branch“ im PR).
3. Pull Request **nach `qas`** öffnen. Jeder weitere Push auf den Branch startet einen neuen
   Lauf und bricht den vorigen ab.
4. Fallback: Actions → „Release (QAS und Abnahme)“ → „Run workflow“ → Feld „Nummer des
   offenen Pull Requests nach qas“. Gebaut wird der aktuelle Head-Commit dieses PRs, mit
   denselben Prüfungen. Der Knopf erscheint erst, wenn `release.yml` auf dem
   Standard-Branch `main` liegt.

PRs aus Forks bauen nie (Bedingung am Job `version`, außerdem bekommen sie von GitHub
keine Secrets). `pull_request_target` wird nicht verwendet.

Einen versionierten Tag `v*` von Hand zu pushen, startet **keinen** Release.

## Testen

- Unter Releases das Pre-Release `v<version>-qas.<n>` öffnen (Link im PR-Kommentar, in der
  Zusammenfassung des Laufs und am wartenden Job `abnahme`) und Installer bzw. DMGs laden.
- Prüfsummen: `sha256sum -c SHA256SUMS.txt`.
- Das Pre-Release enthält keine `latest.json`: Installierte Apps bieten QAS-Stände nie
  als Update an. Den Updater selbst testet man auf einem Fork ([`UPDATER.md`](UPDATER.md)).
- Windows: Ist SignPath im Environment `qas` eingerichtet, sind die Dateien mit dem
  **Test-Zertifikat** signiert. Windows zeigt sie als nicht vertrauenswürdig an; das ist
  in QAS so gewollt. Die Signaturtabelle steht in der Zusammenfassung des Jobs
  `bundle-windows`.
- Die Images liegen als `ghcr.io/bernhardgoetz/code-pruefer-<sprache>:<version>-qas.<n>`
  vor. Die App erwartet online `:<version>`, das erst nach der Abnahme existiert. Bis dahin
  lädt sie die Images aus dem Installer; „Images aktualisieren“ funktioniert in QAS noch
  nicht.

## Abnehmen oder ablehnen

- Actions → der Lauf (Link im PR-Kommentar) → oben „Review deployments“ → Environment
  `production` anhaken → Kommentar → **„Approve and deploy“** (abnehmen) oder
  **„Reject“** (ablehnen).
- Abgenommen werden kann nur der **neueste** PR-Stand: Ein neuer Push bricht den alten
  Lauf ab. Kommt ein Push genau zwischen Start und Freigabe an, bricht `abnahme` nach der
  Freigabe mit „PR … steht inzwischen auf …“ ab; befördert wird dann nichts.
- Ablehnen beendet den Lauf. Nichts wird veröffentlicht, die QAS-Images und das
  Pre-Release bleiben stehen.
- **Zeitlimit:** GitHub wartet höchstens **30 Tage** auf eine Freigabe, danach schlägt
  der Job fehl. Die Artefakte des Laufs bleiben ebenfalls 30 Tage erhalten. Später ist
  ein neuer Lauf nötig (neuer Push oder „Run workflow“).

### Abbruch nach der Abnahme

Nach „Approve and deploy“ **nicht mehr auf den Branch pushen**, bis der PR gemergt ist. Ein
Push bricht den Lauf auch während der Beförderung ab (Concurrency je PR). Je nachdem, wie
weit er war:

- vor dem Job `release`: Es gibt noch keinen Tag. Der neue Lauf zum neuen Stand ist ein
  normaler QAS-Lauf.
- während `release`: Der Tag `v<version>` kann schon auf dem alten Commit stehen. Der neue
  Lauf bricht dann in `version` ab („Version … ist bereits vergeben“). Lösung: entweder den
  alten Stand wiederherstellen und den alten Lauf mit „Re-run failed jobs“ fertig laufen
  lassen (`release` ist wiederholbar), oder Tag und Entwurf `v<version>` löschen und neu
  abnehmen.

## Was nach der Abnahme passiert

1. **abnahme:** prüft nach der Freigabe mit `scripts/pr-head-check.sh`, dass der PR noch
   offen ist, nach `qas` geht, aus diesem Repository kommt und sein Head der getestete
   Commit ist.
2. **promote-images:** Für jede Sprache wird geprüft, dass `:<version>-qas.<n>` noch auf
   den getesteten Digest zeigt. Dann wird `:<version>` per
   `docker buildx imagetools create` auf **denselben Digest** gesetzt, ohne Neubau. Gibt
   es `:<version>` schon mit anderem Digest, bricht der Job ab.
3. **promote-windows:** Nimmt die unsignierten Programmdateien aus `build-windows`. Ihre
   `SHA256SUMS.txt` muss dasselbe SHA-256 haben wie im QAS-Lauf, sonst Abbruch. Dann
   bündelt er sie mit der **Release-Signing**-Policy aus dem Environment `release` neu.
   Neu gebündelt wird nur, weil die Signatur es erfordert: Uninstaller und Installer
   betten signierte Dateien ein und müssen deshalb neu erzeugt werden.
   - **Unterschied zum getesteten QAS-Installer: nur die Signaturen.** Neu signiert sind
     `code-pruefer.exe` und `uninstall.exe` (und mit `SIGNPATH_SIGN_UPSTREAM` die
     Fremdbinaries), außerdem ist der Installer selbst neu gebaut und signiert. Der
     Workflow entpackt beide Installer und vergleicht sie. Die Dateiliste muss gleich
     sein, jede Datei muss bitgleich sein, außer signierten exe/dll. Bei denen müssen die
     unsignierten Ausgangsdateien (SHA-256 im Signatur-Manifest) gleich sein. Die
     Installer-Datei selbst ist nicht bitgleich (neu gepackt, andere Signaturen). Die
     Liste der neu signierten Dateien steht in der Job-Zusammenfassung.
   - Ist SignPath weder in `qas` noch in `release` eingerichtet, wird der QAS-Installer
     **unverändert** übernommen.
   - macOS: Die DMGs aus QAS werden unverändert veröffentlicht. Apple hat keine
     Test-Signatur, sie sind also schon in QAS final signiert (falls eingerichtet).
4. **updater-sign:** Prüft den fertigen Windows-Installer aus `promote-windows` und die
   macOS-Archive (`*.app.tar.gz`) gegen die Manifeste der erzeugenden Jobs, signiert sie
   für den App-Updater und prüft die Signaturen.
   Ohne Updater-Schlüssel: übersprungen mit Warnung. Siehe [`UPDATER.md`](UPDATER.md).
5. **release:** Prüft erneut, dass `v<version>` frei ist (ein erneuter Lauf desselben
   Commits mit Entwurf ist erlaubt) und dass der PR-Head unverändert ist. Dann setzt der
   Job den Tag `v<version>` auf den **getesteten PR-Head-Commit** und legt ein
   **Entwurfs-Release** an, mit Windows-Installer (Release), DMGs, `latest.json` und
   `*.app.tar.gz` (App-Updater, falls eingerichtet), `SHA256SUMS.txt` und
   Build-Provenance-Attestierung. Dateinamen ohne Umlaut (`Code-Pruefer_…`).
6. **abnahme-kommentar:** Kommentar im PR „Abgenommen – bitte in qas mergen
   (Merge-Commit, kein Squash)“. Gemergt wird **nicht** automatisch.
7. **Merge in `qas` (Bernhard, „Create a merge commit“)** → `release-qas.yml`: Ist der
   Tag-Commit in `qas` enthalten (`compare`-API, entspricht
   `git merge-base --is-ancestor`), legt es den PR „Release v<version> nach main“
   (`qas` → `main`) an, mit Link auf den Entwurf. Ist der abgenommene PR gemergt, der
   Tag-Commit aber nicht enthalten (Squash oder Rebase), schlägt der Lauf mit Erklärung
   fehl und kommentiert den PR. Abhilfe dann: Version erhöhen und neu abnehmen.
8. **Merge nach `main` (Bernhard, „Create a merge commit“)** → `release-main.yml`: Ist der
   Tag-Commit in `main` enthalten und das Release noch ein Entwurf, wird es
   **veröffentlicht und als „latest“ markiert**. Erst dann bieten installierte Apps das
   Update an. Passt nichts (kein Tag, nicht enthalten, schon veröffentlicht), passiert
   nichts. Von Hand veröffentlichen geht weiterhin (Release öffnen → „Publish release“,
   „Set as the latest release“ angehakt).

Warum Merge-Commits: Der Tag zeigt auf den getesteten Commit. Nur ein Merge-Commit
übernimmt genau diesen Commit in `qas` und `main`. Squash oder Rebase erzeugen neue
Commits, dann wäre der Tag nicht in `main` enthalten und `release-main.yml`
veröffentlicht nichts. Weil `version` verlangt, dass der PR den aktuellen Stand von `qas`
enthält, ist der Inhalt nach dem Merge gleich dem getesteten, solange `qas` sich bis zum
Merge nicht ändert. Im Zweifel erst den PR aktualisieren (neuer Lauf), dann mergen.

## KI-Modellkatalog: Pins per Pull Request

Der Release pinnt den Katalog nicht selbst. Wäre das Hugging-Face-Repository des
Herausgebers in dem Moment kompromittiert, würde sonst die bösartige Datei gepinnt
(Vertrauen beim ersten Abruf). Stattdessen:

1. **Einmalig (und bei jedem gewollten Modell-Update):** Actions → „KI-Modellkatalog
   pinnen“ (`.github/workflows/pin-models.yml`) → „Run workflow“ auf dem Branch, in den
   der PR soll (z. B. `qas`). Der Workflow trägt Revision, Datei(en), Größe und SHA-256
   aus der Hugging-Face-API ein (`scripts/pin-models.sh`) und legt einen Pull Request mit
   Tabelle und Links an. Voraussetzung: „Allow GitHub Actions to create and approve pull
   requests“ (siehe unten). Für PRs des `GITHUB_TOKEN` startet GitHub keine weiteren
   Workflows; CI für den PR ggf. durch Schließen und Wiederöffnen oder einen eigenen
   Commit anstoßen.
2. **Bernhard prüft** die Werte auf huggingface.co (Revision in der Commit-Historie des
   Herausgebers, Größe und SHA-256 der Datei unter „Files and versions“) und mergt.
3. **Jeder Release** (Job `version`): `scripts/pin-models.sh --check` (vollständig
   gepinnt, sonst Abbruch mit Hinweis auf den Workflow) und `--verify` (die API muss für
   die gepinnte Revision genau dieselben Dateien, Größen und SHA-256 melden; gelöschte
   Revision oder jede Abweichung → Abbruch). Eine neuere Revision auf `main` wird nur als
   Hinweis gemeldet; übernommen wird sie nur über einen neuen Pin-PR.

## Integrität der Artefakte zwischen den Jobs

Jeder Schritt eines späteren Jobs desselben Laufs könnte Artefakte ersetzen
(`upload-artifact` mit `overwrite`, das Laufzeit-Token hängt nicht an `permissions:`).
Ein kompromittierter Baustein in z. B. `promote-images` könnte so das macOS-Updater-Archiv
austauschen, das `updater-sign` danach gültig signieren würde. Deshalb gilt:

- Jeder Job, der ein Release-Artefakt erzeugt, gibt ein **Manifest** (SHA-256 und
  Dateiname je Datei, `scripts/artifact-manifest.sh write`) als Job-Output weiter:
  `version` (gepinnter `catalog.json`), `images` (je Sprache `image-digests.json`, die
  Image-Archive amd64/arm64 und der Registry-Digest), `build-windows` (`SHA256SUMS.txt`
  der unsignierten Programmdateien), `bundle-windows` und `promote-windows` (Installer,
  Signatur-Manifest), `bundle-macos` (je Architektur DMG und `.app.tar.gz`) und
  `updater-sign` (Updater-Signaturen).
- Jeder Job, der ein Artefakt verwendet, prüft es **vor** dem Bündeln, Signieren oder
  Veröffentlichen (`scripts/artifact-manifest.sh verify`): gleiche Dateinamen, gleiche
  SHA-256, keine zusätzlichen und keine fehlenden Dateien. `promote-images` prüft
  außerdem, dass `image-digests.json` genau den Digest nennt, den `images` gebaut hat.
  Abweichung oder fehlender Output → Abbruch.
- Matrix-Jobs (`images`, `bundle-macos`) schreiben je Eintrag einen eigenen Output-Namen
  (z. B. `dmg_macos_arm64`). Kommt ein Wert nicht an, scheitert die Prüfung (nie still
  ungeprüft).
- `updater-sign` installiert nur die Tauri-CLI (`scripts/install-signer.sh`: Version und
  sha512-Integrität aus `package-lock.json`, `npm ci --ignore-scripts` in einem eigenen
  Ordner), kein vollständiges `npm ci`. Der private Updater-Schlüssel steht nur im Schritt
  „Signieren“; geprüft wird in einem eigenen Schritt ohne Schlüssel.

## Nötige GitHub-Einstellungen (einmalig)

Settings → Environments:

| Environment | Pflicht-Reviewer | Deployment branches and tags | Secrets / Variablen |
|---|---|---|---|
| `qas` | keine | siehe unten **(ungeprüft)** | SignPath **Test-Signing** (`SIGNPATH_API_TOKEN`; `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG`, `SIGNPATH_SIGNING_POLICY_SLUG` = `test-signing`, optional `SIGNPATH_ARTIFACT_CONFIGURATION_SLUG`, `SIGNPATH_SIGN_UPSTREAM`), Apple-Secrets (`APPLE_CERTIFICATE`, …) |
| `production` | **Bernhard** (Required reviewers); „Prevent self-review“ **aus** lassen, da Bernhard selbst pusht | siehe unten **(ungeprüft)** | keine (nur Freigabe) |
| `release` | keine (die Freigabe erfolgt über `production`); siehe unten, falls die Branch-Regel entfallen muss | siehe unten **(ungeprüft)** | SignPath **Release-Signing** (dieselben Namen, `SIGNPATH_SIGNING_POLICY_SLUG` = `release-signing`); App-Updater: Secrets `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` ([`UPDATER.md`](UPDATER.md)) |

**Deployment-Branch-Regeln bei `pull_request`-Läufen (ungeprüft, beim ersten Lauf
klären):** Alle Jobs von `release.yml` laufen jetzt im Kontext eines Pull Requests.
`GITHUB_REF` ist dort `refs/pull/<n>/merge`, `GITHUB_HEAD_REF` der Entwicklungsbranch.
Gegen welchen Ref GitHub die Regel „Selected branches and tags“ prüft, konnte ich nicht
nachlesen (docs.github.com war aus der Arbeitsumgebung nicht erreichbar). Die bisherige
Regel „Branch `qas`“ passt sehr wahrscheinlich **nicht** mehr: Der Lauf bleibt dann am
ersten Job mit Environment hängen bzw. schlägt fehl mit „Branch „…“ is not allowed to
deploy to … due to environment protection rules“. Die Meldung nennt den geprüften Ref.
Vorgehen:

1. Erst die Regel in allen drei Environments um ein Muster für die Entwicklungsbranches
   ergänzen (z. B. `claude/*`, `feature/*`). Greift das (Lauf kommt an `bundle-windows`
   vorbei), wird der Head-Branch geprüft.
2. Nennt die Fehlermeldung `refs/pull/<n>/merge`, kann keine Branch-Regel passen. Dann für
   `qas`, `production` und `release` „No restriction“ wählen und **in `release`
   zusätzlich „Required reviewers: Bernhard“** setzen. Ohne Branch-Regel könnte sonst
   jeder Workflow-Lauf mit Schreibrecht die Release-Secrets (Updater-Schlüssel,
   SignPath-Release-Token) benutzen. Bei einem Ein-Personen-Repository ist das
   Restrisiko gering; die zweite Freigabe kommt dann für `promote-windows` und
   `updater-sign` hinzu.

**Ruleset für `qas`** (Settings → Rules → Rulesets → Branch `qas`):

- „Require a pull request before merging“ an. Unter „Allowed merge methods“ **nur
  „Merge“** (Merge-Commit) zulassen, Squash und Rebase abwählen **(ungeprüft: Bezeichnung
  in der Oberfläche)**. Squash würde die Tag-Herkunft brechen (siehe oben).
- „Block force pushes“, „Restrict deletions“.
- Optional „Require status checks to pass“ mit dem CI-Job, „Require branches to be up to
  date“ (deckt sich mit der Prüfung in `version`).
- Bypass nur für Bernhard (Repository admin). Direkte Pushes auf `qas` starten keinen
  Build mehr, sollten aber trotzdem die Ausnahme bleiben.
- Repository-Einstellung (Settings → General → Pull Requests): „Allow merge commits“
  muss an sein. Squash/Rebase können dort an bleiben, wenn das Ruleset sie für `qas` und
  `main` ausschließt.

**Ruleset für `main`:** ebenso „Require a pull request before merging“ mit Merge-Commit,
damit nur der PR `qas` → `main` den Stand nach `main` bringt.

Repository-Variable (Settings → Secrets and variables → Actions → Variables):
`TAURI_UPDATER_PUBKEY` (öffentlicher Updater-Schlüssel; wird in die App eingebaut).

Weitere Einstellungen:

- **Actions darf PRs anlegen:** Settings → Actions → General → Workflow permissions →
  „**Allow GitHub Actions to create and approve pull requests**“ anhaken. Sonst scheitert
  `release-qas.yml` (PR `qas` → `main`). Der Release-Entwurf ist dann trotzdem fertig.
- **„Automatically delete head branches“ aus lassen**, sonst löscht GitHub `qas` nach dem
  Merge des PRs `qas` → `main`.
- **Tag-Regel `v*`** (falls vorhanden, siehe README): Der Workflow legt die Tags
  `v<version>-qas.<n>` und `v<version>` selbst an, mit dem `GITHUB_TOKEN` als
  „github-actions[bot]“. Entweder in der Ruleset-Bypass-Liste die App „GitHub Actions“
  eintragen, oder „Restrict creations“ abschalten und nur „Restrict updates“ und
  „Restrict deletions“ behalten. **[ungeprüft: ob „GitHub Actions“ in der Bypass-Liste
  auswählbar ist]**
- **SignPath:** Die Policies `test-signing` und `release-signing` prüfen den Git-Ref des
  Laufs. Bei PR-Läufen ist das voraussichtlich `refs/pull/<n>/merge` (oder der
  Head-Branch), nicht mehr `refs/heads/qas` **(ungeprüft)**. Den erlaubten Ref in beiden
  Policies entsprechend anpassen; die Fehlermeldung von SignPath nennt den abgelehnten
  Ref. Siehe [`RELEASE_SIGNING.md`](RELEASE_SIGNING.md).

**Sicherheit des PR-Ablaufs:** `pull_request` verwendet die Workflow-Datei des PRs. Wer
einen PR nach `qas` öffnet (nur Personen mit Schreibrecht, Forks sind ausgeschlossen),
kann `release.yml` im PR ändern und so die Secrets des Environments `qas` nutzen, wie
bisher beim Push auf `qas`. Die Release-Secrets und die Beförderung hängen an der
Freigabe in `production`. **Vor „Approve and deploy“ die Änderungen an
`.github/workflows/` und `scripts/` im PR ansehen.**

## Grenzen

- Die Workflows konnten hier nicht laufen (auch `release-qas.yml`, `release-main.yml`
  nicht). Die Skripte `scripts/pr-head-check.sh` und `scripts/release-stage.sh` sind
  lokal mit einem nachgebauten `gh` getestet.
- Der Workflow konnte hier nicht laufen. Geprüft sind Syntax und Sicherheitsregeln
  (actionlint, zizmor). Er nutzt **YAML-Anker** (`&windows-bundle-steps`), damit
  `bundle-windows` und `promote-windows` dieselben Schritte haben. GitHub Actions
  unterstützt Anker; actionlint ab 1.7.8 auch. zizmor weist darauf hin, dass er Anker
  nur eingeschränkt auswertet.
- Ein erneuter Versuch („Re-run failed jobs“) nach der Abnahme ist möglich, solange die
  Artefakte existieren (30 Tage). `release` und `promote-images` sind dafür
  wiederholbar ausgelegt.
