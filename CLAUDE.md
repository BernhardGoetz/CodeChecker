# CLAUDE.md

## Windows-Testversion nach jeder Änderung

Der Nutzer baut die App nicht selbst. Nach jeder Code-Änderung, die committet und gepusht wurde, gilt:

- Eine Windows-Testversion per Cross-Build im Linux-Container bauen. Dieser Ablauf ist verifiziert:
  1. `apt-get install -y mingw-w64 nsis unzip jq` und `rustup target add x86_64-pc-windows-gnu`.
  2. Docker-Daemon starten, falls keiner läuft (`dockerd &`). Danach `scripts/export-image-archives.sh amd64`; das legt die Runner-Images unter `src-tauri/resources/images` ab.
  3. `scripts/fetch-llama-server.sh win-x64` (KI-Laufzeit nach `src-tauri/resources/llama`).
  4. `npm run tauri build -- --target x86_64-pc-windows-gnu --bundles nsis --config src-tauri/tauri.release.conf.json`
  5. Den Installer findest du unter `src-tauri/target/x86_64-pc-windows-gnu/release/bundle/nsis/`. Prüfe den genauen Pfad mit `find`.
- Bekannte Abweichungen vom offiziellen Build, die du dem Nutzer jedes Mal mitteilst:
  - Die Visual-C++-Laufzeit-DLLs fehlen; ohne installierte VC++-Laufzeit startet die KI nicht.
  - Das KI-Modell ist nicht enthalten; die App lädt es selbst herunter.
  - Der Installer ist unsigniert.
  - Gebaut wird mit der GNU-Toolchain statt mit MSVC.
- Die erzeugte `.exe` mit Commit-Hash im Dateinamen (z. B. `Code-Pruefer_test_<hash>_x64-setup.exe`) in das Scratchpad kopieren und dem Nutzer direkt im Chat zum Download senden (SendUserFile).
- Dazu angeben: SHA256, enthaltene bzw. fehlende Ressourcen, Hinweis auf fehlende Signatur (SmartScreen: „Weitere Informationen“ → „Trotzdem ausführen“).
- Schlägt der Build fehl, dem Nutzer das ehrlich mitteilen, mit Fehlerursache.
- Den Nutzer nicht auf lokale Build-Schritte (npm, cargo, Rust-Installation) verweisen.
