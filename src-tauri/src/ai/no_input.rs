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
    })
}

/// The task reads no input and gives fixed values: no input hint and a
/// fixed-value hint (both required). Pure.
pub fn task_without_input(text: &str) -> bool {
    !has_input_hint(text) && has_fixed_value_hint(text)
}

#[cfg(test)]
mod tests {
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
}
