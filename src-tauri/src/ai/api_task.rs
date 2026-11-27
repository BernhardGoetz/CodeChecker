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
//! Backwards. Dieses Programm enthält die Methode …“) is no class-only
//! task.
//!
//! Such a task is a *method task with a demo program* (see
//! [`method_demo_task`]) if it asks for a method that returns a value, speaks
//! of a program, but asks for no console input or output: the tests then
//! assume an input/output behaviour (e.g. „read a line, print it reversed“)
//! the text does not ask for. The pipeline still generates tests, but never
//! shows more than yellow, with a note naming the method.

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

/// Parts of words that name a method or function.
const METHOD_PARTS: &[&str] = &["method", "funktion", "function"];

/// Parts of words that say a method returns a value („zurückliefern“,
/// „zurückgibt“, „Rückgabewert“, "returns").
const RETURN_PARTS: &[&str] = &[
    "zurückliefer",
    "zurückgeliefert",
    "zurückzuliefer",
    "zurückgeb",
    "zurückgib",
    "zurückgegeb",
    "zurückzugeb",
    "rückgabe",
    "return",
];

/// Phrases with „Eingabe“/"input" that name a method parameter, not console
/// input („die einen String als Eingabeparameter erhält“).
const PARAMETER_PHRASES: &[&str] = &[
    "eingabeparameter",
    "eingabe-parameter",
    "input parameter",
    "input-parameter",
    "input argument",
];

/// Words directly before „(“ that are no method name.
const NOT_A_METHOD: &[&str] = &[
    "if", "for", "while", "switch", "catch", "return", "new", "main", "print", "println", "printf",
    "format", "input", "len", "str", "int",
];

/// Whether the text asks for console input; a method's „Eingabeparameter“
/// is no console input. Pure.
pub fn has_console_input_hint(text: &str) -> bool {
    let mut lower = text.to_lowercase();
    for p in PARAMETER_PHRASES {
        lower = lower.replace(p, " ");
    }
    no_input::has_input_hint(&lower)
}

/// The name of the method the text asks for: the first word directly
/// followed by „(“ that starts with a lower-case letter and is no keyword
/// (`public static String reverseString(String str)` → `reverseString`).
/// Pure.
pub fn method_name(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let starts = (c.is_ascii_alphabetic() || c == '_')
            && (i == 0
                || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_' || chars[i - 1] == '.'));
        if !starts {
            i += 1;
            continue;
        }
        let begin = i;
        while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
            i += 1;
        }
        let word: String = chars[begin..i].iter().collect();
        if chars.get(i) == Some(&'(')
            && word.starts_with(|c: char| c.is_ascii_lowercase())
            && !NOT_A_METHOD.contains(&word.as_str())
        {
            return Some(word);
        }
    }
    None
}

/// A method task with a demo program: the text asks for a method that
/// returns a value **and** speaks of a program, but asks neither for
/// console input (see [`has_console_input_hint`]) nor for printed output
/// (see [`has_output_hint`]), and it is no class-only task. Example:
/// „Schreiben Sie das Programm Backwards. Dieses Programm enthält die
/// Methode public static String reverseString(String str) … würde z.B.
/// “inU” zurückliefern.“ Returns the method's name if it can be found
/// (`Some(None)`: a method task without a recognisable name), `None` for
/// every other task. Pure.
pub fn method_demo_task(text: &str) -> Option<Option<String>> {
    let lower = text.to_lowercase();
    let is = METHOD_PARTS.iter().any(|p| lower.contains(p))
        && RETURN_PARTS.iter().any(|p| lower.contains(p))
        && has_program_hint(text)
        && !has_console_input_hint(text)
        && !has_output_hint(text)
        && !class_only_task(text);
    is.then(|| method_name(text))
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
    fn backwards_is_a_method_demo_task() {
        assert!(!has_console_input_hint(BACKWARDS));
        assert!(!has_output_hint(BACKWARDS));
        assert_eq!(
            method_demo_task(BACKWARDS),
            Some(Some("reverseString".to_string()))
        );
    }

    #[test]
    fn class_and_io_tasks_are_no_method_demo_tasks() {
        // Class only: stays an API task.
        assert!(method_demo_task(no_input::tests::BANK_ACCOUNT).is_none());
        // „Geben Sie … aus“: a normal program.
        assert!(method_demo_task(TIME).is_none());
        for t in [
            // Method plus console input.
            "Schreiben Sie das Programm Backwards mit der Methode public static String              reverseString(String str), die den umgekehrten String zurückgibt. Das Programm              liest eine Zeile über die Konsole ein und gibt sie umgekehrt aus.",
            "Schreiben Sie ein Programm mit der Methode int doppelt(int n), die 2·n              zurückliefert. Verwenden Sie einen Scanner für die Eingabe.",
            "Write a program with a method square(int x) that returns x*x. The user enters x.",
            // Method plus output.
            "Schreiben Sie ein Programm mit der Methode summe(int a, int b), die die Summe              zurückgibt, und geben Sie summe(2, 3) aus.",
            "Write a program with a function square(x) that returns x*x and print square(4).",
            // No return value / no program.
            "Schreiben Sie eine Methode, die zwei Zahlen addiert.",
            "Schreiben Sie die Methode max(int a, int b), die das Maximum zurückgibt.",
            "Lies eine ganze Zahl n und gib 2·n aus.",
        ] {
            assert!(method_demo_task(t).is_none(), "{t}");
        }
    }

    #[test]
    fn method_demo_tasks_without_signature() {
        assert_eq!(
            method_demo_task(
                "Schreiben Sie ein Programm mit einer Funktion, die die Quersumme einer Zahl                  zurückgibt."
            ),
            Some(None)
        );
        assert_eq!(
            method_demo_task(
                "Write a program with a method isPrime(int n) that returns true if n is prime."
            ),
            Some(Some("isPrime".to_string()))
        );
    }

    #[test]
    fn method_names() {
        assert_eq!(
            method_name("Aufgabe 6.3 (Backwards) die Methode foo(int x)"),
            Some("foo".into())
        );
        assert_eq!(method_name("if (x) main(args) System.out.println(x)"), None);
        assert_eq!(method_name("keine Klammern"), None);
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
