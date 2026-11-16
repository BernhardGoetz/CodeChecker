# Änderungsprotokoll

Alle nennenswerten Änderungen am Code-Prüfer stehen in dieser Datei.
Format nach [Keep a Changelog](https://keepachangelog.com/de/1.1.0/), Versionen nach
[Semantic Versioning](https://semver.org/lang/de/). Die Release-Notizen auf GitHub
übernehmen den Abschnitt der jeweiligen Version (`scripts/release-notes.sh`).

## [Unveröffentlicht]

### Hinzugefügt

- Alte Prüf-Umgebungen werden automatisch gelöscht, um Speicherplatz zu sparen: Nach dem
  Bereitstellen der aktuellen Images (beim Start und nach „Images aktualisieren“) entfernt
  der Code-Prüfer im Hintergrund in Docker und Podman die eigenen Runner-Images, die älter
  als die aktuelle und die vorherige Version sind. Neuere Versionen, selbst gebaute
  `:dev`-Images, fremde Images und von Containern benutzte Images bleiben unangetastet.
- Java: Die Hauptklasse muss nicht mehr `Main` heißen. Gestartet wird die Klasse mit
  `public static void main(String[] args)` (auch `String... args`, `final`,
  Annotationen); die Datei heißt nach der `public`-Klasse (z. B. `Time.java`).
  Kommentare und Zeichenketten werden dabei ignoriert. Eine `package`-Zeile ist erlaubt
  (übersetzt mit `javac -d .`, gestartet mit dem vollständigen Namen). Fehlt `main` oder
  haben mehrere Klassen eine `main`-Methode, gibt es einen deutschen Hinweis und das
  Ergebnis „Nicht startbar“.
- KI: Nennt das Aufgabenblatt eine Java-Klasse (z. B. „Öffnen Sie die bereitgestellte
  Java-Klasse `Time`“ oder „Time.java“), schreibt die KI ihre Lösungen mit diesem
  Klassennamen, sonst wie bisher mit `Main`.
- Neue Einsteiger-Anleitung `docs/WARNUNG_BEIM_INSTALLIEREN.md`: warum Windows
  (SmartScreen, Smart App Control) und macOS (Gatekeeper) beim Installieren warnen, wie
  man die Warnung Schritt für Schritt übergeht und wie man die Echtheit der Datei mit
  `SHA256SUMS.txt` prüft.
- KI-Testfälle: Ausgabevorlagen mit ausdrücklich erklärtem Platzhalter („12345678 ist ein
  Platzhalter“) oder eindeutigen Platzhaltern (`<n>`, `___`, `XX.XX`, `[Name]`) nach
  „Ausgabe:“ werden jetzt auch ohne die KI erkannt und mit den Vorlagen der KI zusammengeführt.
- KI-Testfälle: Unter „Zeit je Schritt“ zeigt die Statistik, wie lange Analyse, Lösungen,
  Reparatur, Testeingaben und Mehrheitsentscheid gedauert haben (dazu die verarbeiteten
  Prompt-Token), damit sich messen lässt, wo die Zeit bleibt.

### Geändert

- „Testfälle mit KI“: Die Musterlösung ist standardmäßig verborgen und wird über die
  Schaltfläche „Musterlösung anzeigen“ / „Musterlösung ausblenden“ vollständig ein- bzw.
  ausgeblendet (barrierefrei mit `aria-expanded`). Nach jeder neuen KI-Erzeugung ist sie
  wieder verborgen.
- KI-Testfälle schneller: Liest eine Aufgabe keine Eingabe, erzeugt die KI keine Testeingaben
  mehr; es gibt einen Test mit leerer Eingabe. Ein Probelauf mit Eingabe prüft diese
  Einschätzung; liest das Programm doch etwas, werden die Eingaben nachträglich erzeugt.
- KI-Testfälle schneller: Bei Aufgaben ohne Eingabe wird nicht auf die letzte Lösung gewartet,
  wenn die Mehrheit schon feststeht und dadurch keine bessere Konfidenz verloren geht. Nicht
  abgewartete Lösungen zählen als Gegenstimme; die Konfidenz nennt den vorzeitigen Abschluss.
- Mehrere Ausgabevorlagen werden als Folge geprüft: Eine Ausgabe passt, wenn sie zu einer
  Vorlage passt oder aus mehreren Vorlagen in der Reihenfolge des Blatts besteht.

- README: Der Weg zum ersten Start unter macOS folgt jetzt der Anleitung von Apple
  (Systemeinstellungen → „Datenschutz und Sicherheit“ → „Dennoch öffnen“); der
  Rechtsklick-Weg funktioniert seit macOS 15 nicht mehr.
- Signatur-Status in README und Code signing policy aktualisiert: Der Antrag bei der
  SignPath Foundation wurde abgelehnt (Projekt noch zu neu), eine erneute Bewerbung ist
  geplant.

### Behoben

- Das 7B-Modell wurde doppelt heruntergeladen (Einzeldatei und Teile, ~9,4 statt
  ~4,7 GB). Ursache: `scripts/pin-models.sh` trug für Qwen2.5-Coder 7B sowohl die
  einzelne `.gguf`-Datei als auch die beiden Teile der aufgeteilten Datei ein. Das Skript
  trägt jetzt genau eine Variante ein (die Einzeldatei, sonst die Teile; `--check` warnt
  bei alten Einträgen mit beiden). Die App lädt bei solchen Einträgen nur die Einzeldatei
  und zeigt die richtige Größe an; ein früherer Download der Teile gilt weiter als
  installiert und wird nicht erneut geladen. Übrig gebliebene Dateien entfernt „Löschen“
  beim Modell.
- KI-Testfälle: Stehen die Zeilen einer Ausgabevorlage erst einzeln in den Teilaufgaben und
  dann zusammen als Block (z. B. „Aufgabe 2.2 Time“), entsteht wieder eine mehrzeilige
  Vorlage statt drei einzelner. Bisher endete so ein Blatt mit „Rot“, obwohl alle Lösungen
  dasselbe ausgaben.
- KI-Testfälle: Passt keine einzige Ausgabe zur erkannten Vorlage, obwohl alle Lösungen
  einig sind, wird die Vorlage verworfen (höchstens „Gelb“, mit Hinweis); sind sie sich
  nicht einig, nennt „Rot“ jetzt die Vorlage als Grund statt „nicht mehrheitlich einig“.
- Kompatibilität mit sysinfo 0.40 (Dependabot-Update): Der Code wurde an die neue API
  angepasst; ohne diese Anpassung ließ sich `main` nicht bauen.

## [1.0.3] - 2026-11-07

### Hinzugefügt

- Neue Einsteiger-Anleitung (README) mit Installation, Bedienung und Fehlerbehebung,
  einer kurzen englischen Zusammenfassung sowie `CONTRIBUTING.md` für alle, die den
  Code-Prüfer weiterentwickeln.
- Abschnitt „Code signing policy“ in der README; die Release-Notizen verlinken die
  Policy und beschreiben kurz, was der Code-Prüfer tut.
- `SECURITY.md`: Sicherheitslücken vertraulich über GitHub melden.
- `THIRD_PARTY_NOTICES.txt` nennt zusätzlich die NSIS-Plugins und die Basis-Images der
  Runner.

### Geändert

- Die Code signing policy (`docs/CODE_SIGNING_POLICY.md`) ist an die Anforderungen der
  SignPath Foundation angepasst (Rollen, Datenschutz, Status).
- Dokumentation zur Veröffentlichung aktualisiert.

### Entfernt

- Die vorbereitete Signierung von Fremdbinaries (llama.cpp-Laufzeit, NSIS-Plugins) über
  SignPath, samt Artefaktkonfiguration und Workflow-Variable. Signiert werden künftig nur
  selbst gebaute Windows-Dateien. Für Anwender ändert sich nichts.

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

[Unveröffentlicht]: https://github.com/BernhardGoetz/CodeChecker/compare/v1.0.3...HEAD
[1.0.3]: https://github.com/BernhardGoetz/CodeChecker/compare/v1.0.2...v1.0.3
[1.0.2]: https://github.com/BernhardGoetz/CodeChecker/compare/v1.0.1...v1.0.2
[1.0.1]: https://github.com/BernhardGoetz/CodeChecker/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/BernhardGoetz/CodeChecker/releases/tag/v1.0.0
