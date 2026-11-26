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
/// else `Main`. `no_input`: the task reads no input (fixed values).
fn language_rules(language: &str, class_name: Option<&str>, no_input: bool) -> String {
    let base = match language {
        "java" => {
            let class = class_name.unwrap_or(crate::runner::java::DEFAULT_CLASS);
            let input = if no_input {
                "The program reads no input: never use `java.util.Scanner`, \
                 `java.io.BufferedReader` or System.in."
            } else {
                "Read the input with `java.util.Scanner` or `java.io.BufferedReader` from \
                 System.in. Never print a prompt before reading input (no \
                 `System.out.print(\"Zahl eingeben: \")`) – prompts would end up in the output."
            };
            format!(
                "Write Java 21. Put everything into `public class {class}` with \
                 `public static void main(String[] args)` (the file is {class}.java), no \
                 package declaration, no other class with a main method, only the standard \
                 library. {input}"
            )
        }
        _ => if no_input {
            "Write Python 3.12 using only the standard library. The program reads no input: \
             never call `input()` and never read `sys.stdin`."
        } else {
            "Write Python 3.12 using only the standard library. Read the input from \
             standard input (`input()` or `sys.stdin`). Never pass a prompt string to \
             `input()` – prompts would end up in the output."
        }
        .to_string(),
    };
    format!("{base} {FIXED_VALUES_RULE}")
}

/// Rule for tasks with fixed values (regression „Aufgabe 2.2 Time“: the
/// solutions read the given time from standard input).
pub const FIXED_VALUES_RULE: &str = "If the task gives fixed values (e.g. \"the time is \
15:17:51\", \"assign the values … to the variables …\") and does not ask for input, write \
these values as constants or variable assignments in the code and read nothing from \
standard input.";

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
    /// The task reads no input: the solutions must use the given values.
    pub no_input: bool,
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
         block containing the whole program.{NO_COMMENTS_RULE}{}",
        language_rules(language, ctx.class_name, ctx.no_input),
        fence(language),
        // Only without input: with input it made the solutions worse
        // (KI-Benchmark 1.1.1, „parken“).
        if ctx.no_input { STEPS_RULE } else { "" }
    );
    let mut user = task_block(task_text);
    if ctx.no_input {
        user.push_str(&format!("\n\n{NO_INPUT_NOTE}"));
        let fixed = fixed_values_block(task_text);
        if !fixed.is_empty() {
            user.push_str(&format!("\n\n{fixed}"));
        }
    } else if !ctx.input_format.trim().is_empty() {
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

/// Part of every solution system prompt: code only, no comments and no
/// explanations (they made the answers long, up to the token limit; issue
/// #46). Named variables stay welcome: they carry the meaning instead.
pub const NO_COMMENTS_RULE: &str = " Write no comments and no explanations, only code. Keep the \
program short; give intermediate values descriptive variable names instead of commenting them.";

/// Part of the solution system prompt for a task without input: every
/// printed value in its own variable named after its subtask, computed from
/// named variables holding the values of the task. Against solutions that
/// print a variable of another subtask (e.g. the duration instead of the
/// end time). Until 1.1.1 it also asked for one comment per output line;
/// [`NO_COMMENTS_RULE`] replaces that, the variable names carry it now.
pub const STEPS_RULE: &str = " Store every value from the task in its own named variable, \
copied exactly as written in the task. Compute every printed value in its own variable whose \
name says which required output line or subtask it answers; do not reuse a variable of another \
subtask unless the task says so.";

/// Heading of [`fixed_values_block`].
pub const FIXED_VALUES_HEADING: &str = "Fixed values stated in the task text (extracted \
automatically, in order of appearance; use exactly these numbers):";

/// Punctuation trimmed from the end of a word.
const TRAILING: &str = ",.;:!?)";

fn trim_trailing(w: &str) -> &str {
    w.trim_end_matches(|c: char| TRAILING.contains(c))
}

/// Time unit of a German word: Uhr, Stunde(n), Minute(n), Sekunde(n).
fn time_unit(w: &str) -> Option<&'static str> {
    match trim_trailing(w).to_lowercase().as_str() {
        "uhr" => Some("o'clock"),
        "stunde" | "stunden" | "std" => Some("hours"),
        "minute" | "minuten" | "min" => Some("minutes"),
        "sekunde" | "sekunden" | "sek" => Some("seconds"),
        _ => None,
    }
}

/// Other units a stated value may carry (lower case).
const OTHER_UNITS: &[&str] = &[
    "km", "m", "cm", "kg", "g", "liter", "euro", "€", "%", "prozent", "grad", "km/h",
];

/// A plain number („15“, „10,5“, „(3“): decimal comma becomes a point.
fn plain_number(w: &str) -> Option<String> {
    let w = trim_trailing(w.trim_start_matches('('));
    let ok = w.chars().next().is_some_and(|c| c.is_ascii_digit())
        && w.chars()
            .all(|c| c.is_ascii_digit() || c == ',' || c == '.')
        && w.chars().filter(|c| *c == ',' || *c == '.').count() <= 1
        && !w.ends_with([',', '.']);
    ok.then(|| w.replace(',', "."))
}

/// Lines of the task without the blocks that follow a line ending with
/// „:“ (output formats, examples, templates: their numbers are placeholders
/// or example values, not given values). Such a block ends at an empty
/// line, an enumerated line („c)“, „2.“) or a line longer than 80 characters
/// (running text).
fn lines_outside_format_blocks(text: &str) -> Vec<&str> {
    let mut kept = Vec::new();
    let mut in_block = false;
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            in_block = false;
            continue;
        }
        // „c) …“, „2. …“: up to two letters or digits, „)“ or „.“, a space.
        let head: Vec<char> = t.chars().take(4).collect();
        let enumerated = (1..=2).any(|k| {
            head.len() > k + 1
                && head[..k].iter().all(|c| c.is_ascii_alphanumeric())
                && matches!(head[k], ')' | '.')
                && head[k + 1].is_whitespace()
        });
        if in_block && (t.chars().count() > 80 || enumerated) {
            in_block = false;
        }
        if !in_block {
            kept.push(t);
        }
        if t.ends_with(':') {
            in_block = true;
        }
    }
    kept
}

/// Values the task text states, found by rules (no model): runs of number
/// plus time unit („15 Uhr, 17 Minuten und 51 Sekunden“, also a single
/// „437 Minuten“), joined by commas or „und“, and other numbers with a unit
/// (km, kg, Euro, %, …). Format and example blocks are skipped (see
/// [`lines_outside_format_blocks`]). Empty if nothing was found.
///
/// The 7B model read „15 Uhr, 17 Minuten und 51 Sekunden“ as 17:51:00 in
/// every solution alike, which no majority vote can correct (KI-Benchmark
/// 1.1.1); given as numbers, the values are copied correctly.
pub fn fixed_values_block(text: &str) -> String {
    let kept = lines_outside_format_blocks(text).join("\n");
    let words: Vec<&str> = kept.split_whitespace().collect();
    let mut lines: Vec<String> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        // A run of number + time unit, each unit at most once.
        let mut j = i;
        let mut parts: Vec<(String, &str)> = Vec::new();
        while let (Some(n), Some(u)) = (
            words.get(j).and_then(|w| plain_number(w)),
            words.get(j + 1).and_then(|w| time_unit(w)),
        ) {
            if parts.iter().any(|p| p.1 == u) {
                break;
            }
            parts.push((n, u));
            j += 2;
            if words.get(j) == Some(&"und") {
                j += 1;
            }
        }
        if !parts.is_empty() {
            let quote = trim_trailing(&words[i..j].join(" ")).to_string();
            let quote = quote.strip_suffix(" und").unwrap_or(&quote).to_string();
            let desc = parts
                .iter()
                .map(|(n, u)| match *u {
                    "o'clock" => format!("hours = {n}"),
                    u => format!("{u} = {n}"),
                })
                .collect::<Vec<_>>()
                .join(", ");
            let clock = match parts.as_slice() {
                [(h, "o'clock"), (m, "minutes"), (s, "seconds")] => {
                    format!(" (clock time {h:0>2}:{m:0>2}:{s:0>2})")
                }
                _ => String::new(),
            };
            lines.push(format!("- \"{quote}\": {desc}{clock}"));
            i = j;
            continue;
        }
        if let (Some(n), Some(unit)) = (plain_number(words[i]), words.get(i + 1)) {
            let unit = trim_trailing(unit);
            if OTHER_UNITS.contains(&unit.to_lowercase().as_str()) {
                lines.push(format!(
                    "- \"{} {unit}\": {n} {}",
                    trim_trailing(words[i]),
                    unit.to_lowercase()
                ));
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    if lines.is_empty() {
        return String::new();
    }
    format!("{FIXED_VALUES_HEADING}\n{}", lines.join("\n"))
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

/// Repair hint for a solution answer cut off at the token limit (German
/// meaning: „Die Antwort war zu lang; schreibe das Programm kürzer“).
pub const TRUNCATED_REPAIR: &str = "The answer was too long and was cut off before the program \
was complete. Write the program shorter: no comments, no explanations, no unused code.";

/// Follow-up after a solution answer was cut off at the token limit: the
/// cut-off answer plus [`TRUNCATED_REPAIR`] (it was not compiled).
pub fn truncated_repair_messages(
    mut conversation: Vec<ChatMessage>,
    previous_answer: &str,
    language: &str,
) -> Vec<ChatMessage> {
    conversation.push(ChatMessage::assistant(previous_answer));
    conversation.push(ChatMessage::user(format!(
        "{TRUNCATED_REPAIR} Answer with the complete program in exactly one ```{} code block.",
        fence(language)
    )));
    conversation
}

/// Part of the solution prompt for a task without input.
pub const NO_INPUT_NOTE: &str = "This program reads no input. Use the values given in the task \
as constants in the code; do not read anything from standard input.";

/// Repair hint for a solution that read input although the task reads
/// none (German meaning: „Die Aufgabe liest keine Eingabe; verwende die
/// vorgegebenen Werte“).
pub const NO_INPUT_REPAIR: &str = "The program reads input, but the task reads no input: it \
crashed, waited for input or printed something different when run without input. Use the \
values given in the task as constants in the code and remove every read from standard input.";

/// Follow-up after a solution read input although the task reads none.
pub fn no_input_repair_messages(
    mut conversation: Vec<ChatMessage>,
    previous_answer: &str,
    language: &str,
) -> Vec<ChatMessage> {
    conversation.push(ChatMessage::assistant(previous_answer));
    conversation.push(ChatMessage::user(format!(
        "{NO_INPUT_REPAIR} Answer with the complete corrected program in exactly one ```{} \
         code block.",
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
/// Task sheet „Aufgabe 2.2 – Time“, verbatim (KI-Benchmark 1.1.1).
pub(crate) const TIME_SHEET: &str = "Aufgabe 2.2 - Time:
a) Öffnen Sie die bereitgestellte Java-Klasse Time.
b) Legen Sie Variablen mit den Namen hour, minute und second an. Weisen Sie den Variablen die Werte für 15 Uhr, 10 Minuten und 30 Sekunden zu. Das ist eine fiktive Startzeit.
c) Lassen Sie das Programm nun berechnen, wie viele Sekunden seit Mitternacht vergangen sind und geben Sie die Antwort in dem folgenden Format aus:
12345678 seconds have passed since midnight.
d) Berechnen und geben Sie in der nächsten Zeile aus, wie viele Sekunden der Tag noch übrig hat. Das Ausgabeformat ist:
12345678 seconds are left until midnight.
e) Erstellen Sie nun drei Variablen, welche die fiktive Zeit 15 Uhr, 17 Minuten und 51 Sekunden repräsentieren. Lassen Sie das Programm berechnen, wie viel Zeit von der fiktiven Startzeit bis zu der fiktiven aktuellen Zeit vergangen ist. Geben Sie die Antwort in der nächsten Zeile nach dem folgenden Format aus.
Since I started with the task 12345678 seconds have passed.
Beachten Sie, dass die Änderungen in den Teilaufgaben a) bis e) die vorherigen Ausgabeergebnisse und Ausgaben nicht verändern dürfen. Am Ende von Aufgabe 2.2 muss Ihr Programm drei Ausgaben haben. Die Ausgabe sollte folgendermaßen aussehen:
12345678 seconds have passed since midnight.
12345678 seconds are left until midnight.
Since I started with the task 12345678 seconds have passed.";

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
            no_input: false,
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
            no_input: false,
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
    fn no_input_prompt_forbids_reading() {
        let ctx = SolutionContext {
            input_format: "hour minute second",
            output_format: "",
            examples: &[],
            output_templates: &[],
            class_name: Some("Time"),
            no_input: true,
        };
        let m = solution_messages("Aufgabe 2.2 Time", "java", &ctx);
        assert!(m[0].content.contains("never use `java.util.Scanner`"));
        assert!(!m[0].content.contains("Read the input with"));
        assert!(m[0].content.contains(FIXED_VALUES_RULE));
        assert!(m[1].content.contains(NO_INPUT_NOTE));
        // A wrong input format of the model is not passed on.
        assert!(!m[1].content.contains("hour minute second"));
        let m = solution_messages("t", "python", &ctx);
        assert!(m[0].content.contains("never call `input()`"));
        let r = no_input_repair_messages(m, "```python\nx = input()\n```", "python");
        assert_eq!(r.len(), 4);
        assert!(r[3].content.contains(NO_INPUT_REPAIR));
        assert!(r[3].content.contains("```python"));
        // The general rule is part of every solution prompt.
        let ctx = SolutionContext {
            no_input: false,
            ..ctx
        };
        let m = solution_messages("t", "python", &ctx);
        assert!(m[0].content.contains(FIXED_VALUES_RULE));
        assert!(m[0]
            .content
            .contains("Read the input from standard input (`input()`"));
    }

    #[test]
    fn fixed_values_of_the_time_sheet() {
        let b = fixed_values_block(TIME_SHEET);
        assert_eq!(
            b,
            format!(
                "{FIXED_VALUES_HEADING}\n\
                 - \"15 Uhr, 10 Minuten und 30 Sekunden\": hours = 15, minutes = 10, seconds = 30 \
                 (clock time 15:10:30)\n\
                 - \"15 Uhr, 17 Minuten und 51 Sekunden\": hours = 15, minutes = 17, seconds = 51 \
                 (clock time 15:17:51)"
            )
        );
        // The literal sentence on its own: 15:17:51, never 17:51:00.
        let b = fixed_values_block(
            "die fiktive Zeit 15 Uhr, 17 Minuten und 51 Sekunden repräsentieren.",
        );
        assert!(b.contains("hours = 15, minutes = 17, seconds = 51 (clock time 15:17:51)"));
        assert!(!b.contains("17:51:00"));
        // Placeholders of the format lines are not taken.
        assert!(!fixed_values_block(TIME_SHEET).contains("12345678"));
    }

    #[test]
    fn fixed_values_of_other_sheets() {
        let film = "Aufgabe 3 - Film:\nEin Film beginnt um 20 Uhr, 45 Minuten und 10 Sekunden. Er \
                    dauert 2 Stunden, 38 Minuten und 55 Sekunden.\na) Legen Sie Variablen an.\n\
                    b) Berechnen Sie die Dauer des Films in Sekunden und geben Sie sie aus:\n\
                    Der Film dauert 1234 Sekunden.\nc) Geben Sie die Endzeit aus:\n\
                    Der Film endet um hh:mm:ss Uhr.";
        let b = fixed_values_block(film);
        assert!(b.contains(
            "- \"20 Uhr, 45 Minuten und 10 Sekunden\": hours = 20, minutes = 45, seconds = 10 \
             (clock time 20:45:10)"
        ));
        assert!(b.contains(
            "- \"2 Stunden, 38 Minuten und 55 Sekunden\": hours = 2, minutes = 38, seconds = 55"
        ));
        assert!(!b.contains("1234"), "{b}");
        let lauf = "Aufgabe 5 - Lauftraining\nEine Läuferin läuft 10,5 km in 52 Minuten und 30 \
                    Sekunden.\nAusgabeformat:\nGeschwindigkeit: 0.00 km/h\nPace: 0:00 min/km";
        assert_eq!(
            fixed_values_block(lauf),
            format!(
                "{FIXED_VALUES_HEADING}\n- \"10,5 km\": 10.5 km\n\
                 - \"52 Minuten und 30 Sekunden\": minutes = 52, seconds = 30"
            )
        );
        let minuten = "Eine Zugfahrt dauert 437 Minuten. Speichern Sie diesen Wert.\n\
                       Ausgabe (X, Y und Z sind Platzhalter):\nDie Fahrt dauert X Stunden und Y Minuten.";
        assert_eq!(
            fixed_values_block(minuten),
            format!("{FIXED_VALUES_HEADING}\n- \"437 Minuten\": minutes = 437")
        );
        let parken = "Ein Parkhaus berechnet: die erste Stunde kostet 2 Euro, jede weitere \
                      angefangene Stunde 1,50 Euro, höchstens aber 15 Euro pro Tag.\n\
                      Beispiel\nEingabe:\n130\nAusgabe:\nPreis: 5.00 Euro";
        let b = fixed_values_block(parken);
        assert!(
            b.contains("- \"2 Euro\": 2 euro\n- \"1,50 Euro\": 1.50 euro\n- \"15 Euro\": 15 euro")
        );
        assert!(!b.contains("5.00"), "{b}");
    }

    #[test]
    fn fixed_values_need_number_and_unit() {
        for text in [
            "",
            "Lies drei ganze Zahlen ein und gib die größte aus.",
            "Aufgabe 2.2 - Schleifen: Geben Sie die Zahlen von 1 bis 40 aus.",
            "Die erste Stunde kostet extra, jede Minute zählt.",
            "Version 1.2.3 km",
            "Es sind 12345678 seconds.",
            "Seite 5 von 10, Abschnitt 3 Uhrzeit",
            // Format and example lines after a line ending with „:“.
            "Ausgabe:\nDer Film dauert 1234 Sekunden.\nGeschwindigkeit: 0.00 km/h\n5.00 Euro",
            "Beispiel:\n3 Stunden",
        ] {
            assert_eq!(fixed_values_block(text), "", "{text}");
        }
        // The block ends at an empty line or an enumerated line.
        assert!(fixed_values_block("Format:\n1 km\n\nEs sind 5 km.").contains("\"5 km\""));
        assert!(!fixed_values_block("Format:\n1 km\n\nEs sind 5 km.").contains("\"1 km\""));
        assert!(fixed_values_block("Format:\nx\nb) Er läuft 3 km.").contains("\"3 km\""));
    }

    #[test]
    fn steps_rule_and_fixed_values_only_without_input() {
        let ctx = SolutionContext {
            input_format: "",
            output_format: "",
            examples: &[],
            output_templates: &[],
            class_name: Some("Time"),
            no_input: true,
        };
        let m = solution_messages(TIME_SHEET, "java", &ctx);
        assert!(m[0].content.ends_with(STEPS_RULE));
        let u = &m[1].content;
        let note = u.find(NO_INPUT_NOTE).unwrap();
        let fixed = u.find(FIXED_VALUES_HEADING).unwrap();
        assert!(note < fixed);
        assert!(u.contains("(clock time 15:17:51)"));
        // With input: unchanged (it made the solutions worse there).
        let ctx = SolutionContext {
            no_input: false,
            ..ctx
        };
        let m = solution_messages(TIME_SHEET, "java", &ctx);
        assert!(!m[0].content.contains(STEPS_RULE));
        assert!(!m[1].content.contains(FIXED_VALUES_HEADING));
        // Without input but nothing to extract: no empty heading.
        let ctx = SolutionContext {
            no_input: true,
            ..ctx
        };
        let m = solution_messages("Gib die Zahlen von 1 bis 40 aus.", "python", &ctx);
        assert!(!m[1].content.contains(FIXED_VALUES_HEADING));
        assert!(m[0].content.contains(STEPS_RULE));
    }

    #[test]
    fn solutions_get_no_comments_rule_instead_of_comment_duty() {
        for no_input in [false, true] {
            for lang in ["python", "java"] {
                let ctx = SolutionContext {
                    input_format: "eine Zahl",
                    output_format: "",
                    examples: &[],
                    output_templates: &[],
                    class_name: None,
                    no_input,
                };
                let system = &solution_messages(TIME_SHEET, lang, &ctx)[0].content;
                assert!(system.contains(NO_COMMENTS_RULE), "{system}");
                // No prompt asks for comments any more.
                for duty in ["write one short comment", "comment per"] {
                    assert!(!system.contains(duty), "{system}");
                }
                // Named values stay asked for without input.
                assert_eq!(system.contains("own named variable"), no_input);
            }
        }
        assert!(!STEPS_RULE.contains("comment"));
        assert!(STEPS_RULE.contains("copied exactly as written in the task"));
        assert!(NO_COMMENTS_RULE.contains("variable names"));
    }

    #[test]
    fn truncated_repair_asks_for_a_shorter_complete_program() {
        let base = vec![ChatMessage::system("s"), ChatMessage::user("u")];
        let r = truncated_repair_messages(
            base,
            "```java
public class Main {",
            "java",
        );
        assert_eq!(r.len(), 4);
        assert_eq!(r[2].content, "```java\npublic class Main {");
        assert!(r[3].content.starts_with(TRUNCATED_REPAIR));
        assert!(r[3].content.contains("too long"));
        assert!(r[3].content.contains("shorter"));
        assert!(r[3].content.ends_with("exactly one ```java code block."));
    }

    #[test]
    fn solution_prompt_uses_class_from_task() {
        let ctx = SolutionContext {
            input_format: "",
            output_format: "",
            examples: &[],
            output_templates: &[],
            class_name: Some("Time"),
            no_input: false,
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
