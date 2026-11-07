# Mitmachen beim Code-Prüfer (für Entwicklerinnen und Entwickler)

Dieses Dokument richtet sich an alle, die den Code-Prüfer **weiterentwickeln, bauen oder veröffentlichen** wollen. Wer das Programm nur **benutzen** will, liest die [`README.md`](README.md).

Der Code-Prüfer ist eine Desktop-App (Windows und macOS) auf Basis von **Tauri 2**: Das Frontend ist in **React/TypeScript** geschrieben, das Backend in **Rust**. Programme von Studierenden laufen in abgeschotteten **Docker-/Podman-Containern**. Optional erzeugt ein **lokales KI-Modell** (llama.cpp) Testfälle.

Repository: <https://github.com/BernhardGoetz/CodeChecker>
Lizenz: MIT (siehe [`LICENSE`](LICENSE)).

## Inhalt

1. [Architektur](#1-architektur)
2. [Projektstruktur](#2-projektstruktur)
3. [Voraussetzungen](#3-voraussetzungen)
4. [Lokal bauen und starten](#4-lokal-bauen-und-starten)
5. [Mock-Backend für die Frontend-Entwicklung](#5-mock-backend-für-die-frontend-entwicklung)
6. [Runner-Images bauen und lokal nutzen](#6-runner-images-bauen-und-lokal-nutzen)
7. [KI lokal entwickeln und testen](#7-ki-lokal-entwickeln-und-testen)
8. [Tests und CI](#8-tests-und-ci)
9. [Installer lokal bauen](#9-installer-lokal-bauen)
10. [Release-Workflow](#10-release-workflow)
11. [Konventionen](#11-konventionen)
12. [Eine neue Programmiersprache hinzufügen](#12-eine-neue-programmiersprache-hinzufügen)
13. [Ein neues KI-Modell hinzufügen](#13-ein-neues-ki-modell-hinzufügen)
14. [Beitragsablauf (Branch und Pull Request)](#14-beitragsablauf-branch-und-pull-request)
15. [Sicherheit](#15-sicherheit)
16. [Weitere Dokumente](#16-weitere-dokumente)

---

## 1. Architektur

```
┌──────────────────────────── Tauri-Fenster (WebView) ────────────────────────────┐
│  Frontend: React 19 + TypeScript + Monaco-Editor (src/)                          │
│  Tabs „Aufgabe“ / „Testfälle“ / „Ergebnis“, Einstellungen, Update-Dialog         │
└───────────────────────────────┬──────────────────────────────────────────────────┘
                                │ Tauri-Commands (IPC) + Events (Fortschritt)
┌───────────────────────────────▼──────────────────────────────────────────────────┐
│  Backend: Rust (src-tauri/src/lib.rs registriert die Commands)                    │
│                                                                                   │
│  runner/   Prüfung: Engine-Verbindung (bollard), Images, gehärteter Container,    │
│            Bewertung, Ausgabevergleich, Ausgabevorlagen                           │
│  ai/       Lokale KI: llama-server als Sidecar, Modellkatalog, Pipeline für       │
│            Testfälle, OCR-Korrektur, Variationstest                               │
│  docs/     Import von Aufgabenblättern (DOCX, PDF, Bild), systemeigene OCR,       │
│            Hilfsprozess mit Limits                                                │
│  engine.rs / docker_detect.rs   Engine-Auswahl, Erkennung, automatischer Start    │
│  app_update.rs                  App-Updater (tauri-plugin-updater)                │
└───────────────┬───────────────────────────┬──────────────────────────────────────┘
                │ Docker-/Podman-API         │ HTTP auf 127.0.0.1
        ┌───────▼────────┐          ┌────────▼─────────┐
        │ Container je   │          │ llama-server     │
        │ Prüfung        │          │ (llama.cpp)      │
        │ (Python/Java)  │          │ + GGUF-Modell    │
        └────────────────┘          └──────────────────┘
```

**Frontend** (`src/`): `App.tsx` hält den Zustand (Code je Sprache, Aufgabentext, Testfälle, Vergleichsmodus, Engine-/KI-/Update-Status). `src/api.ts` enthält die typisierten Aufrufe der Tauri-Commands und Events. Die Komponenten liegen in `src/components/`.

**Runner** (`src-tauri/src/runner/`): Jede Prüfung startet einen frischen Container, schreibt den Quellcode hinein, kompiliert (falls nötig) und führt den Code pro Testfall einmal mit der Eingabe als stdin aus. Danach wird der Container immer gelöscht.
- `docker.rs`: Verbindung (Docker/Podman, Sockets, Named Pipes, `DOCKER_HOST`), Image-Pull/-Load, gehärteter Container.
- `images.rs`: Image-Auflösung (siehe Abschnitt 6).
- `languages.rs`: datengetriebene Sprachtabelle.
- `compare.rs`: Normalisierung, Zeilen-Diff, Ausgabebegrenzung; `template.rs`: Ausgabevorlagen mit Platzhaltern.
- `mod.rs`: Ablauf und Bewertung einer Prüfung.

**Sandbox-Eigenschaften** (`runner/docker.rs`, `runner/mod.rs`, getestet in den Unit-Tests):
- kein Netzwerk, read-only Root-Dateisystem, `tmpfs` für `/work` und `/tmp` (je 64 MB)
- 512 MB RAM ohne Swap, 1 CPU, höchstens 64 Prozesse, maximale Dateigröße 16 MB
- Benutzer UID 1000, alle Capabilities entfernt, `no-new-privileges`
- Zeitlimit pro Test: Python 5 s, Java 8 s; Kompilieren 30 s
- nach jedem Test werden übrige Prozesse des Sandbox-Benutzers beendet (`kill -9 -1`)
- höchstens 50 Testfälle pro Prüfung (`MAX_TESTS`); stdout/stderr werden nach 64 KB abgeschnitten
- übrig gebliebene Container (Label `de.code-pruefer.sandbox`) werden beim Start und vor jeder Prüfung entfernt (eigene sofort, fremde nach 15 Minuten)

**AI** (`src-tauri/src/ai/`):
- `llama.rs`: llama-server als Sidecar (Start, `/health`, Client, Abbruch). Er lauscht nur auf `127.0.0.1`, läuft mit `--offline` und zufälligem API-Schlüssel.
- `models.rs` + `catalog.json`: gepinnter Modellkatalog, Hardware-Erkennung, Download, Import.
- `pipeline.rs`: Testfälle erzeugen (Struktur → Lösungen → Kompilieren → Beispiele → Testeingaben → Mehrheitsentscheid → Konfidenz). **Kernregel:** Die KI schreibt nie erwartete Ausgaben; diese entstehen durch Ausführen der Lösungen in der Sandbox.
- `prompts.rs`: Prompts, JSON-Schemas, Code aus Antworten extrahieren.
- `ocr_fix.rs`: KI-Korrektur von OCR-Text (Plausibilitätsfilter lässt nur kleine, örtliche Änderungen durch).
- `variation.rs`: Variationstest (Eingaben/Szenarien, Literalersetzung in einfachen Zuweisungen, Vergleich mit der Musterlösung).
- `gguf.rs`: Plausibilitätsprüfung importierter `.gguf`-Dateien.

**Docs** (`src-tauri/src/docs/`): Import von Aufgabenblättern. `docx.rs` (Text aus `word/document.xml`), `pdf.rs` (Textebene, Scan-Erkennung), `pdf_guard.rs` (Prüfung auf Dekompressionsbomben), `layout.rs` (Zeilen und Einrückung aus OCR-Wortboxen), `ocr/` (Windows: `Windows.Media.Ocr` und `Windows.Data.Pdf`; macOS: Vision und PDFKit; `prepare.rs`: Bildvorbereitung), `worker.rs` (begrenzter Hilfsprozess: dieselbe Exe mit verstecktem Argument; Windows: Job-Objekt). Unter Linux gibt es keine OCR.

**Tauri-Commands** (`src-tauri/src/lib.rs`): `docker_status`, `get_engine`, `set_engine`, `start_engine`, `list_languages`, `run_tests`, `prepare_images`, `update_images`, `pick_files`, `import_task_document`, `ai_status`, `download_model`, `import_model`, `delete_model`, `select_model`, `generate_tests`, `warm_up_ai`, `suggest_ocr_fixes`, `check_variations`, `cancel_variations`, `cancel_ai`, `update_info`, `set_update_interval`, `check_for_update`, `install_update`.

**Berechtigungen der Oberfläche** (`src-tauri/capabilities/default.json`): Das Frontend darf nur drei URLs im Browser öffnen (Docker-Desktop-Seite, podman.io, die GitHub-Release-Seite). Dateiauswahldialoge öffnet das Backend (`pick_files`); die WebView bekommt nur ein Token. Ein Test in `lib.rs` gleicht die URLs mit den Literalen in `DockerBanner.tsx` ab.

## 2. Projektstruktur

```
src/                     React-Frontend (Monaco-Editor, Tabs Aufgabe/Testfälle/Ergebnis)
src/styles.css           Design-Tokens als CSS-Variablen (hell/dunkel) und Komponenten
src/api.ts               Typisierte Tauri-Aufrufe und Events
src/dev/mockBackend.ts   Simuliertes Backend für VITE_MOCK=1 npm run dev (nur Entwicklung)
docs/design/             DESIGN.md, design-tokens.json, design-preview.html
src-tauri/src/
  lib.rs                 Tauri-Commands, App-Start, Aufräumen beim Beenden
  main.rs                Einstiegspunkt
  picker.rs              Dateiauswahl (Token-Verfahren)
  proc_guard.rs          Prozessbegrenzung (Windows: Job-Objekt; Unix: setrlimit)
  ai/llama.rs            llama-server als Sidecar (Start, /health, Client, Abbruch)
  ai/models.rs           Modellkatalog (catalog.json), Hardware, Download, Import
  ai/catalog.json        Fester Modellkatalog (Revision, Dateien, Größe, SHA-256)
  ai/gguf.rs             Prüfung importierter GGUF-Dateien
  ai/pipeline.rs         Testfälle erzeugen: Lösungen, Beispiele, Mehrheit, Konfidenz
  ai/prompts.rs          Prompts, JSON-Schemas, Code aus Antworten extrahieren
  ai/ocr_fix.rs          KI-Korrektur von OCR-Text: Prompt, Plausibilitätsfilter
  ai/variation.rs        Variationstest: Eingaben/Szenarien, Literalersetzung, Erklärung
  docs/mod.rs            Import von Aufgabenblättern (Bild, PDF, DOCX), Limits
  docs/docx.rs           Text aus word/document.xml
  docs/pdf.rs            Textebene pro Seite, Erkennung gescannter Seiten
  docs/pdf_guard.rs      PDF-Prüfungen (Dekompressionsbomben usw.)
  docs/layout.rs         Zeilen und Einrückung aus OCR-Wortboxen rekonstruieren
  docs/worker.rs         Begrenzter Hilfsprozess für den Dokumentenimport
  docs/ocr/              Systemeigene Texterkennung: win.rs, mac.rs, prepare.rs
  docker_detect.rs       Erkennung: nicht installiert / nicht gestartet / ok
  engine.rs              Engine-Auswahl (Datei/Registry), Start, Podman-Pipe
  app_update.rs          App-Updater (tauri-plugin-updater): Intervall, Suche, Installation
  runner/mod.rs          Ablauf einer Prüfung, Bewertung der Testfälle
  runner/docker.rs       Engine-Verbindung, Image-Pull/-Load, gehärteter Container
  runner/images.rs       Image-Auflösung: vorhanden → Archiv → GHCR → :dev
  runner/languages.rs    Sprachtabelle (Image, Dateiname, Befehle, Zeitlimits)
  runner/compare.rs      Normalisierung, Zeilen-Diff, Ausgabebegrenzung
  runner/template.rs     Ausgabevorlagen mit Platzhaltern: Prüfung und Abgleich
src-tauri/tests/         Integrationstests (ai_integration, docker_integration,
                         docs_worker, image_bundle; meist #[ignore])
src-tauri/windows/       NSIS-Installer: Template (installer.nsi) + Hooks (hooks.nsh)
src-tauri/tauri.conf.json          Tauri-Konfiguration (Version, Fenster, CSP, Bundle)
src-tauri/tauri.release.conf.json  Release-Overlay: bündelt resources/images/*.tar.gz,
                                   resources/images/image-digests.json, resources/llama/*
images/python, images/java         Dockerfiles der Runner-Images
scripts/                 Hilfsskripte (siehe unten)
.signpath/               SignPath-Artefaktkonfigurationen (zum Einfügen in SignPath)
docs/                    CODE_SIGNING_POLICY.md, RELEASE_SIGNING.md, RELEASE_QAS.md,
                         UPDATER.md, VEROEFFENTLICHUNG.md, updates/<version>.txt, Recherche
.github/workflows/       ci.yml, images.yml, release.yml, release-qas.yml,
                         release-main.yml, pin-models.yml
.github/dependabot.yml   Updates für Actions, npm, cargo, Basis-Images
CHANGELOG.md             Änderungsprotokoll (Keep a Changelog)
THIRD_PARTY_NOTICES.txt  Lizenzen von llama.cpp u. a. (wird mitgeliefert)
```

Skripte in `scripts/`:

| Skript | Zweck |
|---|---|
| `export-image-archives.sh` | Image-Archive (`*.tar.gz`) und `image-digests.json` erzeugen |
| `fetch-llama-server.sh` | llama-server (Version und SHA-256 gepinnt) nach `src-tauri/resources/llama/` holen |
| `pin-models.sh` | Modellkatalog aus der Hugging-Face-API pinnen (`--check`, `--verify`) |
| `check-version-unreleased.sh` | Ist die Version noch nicht als Tag/Release vergeben? |
| `release-notes.sh` | Release-Notizen aus dem Changelog |
| `pr-head-check.sh`, `release-stage.sh` | PR-Stand prüfen; PR `qas` → `main` anlegen und Release veröffentlichen |
| `artifact-manifest.sh` | SHA-256-Manifeste der Release-Artefakte zwischen Jobs |
| `signpath-stage.mjs`, `signpath-round.sh` | Windows-Signatur über SignPath in Runden |
| `updater.mjs` | Updater-Konfiguration (öffentlicher Schlüssel), Signaturprüfung, `latest.json` |
| `install-signer.sh` | Nur die Tauri-CLI für die Updater-Signatur (ohne `npm ci`) |

## 3. Voraussetzungen

- **Node.js** `^20.19.0` oder `>=22.12.0` und npm (Vorgabe von Vite, `engines` in `node_modules/vite/package.json`; TypeScript verlangt nur `>=16.20.0`). Die CI nutzt Node **22**.
- **Rust** (stable, per [rustup](https://rustup.rs/)). Es gibt keine `rust-toolchain`-Datei und kein `rust-version`-Feld; die CI nutzt `stable`. Für Windows lokal: MSVC-Toolchain (Release-Builds laufen mit MSVC).
- **Tauri-Systemabhängigkeiten:** siehe <https://v2.tauri.app/start/prerequisites/>.
  - Linux (so installiert es die CI): `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev build-essential pkg-config libssl-dev`
  - Windows: WebView2 (unter Windows 11 vorhanden) und die Microsoft C++ Build Tools.
  - macOS: Xcode Command Line Tools.
- **Docker oder Podman** zum Ausführen der Prüfungen und für die Integrationstests.
- Für Installer-Cross-Builds unter Linux zusätzlich: `mingw-w64`, `nsis`, `unzip`, `jq` und das Rust-Target `x86_64-pc-windows-gnu`.

## 4. Lokal bauen und starten

```sh
npm install
npm run tauri dev          # startet Vite (Port 1420) und die Tauri-App
```

Weitere Befehle (aus `package.json` und CI):

```sh
npm run dev                # nur das Frontend (Vite, http://localhost:1420)
npm run typecheck          # tsc --noEmit
npm run build              # Typecheck + Frontend-Build (nach dist/)
npm run tauri build        # Installer bauen (unsigniert, ohne eingebettete Images)

cd src-tauri && cargo test                                # Unit-Tests
cd src-tauri && cargo clippy --all-targets -- -D warnings # Lints (CI: Warnungen sind Fehler)
```

Hinweis: `tauri::generate_context!` erwartet das Verzeichnis `dist/`. Vor `cargo test` ohne vorherigen Frontend-Build zuerst `npm run build` ausführen (so macht es auch die CI).

Gestaltung (Tokens, Farben hell/dunkel, Bausteine, Layout): [`docs/design/DESIGN.md`](docs/design/DESIGN.md), Vorschau in `docs/design/design-preview.html`.

## 5. Mock-Backend für die Frontend-Entwicklung

Die Oberfläche lässt sich im Browser entwickeln, ohne Tauri, Docker oder KI. Das Backend wird dann von `src/dev/mockBackend.ts` simuliert. Es wird nur im Entwicklungsmodus geladen (`import.meta.env.DEV`) und ist nicht Teil von Produktions-Builds.

```sh
VITE_MOCK=1 npm run dev    # dann http://localhost:1420/ im Browser öffnen
```

Mit URL-Parametern lassen sich Zustände erzwingen:

- Engine: `?engine=missing` oder `?engine=stopped`
- Updates: `?update=available|none|error|progress|notConfigured|portable`

Unter Windows (PowerShell) wird die Umgebungsvariable anders gesetzt, z. B. `$env:VITE_MOCK=1; npm run dev`.

## 6. Runner-Images bauen und lokal nutzen

Die Images stehen in `images/python/Dockerfile` und `images/java/Dockerfile`. Basis-Images sind per Digest gepinnt (Updates über Dependabot).

```sh
docker build -t code-pruefer-python:dev images/python
docker build -t code-pruefer-java:dev images/java

# App mit den lokalen Images starten (Umgebungsvariablen und :dev gelten nur in Debug-Builds)
CODE_PRUEFER_PYTHON_IMAGE=code-pruefer-python:dev \
CODE_PRUEFER_JAVA_IMAGE=code-pruefer-java:dev \
npm run tauri dev
```

**Reihenfolge der Image-Auflösung** (`runner/images.rs`):

1. nur Debug-Builds: Umgebungsvariable `CODE_PRUEFER_PYTHON_IMAGE` / `CODE_PRUEFER_JAVA_IMAGE` (exklusiv: lokal vorhanden, sonst Download)
2. das gepinnte Image `ghcr.io/bernhardgoetz/code-pruefer-<sprache>:<App-Version>` ist in der Engine bereits vorhanden
3. Laden aus dem mitgelieferten Archiv (`POST /images/load`; Archiv `images/code-pruefer-<sprache>-<arch>.tar.gz` im Ressourcenordner)
4. Download von GHCR
5. nur Debug-Builds: lokal gebautes `code-pruefer-<sprache>:dev`
6. sonst eine deutsche Fehlermeldung mit Lösungsweg

Release-Builds beachten weder die Umgebungsvariablen noch `:dev`-Images.

**Prüfsummen (`images/image-digests.json`):** Builds mit eingebetteten Images enthalten diese Datei (erzeugt von `scripts/export-image-archives.sh`). Sie enthält je Sprache und Architektur die SHA-256 des Archivs und die Image-IDs, bei Release-Builds auch den Registry-Digest. Ist sie vorhanden, prüft die App Archive vor dem Laden und Image-IDs danach und lädt per `@sha256:…` statt per Tag. Ohne die Datei (Entwicklungs- und CI-Builds) gilt Vertrauen über den Tag.

**Image-Referenz der App:** `<IMAGE_REGISTRY>/code-pruefer-<sprache>:<version>` mit `IMAGE_REGISTRY = ghcr.io/bernhardgoetz` (`runner/languages.rs`) und `<version>` = `CARGO_PKG_VERSION`.

## 7. KI lokal entwickeln und testen

Für Entwickler-Builds: llama-server für Linux holen und per Umgebungsvariable setzen (gilt nur in Debug-Builds). Das Modell danach in der App unter „KI-Modell verwalten …“ importieren oder herunterladen.

```sh
scripts/fetch-llama-server.sh linux-x64 /tmp/llama
CODE_PRUEFER_LLAMA_SERVER=/tmp/llama/llama-server npm run tauri dev
```

Plattformen des Skripts: `win-x64`, `macos-arm64`, `macos-x64`, `linux-x64`. Die llama.cpp-Version (`LLAMA_CPP_VERSION` in `src-tauri/src/ai/llama.rs`) und die Prüfsummen im Skript müssen zusammenpassen (Unit-Test).

Integrationstests mit echtem llama-server und beliebigem GGUF (ideal: Qwen2.5-Coder-1.5B):

```sh
cd src-tauri && CODE_PRUEFER_LLAMA_SERVER=/tmp/llama/llama-server \
  CODE_PRUEFER_TEST_MODEL=/pfad/zum/modell.gguf \
  cargo test --test ai_integration -- --ignored --nocapture --test-threads=1
```

## 8. Tests und CI

**Unit-Tests:** `cd src-tauri && cargo test`

**Integrationstests** (mit `#[ignore]` markiert, brauchen Docker bzw. KI):

```sh
# Docker + lokal gebaute :dev-Images (siehe Abschnitt 6)
cd src-tauri && cargo test -- --ignored --test-threads=1
# oder einzeln:
cd src-tauri && cargo test --test docker_integration -- --ignored --test-threads=1
cd src-tauri && cargo test --test image_bundle -- --ignored --test-threads=1   # Laden aus Archiv
cd src-tauri && cargo test --test ai_integration -- --ignored --nocapture --test-threads=1
```

**Wichtige Konsistenz-Tests:**
- `versions_are_in_sync` (`runner/languages.rs`): Version in `Cargo.toml`, `tauri.conf.json` und `package.json` muss identisch sein.
- Ein Test in `lib.rs` gleicht die erlaubten URLs mit `capabilities/default.json` ab.
- Ein Test prüft, dass die llama.cpp-Version in `llama.rs` zu `scripts/fetch-llama-server.sh` passt.

**CI** (`.github/workflows/ci.yml`, bei Push auf `main`/`qas`, bei Pull Requests und manuell):
- Job `frontend`: `npm ci`, `npm run build` (Typecheck + Build)
- Job `rust`: Tauri-Systemabhängigkeiten, `npm ci && npm run build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, Runner-Images lokal bauen, danach `docker_integration` und `image_bundle` mit `--ignored`

`.github/workflows/images.yml` baut die Images bei Änderungen unter `images/` nur als `:edge` / `:sha-…` (Versions-Tags kommen ausschließlich aus dem Release-Workflow).

## 9. Installer lokal bauen

Normale Builds (`npm run tauri build`, CI) enthalten keine Images und keine KI-Laufzeit und bleiben klein; die App holt dann Images über GHCR (bzw. `:dev` in Debug-Builds).

**Lokaler Build mit eingebetteten Images und KI-Laufzeit** (das Skript schreibt auch `image-digests.json`, aber ohne Registry-Digest: die App prüft die Archive, lädt aber nichts herunter):

```sh
scripts/export-image-archives.sh amd64     # -> src-tauri/resources/images/ (Archive + image-digests.json, nicht eingecheckt)
scripts/fetch-llama-server.sh win-x64      # -> src-tauri/resources/llama/ (bzw. macos-arm64 / macos-x64)
npm run tauri build -- --config src-tauri/tauri.release.conf.json
```

**Windows-Installer unter Linux cross-kompilieren** (benötigt `mingw-w64`, `nsis`, `unzip`, `jq`, Rust-Target `x86_64-pc-windows-gnu` und einen laufenden Docker-Daemon für `export-image-archives.sh`):

```sh
rustup target add x86_64-pc-windows-gnu
scripts/export-image-archives.sh amd64
scripts/fetch-llama-server.sh win-x64
npm run tauri build -- --target x86_64-pc-windows-gnu --bundles nsis --config src-tauri/tauri.release.conf.json
# Ergebnis: src-tauri/target/x86_64-pc-windows-gnu/release/bundle/nsis/
```

Abweichungen eines solchen Cross-Builds vom offiziellen Release: Die Visual-C++-Laufzeit-DLLs fehlen (die KI startet ohne installierte VC++-Laufzeit nicht), das KI-Modell ist nicht enthalten (die App lädt es selbst), der Installer ist unsigniert, und es wird mit der GNU-Toolchain statt mit MSVC gebaut.

**Portable Windows-Version:** Die Ordner `images` (mit den Archiven und `image-digests.json`) und `llama` (llama-server und DLLs) neben `code-pruefer.exe` legen. Unter Windows ist der Ressourcenordner immer der Ordner der Exe. Diese Version aktualisiert sich nicht selbst.

**Windows-Installer-Anpassungen:** `src-tauri/windows/hooks.nsh` (Seite „Container-Engine“ mit Docker-Desktop/Podman-Auswahl, winget-Installation auf Nachfrage; winget wird nur über den absoluten Pfad `%LOCALAPPDATA%\Microsoft\WindowsApps\winget.exe` gestartet) und `src-tauri/windows/installer.nsi`. Letzteres ist das unveränderte Tauri-Template (tauri-cli 2.12.4) mit **einer** Ergänzung vor der Installationsseite (Makro `CODE_PRUEFER_CUSTOM_PAGES`). Bei einem Update der Tauri-CLI das Template neu kopieren und diese Stelle wieder einfügen. Bei stiller/passiver Installation (`/S`, `/P`) wird nichts gefragt und nichts nachinstalliert.

Ressourcenordner zur Laufzeit: Windows = Ordner der `code-pruefer.exe`; macOS = `Code-Prüfer.app/Contents/Resources/`.

## 10. Release-Workflow

Ein Release läuft über Pull Requests und eine manuelle Abnahme („QAS“). Ausführliche Beschreibung: [`docs/RELEASE_QAS.md`](docs/RELEASE_QAS.md); Einrichtung Schritt für Schritt: [`docs/VEROEFFENTLICHUNG.md`](docs/VEROEFFENTLICHUNG.md).

**Ablauf in Kurzform:**

1. **Version erhöhen** in `package.json`, `src-tauri/tauri.conf.json` und `src-tauri/Cargo.toml` (alle drei gleich, sonst bricht der Test `versions_are_in_sync` bzw. der Workflow ab). `CHANGELOG.md` ergänzen und für das In-App-Update einen Hinweistext `docs/updates/<version>.txt` anlegen (sonst nutzt der Workflow einen Standardtext).
2. Committen und einen **Pull Request nach `qas`** öffnen (nur Branches dieses Repositorys, keine Forks).
3. Jeder Push auf den PR startet `release.yml` (ein laufender Lauf wird abgebrochen). Der Workflow
   - prüft PR, Versionen, Registry-Namensraum, dass `v<version>` noch frei ist und dass der Modellkatalog gepinnt und korrekt ist,
   - baut die Images multi-arch (`linux/amd64`, `linux/arm64`, ohne Build-Cache), pusht sie als `:<version>-qas.<n>` und `:sha-<commit>` nach GHCR,
   - exportiert daraus per Digest die Archive je Architektur samt `image-digests.json`,
   - baut den Windows-Installer (windows-latest, MSVC, NSIS deutsch) und die macOS-DMGs (Apple Silicon und Intel) mit eingebetteten Images und llama-server,
   - legt ein **Pre-Release** `v<version>-qas.<n>` zum Testen an und verlinkt es im PR.
4. **Abnahme:** Nach dem Testen „Approve and deploy“ im Environment `production`. Danach wird **genau dieser Build** befördert (nichts wird neu gebaut): Images per Digest auf `:<version>` umtaggen, Windows mit Release-Signing neu bündeln, Updater-Signaturen erzeugen, Tag `v<version>` auf dem getesteten PR-Commit und ein **Entwurfs-Release** mit `latest.json`, `SHA256SUMS.txt` und Build-Provenance-Attestierung anlegen. Nach der Abnahme nicht mehr auf den PR pushen.
5. PR in `qas` **mergen (Merge-Commit, kein Squash/Rebase)**. Daraufhin legt `release-qas.yml` den PR `qas` → `main` an.
6. Dessen Merge nach `main` veröffentlicht das Release (`release-main.yml`, als „latest“ markiert).

Direkte Pushes auf `qas` bauen nichts. Einen Tag `v*` von Hand zu pushen startet keinen Release.

**Workflows:**

| Datei | Zweck |
|---|---|
| `ci.yml` | Tests (siehe Abschnitt 8) |
| `images.yml` | Images als `:edge` / `:sha-…` bei Änderungen unter `images/` |
| `release.yml` | PR nach `qas`: QAS-Lauf, Abnahme, Beförderung |
| `release-qas.yml` | Nach Merge in `qas`: PR `qas` → `main` anlegen |
| `release-main.yml` | Nach Merge in `main`: Entwurfs-Release veröffentlichen |
| `pin-models.yml` | Modellkatalog pinnen (manuell gestartet, legt einen PR an) |

**Signierung:**
- **Windows** über die SignPath Foundation (Antrag: siehe [`docs/CODE_SIGNING_POLICY.md`](docs/CODE_SIGNING_POLICY.md); bis zur Genehmigung bleibt Windows unsigniert). Test-Signing läuft im Environment `qas`, Release-Signing im Environment `release`. Windows wird in Jobs aufgeteilt: `build-windows` kompiliert, `bundle-windows` (QAS) und `promote-windows` (nach Abnahme) bündeln mit `tauri bundle` und signieren in Runden (`scripts/signpath-stage.mjs`).
  - **Signiert werden ausschließlich selbst gebaute Dateien:** `code-pruefer.exe`, `uninstall.exe` und der Installer `*-setup.exe` (Artefaktkonfiguration `.signpath/artifact-configurations/default.xml`). Pro Bündelung gibt es zwei Signaturaufträge: (1) `code-pruefer.exe` und Uninstaller, (2) der Installer.
  - **Nicht signiert werden Fremdbinaries:** die unveränderte llama.cpp-Laufzeit (`llama/*.exe`, `llama/*.dll`) und die NSIS-Plugins. Das ist beabsichtigt; die Bedingungen der SignPath Foundation erlauben nur das Signieren selbst gebauter Software. Die Microsoft-Visual-C++-DLLs behalten ihre Microsoft-Signatur. Der Schritt „Signaturen prüfen“ in `release.yml` meldet unsignierte Fremdbinaries als Warnung, eigene unsignierte Dateien als Fehler. Smart App Control kann unsignierte DLLs blockieren; das ist eine bekannte Einschränkung (siehe [`docs/RELEASE_SIGNING.md`](docs/RELEASE_SIGNING.md)).
  - Kommt eine neue exe/dll in den Installer, bricht der Job mit „unbekannte unsignierte Datei“ ab: Zuordnung in `scripts/signpath-stage.mjs` (Funktion `classify`) ergänzen. Neue eigene Dateien zusätzlich in die Artefaktkonfiguration aufnehmen.
  - Ohne `SIGNPATH_API_TOKEN` und die SignPath-Variablen bleibt Windows unsigniert (mit Warnung). Siehe [`docs/RELEASE_SIGNING.md`](docs/RELEASE_SIGNING.md).
- **macOS:** Signatur/Notarisierung ist vorbereitet, läuft aber nur, wenn das Secret `APPLE_CERTIFICATE` (plus `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`) im Environment `qas` gesetzt ist.
- **App-Updater:** Die Repository-Variable `TAURI_UPDATER_PUBKEY` und im Environment `release` die Secrets `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Ohne sie entstehen Releases ohne Updater. Details: [`docs/UPDATER.md`](docs/UPDATER.md).

**Versionsprüfung:** Der Job `version` bricht ab, wenn `Cargo.toml`, `tauri.conf.json` und `package.json` nicht dieselbe Version haben, der Registry-Namensraum nicht zum Repository-Inhaber passt oder `v<version>` schon als Tag/Release existiert (`scripts/check-version-unreleased.sh`).

**Modell-Pinning:** Revision, Dateien, Größe und SHA-256 in `src-tauri/src/ai/catalog.json` werden **nie von Hand** und **nicht im Release** eingetragen. Sie entstehen über den Workflow `pin-models.yml` (Actions → „KI-Modellkatalog pinnen“ → „Run workflow“): Er ruft `scripts/pin-models.sh` auf und legt einen Pull Request an, den der Inhaber auf huggingface.co prüft und mergt. Jeder Release vergleicht danach nur noch (`scripts/pin-models.sh --check` und `--verify`) und bricht bei fehlenden Pins oder Abweichungen zur Hugging-Face-API ab. Solange ein Eintrag nicht gepinnt ist, verweigert die App den Download (Import einer `.gguf` bleibt möglich).

**Einmalige GitHub-Einstellungen** (durch den Repository-Inhaber; Details in `docs/VEROEFFENTLICHUNG.md`):
1. Environments `qas`, `production` (Pflicht-Reviewer für die Abnahme) und `release` anlegen. Apple-Secrets und SignPath-Test-Signing gehören nach `qas`, SignPath-Release-Signing und Updater-Schlüssel nach `release`.
2. Branches `qas` und `main` schützen (nur Pull Requests, nur Merge-Commits, keine Force-Pushes); Tag-Regeln für `v*`.
3. „Allow GitHub Actions to create and approve pull requests“ aktivieren (für den PR `qas` → `main`).
4. GHCR-Pakete `code-pruefer-python` und `code-pruefer-java` auf **public** stellen, sonst funktionieren Download und „Images aktualisieren“ nicht (nach dem ersten Push sind sie standardmäßig privat).

**Supply-Chain-Maßnahmen:** Basis-Images und GitHub Actions sind per Digest bzw. Commit-SHA gepinnt (Dependabot, 7 Tage Wartezeit), Release-Jobs laufen ohne Caches und mit minimalen Token-Rechten, und die Jobs prüfen die Manifeste (SHA-256) ihrer Artefakte gegenseitig (`scripts/artifact-manifest.sh`). Prüfung eines Release-Artefakts: `gh attestation verify <Datei> --repo BernhardGoetz/CodeChecker`.

## 11. Konventionen

Diese Regeln stammen aus `CLAUDE.md` bzw. dem bestehenden Code:

- **Sprache:** Oberfläche, Fehlermeldungen und Dokumentation sind **deutsch** (Code und Code-Kommentare überwiegend englisch, Kommentare zu Workflows/Skripten deutsch). Neue Benutzermeldungen auf Deutsch formulieren.
- **Drei Versionsfelder synchron halten** (`package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`).
- **Lints müssen sauber sein:** `cargo clippy --all-targets -- -D warnings`, `npm run build` (inkl. Typecheck).
- **Entwickler-Schalter nur in Debug-Builds:** Umgebungsvariablen wie `CODE_PRUEFER_*_IMAGE`, `CODE_PRUEFER_LLAMA_SERVER` und `:dev`-Images wirken ausschließlich bei `debug_assertions`. Release-Builds ignorieren sie.
- **Die KI entscheidet nichts:** Erwartete Ausgaben und Logikfehler-Befunde entstehen ausschließlich durch das Ausführen von Code in der Sandbox. Erklärungen der KI sind nur Hinweise.
- **Sicherheit der Sandbox nicht aufweichen:** Netzwerk aus, read-only, Ressourcenlimits, nicht-root (siehe Abschnitt 1). Geprüfter Code, Aufgabenblätter und Modelldateien gelten als nicht vertrauenswürdig.
- **Netzwerkzugriffe sind abschließend aufgelistet** (Tabelle in Abschnitt 15). Neue Verbindungen nur mit ausdrücklicher Begründung und Dokumentation (auch in `docs/CODE_SIGNING_POLICY.md`, die die Datenschutzzusage der App wiedergibt).
- **Frontend darf nur wenige URLs öffnen:** Änderungen an den Links in `DockerBanner.tsx` bzw. an `RELEASES_URL` in `app_update.rs` (von `UpdateSettings.tsx` genutzt) müssen mit `capabilities/default.json` übereinstimmen (Test in `lib.rs`).
- **Neue Abhängigkeiten** werden mit exakter Version in `package.json` eingetragen (so ist es jetzt) und über Dependabot aktuell gehalten.
- **Changelog pflegen** (`CHANGELOG.md`, Abschnitt „Unveröffentlicht“; Keep-a-Changelog-Format, Semantic Versioning).
- **Hinweis für KI-Assistenten (`CLAUDE.md`):** Der Inhaber baut die App nicht selbst. Nach jeder committeten und gepushten Code-Änderung wird eine **Windows-Testversion per Cross-Build im Linux-Container** gebaut (Ablauf wie in Abschnitt 9). Der Assistent liefert die `.exe` mit Commit-Hash im Dateinamen (z. B. `Code-Pruefer_test_<hash>_x64-setup.exe`), nennt SHA-256, enthaltene bzw. fehlende Ressourcen, die genannten Abweichungen (keine VC++-Laufzeit, kein Modell, unsigniert, GNU-Toolchain) und den SmartScreen-Hinweis („Weitere Informationen“ → „Trotzdem ausführen“). Schlägt der Build fehl, wird das ehrlich mit der Ursache gemeldet; der Nutzer wird nicht auf lokale Build-Schritte verwiesen.

## 12. Eine neue Programmiersprache hinzufügen

Die Sprachtabelle ist datengetrieben (`src-tauri/src/runner/languages.rs`). Sie ist der zentrale Ausgangspunkt, aber nicht der einzige Ort. Diese Stellen sind im Code erkennbar:

1. **Dockerfile** unter `images/<id>/Dockerfile` anlegen. Muster (siehe `images/python`, `images/java`): Basis-Image per Digest pinnen, unprivilegierten Benutzer `runner` mit UID/GID 1000 anlegen, `/work` anlegen, `USER 1000:1000`, `WORKDIR /work`, `CMD ["sleep", "infinity"]`.
2. **Eintrag in `LANGUAGES`** (`runner/languages.rs`): `id`, `name` (Anzeigename), `default_image` (`ghcr.io/bernhardgoetz/code-pruefer-<id>:<Cargo-Version>`), `image_env` (z. B. `CODE_PRUEFER_<ID>_IMAGE`), `local_image` (`code-pruefer-<id>:dev`; ein Test erzwingt dieses Schema), `source_file`, `extensions`, optional `compile_cmd`, `run_cmd`, `compile_timeout`, `run_timeout` und `template` (Startcode im Editor). Optional eine sprachspezifische Vorprüfung in `precheck`.
3. **CI und Release:** Die Sprachen sind in den Workflows als Matrix aufgelistet: `language: [python, java]` in `.github/workflows/images.yml` und in `.github/workflows/release.yml` (Job `images`; dessen Outputs `digests_<sprache>`, `amd64_<sprache>`, `arm64_<sprache>` und `image_<sprache>` sind je Sprache einzeln benannt und müssen mitgezogen werden). In `release.yml` verwenden außerdem diese Jobs die Outputs je Sprache als Umgebungsvariablen (`AMD64_PYTHON`/`AMD64_JAVA`, `DIGESTS_PYTHON`/`DIGESTS_JAVA`, `ARCHIVE_PYTHON`/`ARCHIVE_JAVA`, `IMAGE_PYTHON`/`IMAGE_JAVA`) und übergeben sie an `scripts/artifact-manifest.sh verify`: `bundle-windows` und damit `promote-windows` (gleiche Schritte), `bundle-macos` und `promote-images` (dort zusätzlich die Zuordnung `IMAGE_<SPRACHE>` beim Umtaggen). `ci.yml` baut die `:dev`-Images je Sprache einzeln (Schritt „Runner-Images lokal bauen“); `scripts/export-image-archives.sh` verwendet standardmäßig `LANGUAGES="python java"` (`LANGUAGES` lässt sich per Umgebungsvariable überschreiben); `.github/dependabot.yml` führt die Image-Verzeichnisse in der Liste `directories` auf.
4. **Frontend:** Die Sprachliste kommt vom Backend (`list_languages`). Der Monaco-Editor erhält die Sprach-`id` direkt als Sprache (`CodeEditor.tsx`), die `id` sollte deshalb ein von Monaco bekannter Sprachname sein. Java-spezifische Hinweise im Editor (`language?.id === "java"`) sind fest eingebaut.
5. **Variationstest:** `ai/variation.rs` kennt Wertersetzung nur für Python und Java (`language == "java"`, sonst Python-Regeln). Für eine neue Sprache entweder die Ersetzung erweitern oder bewusst auf Python-Regeln verzichten und das testen.
6. **KI-Prompts:** `ai/prompts.rs` und `ai/pipeline.rs` verwenden die Sprachkonfiguration (`languages::get`). `ai/prompts.rs` unterscheidet aber fest nur `"java"` und behandelt jede andere Sprache wie Python: `language_rules` (Zeile 17, Vorgaben „Java 21“ bzw. „Python 3.12“), `fence` (Zeile 33), `looks_like_code` (Zeile 293) und die Codeblock-Kennungen in `extract_code` (Zeile 305). Für eine neue Sprache müssen diese Stellen ergänzt werden.
7. **Mock-Backend:** `src/dev/mockBackend.ts` führt eine eigene Sprachliste; bei Bedarf ergänzen.
8. **Tests und Doku:** Integrationstests in `src-tauri/tests/` ergänzen, `README.md` anpassen.

## 13. Ein neues KI-Modell hinzufügen

Modelle stehen in `src-tauri/src/ai/catalog.json`; der Katalog wird per `include_str!` in die App eingebaut.

1. **Eintrag in `catalog.json`** ergänzen mit den Feldern `id`, `name`, `repo` (Hugging-Face-Repository mit GGUF-Dateien), `quant` (z. B. `q4_k_m`; der Importdialog erwartet eine Datei mit `*<quant>*.gguf`), `license`, `solutions` (Anzahl der Kandidaten-Lösungen pro Aufgabe) sowie `revision` und `files` (Name, Größe, SHA-256).
2. **Pinnen:** `revision`, `files[].size` und `files[].sha256` **nicht von Hand** eintragen, sondern den Workflow `pin-models.yml` starten (er nutzt `scripts/pin-models.sh`) und den entstehenden PR auf huggingface.co prüfen und mergen (siehe Abschnitt 10). Aufgeteilte GGUF-Dateien (`…-00001-of-0000N.gguf`) werden mit allen Teilen eingetragen; llama.cpp lädt über den ersten Teil.
3. **Lizenz prüfen:** Nur Modelle mit Lizenz, die die Nutzung erlaubt (die bisherigen: Apache-2.0). Qwen2.5-Coder-3B fehlt absichtlich, weil seine Lizenz (Qwen Research License) keine kommerzielle Nutzung erlaubt.
4. **Hardware-Empfehlung:** Die Auswahl „empfohlen“ ist in `ai/models.rs` fest verdrahtet: `SMALL_MODEL` (`qwen2.5-coder-1.5b`) unter `SMALL_MODEL_RAM_BYTES` (12 GiB RAM), sonst `LARGE_MODEL` (`qwen2.5-coder-7b`). Ein neues Modell wird also in der Liste angezeigt, aber nur dann empfohlen, wenn diese Konstanten angepasst werden. Warnungen zu wenig RAM (unter 6 GiB) und fehlendem AVX2 stehen in `hardware_warnings`.
5. **Chat-Vorlage:** Modelle ohne geprüfte Prüfsumme laufen mit der eingebauten ChatML-Vorlage statt der Vorlage aus der Datei. Ein neues Modell mit anderem Prompt-Format braucht daher ggf. Anpassungen in `ai/llama.rs`/`ai/prompts.rs`. Gepinnte Katalogmodelle mit geprüfter SHA-256 nutzen die Jinja-Vorlage aus der GGUF-Datei (`ChatTemplate::FromModel`, Auswahl in `ai/models.rs`, `chat_template()`), alle anderen die eingebaute ChatML-Vorlage (`--chat-template chatml`); ob andere Modellfamilien als Qwen damit funktionieren, ist ungeprüft.
6. **Tests:** Die Unit-Tests in `ai/models.rs` referenzieren `SMALL_MODEL`/`LARGE_MODEL`; `ai_integration` kann mit `CODE_PRUEFER_TEST_MODEL` gegen ein beliebiges GGUF laufen.
7. **Dokumentation:** Die Tabelle „Welches KI-Modell passt zu meinem Rechner?“ in der `README.md` anpassen.

Bevor ein Katalogeintrag gepinnt ist, kann man Dateien weiterhin über „Datei importieren …“ einlesen: Ist die SHA-256 unbekannt, wird die Datei erst nach ausdrücklicher Bestätigung übernommen und als „nicht geprüft“ bzw. „unbekanntes Modell“ markiert (Ablage unter `models/unverified/<id>/`). Modelle liegen im App-Datenordner (`models/`), die Auswahl in `ai.json` im Konfigurationsordner.

## 14. Beitragsablauf (Branch und Pull Request)

**Regeln für Beiträge:**

- Externe Beiträge werden nur als **Pull Request** angenommen.
- Jeder Pull Request wird vor dem Merge vom Maintainer (Rolle Reviewer, siehe [`docs/CODE_SIGNING_POLICY.md`](docs/CODE_SIGNING_POLICY.md)) geprüft, auch Änderungen an Workflows und Skripten.
- Release-Pull-Requests nach `qas` kommen nur aus Branches dieses Repositorys, nicht aus Forks.

1. **Issue** anlegen oder bestehendes Issue wählen, besonders bei größeren Änderungen vorher kurz abstimmen.
2. Repository klonen (bzw. als Mitglied einen **Branch** im Repository anlegen), z. B. `feature/<kurzname>` oder `fix/<kurzname>`.
3. Änderung umsetzen. Vorher lokal prüfen:
   ```sh
   npm run build
   cd src-tauri && cargo test && cargo clippy --all-targets -- -D warnings
   ```
   Bei Änderungen am Runner zusätzlich die Integrationstests mit Docker (Abschnitt 8).
4. `CHANGELOG.md` (Abschnitt „Unveröffentlicht“) und ggf. `README.md` anpassen.
5. **Committen** mit einer aussagekräftigen Nachricht und pushen.
6. **Pull Request** öffnen. Die CI (`ci.yml`) läuft bei jedem Pull Request. Für normale Änderungen ist das Ziel `main` bzw. der von den Verantwortlichen vorgegebene Branch.
7. **Release-Pull-Requests** gehen ausschließlich nach `qas` und lösen den QAS-Lauf aus (Abschnitt 10). Forks lösen keinen Release-Lauf aus (sie bekämen auch keine Secrets). Wer das Release auslöst, ist verantwortlich für die Abnahme.
8. Wer einen Release-PR abnimmt, prüft vorher Änderungen an `.github/` und `scripts/` besonders sorgfältig (ein PR nutzt seine eigene Fassung der Workflow-Dateien).

## 15. Sicherheit

- **Bedrohungsmodell:** Geprüfter Code ist nicht vertrauenswürdig und läuft nur im Container. Die App und die Container-Engine gelten als vertrauenswürdig (wer die Engine steuert, hat faktisch Administratorrechte). Aufgabenblätter und Modelldateien sind nicht vertrauenswürdig: Dokumente werden in einem eigenen, begrenzten Hilfsprozess gelesen (1,5 GB Arbeitsspeicher, Zeitlimit, begrenzte Ausgabe; PDFs werden vorher auf Dekompressionsbomben geprüft), importierte `.gguf`-Dateien auf plausible Metadaten.
- **Datenschutz und Netzwerkzugriffe:** Die App hat keine Konten, keine Telemetrie, keine Absturzberichte und sendet weder Code noch Aufgaben oder Testfälle an Dritte. Netzwerk wird nur hier genutzt (mit dem Code abgeglichen):

  | Wann | Wohin | Was |
  |---|---|---|
  | „Herunterladen“ eines KI-Modells | `huggingface.co` (und `*.huggingface.co`, `hf.co`, `*.hf.co`) | nur HTTPS, gepinnte Datei, SHA-256 geprüft (`ai/models.rs`) |
  | Runner-Images fehlen bzw. „Images aktualisieren“ | `ghcr.io`, über Docker/Podman | Pull der gepinnten Images (`runner/images.rs`) |
  | Update-Suche (nur Installer-Version, abschaltbar) | `github.com` (`releases/latest/download/latest.json`, beim Update der Installer) | signaturgeprüft (`app_update.rs`, `docs/UPDATER.md`) |
  | Podman gewählt und von der App gestartet, erstes Mal | über Podman: Download des Podman-VM-Images (`podman machine init`) | einmalig, einige hundert MB (`engine.rs`) |
  | Windows-Installer, nur auf Nachfrage | über winget: Microsoft-winget-Quelle und Download von Docker Desktop bzw. Podman | nur wenn die Engine fehlt und „Ja“ gewählt wird (`windows/hooks.nsh`) |
  | Klick auf Download-/Release-Links | Browser: docker.com, podman.io, GitHub-Release-Seite | nur diese drei Adressen (Capability) |

  Alles andere bleibt lokal: die Container-Engine (Socket bzw. Named Pipe), llama-server auf `127.0.0.1` (startet mit `--offline`), die Container laufen **ohne Netzwerk**.
- **Engine-Auswahl und Verbindung:** Die Engine wird in dieser Reihenfolge bestimmt: (1) `engine.json` im App-Konfigurationsordner (Windows: `%APPDATA%\de.bernhardgoetz.codepruefer\engine.json`, Inhalt z. B. `{"engine": "podman"}`), (2) Registry `HKCU\Software\Code-Pruefer`, Wert `Engine` (vom Installer geschrieben), (3) Standard Docker. Die Umgebungsvariable `DOCKER_HOST` hat Vorrang vor allem (ein `tcp://`- oder `http://`-Endpunkt ist unverschlüsselt; die App zeigt dann eine Warnung). Beim Start (und bei Bedarf vor einer Prüfung) startet die App die gewählte Engine und wartet, bis sie antwortet (alle 2 s, höchstens 120 s): Docker Desktop über `Docker Desktop.exe` bzw. `open -a /Applications/Docker.app`; Podman über `podman machine init` (einmalig) und `podman machine start`. Unter Linux gibt es keinen automatischen Start.
- **Bekanntes Restrisiko (Windows):** Die Named Pipes `docker_engine` und `podman-machine-default` gibt es nur einmal pro Rechner; die App prüft den Prozess hinter der Pipe nicht. Der Code-Prüfer ist für Einzelplatz-Rechner gedacht. Eine frühere Prüfung (`GetNamedPipeServerProcessId`) wurde wieder entfernt (ungetestet, Nebenwirkungen).
- **Weitere Restrisiken:** gemeinsamer Kernel der Container (Engine aktuell halten), `image-digests.json` liegt neben den Archiven im Programmordner (Schutz nur durch signierte Installer), Hintergrundprozesse aus Tests bleiben als Zombies bis zum Ende der Prüfung, und ohne Pflicht-Reviewer im Environment `production` kann jeder mit Schreibrecht per PR nach `qas` einen Release auslösen.
- **Sicherheitslücken** bitte nicht öffentlich als Issue melden, sondern vertraulich über GitHub („Private vulnerability reporting“, Reiter Security → „Report a vulnerability“); Einzelheiten in [`SECURITY.md`](SECURITY.md).

## 16. Weitere Dokumente

| Datei | Inhalt |
|---|---|
| [`docs/RELEASE_QAS.md`](docs/RELEASE_QAS.md) | Release mit QAS-Stufe und Abnahme, GitHub-Einstellungen |
| [`docs/VEROEFFENTLICHUNG.md`](docs/VEROEFFENTLICHUNG.md) | Veröffentlichung einrichten, Schritt für Schritt |
| [`docs/RELEASE_SIGNING.md`](docs/RELEASE_SIGNING.md) | Windows-Signatur über die SignPath Foundation |
| [`docs/CODE_SIGNING_POLICY.md`](docs/CODE_SIGNING_POLICY.md) | Code signing policy mit Rollen und Datenschutz (englisch, verbindlich; Status: Antrag bei der SignPath Foundation in Vorbereitung) |
| [`docs/UPDATER.md`](docs/UPDATER.md) | App-Updater: Ablauf, Schlüssel, Einrichtung, Test |
| [`docs/design/DESIGN.md`](docs/design/DESIGN.md) | Gestaltung: Tokens, Farben, Bausteine, Layout |
| [`docs/ENTSCHEIDUNGSGRUNDLAGE_lokale-KI-Aufgabenchecker.md`](docs/ENTSCHEIDUNGSGRUNDLAGE_lokale-KI-Aufgabenchecker.md) | Recherche zu lokaler KI, OCR und Testfall-Erzeugung (keine Spezifikation; Abschnitte 8 und 10 gelten für diese App nicht) |
| [`CHANGELOG.md`](CHANGELOG.md) | Änderungen je Version |
| [`THIRD_PARTY_NOTICES.txt`](THIRD_PARTY_NOTICES.txt) | Lizenzen mitgelieferter Drittsoftware |
| [`SECURITY.md`](SECURITY.md) | Sicherheitslücken vertraulich melden |
