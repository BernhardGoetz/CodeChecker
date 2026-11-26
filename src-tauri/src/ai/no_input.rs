//! Deterministic check, without the model, whether a task reads no input
//! but works with fixed values given in the text ("Die aktuelle Uhrzeit ist
//! 15:17:51 … Speichern Sie Stunde, Minute und Sekunde in Variablen").
//!
//! Regression (release 1.1.0, „Aufgabe 2.2 Time“, 1.5B model): the model
//! said `readsInput: true`, the solutions read „hour minute second“ from
//! standard input, and running them on a probe input confirmed that – the
//! check was circular. The text alone settles it here: it asks for nothing
//! to be entered and gives the values.
//!
//! Conservative: a task counts as "without input" only if the text has no
//! hint at all of reading input (German or English, also negated phrases
//! like „keine Eingabe“ are removed first) **and** a hint of fixed values
//! (a word about variables/assigning plus a number that is no placeholder
//! and no exercise number). Everything else is left to the model and the
//! probe run as before.

use crate::runner::template;

/// Phrases that say the program reads nothing; removed before looking for
/// input hints (they contain "Eingabe"/"input" themselves).
const NEGATED: &[&str] = &[
    "keine benutzereingabe",
    "keinerlei eingabe",
    "keine eingabe",
    "ohne benutzereingabe",
    "ohne eingabe",
    "eingabe: keine",
    "eingabe: -",
    "nichts eingelesen",
    "nicht eingelesen",
    "nichts einlesen",
    "nicht einlesen",
    "liest keine",
    "liest nichts",
    "no user input",
    "no input",
    "without input",
    "does not read",
    "reads no",
    "reads nothing",
];

/// Parts of words that hint at reading input (German compounds included:
/// „Benutzereingabe“, „eingelesen“, „einzulesen“).
const INPUT_PARTS: &[&str] = &[
    "eingabe",
    "eingeb",
    "eingegeb",
    "einles",
    "eingeles",
    "einlies",
    "einzules",
    "einzugeb",
    "tastatur",
    "kommandozeil",
    "konsole",
    "benutzer",
    "nutzer",
    "anwender",
    "scanner",
    "bufferedreader",
    "system.in",
    "stdin",
    "input",
    "argv",
    "args[",
    "command line",
    "command-line",
    "keyboard",
    "user",
];

/// Whole words that hint at reading input („Lies eine Zahl“, „liest … ein“,
/// "read", "enter").
const INPUT_WORDS: &[&str] = &[
    "lies", "liest", "lesen", "gelesen", "read", "reads", "reading", "enter", "enters", "entered",
    "prompt", "abfragen",
];

/// Parts of words that hint at fixed values in variables („Weisen Sie den
/// Variablen die Werte … zu“, „Legen Sie Variablen … an“).
const VALUE_PARTS: &[&str] = &[
    "variable",
    "konstante",
    "zuweis",
    "weisen sie",
    "zugewiesen",
    "initialisier",
    "assign",
    "constant",
];

/// Words before a number that make it an exercise or version number
/// („Aufgabe 2.2“, „Java 21“), not a value.
const NUMBERING: &[&str] = &[
    "aufgabe",
    "hausaufgabe",
    "teilaufgabe",
    "übung",
    "übungsblatt",
    "blatt",
    "teil",
    "kapitel",
    "abschnitt",
    "seite",
    "nr",
    "nr.",
    "exercise",
    "task",
    "sheet",
    "chapter",
    "java",
    "python",
    "version",
];

fn lower_without_negations(text: &str) -> String {
    let mut t = text.to_lowercase();
    for n in NEGATED {
        t = t.replace(n, " ");
    }
    t
}

/// Whether the text hints at reading input. Pure.
pub fn has_input_hint(text: &str) -> bool {
    let t = lower_without_negations(text);
    if INPUT_PARTS.iter().any(|p| t.contains(p)) {
        return true;
    }
    t.split(|c: char| !c.is_alphanumeric())
        .any(|w| INPUT_WORDS.contains(&w))
}

/// Whether the text gives fixed values for variables: a word about
/// variables or assigning, and at least one number that is no declared
/// placeholder („12345678 ist ein Platzhalter“), no exercise number and
/// no list mark („1)“). Pure.
pub fn has_fixed_value_hint(text: &str) -> bool {
    let lower = text.to_lowercase();
    if !VALUE_PARTS.iter().any(|p| lower.contains(p)) {
        return false;
    }
    let placeholders = template::declared_placeholder_tokens(text);
    let words: Vec<&str> = text.split_whitespace().collect();
    words.iter().enumerate().any(|(i, w)| {
        if w.ends_with(')') && !w.starts_with('(') {
            return false;
        }
        let token = w.trim_matches(|c: char| !c.is_alphanumeric());
        if !token.chars().any(|c| c.is_ascii_digit()) || placeholders.iter().any(|p| p == token) {
            return false;
        }
        let prev = i
            .checked_sub(1)
            .map(|j| {
                words[j]
                    .trim_matches(|c: char| !c.is_alphanumeric() && c != '.')
                    .to_lowercase()
            })
            .unwrap_or_default();
        !NUMBERING.contains(&prev.as_str())
            && !is_score(words.get(i + 1).copied())
            && !default_or_example_value(&words, i)
    })
}

/// Words after a number that make it a score of the exercise („(1 Punkt)“).
const SCORE_WORDS: &[&str] = &["punkt", "punkte", "pkt", "pkt.", "point", "points", "p."];

fn is_score(next: Option<&str>) -> bool {
    next.is_some_and(|w| {
        let w = w
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '.')
            .to_lowercase();
        SCORE_WORDS.contains(&w.as_str())
    })
}

/// Starts of words before a number that make it the default or start value
/// of an attribute or variable („balance …, der standardmäßig auf 0.0
/// gesetzt ist“, „Startwert 0“), not a fixed value of the task.
const DEFAULT_STEMS: &[&str] = &[
    "standard",
    "default",
    "startwert",
    "anfangswert",
    "initialwert",
    "voreingestellt",
    "vorbelegt",
];

/// Words before a number that make it part of an example sentence („z. B.
/// “Your current balance is €100.0”“), not a fixed value of the task.
const EXAMPLE_WORDS: &[&str] = &[
    "z.b.",
    "zb.",
    "bspw.",
    "beispielsweise",
    "beispiel",
    "e.g.",
    "example",
];

/// How many words before a number are searched for [`DEFAULT_STEMS`] and
/// [`EXAMPLE_WORDS`].
const CONTEXT_WORDS: usize = 6;

/// Whether the number at `words[i]` is a default/start value or part of an
/// example sentence (see [`DEFAULT_STEMS`], [`EXAMPLE_WORDS`]): such numbers
/// are no fixed values of the task (regression „Aufgabe 6.2 BankAccount“:
/// „standardmäßig auf 0.0 gesetzt“ made a class task one with fixed values).
fn default_or_example_value(words: &[&str], i: usize) -> bool {
    let from = i.saturating_sub(CONTEXT_WORDS);
    let before: Vec<String> = words[from..i]
        .iter()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric() && c != '.')
                .to_lowercase()
        })
        .collect();
    before.iter().enumerate().any(|(k, w)| {
        DEFAULT_STEMS.iter().any(|s| w.starts_with(s))
            || EXAMPLE_WORDS.contains(&w.as_str())
            || (w == "z." && before.get(k + 1).is_some_and(|n| n == "b."))
            || (w == "e." && before.get(k + 1).is_some_and(|n| n == "g."))
            || (w == "zum" && before.get(k + 1).is_some_and(|n| n == "beispiel"))
    })
}

/// The task reads no input and gives fixed values: no input hint and a
/// fixed-value hint (both required). Pure.
pub fn task_without_input(text: &str) -> bool {
    !has_input_hint(text) && has_fixed_value_hint(text)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The sheet of regression case „Aufgabe 2.2 Time“ (as in the pipeline
    /// tests).
    const TIME_TASK: &str = "Aufgabe 2.2 Time\n\
        Schreiben Sie eine Klasse Time. Die aktuelle Uhrzeit ist 15:17:51, begonnen haben Sie um 15:10:30.\n\
        a) Speichern Sie Stunde, Minute und Sekunde in Variablen.\n\
        b) Berechnen Sie die Sekunden seit Mitternacht.\n\
        c) Geben Sie die Sekunden seit Mitternacht in folgendem Format aus:\n\
        12345678 seconds have passed since midnight.\n\
        Die Ausgabe sollte folgendermaßen aussehen (12345678 ist ein Platzhalter):\n\
        12345678 seconds have passed since midnight.\n";

    #[test]
    fn time_sheet_reads_no_input() {
        assert!(!has_input_hint(TIME_TASK));
        assert!(has_fixed_value_hint(TIME_TASK));
        assert!(task_without_input(TIME_TASK));
    }

    #[test]
    fn input_hints_win() {
        for t in [
            "Lies eine ganze Zahl n und gib 2·n aus. Speichere sie in einer Variablen n = 5.",
            "Der Benutzer gibt Stunde, Minute und Sekunde ein. Variable hour = 15.",
            "Das Programm liest drei Zahlen ein und weist sie den Variablen a, b, c zu (z. B. 1 2 3).",
            "Verwenden Sie die Klasse Scanner. Variablen x = 3.",
            "Read two integers and assign them to variables a = 1, b = 2.",
            "Enter a number. Store it in variable n (e.g. 42).",
            "Lesen Sie die Werte über die Kommandozeile ein: Variablen a=1.",
            "Eingabe: zwei Zahlen. Variablen a = 4, b = 5.",
            "x = input(); Variable x = 7",
        ] {
            assert!(!task_without_input(t), "{t}");
        }
    }

    #[test]
    fn negated_input_phrases_are_no_hint() {
        let t = "Das Programm erhält keine Eingabe. Weisen Sie den Variablen a und b die Werte 3 und 4 zu.";
        assert!(!has_input_hint(t));
        assert!(task_without_input(t));
        assert!(task_without_input(
            "No input. Assign the values 3 and 4 to the variables a and b."
        ));
    }

    #[test]
    fn fixed_values_need_a_real_number_and_a_variable_word() {
        // No variable word.
        assert!(!has_fixed_value_hint("Gib die Summe von 3 und 4 aus."));
        // Only an exercise number and a placeholder.
        assert!(!has_fixed_value_hint(
            "Aufgabe 2.2\nSpeichern Sie das Ergebnis in einer Variablen und geben Sie aus \
             (12345678 ist ein Platzhalter):\n12345678 seconds"
        ));
        // List marks are no values.
        assert!(!has_fixed_value_hint(
            "1) Legen Sie Variablen an.\n2) Geben Sie sie aus."
        ));
        assert!(!has_fixed_value_hint("Variablen in Java 21 anlegen."));
        assert!(has_fixed_value_hint(
            "Legen Sie Variablen a = 12 und b = 30 an."
        ));
        assert!(has_fixed_value_hint("Konstante PI mit dem Wert 3.14159."));
    }

    /// Aufgabe 6.2 BankAccount (OOP, Hausaufgabenblatt 6), as in the
    /// KI-Benchmark.
    pub(crate) const BANK_ACCOUNT: &str = "Aufgabe 6.2 (BankAccount (1 Punkt))\n\
        a) Erstellen Sie die Klasse-BankAccount, die ein einfaches Bankkonto simuliert. Die Klasse besitzt zwei Variablen: String ownerName für den Namen des Kontoinhabers und double balance für den Kontostand, der standardmäßig auf 0.0 gesetzt ist. Jede Instanz der Klasse BankAccount soll ihren eigenen Kontostand haben.\n\
        b) Implementieren Sie die Getter- und Setter-Methoden getOwnerName, setOwnerName und getBalance für die Instanzvariablen.\n\
        (Hinweis: Für balance benötigen wir keine Setter-Methode, da der Kontostand entweder erhöht oder verringert, aber nicht gesetzt werden kann.)\n\
        c) Implementieren Sie folgende Konstruktoren:\n\
        i. Ein Konstruktor, der ownerName initialisiert, wobei der Standardwert für balance 0.0 sein soll.\n\
        ii. Ein Konstruktor, der keinen Parameter hat und für ownerName als Standardwert “Unknown” setzt und für balance den Standardwert 0.0 setzt.\n\
        d) Implementieren Sie die folgenden Methoden:\n\
        - deposit(double amount): Diese Methode soll es ermöglichen, einen Betrag zur balance hinzuzufügen, wenn der Betrag größer als null ist. Wenn die Einzahlung erfolgreich ist, soll eine formatierte Nachricht zurückgegeben werden, die den aktuellen Kontostand angibt, z.B. “Your current balance is €100.0”.\n\
        Ist der Betrag nicht positiv, soll die Nachricht, “The amount must be greater than zero.” zurückgegeben werden.\n\
        - withdraw(double amount): Diese Methode soll es ermöglichen, einen Betrag von balance abzuziehen, wenn ausreichend Mittel vorhanden sind. Wenn das Guthaben nicht ausreicht, soll die Nachricht, “You have insufficient funds.” zurückgegeben werden. Bei erfolgreicher Abhebung soll eine formatierte Nachricht zurückgegeben werden, die den aktuellen Kontostand angibt, z.B. “Your current balance is €100.0”.\n\
        Laden Sie Ihren Quellcode auf den DevCheck hoch.";

    #[test]
    fn default_values_of_attributes_are_no_fixed_values() {
        // Regression 1.1.2: BankAccount counted as a task with fixed values
        // („Variablen“, „standardmäßig auf 0.0“, „z.B. … €100.0“, „1 Punkt“).
        assert!(!has_input_hint(BANK_ACCOUNT));
        assert!(!has_fixed_value_hint(BANK_ACCOUNT));
        assert!(!task_without_input(BANK_ACCOUNT));
        for t in [
            "Die Variable count hat den Standardwert 0.",
            "Die Variable x ist standardmäßig auf 1.5 gesetzt.",
            "Die Variable n hat den Startwert 10.",
            "Die Variable n bekommt den default value 3.",
            "Variablen anlegen, z. B. „Kontostand 100.0“.",
            "Variablen anlegen, e.g. total 12.",
            "Aufgabe (2 Punkte): Legen Sie Variablen an.",
        ] {
            assert!(!has_fixed_value_hint(t), "{t}");
        }
        // Real fixed values next to a default value still count.
        assert!(has_fixed_value_hint(
            "Die Variable a hat den Standardwert 0. Weisen Sie b den Wert 7 zu."
        ));
        assert!(task_without_input(TIME_TASK));
    }
}
