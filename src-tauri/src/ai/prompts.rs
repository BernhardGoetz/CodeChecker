//! Prompts and JSON schemas for the test generation pipeline, and robust
//! extraction of code from model answers. System prompts are English (more
//! reliable with Qwen2.5-Coder); the task text is passed on unchanged.

use serde_json::{json, Value};

use super::llama::ChatMessage;
use crate::runner::template::{
    placeholder_kind, OutputTemplate, PlaceholderKind, MAX_PLACEHOLDERS, MAX_TEMPLATES,
};

/// Upper bound for examples taken from the task sheet.
pub const MAX_EXAMPLES: usize = 5;
/// Upper bound for generated test inputs.
pub const MAX_INPUTS: usize = 10;
/// Longest input/output format description of the structure step.
pub const MAX_FORMAT_CHARS: usize = 300;

/// `class_name`: Java class named by the task ([`class_name_from_task`]),
/// else `Main`.
fn language_rules(language: &str, class_name: Option<&str>) -> String {
    match language {
        "java" => {
            let class = class_name.unwrap_or(crate::runner::java::DEFAULT_CLASS);
            format!(
                "Write Java 21. Put everything into `public class {class}` with \
                 `public static void main(String[] args)` (the file is {class}.java), no \
                 package declaration, no other class with a main method, only the standard \
                 library. Read the input with `java.util.Scanner` or `java.io.BufferedReader` \
                 from System.in. Never print a prompt before reading input (no \
                 `System.out.print(\"Zahl eingeben: \")`) – prompts would end up in the output."
            )
        }
        _ => "Write Python 3.12 using only the standard library. Read the input from \
              standard input (`input()` or `sys.stdin`). Never pass a prompt string to \
              `input()` – prompts would end up in the output."
            .to_string(),
    }
}

/// Well-known JDK classes that task sheets mention ("Verwenden Sie die
/// Klasse Scanner") – never the class to write.
const JDK_CLASSES: &[&str] = &[
    "ArrayList",
    "Arrays",
    "BigDecimal",
    "BigInteger",
    "Boolean",
    "BufferedReader",
    "Character",
    "Collections",
    "Comparable",
    "Comparator",
    "Double",
    "Exception",
    "File",
    "Files",
    "HashMap",
    "HashSet",
    "InputStreamReader",
    "Integer",
    "Iterator",
    "LinkedList",
    "List",
    "LocalDate",
    "LocalDateTime",
    "LocalTime",
    "Long",
    "Map",
    "Math",
    "Object",
    "Optional",
    "Path",
    "Random",
    "Scanner",
    "Set",
    "Stream",
    "String",
    "StringBuilder",
    "System",
    "Thread",
    "TreeMap",
];

/// German nouns that follow "Klasse" without being a class name ("in der
/// Klasse Methoden ergänzen"); only checked for unquoted names.
const GERMAN_NOUNS: &[&str] = &[
    "Attribute",
    "Datei",
    "Eigenschaften",
    "Felder",
    "Instanzen",
    "Konstruktor",
    "Konstruktoren",
    "Methode",
    "Methoden",
    "Objekte",
    "Variablen",
    "Werte",
];

/// A plausible class name: ASCII Java identifier with a capital first
/// letter, not a JDK class.
fn plausible_class(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name.len() <= 64
        && !JDK_CLASSES.contains(&name)
}

const QUOTES: &[char] = &['`', '"', '\'', '„', '“', '”', '‚', '‘', '’', '»', '«', '*'];

/// The Java class an exercise sheet asks for, e.g. „Öffnen Sie die
/// bereitgestellte Java-Klasse `Time`“ → `Time`. Conservative: recognised
/// are `<Name>.java`, `class <Name>` and „Klasse <Name>“ (also
/// „Java-Klasse“, „Hauptklasse“, with or without quotes), in this order of
/// priority; the name must be a plausible class name. If the strongest
/// pattern yields different names, the result is `None` (→ `Main`). Pure.
pub fn class_name_from_task(task: &str) -> Option<String> {
    let words: Vec<&str> = task.split_whitespace().collect();
    let mut by_file = Vec::new();
    let mut by_class = Vec::new();
    let mut by_klasse = Vec::new();
    for (i, w) in words.iter().enumerate() {
        // `Time.java`, „Time.java“, (Time.java)
        let bare = w.trim_matches(|c: char| QUOTES.contains(&c) || "()[],;:!?".contains(c));
        let bare = bare.strip_suffix('.').unwrap_or(bare);
        if let Some(stem) = bare.strip_suffix(".java") {
            if plausible_class(stem) {
                by_file.push(stem.to_string());
            }
        }
        let Some(next) = words.get(i + 1) else {
            continue;
        };
        let quoted = next.starts_with(QUOTES);
        let name = next
            .trim_start_matches(QUOTES)
            .trim_end_matches(|c: char| QUOTES.contains(&c) || ".,;:!?)({<".contains(c));
        let name = name.strip_suffix(".java").unwrap_or(name);
        if !plausible_class(name) {
            continue;
        }
        if *w == "class" {
            by_class.push(name.to_string());
        } else if w.to_lowercase().ends_with("klasse") && (quoted || !GERMAN_NOUNS.contains(&name))
        {
            by_klasse.push(name.to_string());
        }
    }
    [by_file, by_class, by_klasse]
        .into_iter()
        .find(|names| !names.is_empty())
        .and_then(|names| {
            names
                .iter()
                .all(|n| *n == names[0])
                .then(|| names[0].clone())
        })
}

fn fence(language: &str) -> &'static str {
    match language {
        "java" => "java",
        _ => "python",
    }
}

fn task_block(task_text: &str) -> String {
    format!("Task (verbatim from the exercise sheet):\n<<<\n{task_text}\n>>>")
}

// --- Step 1: structure ------------------------------------------------------

pub const STRUCTURE_SYSTEM: &str = "You analyse programming exercises whose programs read \
from standard input and write to standard output. Extract:\n\
- inputFormat: how the input looks (one or two sentences; \"no input\" if the program \
reads nothing),\n\
- readsInput: true if the program reads standard input, false if it reads nothing,\n\
- inputConstraints: the limits the task states for the input (value ranges, sizes, allowed \
values or characters), e.g. \"1 <= n <= 1000, whole numbers\"; an empty string if the task \
states none. Never invent limits,\n\
- outputFormat: what exactly has to be printed (one or two sentences),\n\
- examples: every example input/output pair that is printed in the task text, copied \
character by character. Never invent, compute, complete or reformat examples. If the text \
contains no example, return an empty list. If an example has no input, use an empty \
string. Never put an output template (see below) into examples.\n\
- outputTemplates: output format templates, in which placeholder values stand in for the \
real values (e.g. \"12345678 seconds have passed\" when the task says 12345678 is only a \
placeholder, or \"<n>\"/\"X\"). text: the template copied character by character; \
placeholders: the placeholder tokens exactly as they appear in the text. A template spanning \
several consecutive lines is ONE template; never split it into one template per line. A \
template is not an example. Empty list if there is none. Do not repeat a template in \
examples; keep inputFormat and outputFormat short.\n\
- compareMode: \"float\" if the output contains decimal numbers where tiny rounding \
differences are acceptable, \"tokens\" if only the words/numbers matter and spacing or \
line breaks do not, otherwise \"lines\".\n\
Answer with JSON only.";

pub fn structure_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            // Short descriptions bound runaway answers. Examples and
            // templates stay unbounded: a cut-off one would not be verbatim.
            "inputFormat": { "type": "string", "maxLength": MAX_FORMAT_CHARS },
            "readsInput": { "type": "boolean" },
            "inputConstraints": { "type": "string", "maxLength": MAX_FORMAT_CHARS },
            "outputFormat": { "type": "string", "maxLength": MAX_FORMAT_CHARS },
            "examples": {
                "type": "array",
                "maxItems": MAX_EXAMPLES,
                "items": {
                    "type": "object",
                    "properties": {
                        "input": { "type": "string" },
                        "output": { "type": "string" }
                    },
                    "required": ["input", "output"]
                }
            },
            "outputTemplates": {
                "type": "array",
                "maxItems": MAX_TEMPLATES,
                "items": {
                    "type": "object",
                    "properties": {
                        "text": { "type": "string" },
                        "placeholders": {
                            "type": "array",
                            "maxItems": MAX_PLACEHOLDERS,
                            "items": { "type": "string" }
                        }
                    },
                    "required": ["text", "placeholders"]
                }
            },
            "compareMode": { "type": "string", "enum": ["lines", "tokens", "float"] }
        },
        "required": [
            "inputFormat",
            "readsInput",
            "inputConstraints",
            "outputFormat",
            "examples",
            "outputTemplates",
            "compareMode"
        ]
    })
}

pub fn structure_messages(task_text: &str) -> Vec<ChatMessage> {
    vec![
        ChatMessage::system(STRUCTURE_SYSTEM),
        ChatMessage::user(task_block(task_text)),
    ]
}

// --- Step 2: solutions ------------------------------------------------------

/// What the solution prompt knows about the task besides its text.
pub struct SolutionContext<'a> {
    pub input_format: &'a str,
    pub output_format: &'a str,
    pub examples: &'a [(String, String)],
    /// Verified output templates (format only, placeholder values).
    pub output_templates: &'a [OutputTemplate],
    /// Java: class named by the task ([`class_name_from_task`]), else `Main`.
    pub class_name: Option<&'a str>,
}

pub fn solution_messages(
    task_text: &str,
    language: &str,
    ctx: &SolutionContext,
) -> Vec<ChatMessage> {
    let system = format!(
        "You are an expert programmer. Solve the programming exercise with a complete, \
         correct program. {} Print exactly the required output and nothing else (no \
         prompts, no explanations, no extra spaces). Answer with exactly one ```{} code \
         block containing the whole program.",
        language_rules(language, ctx.class_name),
        fence(language)
    );
    let mut user = task_block(task_text);
    if !ctx.input_format.trim().is_empty() {
        user.push_str(&format!("\n\nInput format: {}", ctx.input_format.trim()));
    }
    if !ctx.output_format.trim().is_empty() {
        user.push_str(&format!("\nOutput format: {}", ctx.output_format.trim()));
    }
    for (i, (input, output)) in ctx.examples.iter().enumerate() {
        user.push_str(&format!(
            "\n\nExample {}\nInput:\n```\n{}\n```\nExpected output:\n```\n{}\n```",
            i + 1,
            input.trim_end(),
            output.trim_end()
        ));
    }
    let several = ctx.output_templates.len() > 1;
    if several {
        user.push_str(
            "\n\nThe sheet shows several output format templates. The output consists of one or \
             more of these blocks, in this order.",
        );
    }
    for t in ctx.output_templates {
        let tokens = t
            .placeholders
            .iter()
            .map(|p| format!("`{p}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let print = if several {
            "Print this block in exactly this format where it applies"
        } else {
            "Print exactly this format"
        };
        user.push_str(&format!(
            "\n\nOutput format template (placeholders, not real values). Placeholders: {tokens}. \
             {print}, replace every placeholder by the real value your program computes, and keep \
             all other text unchanged:\n```\n{}\n```",
            t.text.trim_end()
        ));
        for p in &t.placeholders {
            if placeholder_kind(p) == PlaceholderKind::Integer {
                user.push_str(&format!(
                    "\nPlaceholder `{p}` stands for an integer: print it as a whole number \
                     without decimal places."
                ));
            }
        }
    }
    vec![ChatMessage::system(system), ChatMessage::user(user)]
}

/// Start of a solution answer (written into the prompt): the model
/// continues inside the code block.
pub fn code_prefill(language: &str) -> String {
    format!("```{}\n", fence(language))
}

/// Generation stops at the closing fence: nothing after the code.
pub const CODE_STOP: &str = "\n```";

/// Follow-up after a failed compile: the previous answer plus the
/// compiler message (truncated).
pub fn repair_messages(
    mut conversation: Vec<ChatMessage>,
    previous_answer: &str,
    compiler_output: &str,
    language: &str,
) -> Vec<ChatMessage> {
    let mut errors: String = compiler_output.chars().take(2000).collect();
    if errors.trim().is_empty() {
        errors = "(no compiler message)".into();
    }
    conversation.push(ChatMessage::assistant(previous_answer));
    conversation.push(ChatMessage::user(format!(
        "The program does not compile:\n```\n{}\n```\nFix the error. Answer with the complete \
         corrected program in exactly one ```{} code block.",
        errors.trim_end(),
        fence(language)
    )));
    conversation
}

// --- Step 5: test inputs ----------------------------------------------------

pub const INPUTS_SYSTEM: &str = "You write test inputs for programs that solve a \
programming exercise. Produce up to 10 different inputs that are valid according to the \
task and its input format: typical cases and edge cases (smallest and largest allowed \
values, boundaries, special cases mentioned in the task). Every input must respect the \
input limits; edge cases only inside the limits. Never write inputs the task does not \
require the program to handle (invalid or malformed input) unless the task says what the \
program has to do with them. Prefer inputs that lead to different outputs over many inputs \
with the same output. Each input is the complete text given to the program on standard \
input, with line breaks as \\n. Only inputs, never outputs. Answer with JSON only.";

pub fn inputs_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "inputs": {
                "type": "array",
                "maxItems": MAX_INPUTS,
                "items": { "type": "string" }
            }
        },
        "required": ["inputs"]
    })
}

/// What the inputs prompt knows about the task besides its text.
#[derive(Debug, Clone, Copy, Default)]
pub struct InputsContext<'a> {
    pub input_format: &'a str,
    pub output_format: &'a str,
    /// Limits for the input stated by the task (`inputConstraints`).
    pub constraints: &'a str,
    /// Verified example pairs of the sheet (input, output verbatim from the
    /// sheet – never from the model), at most [`MAX_EXAMPLES`].
    pub examples: &'a [(String, String)],
    /// Further known inputs without an output (e.g. existing tests).
    pub known_inputs: &'a [String],
}

pub fn inputs_messages(task_text: &str, ctx: &InputsContext) -> Vec<ChatMessage> {
    let mut user = task_block(task_text);
    if !ctx.input_format.trim().is_empty() {
        user.push_str(&format!("\n\nInput format: {}", ctx.input_format.trim()));
    }
    if !ctx.output_format.trim().is_empty() {
        user.push_str(&format!("\nOutput format: {}", ctx.output_format.trim()));
    }
    if !ctx.constraints.trim().is_empty() {
        user.push_str(&format!(
            "\nInput limits (every input must respect them): {}",
            ctx.constraints.trim()
        ));
    }
    let examples = &ctx.examples[..ctx.examples.len().min(MAX_EXAMPLES)];
    if !examples.is_empty() {
        user.push_str(
            "\n\nExamples from the sheet (input and expected output; do not repeat them):",
        );
        for (i, (input, output)) in examples.iter().enumerate() {
            user.push_str(&format!(
                "\n\nExample {}\nInput:\n```\n{}\n```\nExpected output:\n```\n{}\n```",
                i + 1,
                input.trim_end(),
                output.trim_end()
            ));
        }
    }
    if !ctx.known_inputs.is_empty() {
        user.push_str("\n\nKnown inputs (do not repeat them):");
        for e in ctx.known_inputs {
            user.push_str(&format!("\n```\n{}\n```", e.trim_end()));
        }
    }
    vec![ChatMessage::system(INPUTS_SYSTEM), ChatMessage::user(user)]
}

// --- Code extraction --------------------------------------------------------

struct Block<'a> {
    info: &'a str,
    body: String,
}

/// Fenced blocks (```info … ```); an unclosed last block (answer cut off)
/// runs to the end.
fn fenced_blocks(text: &str) -> Vec<Block<'_>> {
    let mut blocks = Vec::new();
    let mut current: Option<(&str, Vec<&str>)> = None;
    for line in text.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("```") {
            match current.take() {
                Some((info, lines)) => blocks.push(Block {
                    info,
                    body: lines.join("\n"),
                }),
                None => current = Some((rest.trim(), Vec::new())),
            }
        } else if let Some((_, lines)) = current.as_mut() {
            lines.push(line);
        }
    }
    if let Some((info, lines)) = current {
        blocks.push(Block {
            info,
            body: lines.join("\n"),
        });
    }
    blocks
}

fn looks_like_code(text: &str, language: &str) -> bool {
    match language {
        "java" => text.contains("class ") && text.contains("main"),
        _ => ["print(", "input(", "def ", "import ", "sys.stdin"]
            .iter()
            .any(|k| text.contains(k)),
    }
}

/// Extracts the program from a model answer: a fenced block tagged with the
/// language (else any fenced block that looks like code, the longest wins);
/// without fences the whole answer if it looks like code.
pub fn extract_code(answer: &str, language: &str) -> Option<String> {
    let tags: &[&str] = match language {
        "java" => &["java"],
        _ => &["python", "py", "python3"],
    };
    let blocks = fenced_blocks(answer);
    let pick = |pred: &dyn Fn(&Block) -> bool| {
        blocks
            .iter()
            .filter(|b| pred(b) && !b.body.trim().is_empty())
            .max_by_key(|b| b.body.len())
            .map(|b| b.body.clone())
    };
    let code = pick(&|b| tags.contains(&b.info.to_ascii_lowercase().as_str()))
        .or_else(|| pick(&|b| looks_like_code(&b.body, language)))
        .or_else(|| {
            (blocks.is_empty() && looks_like_code(answer, language)).then(|| answer.to_string())
        })?;
    let code = code.trim_matches('\n').to_string();
    (!code.trim().is_empty()).then(|| format!("{code}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_tagged_block() {
        let a = "Hier die Lösung:\n```python\nn = int(input())\nprint(n * 2)\n```\nFertig.";
        assert_eq!(
            extract_code(a, "python").unwrap(),
            "n = int(input())\nprint(n * 2)\n"
        );
    }

    #[test]
    fn prefers_language_block_over_others() {
        let a = "```text\nInput: 5\n```\n```java\npublic class Main { public static void main(String[] a) {} }\n```";
        assert!(extract_code(a, "java")
            .unwrap()
            .starts_with("public class Main"));
        // Python answer with an untagged block and a shell snippet.
        let a = "```\npython main.py\n```\n```\nimport sys\nprint(sum(map(int, sys.stdin.read().split())))\n```";
        assert!(extract_code(a, "python").unwrap().starts_with("import sys"));
    }

    #[test]
    fn handles_unclosed_fence_and_no_fence() {
        let a = "```python\nprint(input())";
        assert_eq!(extract_code(a, "python").unwrap(), "print(input())\n");
        assert_eq!(
            extract_code("print(int(input()) + 1)", "python").unwrap(),
            "print(int(input()) + 1)\n"
        );
        assert_eq!(extract_code("Ich weiß es nicht.", "python"), None);
        assert_eq!(extract_code("```python\n\n```", "python"), None);
        assert_eq!(extract_code("```\nhello\n```", "java"), None);
    }

    #[test]
    fn keeps_indentation() {
        let a = "```py\ndef f(x):\n    return x\n\nprint(f(1))\n```";
        assert_eq!(
            extract_code(a, "python").unwrap(),
            "def f(x):\n    return x\n\nprint(f(1))\n"
        );
    }

    #[test]
    fn prompts_contain_task_and_rules() {
        let ctx = SolutionContext {
            input_format: "Eine Zahl n",
            output_format: "2n",
            examples: &[("3\n".into(), "6\n".into())],
            output_templates: &[],
            class_name: None,
        };
        let m = solution_messages("Verdopple n.", "java", &ctx);
        assert!(m[0].content.contains("public class Main"));
        assert!(m[0].content.contains("```java"));
        assert!(m[0]
            .content
            .contains("Never print a prompt before reading input"));
        assert!(!m[0].content.contains("Never pass a prompt string"));
        assert!(m[1].content.contains("Verdopple n."));
        assert!(m[1].content.contains("Example 1\nInput:\n```\n3\n```"));
        let m = solution_messages("x", "python", &ctx);
        assert!(m[0].content.contains("Never pass a prompt string"));
        let r = repair_messages(m, "```python\nprint(\n```", "SyntaxError: x", "python");
        assert_eq!(r.len(), 4);
        assert!(r[3].content.contains("SyntaxError: x"));
        let i = inputs_messages(
            "x",
            &InputsContext {
                input_format: "n",
                known_inputs: &["3\n".into()],
                ..InputsContext::default()
            },
        );
        assert!(i[1]
            .content
            .contains("Known inputs (do not repeat them):\n```\n3\n```"));
        assert!(!i[1].content.contains("Output format"));
        assert!(!i[1].content.contains("Input limits"));
        assert_eq!(structure_schema()["properties"]["examples"]["maxItems"], 5);
        assert_eq!(
            structure_schema()["properties"]["outputTemplates"]["maxItems"],
            MAX_TEMPLATES
        );
        assert!(STRUCTURE_SYSTEM.contains("Never put an output template"));
        let templates = [OutputTemplate {
            text: "12345678 seconds have passed.".into(),
            placeholders: vec!["12345678".into()],
        }];
        let ctx = SolutionContext {
            input_format: "",
            output_format: "",
            examples: &[],
            output_templates: &templates,
            class_name: None,
        };
        let m = solution_messages("t", "python", &ctx);
        assert!(m[1].content.contains("Placeholders: `12345678`"));
        assert!(m[1]
            .content
            .contains("```\n12345678 seconds have passed.\n```"));
        assert!(m[1].content.contains(
            "Placeholder `12345678` stands for an integer: print it as a whole number without \
             decimal places."
        ));
        assert!(STRUCTURE_SYSTEM.contains("never split it into one template per line"));
        assert!(!m[1].content.contains("several output format templates"));
        assert_eq!(
            structure_schema()["properties"]["inputFormat"]["maxLength"],
            MAX_FORMAT_CHARS
        );
        assert!(
            structure_schema()["properties"]["examples"]["items"]["properties"]["output"]
                .get("maxLength")
                .is_none()
        );
        let two = [
            templates[0].clone(),
            OutputTemplate {
                text: "Fertig nach 12345678 s.".into(),
                placeholders: vec!["12345678".into()],
            },
        ];
        let ctx = SolutionContext {
            output_templates: &two,
            ..ctx
        };
        let m = solution_messages("t", "python", &ctx);
        assert!(m[1]
            .content
            .contains("one or more of these blocks, in this order"));
        assert!(!m[1].content.contains("Print exactly this format"));
        assert_eq!(inputs_schema()["properties"]["inputs"]["maxItems"], 10);
    }

    #[test]
    fn solution_prompt_uses_class_from_task() {
        let ctx = SolutionContext {
            input_format: "",
            output_format: "",
            examples: &[],
            output_templates: &[],
            class_name: Some("Time"),
        };
        let m = solution_messages("Klasse Time", "java", &ctx);
        assert!(m[0].content.contains("`public class Time`"));
        assert!(m[0].content.contains("Time.java"));
        assert!(!m[0].content.contains("class Main"));
        assert!(m[0]
            .content
            .contains("Never print a prompt before reading input"));
        // Python ignores it.
        let m = solution_messages("x", "python", &ctx);
        assert!(!m[0].content.contains("Time"));
    }

    #[test]
    fn class_name_from_task_sheet() {
        let time = "Aufgabe 2: Uhrzeit\nÖffnen Sie die bereitgestellte Java-Klasse `Time`. \
                    Ergänzen Sie die Methode tick(), die die Uhrzeit um eine Sekunde weiterstellt.";
        assert_eq!(class_name_from_task(time).as_deref(), Some("Time"));
        for (text, want) in [
            (
                "Schreiben Sie eine Klasse Time, die eine Uhrzeit speichert.",
                Some("Time"),
            ),
            ("Die Klasse „Bruch“ soll Brüche kürzen.", Some("Bruch")),
            ("Ergänzen Sie die Datei Rechner.java.", Some("Rechner")),
            ("public class Konto {\n  // TODO\n}", Some("Konto")),
            ("Die Hauptklasse heißt Main.", None),
            ("Die Hauptklasse Main liest eine Zahl.", Some("Main")),
            // File name wins over a mentioned helper class.
            ("Klasse Helper hilft. Abgabe: Uhr.java", Some("Uhr")),
            // JDK classes and German nouns are no class names.
            ("Verwenden Sie die Klasse Scanner zum Einlesen.", None),
            ("Lesen Sie mit der Klasse `String` ein.", None),
            (
                "Ergänzen Sie in der Klasse Methoden für die Addition.",
                None,
            ),
            ("Schreiben Sie eine Klasse, die zwei Zahlen addiert.", None),
            ("Die Klassen Time und Date.", None),
            ("Eine klasse x mit kleinem Namen.", None),
            ("Klasse Ühr", None),
            // Contradicting names: no guess.
            ("Klasse Time und Klasse Datum", None),
            ("Lies zwei Zahlen ein und gib die Summe aus.", None),
            ("", None),
        ] {
            assert_eq!(class_name_from_task(text).as_deref(), want, "{text}");
        }
    }

    #[test]
    fn java_code_without_main_class_is_found() {
        let a = "```\npublic class Time { public static void main(String[] a) {} }\n```";
        assert!(extract_code(a, "java")
            .unwrap()
            .starts_with("public class Time"));
    }

    #[test]
    fn inputs_prompt_contains_format_examples_constraints() {
        let examples: Vec<(String, String)> = (1..=7)
            .map(|i| (format!("{i}\n"), format!("{}\n", i * 2)))
            .collect();
        let m = inputs_messages(
            "Verdopple n.",
            &InputsContext {
                input_format: "Eine Zahl n",
                output_format: "Das Doppelte von n",
                constraints: "1 <= n <= 1000",
                examples: &examples,
                known_inputs: &[],
            },
        );
        assert_eq!(m[0].content, INPUTS_SYSTEM);
        let u = &m[1].content;
        assert!(u.contains("Verdopple n."));
        assert!(u.contains("Input format: Eine Zahl n"));
        assert!(u.contains("Output format: Das Doppelte von n"));
        assert!(u.contains("Input limits (every input must respect them): 1 <= n <= 1000"));
        assert!(u.contains("Example 1\nInput:\n```\n1\n```\nExpected output:\n```\n2\n```"));
        // At most MAX_EXAMPLES pairs.
        assert!(u.contains(&format!("Example {MAX_EXAMPLES}\n")));
        assert!(!u.contains(&format!("Example {}\n", MAX_EXAMPLES + 1)));
        assert!(!u.contains("Known inputs"));
        assert!(INPUTS_SYSTEM.contains("respect the input limits"));
        assert!(INPUTS_SYSTEM.contains("Never write inputs the task does not require"));
        assert!(STRUCTURE_SYSTEM.contains("inputConstraints"));
        let schema = structure_schema();
        assert_eq!(
            schema["properties"]["inputConstraints"]["maxLength"],
            MAX_FORMAT_CHARS
        );
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "inputConstraints"));
    }
}
