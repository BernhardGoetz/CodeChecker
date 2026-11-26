//! Deterministic check, without the model, whether a task only asks for a
//! class (attributes, constructors, methods) without a main program, e.g.
//! „Aufgabe 6.2 BankAccount“: „Erstellen Sie die Klasse BankAccount …
//! Implementieren Sie die Methoden deposit und withdraw …“ – nothing is
//! read, nothing is printed, the methods return values.
//!
//! Such a task has no stdin → stdout behaviour the test generation could
//! check: the solutions would have to invent a `main`, and its output would
//! become the "truth". Until class tests exist (issue #59, planned for
//! 1.2.0) the pipeline reports this honestly instead (see
//! [`super::pipeline::Unsupported::ApiTask`]).
//!
//! Conservative: a task is class-only if it names a class **and** members
//! (method, constructor, getter/setter, attribute) **and** has no hint of a
//! program (main program, „Programm“), of reading input (see
//! [`super::no_input::has_input_hint`]) or of printing output. A task that
//! asks for methods *and* a program (e.g. „Schreiben Sie das Programm
//! Backwards. Dieses Programm enthält die Methode …“) is a normal program
//! task.

use super::no_input;

/// Parts of words that name a class.
const CLASS_PARTS: &[&str] = &["klasse", "class"];

/// Parts of words that name members of a class.
const MEMBER_PARTS: &[&str] = &[
    "methode",
    "method",
    "konstruktor",
    "constructor",
    "getter",
    "setter",
    "attribut",
    "attribute",
    "instanzvariable",
    "instance variable",
];

/// Whole words that hint at a program with a main method.
const PROGRAM_WORDS: &[&str] = &[
    "programm",
    "programms",
    "hauptprogramm",
    "testprogramm",
    "program",
    "programs",
    "main",
    "hauptmethode",
    "konsolenprogramm",
    "konsolenanwendung",
];

/// Parts of words that hint at printing output.
const OUTPUT_PARTS: &[&str] = &[
    "ausgabe",
    "ausgeb",
    "ausgegeb",
    "auszugeb",
    "system.out",
    "println",
    "printf",
    "print(",
    "stdout",
    "konsole",
    "bildschirm",
    "output",
    "prints",
    "display",
];

/// Whole words that hint at printing output („Gib … aus“, "print").
const OUTPUT_WORDS: &[&str] = &["gib", "print", "drucke", "anzeigen"];

fn words(lower: &str) -> Vec<&str> {
    lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect()
}

/// Whether the text hints at printing output. Pure.
pub fn has_output_hint(text: &str) -> bool {
    let lower = text.to_lowercase();
    if OUTPUT_PARTS.iter().any(|p| lower.contains(p)) {
        return true;
    }
    let w = words(&lower);
    // „Geben Sie die Sekunden … aus“: a form of „geben“ and a separate „aus“.
    let geben = w
        .iter()
        .any(|x| matches!(*x, "geben" | "gebe" | "gibt" | "gebt"));
    w.iter().any(|x| OUTPUT_WORDS.contains(x)) || (geben && w.contains(&"aus"))
}

/// Whether the text asks for a program (main program, „Programm“). Pure.
pub fn has_program_hint(text: &str) -> bool {
    let lower = text.to_lowercase();
    words(&lower).iter().any(|w| PROGRAM_WORDS.contains(w))
}

/// The task asks only for a class with members, without a main program,
/// input or output (see the module documentation). Pure.
pub fn class_only_task(text: &str) -> bool {
    let lower = text.to_lowercase();
    CLASS_PARTS.iter().any(|p| lower.contains(p))
        && MEMBER_PARTS.iter().any(|p| lower.contains(p))
        && !has_program_hint(text)
        && !no_input::has_input_hint(text)
        && !has_output_hint(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Aufgabe 6.3 Backwards (OOP, Hausaufgabenblatt 6), verbatim.
    const BACKWARDS: &str = "Aufgabe 6.3 (Backwards)\n\
        Schreiben Sie das Programm Backwards. Dieses Programm enthält die Methode \
        public static String reverseString(String str), die einen String als \
        Eingabeparameter erhält. Die Methode soll die darin enthaltene Buchstabenfolge \
        umkehren und in einen String speichern, der der Rückgabewert der Methode ist. Der \
        Aufruf von reverseString(“Uni”) würde z.B. “inU” zurückliefern.\n\
        Laden Sie Ihren Quellcode auf den DevCheck hoch.";

    const TIME: &str = "Aufgabe 2.2 Time\n\
        Schreiben Sie eine Klasse Time. Die aktuelle Uhrzeit ist 15:17:51.\n\
        a) Speichern Sie Stunde, Minute und Sekunde in Variablen.\n\
        b) Geben Sie die Sekunden seit Mitternacht in folgendem Format aus:\n\
        12345678 seconds have passed since midnight.";

    #[test]
    fn bank_account_is_class_only() {
        assert!(class_only_task(no_input::tests::BANK_ACCOUNT));
    }

    #[test]
    fn backwards_is_a_program() {
        // „Schreiben Sie das Programm Backwards“: methods plus a program.
        assert!(has_program_hint(BACKWARDS));
        assert!(!class_only_task(BACKWARDS));
    }

    #[test]
    fn classes_with_output_or_input_are_programs() {
        assert!(!class_only_task(TIME));
        for t in [
            "Schreiben Sie eine Klasse Punkt mit der Methode abstand. Lies zwei Punkte ein.",
            "Erstellen Sie die Klasse Kreis mit einer Methode flaeche und geben Sie die Fläche \
             für r = 2 aus.",
            "Erstellen Sie die Klasse Zaehler mit der Methode erhoehen. Testen Sie sie in einer \
             main-Methode.",
            "Write a class Counter with a method increment and print the result.",
            "Erstellen Sie die Klasse Konto mit der Methode einzahlen. Die Ausgabe soll so \
             aussehen: Kontostand: 5",
        ] {
            assert!(!class_only_task(t), "{t}");
        }
    }

    #[test]
    fn plain_programs_are_no_class_tasks() {
        for t in [
            "Lies eine ganze Zahl n und gib 2·n aus.",
            "Note aus Punktzahl\n\nLies eine ganze Punktzahl zwischen 0 und 100 und gib die \
             Note als Zahl aus.",
            "Schreiben Sie eine Methode, die zwei Zahlen addiert.",
        ] {
            assert!(!class_only_task(t), "{t}");
        }
    }

    #[test]
    fn english_class_tasks_are_recognised() {
        assert!(class_only_task(
            "Write a class Stack with the methods push(int x), pop() and isEmpty(). pop() \
             returns the removed value."
        ));
        assert!(class_only_task(
            "Implementieren Sie die Klasse Bruch mit einem Konstruktor Bruch(int z, int n) und \
             der Methode kuerzen(), die den gekürzten Bruch zurückgibt."
        ));
    }
}
