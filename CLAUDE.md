# CLAUDE.md

## Windows-Testversion nach jeder Änderung

Der Nutzer baut die App nicht selbst. Nach jeder Code-Änderung, die committet und gepusht wurde, gilt:

- Eine Windows-Testversion bauen: per Cross-Build im Linux-Container, siehe README Abschnitt „Entwicklung“: `npm run tauri build -- --target x86_64-pc-windows-gnu --bundles nsis`. Benötigt mingw-w64, nsis und `rustup target add x86_64-pc-windows-gnu`.
- Die erzeugte `.exe` mit Commit-Hash im Dateinamen (z. B. `Code-Pruefer_test_<hash>_x64-setup.exe`) in das Scratchpad kopieren und dem Nutzer direkt im Chat zum Download senden (SendUserFile).
- Dazu angeben: SHA256, enthaltene bzw. fehlende Ressourcen, Hinweis auf fehlende Signatur (SmartScreen: „Weitere Informationen“ → „Trotzdem ausführen“).
- Schlägt der Build fehl, dem Nutzer das ehrlich mitteilen, mit Fehlerursache.
- Den Nutzer nicht auf lokale Build-Schritte (npm, cargo, Rust-Installation) verweisen.
