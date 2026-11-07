# Sicherheit

Please report security vulnerabilities privately via GitHub: <https://github.com/BernhardGoetz/CodeChecker/security/advisories/new>.

## Sicherheitslücke melden

Bitte melde Sicherheitslücken **nicht** als öffentliches Issue, sondern vertraulich über
GitHub („Private vulnerability reporting“):

1. Im Repository den Reiter **Security** öffnen.
2. **„Report a vulnerability“** wählen (direkt:
   <https://github.com/BernhardGoetz/CodeChecker/security/advisories/new>).
3. Beschreiben, was betroffen ist (Version, Betriebssystem, Container-Engine), wie sich
   das Problem nachstellen lässt und welche Auswirkung es hat.

Die Meldung sieht nur der Maintainer. Er bestätigt den Eingang, prüft die Meldung und
veröffentlicht nach der Behebung einen Sicherheitshinweis (Security Advisory).

## Unterstützte Versionen

Sicherheitskorrekturen gibt es für die jeweils neueste veröffentlichte Version
(<https://github.com/BernhardGoetz/CodeChecker/releases>). Installierte Versionen
aktualisieren sich über die eingebaute Update-Suche.

## Was als Sicherheitslücke gilt

Zum Beispiel: Code, der aus der Prüf-Sandbox (Container) ausbricht oder Netzwerkzugriff
erhält, Aufgabenblätter oder Modelldateien, die beim Einlesen Code ausführen, oder
Wege, ein Update ohne gültige Signatur zu installieren. Das Bedrohungsmodell steht in
[`CONTRIBUTING.md`](CONTRIBUTING.md), Abschnitt 15.
