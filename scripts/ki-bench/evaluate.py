#!/usr/bin/env python3
"""Wertet KI-Benchmark-Ergebnisse (JSONL von src-tauri/tests/ki_bench.rs) aus.

Für jeden erzeugten Test wird das Referenzprogramm der Aufgabe
(scripts/ki-bench/tasks/<name>.json, Feld "reference") mit der Eingabe
ausgeführt und seine Ausgabe normalisiert (Leerzeichen am Zeilenende,
Leerzeilen am Ende) mit der Erwartung verglichen.

Einstufung je Lauf:
  korrekt      Grün/Gelb und alle Tests stimmen mit der Referenz überein
  falsch-grün  Grün, aber mindestens ein Test weicht ab
  falsch-gelb  Gelb, aber mindestens ein Test weicht ab
  rot-leer     Rot (keine Tests)
  fehler       der Lauf brach ab (Fehlermeldung im JSONL)
Aufgaben mit kind "api" haben keine ausführbare Referenz: Rot gilt dort als
erwartet ("rot-leer"), Grün/Gelb als "api-ungeprüft". Ab 1.1.2-e3 meldet die
Pipeline solche Klassen-Aufgaben ehrlich ("unsupported": "API_TASK", in der
Ampel-Spalte "A"; zählt als "rot-leer").
Aufgaben mit kind "method_demo" (Methode plus Programm, aber keine Konsolen-
Ein-/Ausgabe verlangt, z. B. backwards ab 1.1.2-e3b): Die Ein-/Ausgabe der
Tests ist dort eine Annahme, deshalb wird nicht gegen die Referenz geprüft.
  korrekt      Gelb und Hinweis vorhanden ("ioAssumptionNote")
  falsch-grün  Grün
  falsch-gelb  Gelb ohne Hinweis
  rot-leer     Rot (Gründe aus der Pipeline, keine Tests)
In der Ampel-Spalte steht dort "M" für Gelb mit Hinweis.

Aufruf (nur Standardbibliothek):
  python3 -I scripts/ki-bench/evaluate.py ERGEBNIS.jsonl [...]
  python3 -I scripts/ki-bench/evaluate.py --compare vorher.jsonl nachher.jsonl
  Optionen: --tasks VERZEICHNIS, --details (abweichende Tests zeigen)
"""

import argparse
import json
import os
import subprocess
import sys
import tempfile
from collections import OrderedDict

HERE = os.path.dirname(os.path.abspath(__file__))
CATEGORIES = ["korrekt", "falsch-grün", "falsch-gelb", "rot-leer", "api-ungeprüft", "fehler"]
PHASES = ["structure", "solutions", "repairs", "inputs", "majority"]


def normalize(text):
    lines = [line.rstrip() for line in text.replace("\r\n", "\n").split("\n")]
    while lines and lines[-1] == "":
        lines.pop()
    return "\n".join(lines)


def load_tasks(directory):
    tasks = {}
    for name in sorted(os.listdir(directory)):
        if name.endswith(".json"):
            with open(os.path.join(directory, name), encoding="utf-8") as f:
                t = json.load(f)
            tasks[t["name"]] = t
    return tasks


def load_runs(path):
    runs = []
    with open(path, encoding="utf-8") as f:
        for n, line in enumerate(f, 1):
            line = line.strip()
            if not line:
                continue
            try:
                runs.append(json.loads(line))
            except json.JSONDecodeError as e:
                print(f"{path}:{n}: unlesbare Zeile ({e})", file=sys.stderr)
    return runs


class Reference:
    """Führt Referenzprogramme aus (Ergebnisse je Eingabe zwischengespeichert)."""

    def __init__(self, tasks):
        self.tasks = tasks
        self.dir = tempfile.mkdtemp(prefix="ki-bench-ref-")
        self.cache = {}

    def output(self, task, stdin):
        key = (task, stdin)
        if key not in self.cache:
            path = os.path.join(self.dir, f"{task}.py")
            if not os.path.exists(path):
                with open(path, "w", encoding="utf-8") as f:
                    f.write(self.tasks[task]["reference"])
            try:
                p = subprocess.run(
                    [sys.executable, "-I", path],
                    input=stdin,
                    capture_output=True,
                    text=True,
                    timeout=10,
                    cwd=self.dir,
                )
                self.cache[key] = (p.returncode, p.stdout)
            except subprocess.TimeoutExpired:
                self.cache[key] = (None, "")
        return self.cache[key]


def classify(run, tasks, ref):
    """Kategorie und Liste der abweichenden Tests eines Laufs."""
    if run.get("error"):
        return "fehler", []
    level = run.get("level")
    task = tasks.get(run["task"])
    if level == "red":
        return "rot-leer", []
    if task is not None and task.get("kind") == "method_demo":
        if level == "green":
            return "falsch-grün", []
        return ("korrekt" if run.get("ioAssumptionNote") else "falsch-gelb"), []
    if task is None or task.get("kind") == "api":
        return "api-ungeprüft", []
    wrong = []
    for t in run.get("tests", []):
        code, out = ref.output(run["task"], t["input"])
        if code != 0:
            wrong.append((t, f"Referenz lehnt die Eingabe ab (Exit {code})"))
        elif normalize(out) != normalize(t["expected"]):
            wrong.append((t, out))
    if not wrong:
        return "korrekt", []
    return ("falsch-grün" if level == "green" else "falsch-gelb"), wrong


def mean(values):
    values = [v for v in values if v is not None]
    return sum(values) / len(values) if values else None


def fmt(v, digits=0):
    if v is None:
        return "–"
    return f"{v:.{digits}f}"


def summarize(runs, tasks, ref, details=False):
    """Zusammenfassung je Aufgabe: OrderedDict name -> dict."""
    out = OrderedDict()
    for run in sorted(runs, key=lambda r: (r["task"], r.get("seed", 0))):
        s = out.setdefault(
            run["task"],
            {"runs": 0, "cat": {c: 0 for c in CATEGORIES}, "levels": [], "phase": {p: [] for p in PHASES},
             "wall": [], "prompt": [], "cached": [], "completion": [], "tests": [], "truncated": 0,
             "solutions": 0, "wrong": []},
        )
        cat, wrong = classify(run, tasks, ref)
        s["runs"] += 1
        s["cat"][cat] += 1
        level = {"green": "G", "yellow": "Y", "red": "R"}.get(run.get("level"), "E")
        if run.get("unsupported") == "API_TASK":
            level = "A"
        elif level == "Y" and run.get("ioAssumptionNote"):
            level = "M"
        s["levels"].append(level)
        if not run.get("error"):
            for p in PHASES:
                s["phase"][p].append(run.get("phaseMs", {}).get(p, 0) / 1000.0)
            s["prompt"].append(run.get("promptTokens"))
            s["cached"].append(run.get("cachedTokens"))
            s["completion"].append(run.get("completionTokens"))
            s["tests"].append(run.get("testCount"))
            s["truncated"] += run.get("solutionsTruncated", 0)
            s["solutions"] += len(run.get("solutionTokens", []))
        s["wall"].append(run.get("wallMs", 0) / 1000.0)
        if details:
            for t, got in wrong:
                s["wrong"].append((run.get("seed"), t, got))
    return out


def print_table(label, summary, details=False):
    print(f"\n## {label}\n")
    head = ["Aufgabe", "Läufe", "korrekt", "f-grün", "f-gelb", "rot", "api?", "Fehler", "Ampel",
            "Tests", "Struktur s", "Lösungen s", "Repar. s", "Eingaben s", "Mehrheit s", "Gesamt s",
            "Prompt", "Cache", "Ausgabe", "abgeschn."]
    rows = []
    total = {c: 0 for c in CATEGORIES}
    for name, s in summary.items():
        for c in CATEGORIES:
            total[c] += s["cat"][c]
        rows.append([
            name, str(s["runs"]), str(s["cat"]["korrekt"]), str(s["cat"]["falsch-grün"]),
            str(s["cat"]["falsch-gelb"]), str(s["cat"]["rot-leer"]), str(s["cat"]["api-ungeprüft"]),
            str(s["cat"]["fehler"]), "".join(s["levels"]), fmt(mean(s["tests"]), 1),
            *[fmt(mean(s["phase"][p]), 1) for p in PHASES], fmt(mean(s["wall"]), 1),
            fmt(mean(s["prompt"])), fmt(mean(s["cached"])), fmt(mean(s["completion"])),
            f"{s['truncated']}/{s['solutions']}",
        ])
    widths = [max(len(h), *(len(r[i]) for r in rows)) if rows else len(h) for i, h in enumerate(head)]
    print("| " + " | ".join(h.ljust(w) for h, w in zip(head, widths)) + " |")
    print("|" + "|".join("-" * (w + 2) for w in widths) + "|")
    for r in rows:
        print("| " + " | ".join(c.ljust(w) for c, w in zip(r, widths)) + " |")
    runs = sum(s["runs"] for s in summary.values())
    print(f"\nSumme über {runs} Läufe: " + ", ".join(f"{c} {total[c]}" for c in CATEGORIES))
    print("Ampel: G grün, Y gelb, R rot, A Klassen-Aufgabe (ehrliche Meldung, keine Ampel), "
          "M gelb mit Hinweis zur Methoden-Aufgabe, E Fehler; Zeiten und Tokens: Mittel je Lauf; "
          "abgeschn.: abgeschnittene / alle Lösungsantworten.")
    if details:
        for name, s in summary.items():
            for seed, t, got in s["wrong"]:
                print(f"\n- {name} Seed {seed}, {t['name']}: Eingabe {t['input']!r}\n"
                      f"  erwartet (KI): {t['expected']!r}\n  Referenz:      {got!r}")


def print_compare(la, sa, lb, sb):
    print(f"\n## Vergleich: {la} → {lb}\n")
    head = ["Aufgabe", "korrekt", "falsch (g+y)", "rot", "Ampel", "Gesamt s", "Prompt", "Cache",
            "Ausgabe", "abgeschn."]
    print("| " + " | ".join(head) + " |")
    print("|" + "|".join("---" for _ in head) + "|")

    def pair(a, b, f):
        return f"{f(a) if a else '–'} → {f(b) if b else '–'}"

    for name in list(OrderedDict.fromkeys([*sa.keys(), *sb.keys()])):
        a, b = sa.get(name), sb.get(name)
        cells = [
            name,
            pair(a, b, lambda s: f"{s['cat']['korrekt']}/{s['runs']}"),
            pair(a, b, lambda s: str(s["cat"]["falsch-grün"] + s["cat"]["falsch-gelb"])),
            pair(a, b, lambda s: str(s["cat"]["rot-leer"])),
            pair(a, b, lambda s: "".join(s["levels"])),
            pair(a, b, lambda s: fmt(mean(s["wall"]), 1)),
            pair(a, b, lambda s: fmt(mean(s["prompt"]))),
            pair(a, b, lambda s: fmt(mean(s["cached"]))),
            pair(a, b, lambda s: fmt(mean(s["completion"]))),
            pair(a, b, lambda s: f"{s['truncated']}/{s['solutions']}"),
        ]
        print("| " + " | ".join(cells) + " |")


def label_of(path, runs):
    for r in runs:
        if r.get("label"):
            return r["label"]
    return os.path.splitext(os.path.basename(path))[0]


def main():
    ap = argparse.ArgumentParser(description="Wertet KI-Benchmark-Ergebnisse aus.")
    ap.add_argument("files", nargs="*", help="JSONL-Dateien")
    ap.add_argument("--compare", nargs=2, metavar=("VORHER", "NACHHER"))
    ap.add_argument("--tasks", default=os.path.join(HERE, "tasks"))
    ap.add_argument("--details", action="store_true", help="abweichende Tests anzeigen")
    args = ap.parse_args()
    if not args.files and not args.compare:
        ap.error("JSONL-Datei(en) oder --compare angeben")
    tasks = load_tasks(args.tasks)
    ref = Reference(tasks)
    for path in args.files:
        runs = load_runs(path)
        print_table(label_of(path, runs), summarize(runs, tasks, ref, args.details), args.details)
    if args.compare:
        a, b = (load_runs(p) for p in args.compare)
        print_compare(label_of(args.compare[0], a), summarize(a, tasks, ref),
                      label_of(args.compare[1], b), summarize(b, tasks, ref))


if __name__ == "__main__":
    main()
