# Änderungsprotokoll

Alle nennenswerten Änderungen am Code-Prüfer stehen in dieser Datei.
Format nach [Keep a Changelog](https://keepachangelog.com/de/1.1.0/), Versionen nach
[Semantic Versioning](https://semver.org/lang/de/). Die Release-Notizen auf GitHub
übernehmen den Abschnitt der jeweiligen Version (`scripts/release-notes.sh`).

## [Unveröffentlicht]

## [1.1.1] - 2026-11-16

### Behoben

- KI: Die Testfall-Erzeugung konnte vorzeitig enden, sobald eine bloße Mehrheit (z. B. 2 von
  3 Lösungen) übereinstimmte, auch wenn diese falsch war und die nicht abgewartete Lösung
  richtig gewesen wäre (Blatt „Aufgabe 2.2 Time“, 7B-Modell: „9630“ statt „441“ Sekunden).
  Die Lösungen werden jetzt in einem Durchgang gemeinsam geschrieben und kommen zusammen
  an; es wird also immer jede Lösung abgewartet und mitgezählt. Das vorzeitige Ende und die
  Angabe „Lösung(en) nicht abgewartet“ entfallen.
- KI: Aufgaben mit festen Werten und ohne Eingabe (z. B. „Die aktuelle Uhrzeit ist
  15:17:51 … Speichern Sie Stunde, Minute und Sekunde in Variablen“) wurden als
  „liest Eingabe“ behandelt; die Lösungen lasen die Werte von der Standardeingabe und die
  KI erzeugte passende Eingaben (1.5B-Modell). Die Lösungen sollen solche Werte jetzt als
  Konstanten verwenden. Zusätzlich erkennt eine Prüfung ohne KI solche Aufgaben am Text
  (keine Eingabe-Hinweise wie „einlesen“, „Eingabe“, „Scanner“, „stdin“, aber feste Werte
  für Variablen); dann gibt es keine KI-Eingaben, und Lösungen, die ohne Eingabe abstürzen,
  auf Eingabe warten oder mit Eingabe etwas anderes ausgeben, zählen als fehlgeschlagen
  und werden mit einem entsprechenden Hinweis repariert.
- KI: Die Prompt-Statistik im KI-Panel („Prompt: … Token in … s“) war bei gleichzeitig
  laufenden Anfragen um ein Vielfaches zu hoch, weil die Zeiten paralleler Anfragen addiert
  wurden. Sie zeigt jetzt die tatsächlich vergangene Zeit; ein gemeinsamer Prompt mehrerer
  Lösungen zählt einmal.

### Geändert

- KI: Testfälle werden schneller erzeugt. Alle Lösungen der ersten Runde gehen als eine
  Anfrage an die KI-Laufzeit, die den (langen) Prompt nur noch einmal statt einmal je
  Lösung verarbeitet. Der KV-Cache bleibt in voller Genauigkeit (f16), weil die
  Komprimierung (q8_0) die Prompt-Verarbeitung um etwa 20 % bremste; komprimiert wird nur
  noch, wenn der freie Arbeitsspeicher knapp ist (weniger als Modellgröße + 2 GB). Die
  Prompt-Verarbeitung nutzt so viele Threads wie physische Kerne. Gemessen mit dem
  1.5B-Modell auf 4 Kernen: Lösungsphase etwa 144 s → 84 s. Reparaturen und die
  Testeingaben bleiben einzelne Anfragen.
- KI: Die Restdauer ist adaptiv und eher vorsichtig (vorher zu optimistisch). Sie wird als
  Spanne angezeigt („noch ca. 4–6 Min.“) und alle 5 Sekunden aktualisiert. Sie rechnet
  mit den verbleibenden Prompt- und Ausgabe-Token, der gemessenen Geschwindigkeit des
  ganzen KI-Servers (alle parallelen Lösungen zusammen) und den Sandbox-Zeiten, mit
  Sicherheitsaufschlag. Startwert sind die Messwerte des letzten Laufs je Modell
  (`ai-speed.json` im Konfigurationsordner), beim ersten Lauf vorsichtige Annahmen.
- KI: Sind sich die Lösungen bei einem Testfall uneinig, zeigt der Testfall (und das
  KI-Panel) die abweichenden Ausgaben der anderen Lösungen mit Anzahl an, damit die
  übernommene Mehrheitsausgabe geprüft werden kann. Während die Lösungen geschrieben
  werden, zählt die Anzeige sie erst beim Prüfen hoch (sie kommen zusammen an).

## [1.1.0] - 2026-11-16

### Hinzugefügt

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
- KI-Testfälle tolerieren Eingabeaufforderungen: Schreibt ein Programm vor dem Einlesen
  einen Text wie „Zahl eingeben: “ (Python `input("…")`, Java `System.out.print("…")`),
  besteht es einen KI-Testfall trotzdem, wenn die Werte stimmen; das Ergebnis nennt die
  ignorierten Texte. Beschriftungen wie „Ergebnis:“ oder „Summe:“ werden nie ignoriert,
  ohne Eingabe gibt es keine Toleranz. Handgeschriebene Testfälle bleiben streng. Ein
  bearbeiteter KI-Testfall bleibt markiert (mit Hinweis bei geänderter Erwartung); per
  Häkchen „KI-Markierung entfernen“ wird er wie handgeschrieben streng geprüft. Der
  Variationstest wertet ebenso.
- KI-Testfälle: Ausgabevorlagen mit ausdrücklich erklärtem Platzhalter („12345678 ist ein
  Platzhalter“) oder eindeutigen Platzhaltern (`<n>`, `___`, `XX.XX`, `[Name]`) nach
  „Ausgabe:“ werden jetzt auch ohne die KI erkannt und mit den Vorlagen der KI
  zusammengeführt.
- KI-Testfälle: Unter „Zeit je Schritt“ zeigt die Statistik, wie lange Analyse, Lösungen,
  Reparatur, Testeingaben und Mehrheitsentscheid gedauert haben (dazu die verarbeiteten
  Prompt-Token), damit sich messen lässt, wo die Zeit bleibt.
- Alte Prüf-Umgebungen werden automatisch gelöscht, um Speicherplatz zu sparen: Nach dem
  Bereitstellen der aktuellen Images (beim Start und nach „Images aktualisieren“) entfernt
  der Code-Prüfer im Hintergrund in Docker und Podman die eigenen Runner-Images, die älter
  als die aktuelle und die vorherige Version sind. Neuere Versionen, selbst gebaute
  `:dev`-Images, fremde Images und von Containern benutzte Images bleiben unangetastet.
- Neue Einsteiger-Anleitung `docs/WARNUNG_BEIM_INSTALLIEREN.md`: warum Windows
  (SmartScreen, Smart App Control) und macOS (Gatekeeper) beim Installieren warnen, wie
  man die Warnung Schritt für Schritt übergeht und wie man die Echtheit der Datei mit
  `SHA256SUMS.txt` prüft.

### Geändert

- KI-Testfälle schneller: Liest eine Aufgabe keine Eingabe, erzeugt die KI keine
  Testeingaben mehr; es gibt einen Test mit leerer Eingabe. Ein Probelauf mit Eingabe prüft
  diese Einschätzung; liest das Programm doch etwas, werden die Eingaben nachträglich
  erzeugt.
- KI-Testfälle schneller: Bei Aufgaben ohne Eingabe wird nicht auf die letzte Lösung
  gewartet, wenn die Mehrheit schon feststeht und dadurch keine bessere Konfidenz verloren
  geht. Nicht abgewartete Lösungen zählen als Gegenstimme; die Konfidenz nennt den
  vorzeitigen Abschluss.
- KI-Testfälle passen besser zur Aufgabe: Die KI bekommt für die Testeingaben das
  Ausgabeformat, die Beispielpaare des Blatts und die Grenzen der Eingabe (neu erkannt,
  z. B. „1 <= n <= 1000“); die Eingaben sollen die Grenzen einhalten und keine Fälle
  enthalten, die das Programm laut Aufgabe nicht verarbeiten muss. Der Variationstest
  nutzt die Grenzen ebenfalls.
- KI-Testfälle: Brechen alle Lösungen bei einer KI-Eingabe ab oder überschreiten das
  Zeitlimit, wird die Eingabe als „passt nicht zur Aufgabe“ weggelassen und nur als Hinweis
  genannt (kein Grund mehr für Gelb). Von KI-Testfällen mit derselben erwarteten Ausgabe
  werden höchstens drei übernommen.
- KI-Testfälle: Geben einige der erzeugten Lösungen Eingabeaufforderungen aus, wird die
  Ausgabe ohne sie als Erwartung genommen (Einstufung dann höchstens Gelb). Auch die
  Java-Lösungen der KI sollen keine Eingabeaufforderungen mehr ausgeben (wie schon bei
  Python).
- Mehrere Ausgabevorlagen werden als Folge geprüft: Eine Ausgabe passt, wenn sie zu einer
  Vorlage passt oder aus mehreren Vorlagen in der Reihenfolge des Blatts besteht.
- „Testfälle mit KI“: Die Musterlösung ist standardmäßig verborgen und wird über die
  Schaltfläche „Musterlösung anzeigen“ / „Musterlösung ausblenden“ vollständig ein- bzw.
  ausgeblendet (barrierefrei mit `aria-expanded`). Nach jeder neuen KI-Erzeugung ist sie
  wieder verborgen.
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
- KI-Testfälle: Ein Beispiel aus dem Blatt zählt nur noch, wenn seine Ausgabe nach der
  Eingabe und nahe bei ihr steht. Bisher reichte es, dass Eingabe und Ausgabe irgendwo im
  Text vorkamen (bei kurzen Eingaben wie „5“ fast immer).
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

[Unveröffentlicht]: https://github.com/BernhardGoetz/CodeChecker/compare/v1.1.1...HEAD
[1.1.1]: https://github.com/BernhardGoetz/CodeChecker/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/BernhardGoetz/CodeChecker/compare/v1.0.3...v1.1.0
[1.0.3]: https://github.com/BernhardGoetz/CodeChecker/compare/v1.0.2...v1.0.3
[1.0.2]: https://github.com/BernhardGoetz/CodeChecker/compare/v1.0.1...v1.0.2
[1.0.1]: https://github.com/BernhardGoetz/CodeChecker/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/BernhardGoetz/CodeChecker/releases/tag/v1.0.0
