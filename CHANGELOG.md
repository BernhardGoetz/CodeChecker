# Änderungsprotokoll

Alle nennenswerten Änderungen am Code-Prüfer stehen in dieser Datei.
Format nach [Keep a Changelog](https://keepachangelog.com/de/1.1.0/), Versionen nach
[Semantic Versioning](https://semver.org/lang/de/). Die Release-Notizen auf GitHub
übernehmen den Abschnitt der jeweiligen Version (`scripts/release-notes.sh`).

## [Unveröffentlicht]

## [1.0.2] - 2026-11-06

### Geändert

- Update-Adresse und Links auf das umbenannte Repository CodeChecker umgestellt.

## [1.0.1] - 2026-11-04

### Hinzugefügt

- **Ausgabevorlagen mit Platzhaltern** (z. B. `Summe: {n}`) werden im erwarteten Ergebnis
  erkannt und nur als Formatprüfung genutzt, nicht als feste Soll-Ausgabe.
- **Variationstest:** erkennt Code, der nur für die gegebenen Werte zufällig stimmt
  (etwa fest eingetragene Ergebnisse), indem die Eingaben variiert werden.
- **Gezielter Abbruch** des Variationstests.

### Behoben

- Fehler im KI-Gate behoben.

### Geändert

- README überarbeitet.

## [1.0.0] - 2026-11-03

Erste stabile Version.

### Hinzugefügt

- **Lokale Prüfung** von Programmieraufgaben gegen eigene Testfälle (stdin → stdout),
  ohne zentralen Server: Ausführung in Docker oder Podman, je Prüfung ein frischer,
  abgeschotteter Container (kein Netzwerk, read-only Dateisystem, nicht-root, ohne
  Capabilities, Speicher-, CPU-, Prozess- und Zeitlimits; übrig gebliebene Container
  werden automatisch entfernt).
- **Sprachen:** Python 3.12 und Java 21; die Runner-Images sind im Installer enthalten
  und werden beim ersten Start geladen („Images aktualisieren“ lädt sie neu).
- **Ergebnisse** je Testfall (Bestanden, Falsche Ausgabe, Zeitüberschreitung,
  Laufzeitfehler) mit Zeilen-Diff, stderr und Kompilierfehlern.
- **Vergleichsmodi:** Zeilen (exakt), Wörter (Leerraum egal) und Zahlen mit Toleranz.
  Leerzeichen und Zeilenumbruch am Zeilenende werden sichtbar gemacht (`·`, `⏎`), mit
  Hinweis, wenn sich Ausgaben nur dort unterscheiden. Option „Zeilenumbruch am Ende
  verbieten“.
- **Aufgabenblatt hochladen:** Word (DOCX), PDF und Bilder; Scans und Bilder per
  systemeigener Texterkennung (Windows OCR, Apple Vision) mit Bildvorbereitung
  (Graustufen, Invertieren dunkler Hintergründe, Kontrast, Zuschnitt, Hochskalieren) und
  Rekonstruktion der Einrückung. Erkennungsfehler lassen sich mit der lokalen KI
  korrigieren: Vorschläge (alt → neu) einzeln oder alle übernehmen.
- **Testfälle mit KI:** lokales Sprachmodell (Qwen2.5-Coder über llama.cpp, ohne
  Cloud-Dienst). Erwartete Ausgaben entstehen durch Ausführen mehrerer geprüfter
  Lösungen und Mehrheitsentscheid, mit Konfidenz (grün/gelb/rot). Modell-Download in der
  App mit Prüfsummenkontrolle oder Import einer `.gguf`-Datei.
- **Windows-Installer** auf Deutsch mit Auswahl der Container-Engine (Docker Desktop oder
  Podman) und optionaler Installation über winget; die App startet die gewählte Engine
  bei Bedarf selbst.
- **Einstellungen** (Darstellung, Container-Engine, KI-Modell, Updates) und helles/dunkles
  Erscheinungsbild.
- **Updates:** Die App sucht nach neuen veröffentlichten Versionen, prüft deren Signatur
  und installiert sie auf Wunsch.
- **Release-Ablauf über QAS:** Jede Version wird als QAS-Pre-Release gebaut und abgenommen;
  das Release entsteht aus genau diesem Build und wird nach dem Merge nach `main`
  veröffentlicht.

### Behoben

- Integrationstest für den Rückgriff auf das lokale `:dev`-Image hängt nicht mehr davon
  ab, welche Images in der Registry (GHCR) veröffentlicht sind.
- Die Release-Notizen beginnen jetzt mit „Code-Prüfer <Version>“, ohne Hinweis auf den
  Vorab-Status des Releases.

### Bekannte Einschränkungen

- Windows-Installer und macOS-DMGs sind noch nicht signiert (SignPath bzw. Apple-Signatur
  und Notarisierung sind noch nicht eingerichtet); SmartScreen und Gatekeeper warnen beim
  ersten Start.

[Unveröffentlicht]: https://github.com/BernhardGoetz/CodeChecker/compare/v1.0.2...HEAD
[1.0.2]: https://github.com/BernhardGoetz/CodeChecker/compare/v1.0.1...v1.0.2
[1.0.0]: https://github.com/BernhardGoetz/Entwicklung-test/compare/v0.1.0...v1.0.0
