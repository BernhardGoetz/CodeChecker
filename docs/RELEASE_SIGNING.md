# Windows-Signatur über die SignPath Foundation

Ziel: Der Windows-Installer und die App sind mit einem vertrauenswürdigen
Code-Signing-Zertifikat signiert, damit SmartScreen und Smart App Control (SAC) sie
akzeptieren. Die [SignPath Foundation](https://signpath.org/) stellt Open-Source-Projekten
kostenlos ein Zertifikat zur Verfügung; signiert wird über [SignPath.io](https://about.signpath.io/)
direkt aus GitHub Actions heraus. Ein exportierbares `.pfx`-Zertifikat gibt es dafür nicht
(und bei aktuellen Code-Signing-Zertifikaten generell nicht mehr: der Schlüssel muss in
einem HSM liegen). Der frühere PFX-Weg (`WINDOWS_CERTIFICATE`) ist deshalb entfernt.

Kennzeichnung: **[geprüft]** = im Code/an der Tauri-CLI nachvollzogen oder lokal getestet,
**[ungeprüft]** = aus Doku/Erinnerung, konnte hier nicht verifiziert werden (signpath.org
und docs.signpath.io waren aus der Entwicklungsumgebung nicht erreichbar) – bitte beim
Einrichten gegenprüfen.

## Was signiert wird – und was nicht

| Datei | Herkunft | Signatur |
|---|---|---|
| `Code-Pruefer_<version>_x64-setup.exe` (Release-Asset; lokal gebaut `Code-Prüfer_…`) | NSIS-Installer, im Workflow gebaut | SignPath |
| `code-pruefer.exe` | die App, im Workflow gebaut | SignPath |
| `uninstall.exe` | von makensis aus `src-tauri/windows/installer.nsi` erzeugt | SignPath |
| `llama/llama-server.exe`, `llama/*.dll` (ggml, llama, mtmd, libomp) | unverändert aus dem llama.cpp-Release | nicht signiert (bewusst) |
| NSIS-Plugins (`System.dll`, `nsDialogs.dll`, `nsExec.dll`, `StartMenu.dll`, `NSISdl.dll`, `nsis_tauri_utils.dll`), zur Laufzeit in `$PLUGINSDIR` entpackt | NSIS bzw. Tauri | nicht signiert (bewusst) |
| `llama/msvcp140.dll`, `vcruntime140*.dll` | Microsoft, bereits signiert | bleibt Microsoft-signiert |

`WebView2Loader.dll` gibt es im MSVC-Build nicht (statisch gelinkt). **[geprüft]**

### Fremdbinaries (llama.cpp, NSIS-Plugins)

Fremdbinaries werden nicht signiert. Die Bedingungen der SignPath Foundation
(<https://signpath.org/terms>, Abschnitt „Sign your own binaries only“) sagen: „The team
must only sign software artifacts built from their own source code.“ Unsignierte
Binaries anderer OSS-Projekte (z. B. DLL-Dateien) dürfen laut demselben Abschnitt in
signierten Paketen wie Installern enthalten sein. **[Wortlaut am 2026-11-06 auf
signpath.org/terms nachgelesen]**

Folge: Der Workflow signiert nur die drei eigenen Dateien. Die llama.cpp-Laufzeit und die
NSIS-Plugins bleiben unsigniert. Smart App Control prüft jede geladene exe/dll;
unsignierte Dateien ohne Cloud-Reputation kann es blockieren. Mögliche Auswirkungen auf
Rechnern mit SAC „Ein“:

- Die KI-Funktionen (llama-server) starten nicht. Code-Prüfung ohne KI funktioniert.
- Der Installer kann scheitern, wenn SAC die NSIS-Plugins in `%TEMP%` blockiert.
  **[ungeprüft]**

## Ablauf im Workflow

`.github/workflows/release.yml`:

- **build-windows** (ohne Secrets): `tauri build --no-bundle` baut `code-pruefer.exe`;
  dazu kommen die llama.cpp-Laufzeit und die Visual-C++-Laufzeit. Alles landet als
  Artefakt `windows-unsigned` mit `SHA256SUMS.txt`.
- **bundle-windows** (QAS, Environment `qas`, Test-Signing) und **promote-windows**
  (nach der Abnahme, Environment `release`, Release-Signing; siehe
  [`RELEASE_QAS.md`](RELEASE_QAS.md)) haben dieselben Schritte. Sie prüfen die
  SHA-256-Summen und bündeln dann mit `tauri bundle --bundles nsis --no-binary-patching`.
  Die App wird dabei **nicht** neu gebaut. Mit SignPath läuft das in Runden:
  1. Runde 1: Tauri ruft für jede zu signierende Datei den `signCommand` auf, hier
     `node scripts/signpath-stage.mjs <datei>`. Das Skript legt eine Kopie in
     `signpath/stage-1/` ab. Das Verzeichnis geht als GitHub-Artefakt an SignPath
     (`signpath/github-action-submit-signing-request`, auf Commit gepinnt). Die Action
     wartet auf die Signatur und lädt das signierte Ergebnis herunter.
  2. Runde 2: Wieder `tauri bundle`. Diesmal setzt das Skript die signierten Fassungen ein
     (Zuordnung über das SHA-256 der unsignierten Datei) und sammelt, was jetzt erst
     signierbar ist: den Installer.
  3. Es gibt zwei Signaturaufträge: (a) `code-pruefer.exe` und Uninstaller, (b) der
     Installer.
  4. Danach entpackt der Workflow den Installer mit 7-Zip und prüft jede PE-Datei mit
     `Get-AuthenticodeSignature`. Der Job schlägt fehl, wenn eine eigene Datei unsigniert
     ist, wenn eine Signatur ungültig ist (bei Release-Signing) oder wenn eine unbekannte
     PE-Datei unsigniert ist. Unsignierte Fremdbinaries erzeugen eine Warnung. Die Tabelle
     steht in der Job-Zusammenfassung.
- Fehlen Secret und Variablen, wird wie bisher unsigniert gebündelt, mit einer Warnung in
  der Job-Zusammenfassung. Ist nur ein Teil gesetzt, bricht der Job ab.
- `SHA256SUMS.txt` und die Build-Provenance-Attestierung im Job `release` werden über die
  fertigen, also signierten, Installer berechnet.

### Warum so (Abwägung)

- SignPath signiert bei der Open-Source-Signierung nur mit **Origin Verification**. Jeder
  Signaturauftrag muss sich auf ein **GitHub-Artefakt dieses Workflow-Laufs** beziehen,
  und SignPath lädt dieses Artefakt selbst herunter. Einzelne Dateien synchron aus dem
  `signCommand` heraus zu signieren (REST-API, PowerShell-Modul) passt nicht zur Origin
  Verification. Aus einem `run`-Schritt heraus lassen sich auch keine Artefakte hochladen.
  Deshalb die Runden. **[ungeprüft: genaue Regeln der Origin Verification]**
- SignPath kann NSIS-Installer nicht „tief“ signieren, also den eingebetteten Uninstaller
  und die Plugins von außen. Der Uninstaller wird von makensis über `!uninstfinalize`
  signiert und erst danach in den Installer eingebettet. Das geht nur, wenn seine
  signierte Fassung beim nächsten makensis-Lauf schon vorliegt. Das funktioniert, weil
  makensis den Uninstaller **reproduzierbar** erzeugt: gleiche Eingaben ergeben dasselbe
  SHA-256, auch in einem anderen Verzeichnis. **[geprüft lokal mit NSIS 3 unter Linux;
  unter Windows wird es durch den Job abgesichert:]** Wäre der Uninstaller nicht
  reproduzierbar, fände Runde 2 keine signierte Fassung. Dann sammelt sie ihn erneut,
  und nach der letzten Runde bricht der Job ab, statt unsigniert auszuliefern.
- Der Uninstaller enthält die NSIS-Plugins, ändert sich also, wenn die Plugins signiert
  werden. Darum hält das Skript ihn zurück, solange in derselben Runde Plugins gesammelt
  werden. Den Installer hält es zurück, solange überhaupt etwas gesammelt wird.
  **[geprüft lokal]**
- `--no-binary-patching`: Sonst schreibt Tauri vor dem Signieren den Bundle-Typ in
  `code-pruefer.exe`. Damit bliebe die Exe nicht bitgleich. Der App-Updater braucht die
  Markierung nicht ([`UPDATER.md`](UPDATER.md)).
- Die **Updater-Signatur** (minisign, nicht Authenticode) entsteht erst nach der
  letzten SignPath-Runde im Job `updater-sign`, über den fertigen Installer
  ([`UPDATER.md`](UPDATER.md)). **[geprüft: tauri-cli 2.12.4 / tauri-bundler 2.10.4]**
- Tauri 2.10.4 ruft den `signCommand` auf für: die Haupt-Exe, die exe/dll unter den
  Ressourcen (sofern sie nicht schon gültig signiert sind, `signtool verify /pa`), die
  NSIS-Plugins aus `NSISdl.dll`, `StartMenu.dll`, `System.dll`, `nsDialogs.dll` und
  `nsis_tauri_utils.dll`, den Uninstaller und den Installer. Die Ressourcen werden also
  entgegen älterer Doku sehr wohl angefasst. Die llama-Dateien und NSIS-Plugins legt das
  Skript nicht zum Signieren ab (Fremdbinaries, siehe oben).
  **[geprüft im Quellcode tauri-bundler 2.10.4]**
- Dateien, die schon eine Signatur haben (Microsoft-DLLs), sammelt das Skript nie.
- Der ganze Ablauf wurde lokal simuliert: Linux-Cross-Build, `tauri bundle`
  mit `scripts/signpath-stage.mjs`, osslsigncode mit Testzertifikat statt SignPath,
  danach Prüfung mit 7-Zip. Ergebnis: Installer, `code-pruefer.exe` und `uninstall.exe`
  sind signiert. Unter Windows und mit dem echten SignPath konnte der Ablauf hier
  **nicht** laufen.

## Einrichtung – Schritt für Schritt

### 1. Voraussetzungen im Repository (erledigt)

- Lizenz MIT (`LICENSE`, `package.json`, `Cargo.toml`). Die mitgelieferten
  Drittkomponenten sind damit verträglich (`THIRD_PARTY_NOTICES.txt`: llama.cpp MIT,
  LLVM OpenMP Apache-2.0 mit LLVM-Ausnahme, VC++-Laufzeit als „Distributable Code“).
  Unter den Rust-Abhängigkeiten sind einige MPL-2.0 (cssparser, selectors, option-ext,
  dtoa-short; Copyleft auf Dateiebene, unverändert genutzt). Das ist verträglich.
- Code-Signing-Policy mit Rollen, Datenschutzerklärung und Hinweis:
  [`docs/CODE_SIGNING_POLICY.md`](CODE_SIGNING_POLICY.md), verlinkt aus der README, Abschnitt „Code signing policy“ (die
  Projektseite ist das GitHub-Repository).
- Das Repository muss **öffentlich** sein.

### 2. MFA einschalten

- GitHub: Settings → Password and authentication → Two-factor authentication.
- SignPath: nach der Einladung im Benutzerprofil MFA aktivieren. Die Foundation verlangt
  MFA für alle Teammitglieder bei GitHub und SignPath.

### 3. Antrag bei der SignPath Foundation

Auf <https://signpath.org/> → „Apply“ (Formular). **[ungeprüft: genaue Felder]** Angeben:

- Projektname „Code-Prüfer“, Repository `https://github.com/BernhardGoetz/CodeChecker`,
  Projektseite = Repository-URL, Lizenz MIT.
- Kurzbeschreibung: Desktop-App für Studierende, prüft Programmieraufgaben lokal in
  Containern; Windows-Installer (NSIS) aus GitHub Actions.
- Link zur Code-Signing-Policy: `docs/CODE_SIGNING_POLICY.md` (Datenschutz steht dort mit).
- Build-System: GitHub Actions (GitHub-hosted Runner), Workflow `release.yml`.
- Was signiert wird: `code-pruefer.exe`, `uninstall.exe`, Installer.
- Hinweis: Die Foundation prüft Projekte manuell. Neue oder kleine Projekte ohne
  Verbreitung werden womöglich abgelehnt oder vertröstet. **[ungeprüft]**

### 4. SignPath einrichten (nach der Zusage)

Die Foundation legt die Organisation an und lädt Bernhard ein. Dann in SignPath:

1. **Projekt** anlegen (oder das angelegte verwenden), Slug merken (z. B. `code-pruefer`).
   Als Repository-URL das GitHub-Repository eintragen.
2. **Trusted Build System** „GitHub.com“ mit dem Projekt verknüpfen (Origin Verification).
   Falls SignPath dazu eine GitHub-App-Installation oder ein Token verlangt, der
   Anleitung in SignPath folgen. **[ungeprüft]**
3. **Artefaktkonfiguration**: Inhalt von `.signpath/artifact-configurations/default.xml`
   einfügen und als Default setzen. SignPath liest die Datei nicht aus dem Repository, sie liegt dort nur
   versioniert. Hat die Konfiguration nicht den Default-Slug, den Slug als
   GitHub-Variable `SIGNPATH_ARTIFACT_CONFIGURATION_SLUG` setzen.
4. **Signing Policies**:
   - `test-signing`: Testzertifikat, ohne manuelle Freigabe. Zum Ausprobieren.
   - `release-signing`: Zertifikat der Foundation, Origin Verification an, manuelle
     Freigabe durch Bernhard (Approver). Erlaubter Ref: der Branch `qas`
     (`refs/heads/qas`). Signiert wird erst nach der QAS-Abnahme im Job
     `promote-windows` desselben Workflow-Laufs, und zwar die unsignierten Dateien,
     die in QAS getestet wurden ([`RELEASE_QAS.md`](RELEASE_QAS.md)).
   - Für `test-signing` ebenfalls `refs/heads/qas` zulassen.
   - **PR-Ablauf (ungeprüft):** `release.yml` läuft jetzt bei Pull Requests nach `qas`
     (`pull_request`). Der Ref des Laufs ist dann `refs/pull/<n>/merge`
     (`GITHUB_REF`, im Lauf 41503339956 geprüft), der Head-Branch steht in
     `GITHUB_HEAD_REF`. Welchen Ref SignPath prüft, ist nicht nachgeprüft; die Ablehnung nennt ihn. In beiden Policies den
     tatsächlich gemeldeten Ref bzw. ein passendes Muster erlauben.
5. **CI-Benutzer / API-Token**: einen Benutzer für GitHub Actions anlegen (bzw. das eigene
   Konto), ihn in beiden Policies als **Submitter** eintragen und ein **API-Token**
   erzeugen.
6. Die **Organisations-ID** steht in SignPath unter dem Organisationsnamen (oben rechts)
   bzw. in den Organisationseinstellungen.

Ob SignPath zusätzlich **Policy-Dateien im Repository** verlangt (z. B. unter
`.signpath/policies/…`, etwa für Runner- oder Branch-Regeln), ist hier unbekannt. Für das
Signieren nötig sind sie nach dem Stand der Action-Doku nicht. **[ungeprüft]** Sollte
SignPath sie verlangen, legt man sie nach der Doku an; der Workflow bleibt unverändert.

### 5. GitHub einrichten

Settings → Environments (vollständige Tabelle in [`RELEASE_QAS.md`](RELEASE_QAS.md)):

- Environment **qas** (Test-Signing, Jobs `bundle-windows`/`bundle-macos`):
  - **Environment secret** `SIGNPATH_API_TOKEN` = API-Token aus SignPath,
  - **Environment variables**: `SIGNPATH_ORGANIZATION_ID` (Organisations-ID),
    `SIGNPATH_PROJECT_SLUG` (Projekt-Slug), `SIGNPATH_SIGNING_POLICY_SLUG` = `test-signing`,
    optional `SIGNPATH_ARTIFACT_CONFIGURATION_SLUG` (leer = Default-Konfiguration).
- Environment **release** (Release-Signing, Job `promote-windows` nach der Abnahme):
  dieselben Namen, aber `SIGNPATH_SIGNING_POLICY_SLUG` = `release-signing`.
- Environment **production**: nur die Abnahme (Pflicht-Reviewer Bernhard), keine Secrets.

Ein Environment ohne SignPath-Einträge bündelt unsigniert und warnt. Das alte Secret
`WINDOWS_CERTIFICATE` bzw. `WINDOWS_CERTIFICATE_PASSWORD` wird nicht mehr verwendet und
kann gelöscht werden.

### 6. Testlauf mit Test-Signing (= QAS-Lauf)

- Zunächst nur das Environment `qas` mit SignPath befüllen und einen QAS-Lauf starten
  (Pull Request nach `qas` oder „Run workflow“ mit der PR-Nummer).
- Erwartung: Im Job `bundle-windows` laufen zwei Signaturaufträge, in SignPath sind zwei
  Signing Requests zu sehen. Die Tabelle „Signaturen im Installer“ zeigt für Installer,
  `code-pruefer.exe` und `uninstall.exe` eine Signatur mit dem Status `UnknownError` bzw.
  `NotTrusted` (Testzertifikat, nicht vertrauenswürdig). Das ist bei Test-Signing normal.
  Enthält der Slug „test“, verlangt der Workflow keine gültige Kette.
- Den Installer aus dem Pre-Release testweise installieren und deinstallieren.
- Zum reinen Testen die Abnahme ablehnen („Reject“).

### 7. Erster echter Release

1. Das Environment `release` mit SignPath (`release-signing`) befüllen.
2. QAS-Lauf starten, testen, abnehmen („Approve and deploy“ in `production`).
3. Danach in SignPath **jeden** Signing Request von `promote-windows` freigeben: zwei.
   Jeder wartet bis zu einer Stunde, dann bricht der Job ab und
   kann mit „Re-run failed jobs“ neu gestartet werden.
4. Der Workflow verlangt jetzt gültige Signaturen (`Valid`). Danach das Entwurfs-Release
   prüfen und veröffentlichen.

## Signatur unter Windows prüfen

- Explorer: Rechtsklick auf den Installer → Eigenschaften → **Digitale Signaturen**.
  Signaturgeber: **SignPath Foundation**. Herausgeber ist die Foundation, nicht Bernhard
  Götz; so steht es auch im UAC-Dialog und bei SmartScreen. **[ungeprüft: genauer
  Zertifikatsname]**
- PowerShell:

  ```powershell
  Get-AuthenticodeSignature .\Code-Pruefer_0.2.0_x64-setup.exe | Format-List *
  Get-ChildItem "$env:LOCALAPPDATA\Code-Prüfer" -Recurse -Include *.exe,*.dll |
    Get-AuthenticodeSignature | Format-Table Status, Path, @{n='Signer';e={$_.SignerCertificate.Subject}}
  ```

  (Installationsordner je nach Installationsart ggf. `C:\Program Files\Code-Prüfer`.)
- Mit dem Windows SDK: `signtool verify /pa /v <datei>`.

## SmartScreen und Smart App Control

- Eine gültige Signatur ist Voraussetzung, aber keine Garantie. SmartScreen und SAC
  bewerten auch die **Reputation** von Zertifikat und Datei. Bei neuen Releases kann es
  anfangs noch Warnungen bzw. Blockaden geben, bis sich Reputation aufgebaut hat. Das
  Zertifikat der SignPath Foundation wird von vielen Projekten genutzt, das hilft
  vermutlich. **[ungeprüft]**
- SAC blockiert unsignierte Fremdbinaries ohne Reputation (siehe „Fremdbinaries“).
- Bleibt ein signierter Release blockiert, kann man die Datei bei Microsoft zur Analyse
  einreichen: <https://www.microsoft.com/wdsi/filesubmission>.

## Fehlerbilder

| Meldung | Ursache / Abhilfe |
|---|---|
| „SignPath ist nur teilweise eingerichtet“ | Secret bzw. eine der drei Variablen fehlt im Environment `qas` bzw. `release`. |
| „Build artifact … cannot be signed because of build system validation errors“ | Origin Verification: Trusted Build System nicht verknüpft, Ref nicht in der Policy erlaubt, oder das Repository passt nicht zur Projekt-URL. |
| Zeitüberschreitung beim Warten | Signing Request in SignPath nicht (rechtzeitig) freigegeben; Job neu starten. |
| „SignPath lieferte andere Dateien zurück“ / „kam ohne Signatur zurück“ | Die Artefaktkonfiguration passt nicht zu den gesammelten Dateinamen. |
| „Nach der letzten Runde ist noch … unsigniert“ | Der Uninstaller war zwischen zwei Läufen nicht bitgleich (NSIS-Version gewechselt?) oder eine neue Datei ist hinzugekommen. Log der Runde ansehen. |
| „unbekannte unsignierte Datei“ | Neue exe/dll in den Ressourcen; Zuordnung in `scripts/signpath-stage.mjs` und Artefaktkonfiguration ergänzen. |
