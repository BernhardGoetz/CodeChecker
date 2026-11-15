# Warum warnt Windows bzw. macOS beim Installieren?

Diese Anleitung ist für alle, die den Code-Prüfer zum ersten Mal installieren und von
einer Warnmeldung überrascht werden. Kurz gesagt: Die Warnung bedeutet **nicht**, dass
die Datei gefährlich ist. Sie bedeutet, dass Windows bzw. macOS die Datei noch nicht
kennt, weil sie **nicht digital signiert** ist.

## Inhalt

1. [Kurz erklärt: Was ist eine Signatur?](#1-kurz-erklärt-was-ist-eine-signatur)
2. [Warum ist der Code-Prüfer nicht signiert?](#2-warum-ist-der-code-prüfer-nicht-signiert)
3. [Windows: Schritt für Schritt](#3-windows-schritt-für-schritt)
4. [macOS: Schritt für Schritt](#4-macos-schritt-für-schritt)
5. [Echtheit prüfen](#5-echtheit-prüfen)
6. [Quellen](#6-quellen)

---

## 1. Kurz erklärt: Was ist eine Signatur?

Eine **digitale Signatur** ist wie ein Siegel auf der Installationsdatei. Sie zeigt, von
wem die Datei stammt und dass sie seitdem nicht verändert wurde. Windows und macOS
vertrauen signierten Dateien von bekannten Herstellern eher.

- **Windows** nutzt dafür **SmartScreen**. SmartScreen merkt sich, wie oft eine Datei
  schon heruntergeladen wurde und ob es Probleme gab (die sogenannte **Reputation**).
  Bei unsignierten Dateien beginnt diese Reputation **bei jeder neuen Version wieder bei
  null**. Deshalb warnt Windows auch dann, wenn eine ältere Version schon vielen
  Menschen bekannt war. Man kann diese Reputation nicht von Hand bei Microsoft
  beantragen ([Quelle 1](#6-quellen)).
- **macOS** nutzt dafür **Gatekeeper**. Programme ohne Apple-Signatur und
  Notarisierung blockiert macOS beim ersten Öffnen ([Quelle 3](#6-quellen)).

## 2. Warum ist der Code-Prüfer nicht signiert?

- Für Windows hat sich das Projekt bei der **SignPath Foundation** um eine kostenlose
  Code-Signierung für Open-Source-Projekte beworben. Die Bewerbung wurde im November 2026
  abgelehnt, weil das Projekt noch zu neu und zu wenig bekannt ist. Eine erneute
  Bewerbung ist geplant. Einzelheiten stehen in der
  [Code signing policy](CODE_SIGNING_POLICY.md).
- Kostenpflichtige Zertifikate sind für ein privates, kostenloses Projekt nicht
  finanzierbar.

Bis dahin hilft dir diese Anleitung, die Warnung sicher zu umgehen, und zeigt dir, wie du
prüfst, dass die Datei echt ist.

**Wichtig:** Lade den Code-Prüfer **nur** von der offiziellen Release-Seite:
<https://github.com/BernhardGoetz/CodeChecker/releases>

---

## 3. Windows: Schritt für Schritt

Die Windows-Installationsdatei heißt `Code-Pruefer_<Version>_x64-setup.exe` (statt
`<Version>` steht eine Zahl wie `1.0.3`).

1. Starte die heruntergeladene Datei mit einem Doppelklick.
2. Windows zeigt ein blaues Fenster: **„Der Computer wurde durch Windows geschützt“**.
   Darin steht nur ein Knopf „Nicht ausführen“.
3. Klicke im Text auf **„Weitere Informationen“**.
4. Jetzt erscheint der Knopf **„Trotzdem ausführen“**. Klicke darauf – aber nur, wenn
   du die Datei selbst von der offiziellen Release-Seite geladen hast.
5. Der Installer startet. Weiter geht es in der
   [README, Abschnitt 3](../README.md#schritt-1-den-code-prüfer-installieren-mit-container-engine).

Tipp: Prüfe am besten **vor** Schritt 1 die Echtheit der Datei
(siehe [Abschnitt 5](#5-echtheit-prüfen)).

### Wenn es keinen Knopf „Trotzdem ausführen“ gibt: Smart App Control

Auf manchen Windows-11-Rechnern ist **Smart App Control** eingeschaltet. Diese Funktion
blockiert unsignierte Dateien, die noch keine positive Reputation haben
([Quelle 1](#6-quellen)). Dann lässt Windows den Installer möglicherweise **gar nicht
starten**, und es gibt keinen Knopf zum Überspringen.

Was du dann tun kannst:

- Auf einem Schul- oder Firmenrechner: Frag deine IT-Betreuung.
- Auf deinem eigenen Rechner: Ob und wie du Smart App Control ändern willst, entscheidest
  du selbst. Informiere dich dazu bitte in der Hilfe von Microsoft, bevor du etwas
  umstellst – die Funktion schützt deinen Rechner.
- Sobald der Code-Prüfer signiert ist, entfällt dieses Problem.

### Fehlalarm des Virenschutzes melden

Meldet der Microsoft-Virenschutz (Microsoft Defender) den Code-Prüfer fälschlich als
Schadsoftware, kann der Entwickler das bei Microsoft zur Prüfung einreichen:
<https://www.microsoft.com/en-us/wdsi/filesubmission> (Ansicht „Software developer“).
Bitte melde solche Fälle über die
[Issues auf GitHub](https://github.com/BernhardGoetz/CodeChecker/issues).

---

## 4. macOS: Schritt für Schritt

Die Mac-Dateien heißen `Code-Pruefer_<Version>_aarch64.dmg` (Apple Silicon) bzw.
`Code-Pruefer_<Version>_x64.dmg` (Intel-Mac).

1. Öffne die DMG-Datei und ziehe „Code-Prüfer“ in den Ordner **„Programme“**.
2. Öffne „Code-Prüfer“ aus dem Ordner „Programme“. macOS meldet, dass die App nicht
   geöffnet werden kann. Schließe diese Meldung (nicht „In den Papierkorb legen“
   wählen).
3. Öffne die **„Systemeinstellungen“** und dort **„Datenschutz und Sicherheit“**.
4. Scrolle **nach unten**. Dort steht ein Hinweis, dass „Code-Prüfer“ blockiert wurde.
   Klicke auf **„Dennoch öffnen“**.
5. Bestätige die folgende Rückfrage (macOS fragt dabei ggf. nach deinem Passwort).

Danach startet der Code-Prüfer, und macOS merkt sich die Ausnahme. So beschreibt es
Apple ([Quelle 3](#6-quellen)).

**Hinweis zum Rechtsklick:** Früher konnte man die Warnung mit Rechtsklick (oder
Ctrl-Klick) → „Öffnen“ umgehen. Das gilt seit **macOS 15 Sequoia** nicht mehr
([Quelle 4](#6-quellen)). Auf macOS 14 und älter funktioniert der Rechtsklick weiterhin.

**Für Fortgeschrittene:** Statt der Systemeinstellungen kannst du im Terminal die
Quarantäne-Markierung entfernen:

```sh
xattr -dr com.apple.quarantine "/Applications/Code-Prüfer.app"
```

---

## 5. Echtheit prüfen

So stellst du sicher, dass deine Datei genau die ist, die auf GitHub veröffentlicht
wurde, und unterwegs nicht verändert wurde.

### Schritt A: Prüfsumme vergleichen (empfohlen)

Eine **Prüfsumme** (SHA-256) ist ein „Fingerabdruck“ der Datei: eine lange Zeichenfolge,
die sich schon bei der kleinsten Änderung völlig ändert.

1. Lade auf der [Release-Seite](https://github.com/BernhardGoetz/CodeChecker/releases)
   unter „Assets“ zusätzlich die Datei **`SHA256SUMS.txt`** herunter (aus **demselben**
   Release wie deine Installationsdatei). Öffne sie mit einem Texteditor. Sie enthält
   für jede Datei des Releases eine Zeile: zuerst die Prüfsumme, dann den Dateinamen.
2. Berechne die Prüfsumme deiner heruntergeladenen Datei:
   - **Windows:** Klicke auf Start, tippe **PowerShell** und öffne „Windows
     PowerShell“. Wechsle in den Download-Ordner und berechne die Prüfsumme (statt
     `<Version>` deine Versionsnummer):

     ```powershell
     cd $HOME\Downloads
     Get-FileHash .\Code-Pruefer_<Version>_x64-setup.exe -Algorithm SHA256
     ```

   - **macOS:** Öffne das Programm „Terminal“ und tippe (mit deinem Dateinamen):

     ```sh
     shasum -a 256 ~/Downloads/Code-Pruefer_<Version>_aarch64.dmg
     ```

3. Vergleiche die angezeigte Zeichenfolge mit der Zeile für deine Datei in
   `SHA256SUMS.txt`. Groß- und Kleinschreibung spielt keine Rolle (PowerShell zeigt
   Großbuchstaben, die Datei enthält Kleinbuchstaben).
   - **Stimmt sie überein:** Die Datei ist echt.
   - **Stimmt sie nicht überein:** Starte die Datei **nicht**. Lösche sie und lade sie
     erneut von der offiziellen Release-Seite.

### Schritt B: Zweitmeinung von VirusTotal (freiwillig)

Du kannst die Datei zusätzlich bei <https://www.virustotal.com> hochladen. Dort prüfen
viele Virenscanner gleichzeitig. Beachte: Einzelne Scanner melden unsignierte Programme
manchmal **fälschlich** als verdächtig. Ein oder zwei solche Treffer bei sonst sauberem
Ergebnis sind bei unsignierten Programmen nicht ungewöhnlich. Im Zweifel: Prüfsumme
vergleichen (Schritt A) und ein Issue auf GitHub öffnen.

---

## 6. Quellen

1. Microsoft: SmartScreen reputation for Windows app developers –
   <https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation>
   (Reputation unsignierter Dateien bei jeder Version neu, Smart App Control, kein
   manueller Antrag auf Reputation)
2. Microsoft: Datei zur Analyse einreichen –
   <https://www.microsoft.com/en-us/wdsi/filesubmission>
3. Apple: Apps sicher auf dem Mac öffnen – <https://support.apple.com/de-de/102445>
4. Michael Tsai: Sequoia Removes Gatekeeper Contextual Menu Override –
   <https://mjtsai.com/blog/2024/07/05/sequoia-removes-gatekeeper-contextual-menu-override/>
