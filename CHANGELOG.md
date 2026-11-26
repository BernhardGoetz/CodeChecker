# Änderungsprotokoll

Alle nennenswerten Änderungen am Code-Prüfer stehen in dieser Datei.
Format nach [Keep a Changelog](https://keepachangelog.com/de/1.1.0/), Versionen nach
[Semantic Versioning](https://semver.org/lang/de/). Die Release-Notizen auf GitHub
übernehmen den Abschnitt der jeweiligen Version (`scripts/release-notes.sh`).

## [Unveröffentlicht]

## [1.1.2] - 2026-11-26

### Hinzugefügt

- Website für den Code-Prüfer (GitHub Pages, https://bernhardgoetz.github.io/CodeChecker/):
  Startseite mit Download-Knöpfen für Windows und macOS, „So funktioniert es“,
  Screenshots, Voraussetzungen, Datenschutz in Kürze und häufige Fragen, dazu die
  Anleitung und die übrigen Dokumente als Unterseiten, eine englische Kurzseite und
  die Kontaktseite „Wie du uns erreichst“. Die Seiten entstehen aus den vorhandenen
  Markdown-Dateien (`npm run build:site`) und laden keine externen Ressourcen. Die App
  selbst ist dadurch unverändert.
- KI-Benchmark im Repository (für die Entwicklung, `scripts/ki-bench/`,
  `src-tauri/tests/ki_bench.rs`): feste Aufgaben mit Referenzprogrammen, mit denen sich
  die echte Testfall-Erzeugung messen und Vorher/Nachher vergleichen lässt
  (`evaluate.py`). Jede Änderung an der KI in dieser Version wurde damit gemessen. Die
  Statistik einer Testfall-Erzeugung zählt zusätzlich die Prompt-Tokens aus dem
  Zwischenspeicher, die Länge jeder Lösung und abgeschnittene Lösungen. Für dich als
  Nutzer ändert sich dadurch nichts.

### Geändert

- KI: Die Musterlösungen sind kürzer und schneller fertig. Die KI schreibt keine
  Kommentare und Erklärungen mehr in den Code, sondern benennt Zwischenwerte mit
  sprechenden Variablennamen. Bei Aufgaben ohne Eingabe bleibt vor jeder Ausgabezeile
  ein kurzer Kommentar mit der Teilaufgabe (z. B. „c)“), weil die Lösungen ohne ihn
  öfter mit dem Wert einer anderen Teilaufgabe rechneten. Eine Lösung darf höchstens
  640 statt 1024 Tokens lang sein; ist sie länger, wird sie mit dem Hinweis „kürzer
  schreiben“ neu angefordert. Es bleibt bei drei Lösungen je Aufgabe (#46).
- KI: Alle Schritte der Testfall-Erzeugung beginnen mit demselben Prompt-Anfang
  (Systemprompt und Aufgabentext); die Regeln des jeweiligen Schritts folgen danach.
  So muss die KI-Laufzeit den Aufgabentext nicht bei jedem Schritt neu lesen, sondern
  nimmt ihn aus ihrem Zwischenspeicher. Die Regeln selbst sind unverändert. Die
  Restzeit-Schätzung zählt nur noch den Teil des Prompts, der wirklich neu verarbeitet
  wird (#47).
- KI: Das Häkchen „Auf zufällig richtige Ergebnisse prüfen (Variationstest)“ wird nach
  jeder Testfall-Erzeugung automatisch passend gesetzt: an bei Aufgaben mit festen
  Werten oder ohne Eingabe, sonst aus. Ein kurzer Hinweis am Häkchen zeigt, dass die KI
  es gesetzt hat; du kannst es danach jederzeit von Hand umschalten. Deine von Hand
  gewählte Voreinstellung bleibt gespeichert.
- KI: Aufgaben, die nur eine Klasse mit Methoden und kein Hauptprogramm verlangen
  (z. B. „BankAccount“ mit Konstruktoren, `deposit` und `withdraw`), werden erkannt.
  Statt sich ein Hauptprogramm auszudenken und dessen Ausgabe als richtig zu nehmen,
  sagt die App ehrlich, dass automatische Testfälle dafür noch nicht möglich sind –
  ohne Ampel und ohne KI-Aufruf. Ein passendes Testprogramm für solche Aufgaben ist für
  1.2.0 geplant (#59). Aufgaben, die Methoden *und* ein Programm verlangen (z. B.
  „Schreiben Sie das Programm Backwards …“), bleiben normale Programmaufgaben.

### Behoben

- KI: Lesen die Musterlösungen die Eingabe gar nicht und geben für verschiedene
  Eingaben immer dasselbe aus (z. B. nur `reverseString("Uni")`), gibt es nie mehr Grün.
  Die betroffenen Lösungen werden einmal mit einem Hinweis repariert; lesen sie danach
  die Eingabe, ist das Ergebnis höchstens Gelb, sonst Rot mit verständlichem Grund und
  ohne Testfälle.
- KI: Beispiele aus dem Aufgabentext werden strenger geprüft. Sie zählen nur noch, wenn
  Eingabe und Ausgabe dort wirklich zusammen als Beispiel stehen. Zahlen, die nur
  verstreut im Text vorkommen („75 bis 89 Punkte 2, 60 bis 74 Punkte 3“), ergeben kein
  Beispiel „75 → 3“ mehr; bisher konnte so ein erfundenes Beispiel richtige Lösungen
  verwerfen.
- KI: Standard- und Startwerte von Attributen oder Variablen („standardmäßig auf 0.0
  gesetzt“), Punktangaben („1 Punkt“) und Zahlen in Beispielsätzen („z. B. …“) gelten
  nicht mehr als feste Werte einer Aufgabe ohne Eingabe.
- KI: Beispielsätze wie „z. B. “Your current balance is €100.0”“ werden nicht mehr als
  feste Ausgabevorlage übernommen, wenn das Blatt darin keinen Platzhalter angibt.

## [1.1.1] - 2026-11-16

### Entfernt

- KI: Das kleine Modell Qwen2.5-Coder 1.5B wird nicht mehr angeboten. Es lieferte bei
  Aufgaben ohne Eingabe in einem Vergleich mit 12 Läufen nur 2 richtige Ergebnisse. Die KI
  verwendet nur noch das 7B-Modell. Wer das 1.5B schon heruntergeladen hat, sieht unter
  „KI-Modell“ den Hinweis, dass es nicht mehr unterstützt wird, und kann es dort löschen,
  um Speicherplatz freizugeben. War es ausgewählt, übernimmt das 7B-Modell, wenn es
  eingerichtet ist; sonst ist kein Modell ausgewählt. Eine Auswahl oder ein Import der
  1.5B-Datei wird abgelehnt.
- KI: Rechner mit weniger als 12 GB Arbeitsspeicher bekamen bisher das 1.5B-Modell
  empfohlen. Jetzt zeigt die App dort ehrlich an, dass die KI-Funktion auf diesem Rechner
  nicht verfügbar ist. Code prüfen und eigene Testfälle funktionieren weiterhin.

### Behoben

- KI: Bei Aufgaben ohne Eingabe lasen alle Lösungen eine Zeitangabe gleich falsch
  („15 Uhr, 17 Minuten und 51 Sekunden“ als 17:51:00, Blatt „Aufgabe 2.2 Time“,
  7B-Modell); ein Mehrheitsentscheid kann das nicht korrigieren. Die App liest jetzt
  Angaben aus Zahl und Einheit (Uhr, Stunden, Minuten, Sekunden, km, Euro, % …) ohne KI
  aus dem Aufgabentext und gibt sie den Lösungen als feste Werte vor; Format- und
  Beispielzeilen zählen nicht dazu. Außerdem schreibt jede Lösung je Ausgabezeile einen
  Kommentar mit den verwendeten Werten. Im Vergleich stimmte das Blatt „Time“ damit in 3
  von 3 Läufen (vorher 0 von 3). Aufgaben mit Eingabe sind davon nicht betroffen. Die
  Lösungen werden dadurch länger; die Restdauer rechnet damit.
- KI: Eingaben der KI, die außerhalb der Grenzen der Aufgabe liegen (z. B. „1 <= n <= 100“,
  „von 1 bis 1440“, „positive Zahl“), werden verworfen. Eine Grenze zählt nur, wenn sie im
  Aufgabentext steht.
- KI: Testfälle mit einem Ganzzahl-Überlauf konnten grün sein. Beispiel: Für die Summe der
  Quadrate bis 5000 gaben alle Java-Lösungen mit `int` dieselbe negative Zahl aus. Solche
  Java-Testfälle (große negative Zahl, obwohl Eingabe und Beispiele keine negativen Zahlen
  haben) werden jetzt verworfen, und das Ergebnis ist höchstens gelb, mit Begründung. Ein
  Überlauf, der eine positive Zahl ergibt, wird so nicht erkannt.
- KI: Grün gibt es nur noch, wenn alle Lösungen bei jedem Testfall dieselbe Ausgabe haben.
  Bisher genügten 4 von 5.

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

[Unveröffentlicht]: https://github.com/BernhardGoetz/CodeChecker/compare/v1.1.2...HEAD
[1.1.2]: https://github.com/BernhardGoetz/CodeChecker/compare/v1.1.1...v1.1.2
[1.1.1]: https://github.com/BernhardGoetz/CodeChecker/compare/v1.1.0...v1.1.1
[1.1.0]: https://github.com/BernhardGoetz/CodeChecker/compare/v1.0.3...v1.1.0
[1.0.3]: https://github.com/BernhardGoetz/CodeChecker/compare/v1.0.2...v1.0.3
[1.0.2]: https://github.com/BernhardGoetz/CodeChecker/compare/v1.0.1...v1.0.2
[1.0.1]: https://github.com/BernhardGoetz/CodeChecker/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/BernhardGoetz/CodeChecker/releases/tag/v1.0.0
