# KI-Benchmark

Misst die Testfall-Erzeugung (`pipeline::generate`) mit echtem Modell, echtem
llama-server und echter Sandbox auf festen Aufgaben, damit Änderungen an
Prompts und Pipeline vorher/nachher verglichen werden können.

- `tasks/*.json`: Aufgaben (`name`, `language` `python`/`java`, `text`,
  `kind` `program`/`api`, `reference`). `reference` ist bei `program` ein
  Python-Programm (stdin → stdout), das die richtigen Ausgaben liefert; bei
  `api` (Klassen/Methoden ohne Konsolen-Ein-/Ausgabe) nur eine Beschreibung,
  dort gilt Rot/keine Tests als erwartet (ab 1.1.2-e3 meldet die Pipeline das
  ehrlich als Klassen-Aufgabe, Ampel-Spalte „A“). `backwards` ist seit
  1.1.2-e3 `program` (vorher `api`); frühere Läufe sind für diese Aufgabe nicht
  direkt vergleichbar.
- `src-tauri/tests/ki_bench.rs`: der Lauf (`#[ignore]`), schreibt je Lauf
  eine JSON-Zeile nach `src-tauri/target/ki-bench/<label>.jsonl`.
- `evaluate.py`: prüft die erzeugten Tests gegen die Referenzen.
- `results/`: eingecheckte Messungen (z. B. `1.1.1-baseline.jsonl`).

## Vorbereitung (Linux)

```sh
scripts/ki-bench/fetch-model.sh ~/llm-bench/models          # 4,7 GB, prüft SHA-256
scripts/fetch-llama-server.sh linux-x64 ~/llm-bench/llama   # gepinnte Version
docker build -t code-pruefer-python:dev images/python
docker build -t code-pruefer-java:dev images/java
```

## Lauf

```sh
cd src-tauri
KI_BENCH_MODEL=~/llm-bench/models/qwen2.5-coder-7b-instruct-q4_k_m.gguf \
KI_BENCH_LLAMA_SERVER=~/llm-bench/llama/llama-server \
KI_BENCH_LABEL=mein-versuch \
cargo test --test ki_bench -- --ignored --nocapture
```

Weitere Variablen: `KI_BENCH_RUNS` (Läufe je Aufgabe, Standard 3),
`KI_BENCH_SEED` (erster Seed, Standard 1), `KI_BENCH_TASKS`
(z. B. `time,py-summe`), `KI_BENCH_SOLUTIONS` (Standard 3),
`KI_BENCH_TIMEOUT_S` (je Lauf, Standard 3600). Läufe, die schon in der
Ergebnisdatei stehen (gleiche Aufgabe und Seed), werden übersprungen; ein
abgebrochener Benchmark setzt also einfach fort. Auf einer CPU mit 4 Kernen
dauert ein Lauf einige Minuten, alle 11 Aufgaben × 3 Seeds einige Stunden;
am besten mit `nohup … > bench.log 2>&1 &` starten.

Die Seeds machen die Anfragen wiederholbar, aber nicht bitgenau: parallele
Slots rechnen in gemeinsamen Batches.

## Auswertung

```sh
python3 -I scripts/ki-bench/evaluate.py src-tauri/target/ki-bench/mein-versuch.jsonl
python3 -I scripts/ki-bench/evaluate.py --details …          # abweichende Tests zeigen
python3 -I scripts/ki-bench/evaluate.py --compare \
  scripts/ki-bench/results/1.1.1-baseline.jsonl src-tauri/target/ki-bench/mein-versuch.jsonl
```

Kategorien je Lauf: `korrekt` (Grün/Gelb, alle Tests stimmen mit der
Referenz), `falsch-grün`, `falsch-gelb` (mindestens ein Test weicht ab),
`rot-leer` (Rot, keine Tests), `api-ungeprüft` (Grün/Gelb bei einer
`api`-Aufgabe), `fehler` (Lauf abgebrochen). Dazu je Aufgabe die mittleren
Zeiten je Phase, Prompt-, Cache- und Ausgabe-Tokens und die Zahl der am
Token-Limit abgeschnittenen Lösungen.
