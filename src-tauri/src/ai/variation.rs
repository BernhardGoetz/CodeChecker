//! Variation check: is the student's program only right by chance for the
//! values of the task? Example: "seconds until midnight" computed as
//! `86400 % s` instead of `86400 - s` gives the right result for 15:10:30
//! (54630 s > 43200 s), but not for 03:10:30.
//!
//! The decision is made by running programs, never by the model: the
//! reference solution (from the test generation) and the student's program
//! run on changed values, their outputs are compared with the comparison
//! mode (plus the output template check).
//!
//! - Case A, the task reads input: the model writes additional valid inputs
//!   (as in the test generation); both programs run them. Inputs on which
//!   the reference solution fails are dropped.
//! - Case B, no input / start values as constants (`hour = 15`): the model
//!   names the given values and proposes scenarios (`15 → 3`). The
//!   replacement is deterministic and only touches simple assignments
//!   `[type] ident = <number>` outside comments and strings, in the
//!   student's code and in the reference solution alike. Scenarios whose
//!   replacement counts differ, or that do not change the reference output,
//!   are skipped.
//!
//! On a finding a second model call only *explains* the error (German text
//! plus a quote that must occur in the student's code); it decides nothing.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::llama::{ChatMessage, ChatRequest, Llm};
use super::pipeline::{
    as_stdin, cancellable, clean_inputs, fits_templates, parse_json, valid, AiGate, AiProgress,
    CompileOutcome, Executor, InputsAnswer, Phase, SandboxExecutor, Session,
};
use super::prompts;
use super::AiError;
use crate::runner::compare::{line_diff, CompareMode, DiffLine};
use crate::runner::template::OutputTemplate;
use crate::runner::{languages, StepOutput, TestCase, MAX_INPUT_BYTES, MAX_TESTS};

/// The task text in prompts is shortened to about this many characters
/// (beginning and end), the model context is 8192 tokens.
pub const MAX_TASK_PROMPT_CHARS: usize = 4000;
/// Longer student code gets no explanation (the check still runs).
pub const MAX_CODE_EXPLAIN_CHARS: usize = 6000;
/// Additional inputs in case A.
pub const MAX_VARIATION_INPUTS: usize = 8;
/// Scenarios in case B.
pub const MAX_SCENARIOS: usize = 3;
/// Changes per scenario.
pub const MAX_CHANGES: usize = 6;
/// Findings returned (the first ones).
pub const MAX_FINDINGS: usize = 5;
const MAX_QUOTE_CHARS: usize = 200;
const MAX_EXPLANATION_CHARS: usize = 600;
const SCENARIO_TEMPERATURE: f32 = 0.2;
const INPUTS_TEMPERATURE: f32 = 0.4;
const EXPLAIN_TEMPERATURE: f32 = 0.1;
const JSON_MAX_TOKENS: u32 = 768;
/// How long a variation check waits for the AI gate.
const GATE_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariationRequest {
    pub task_text: String,
    pub language: String,
    pub student_code: String,
    #[serde(default)]
    pub reference_solution: Option<String>,
    #[serde(default)]
    pub compare_mode: Option<CompareMode>,
    #[serde(default)]
    pub output_templates: Vec<OutputTemplate>,
    /// The tests of the run (their inputs are used, expectations not).
    #[serde(default)]
    pub tests: Vec<TestCase>,
    /// Input format from the test generation (`GenerateResult.inputFormat`),
    /// helps the model write valid inputs.
    #[serde(default)]
    pub input_format: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VariationState {
    /// The program is also right for changed values.
    Ok,
    /// The program is only right for the given values.
    Failed,
    /// Not checked (reason in `note`).
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VariationFinding {
    /// German, e.g. "hour = 3 statt 15" or "Eingabe: 7".
    pub description: String,
    pub input: String,
    /// Output of the reference solution.
    pub expected: String,
    /// Output of the student's program.
    pub actual: String,
    pub diff: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VariationResult {
    pub state: VariationState,
    pub findings: Vec<VariationFinding>,
    /// German explanation of the model (only explains, decides nothing).
    pub explanation: Option<String>,
    /// Piece of the student's code the explanation refers to (verified).
    pub explanation_quote: Option<String>,
    /// German note: reason for skipping, or what was checked.
    pub note: Option<String>,
    /// Number of variants (inputs or scenarios) compared.
    pub checked: usize,
    /// German hints on variants where the student's program crashed or
    /// timed out: unclear, never counted as a logic error.
    pub unclear: Vec<String>,
}

impl VariationResult {
    pub fn skipped(note: impl Into<String>) -> Self {
        Self {
            state: VariationState::Skipped,
            findings: Vec::new(),
            explanation: None,
            explanation_quote: None,
            note: Some(note.into()),
            checked: 0,
            unclear: Vec::new(),
        }
    }
}

impl VariationRequest {
    /// Why the check cannot run at all (German), before anything starts.
    pub fn skip_reason(&self) -> Option<String> {
        let reference = self.reference_solution.as_deref().unwrap_or("");
        if reference.trim().is_empty() {
            return Some(
                "Keine Musterlösung vorhanden – der Variationstest braucht die Musterlösung aus \
                 „Testfälle mit KI erzeugen“."
                    .into(),
            );
        }
        if self.student_code.trim().is_empty() {
            return Some("Kein Code zum Prüfen.".into());
        }
        if self.task_text.trim().is_empty() {
            return Some("Keine Aufgabenstellung vorhanden.".into());
        }
        if languages::get(&self.language).is_none() {
            return Some(format!("Unbekannte Sprache: {}", self.language));
        }
        if self.student_code.len() > MAX_INPUT_BYTES || reference.len() > MAX_INPUT_BYTES {
            return Some("Der Code ist zu groß für den Variationstest.".into());
        }
        if let Some(Err(e)) = self.compare_mode.map(|m| m.validate()) {
            return Some(e);
        }
        None
    }

    fn reference(&self) -> &str {
        self.reference_solution.as_deref().unwrap_or("")
    }

    /// Distinct test inputs (stdin form), at most [`MAX_TESTS`].
    fn test_inputs(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for t in &self.tests {
            let i = as_stdin(&t.input);
            if i.len() <= MAX_INPUT_BYTES && !out.contains(&i) && out.len() < MAX_TESTS {
                out.push(i);
            }
        }
        out
    }

    fn has_inputs(&self) -> bool {
        self.tests.iter().any(|t| !t.input.trim().is_empty())
    }
}

/// The task text for prompts: at most `max` characters (beginning and end
/// kept, the middle marked as left out).
pub fn shorten_task(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    let half = max / 2;
    let head: String = chars[..half].iter().collect();
    let tail: String = chars[chars.len() - half..].iter().collect();
    format!("{head}\n[…]\n{tail}")
}

// ---------------------------------------------------------------------------
// Literal replacement (pure)

/// One change of a scenario: the literal as in the task, the new value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub from: String,
    pub to: String,
}

/// Result of [`substitute`].
#[derive(Debug, Clone, PartialEq)]
pub struct Substitution {
    pub code: String,
    /// Replacements per change (same order as the changes).
    pub counts: Vec<usize>,
    /// The first variable assigned per change.
    pub idents: Vec<Option<String>>,
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// `true` for bytes that are code (not inside a comment or string literal).
pub fn code_mask(code: &str, language: &str) -> Vec<bool> {
    let b = code.as_bytes();
    let n = b.len();
    let mut mask = vec![true; n];
    let java = language == "java";
    let mut i = 0;
    // Marks b[from..to] as no code.
    let mut blank = |from: usize, to: usize| {
        for m in &mut mask[from..to.min(n)] {
            *m = false;
        }
    };
    let starts = |i: usize, s: &[u8]| b[i..].starts_with(s);
    while i < n {
        let c = b[i];
        // Comments.
        let line_comment = if java { starts(i, b"//") } else { c == b'#' };
        if line_comment {
            let end = b[i..].iter().position(|&x| x == b'\n').map_or(n, |p| i + p);
            blank(i, end);
            i = end;
            continue;
        }
        if java && starts(i, b"/*") {
            let end = b[i + 2..]
                .windows(2)
                .position(|w| w == b"*/")
                .map_or(n, |p| i + 2 + p + 2);
            blank(i, end);
            i = end;
            continue;
        }
        // Strings.
        if c == b'"' || c == b'\'' {
            let triple = [c, c, c];
            let (open, close): (usize, &[u8]) = if starts(i, &triple) && (c == b'"' || !java) {
                (3, &triple[..])
            } else {
                (1, std::slice::from_ref(&b[i]))
            };
            let mut j = i + open;
            let mut end = n;
            while j < n {
                if b[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if open == 1 && b[j] == b'\n' {
                    end = j;
                    break;
                }
                if b[j..].starts_with(close) {
                    end = j + close.len();
                    break;
                }
                j += 1;
            }
            blank(i, end);
            i = end;
            continue;
        }
        i += 1;
    }
    mask
}

fn skip_back_blanks(b: &[u8], mut p: usize) -> usize {
    while p > 0 && (b[p - 1] == b' ' || b[p - 1] == b'\t') {
        p -= 1;
    }
    p
}

fn ident_before(b: &[u8], end: usize) -> Option<usize> {
    let mut s = end;
    while s > 0 && is_ident_byte(b[s - 1]) {
        s -= 1;
    }
    (s < end && !b[s].is_ascii_digit()).then_some(s)
}

/// Whether position `s` lies inside unclosed parentheses of the current
/// block (Java: `for (…)` head, method arguments). Comments and strings
/// are skipped; `{`/`}` end the search.
fn inside_parens(b: &[u8], mask: &[bool], s: usize) -> bool {
    let mut depth = 0usize;
    for k in (0..s).rev() {
        if !mask[k] {
            continue;
        }
        match b[k] {
            b')' => depth += 1,
            b'(' if depth == 0 => return true,
            b'(' => depth -= 1,
            b'{' | b'}' if depth == 0 => return false,
            _ => {}
        }
    }
    false
}

/// Checks the left side of a number at `i`: `ident =` (`ident: type =` in
/// Python) at the start of a statement, or a declaration `[final] type
/// ident =` (Java, also after `,` in a declaration list), also with
/// `this.`/`self.`. Never inside parentheses (Java `for` head). Returns
/// the variable name.
fn assignment_target(code: &str, mask: &[bool], i: usize, java: bool) -> Option<String> {
    let b = code.as_bytes();
    let p = skip_back_blanks(b, i);
    if p == 0 || b[p - 1] != b'=' {
        return None;
    }
    let eq = p - 1;
    if eq > 0 && b"=!<>+-*/%&|^:~".contains(&b[eq - 1]) {
        return None;
    }
    let p = skip_back_blanks(b, eq);
    let mut s = ident_before(b, p)?;
    let mut end = p;
    if !java {
        // `x: int = 15`: the word before `=` is the annotation.
        let q = skip_back_blanks(b, s);
        if q > 0 && b[q - 1] == b':' {
            end = skip_back_blanks(b, q - 1);
            s = ident_before(b, end)?;
        }
    }
    // `this.hour = 15` (Java) / `self.hour = 15` (Python).
    if s > 0 && b[s - 1] == b'.' {
        let owner = ident_before(b, s - 1)?;
        if &code[owner..s - 1] != (if java { "this" } else { "self" }) {
            return None;
        }
        s = owner;
    }
    let ident = &code[s..end];
    if !java {
        let q = skip_back_blanks(b, s);
        let ok = q == 0 || b[q - 1] == b'\n' || b[q - 1] == b'\r' || b[q - 1] == b';';
        return (ok && mask[s]).then(|| ident.to_string());
    }
    if !mask[s] || inside_parens(b, mask, s) {
        return None;
    }
    // Java: look at what precedes the variable (whitespace and comments
    // skipped); a type may precede it.
    let mut q = s;
    let skippable = |k: usize| !mask[k] || b[k].is_ascii_whitespace();
    while q > 0 && skippable(q - 1) {
        q -= 1;
    }
    if q == 0 || b";{},".contains(&b[q - 1]) {
        return Some(ident.to_string());
    }
    if !(is_ident_byte(b[q - 1]) || b"<>[]".contains(&b[q - 1])) {
        return None;
    }
    // `final int`, `long`, `var`, `List<Integer>`: walk back over the type.
    while q > 0 && (skippable(q - 1) || is_ident_byte(b[q - 1]) || b"<>[].?".contains(&b[q - 1])) {
        q -= 1;
    }
    (q == 0 || b";{}".contains(&b[q - 1])).then(|| ident.to_string())
}

/// Whether the statement ends right after the number at `k`.
fn statement_ends(b: &[u8], mask: &[bool], mut k: usize, java: bool) -> bool {
    while k < b.len() && (b[k] == b' ' || b[k] == b'\t') {
        k += 1;
    }
    if java {
        k < b.len() && (b[k] == b';' || b[k] == b',')
    } else {
        k == b.len() || b[k] == b'\n' || b[k] == b'\r' || b[k] == b';' || !mask[k]
    }
}

/// Replaces the number literals of `changes` in simple assignments
/// (`x = 15`, `int x = 15;`, `final int a = 15, b = 10;`, `x: int = 15`).
/// Only literals that equal a `from` exactly; comments and strings stay
/// untouched. Deterministic and pure.
pub fn substitute(code: &str, language: &str, changes: &[Change]) -> Substitution {
    let java = language == "java";
    let mask = code_mask(code, language);
    let b = code.as_bytes();
    let n = b.len();
    let mut counts = vec![0; changes.len()];
    let mut idents: Vec<Option<String>> = vec![None; changes.len()];
    let mut replacements: Vec<(usize, usize, &str)> = Vec::new();
    let mut i = 0;
    while i < n {
        let starts_number = mask[i]
            && b[i].is_ascii_digit()
            && (i == 0 || !(is_ident_byte(b[i - 1]) || b[i - 1] == b'.'));
        if !starts_number {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < n && b[j].is_ascii_digit() {
            j += 1;
        }
        if j + 1 < n && b[j] == b'.' && b[j + 1].is_ascii_digit() {
            j += 1;
            while j < n && b[j].is_ascii_digit() {
                j += 1;
            }
        }
        let mut k = j;
        if java && k < n && b"lLfFdD".contains(&b[k]) {
            k += 1;
        }
        if k < n && (is_ident_byte(b[k]) || b[k] == b'.') {
            i = k.max(i + 1);
            while i < n && (is_ident_byte(b[i]) || b[i] == b'.') {
                i += 1;
            }
            continue;
        }
        // A sign directly before the number belongs to it if the number is
        // the whole right side (`x = -15`); `y - 15` is no assignment anyway.
        let start = if i > 0 && b[i - 1] == b'-' && mask[i - 1] {
            i - 1
        } else {
            i
        };
        let literal = &code[start..j];
        if let Some(c) = changes.iter().position(|c| c.from == literal) {
            if statement_ends(b, &mask, k, java) {
                if let Some(ident) = assignment_target(code, &mask, start, java) {
                    counts[c] += 1;
                    idents[c].get_or_insert(ident);
                    replacements.push((start, j, changes[c].to.as_str()));
                }
            }
        }
        i = k;
    }
    let mut out = String::with_capacity(code.len());
    let mut last = 0;
    for (a, z, to) in replacements {
        out.push_str(&code[last..a]);
        out.push_str(to);
        last = z;
    }
    out.push_str(&code[last..]);
    Substitution {
        code: out,
        counts,
        idents,
    }
}

// ---------------------------------------------------------------------------
// Scenarios (case B)

pub const SCENARIO_SYSTEM: &str = "You help to find out whether a student's program really \
computes its result or only happens to print the right output for the values given in a \
programming exercise. Such exercises often fix start values in the text (for example the time \
15:10:30, which the program stores as hour = 15, minute = 10, second = 30).\n\
- givenValues: every number that the task gives as a start value, exactly as written in the \
text, with its meaning.\n\
- scenarios: 2 or 3 scenarios. A scenario replaces some given values by other values: \
changes is a list of {from: the original number exactly as in the text, to: the new number}. \
The new values must keep the task valid (a valid time of day, an end time after a start time, \
positive amounts, …) and should make a wrong formula give a different result than the right \
one: for example a time before 12:00 if the given time is in the afternoon, values that are \
not multiples of each other, no 0 or 1. Only numbers that appear in the task text may be used \
as from; plain integers or decimals without units (a negative number with a leading minus).\n\
Example: the task says \"It is 15:10:30\" → scenarios [{\"changes\": [{\"from\": \"15\", \"to\": \
\"3\"}], \"why\": \"morning instead of afternoon\"}, {\"changes\": [{\"from\": \"15\", \"to\": \
\"9\"}, {\"from\": \"10\", \"to\": \"45\"}], \"why\": \"other hour and minute\"}].\n\
Answer with JSON only.";

pub fn scenario_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "givenValues": {
                "type": "array",
                "maxItems": 12,
                "items": {
                    "type": "object",
                    "properties": {
                        "value": { "type": "string" },
                        "meaning": { "type": "string" }
                    },
                    "required": ["value", "meaning"]
                }
            },
            "scenarios": {
                "type": "array",
                "maxItems": MAX_SCENARIOS,
                "items": {
                    "type": "object",
                    "properties": {
                        "changes": {
                            "type": "array",
                            "maxItems": MAX_CHANGES,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "from": { "type": "string" },
                                    "to": { "type": "string" }
                                },
                                "required": ["from", "to"]
                            }
                        },
                        "why": { "type": "string" }
                    },
                    "required": ["changes", "why"]
                }
            }
        },
        "required": ["givenValues", "scenarios"]
    })
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RawChange {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RawScenario {
    pub changes: Vec<RawChange>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct ScenarioAnswer {
    scenarios: Vec<RawScenario>,
}

/// Digits with at most one decimal point inside, optionally negative.
fn plain_number(s: &str) -> bool {
    let s = s.strip_prefix('-').unwrap_or(s);
    let (int, frac) = s.split_once('.').unwrap_or((s, "0"));
    !int.is_empty()
        && !frac.is_empty()
        && int.bytes().all(|b| b.is_ascii_digit())
        && frac.bytes().all(|b| b.is_ascii_digit())
}

/// Whether the number `lit` occurs in `text` as a whole number (not as
/// part of a longer number: "15" is in "15:10:30", not in "150" or "1.15").
pub fn number_in_text(text: &str, lit: &str) -> bool {
    let b = text.as_bytes();
    text.match_indices(lit).any(|(i, _)| {
        let end = i + lit.len();
        let before_ok = i == 0 || !(b[i - 1].is_ascii_digit() || b[i - 1] == b'.');
        let after_ok = end == b.len()
            || !(b[end].is_ascii_digit()
                || (b[end] == b'.' && b.get(end + 1).is_some_and(u8::is_ascii_digit)));
        before_ok && after_ok
    })
}

/// Scenarios whose every change is valid: `from` and `to` plain numbers,
/// different, `from` occurring in the task text (as a whole number), no
/// literal changed twice. At most [`MAX_SCENARIOS`], duplicates removed.
pub fn validate_scenarios(task_text: &str, raw: &[RawScenario]) -> Vec<Vec<Change>> {
    let mut out: Vec<Vec<Change>> = Vec::new();
    for s in raw {
        let changes: Vec<Change> = s
            .changes
            .iter()
            .map(|c| Change {
                from: c.from.trim().to_string(),
                to: c.to.trim().to_string(),
            })
            .collect();
        let valid = !changes.is_empty()
            && changes.len() <= MAX_CHANGES
            && changes.iter().enumerate().all(|(k, c)| {
                plain_number(&c.from)
                    && plain_number(&c.to)
                    && c.from != c.to
                    && number_in_text(task_text, &c.from)
                    && !changes[..k].iter().any(|d| d.from == c.from)
            });
        if valid && !out.contains(&changes) && out.len() < MAX_SCENARIOS {
            out.push(changes);
        }
    }
    out
}

fn scenario_messages(task: &str) -> Vec<ChatMessage> {
    vec![
        ChatMessage::system(SCENARIO_SYSTEM),
        ChatMessage::user(format!(
            "Task (verbatim from the exercise sheet):\n<<<\n{task}\n>>>"
        )),
    ]
}

/// German description of a scenario: "hour = 3 statt 15, minute = 45 statt 10".
fn describe(changes: &[Change], idents: &[Option<String>]) -> String {
    changes
        .iter()
        .zip(idents)
        .map(|(c, id)| match id {
            Some(id) => format!("{id} = {} statt {}", c.to, c.from),
            None => format!("{} statt {}", c.to, c.from),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

// ---------------------------------------------------------------------------
// Explanation

pub const EXPLAIN_SYSTEM: &str = "You explain to a teacher why a student's program is wrong. \
The program prints the expected output for the values given in the exercise, but for other \
values it prints a wrong result (see the finding below), so its logic is wrong. Find the \
error in the student's code. Answer with JSON: {\"quote\": a short piece of the student's \
code that contains the error, copied exactly, \"explanation\": one or two short sentences in \
German that explain the error and what would be right}. Answer with JSON only.";

pub fn explain_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "quote": { "type": "string" },
            "explanation": { "type": "string" }
        },
        "required": ["quote", "explanation"]
    })
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ExplainAnswer {
    pub quote: String,
    pub explanation: String,
}

fn fence(language: &str) -> &'static str {
    if language == "java" {
        "java"
    } else {
        "python"
    }
}

fn explain_messages(
    task: &str,
    language: &str,
    code: &str,
    findings: &[VariationFinding],
) -> Vec<ChatMessage> {
    let mut user = format!(
        "Task (verbatim from the exercise sheet):\n<<<\n{task}\n>>>\n\nStudent's program:\n```{}\n{}\n```",
        fence(language),
        code.trim_end()
    );
    for f in findings.iter().take(2) {
        user.push_str(&format!(
            "\n\nFinding: with changed values ({})\nExpected output:\n```\n{}\n```\nStudent's \
             output:\n```\n{}\n```",
            f.description,
            f.expected.trim_end(),
            f.actual.trim_end()
        ));
    }
    vec![ChatMessage::system(EXPLAIN_SYSTEM), ChatMessage::user(user)]
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The explanation is kept only if its quote occurs in the student's code
/// (whitespace-insensitive) and both are non-empty. Returns (explanation,
/// quote).
pub fn validate_explanation(code: &str, a: &ExplainAnswer) -> Option<(String, String)> {
    let quote = a.quote.trim();
    let explanation = a.explanation.trim();
    if quote.is_empty()
        || explanation.is_empty()
        || quote.chars().count() > MAX_QUOTE_CHARS
        || !collapse_ws(code).contains(&collapse_ws(quote))
    {
        return None;
    }
    let explanation: String = explanation.chars().take(MAX_EXPLANATION_CHARS).collect();
    Some((explanation, quote.to_string()))
}

// ---------------------------------------------------------------------------
// Flow

fn json_request(messages: Vec<ChatMessage>, schema: Value, temperature: f32) -> ChatRequest {
    ChatRequest {
        messages,
        temperature,
        max_tokens: JSON_MAX_TOKENS,
        json_schema: Some(schema),
        seed: None,
        prefill: None,
        stop: Vec::new(),
    }
}

async fn ask<L: Llm + Sync, T: serde::de::DeserializeOwned>(
    llm: &L,
    req: ChatRequest,
    cancel: &CancellationToken,
) -> Result<Option<T>, AiError> {
    let r = cancellable(cancel, llm.chat(&req)).await?;
    Ok(parse_json(&r.content))
}

/// What a program printed, with a German note for crashes and timeouts.
fn shown_output(o: &StepOutput) -> String {
    if o.timed_out {
        format!("{}[Zeitüberschreitung]", o.stdout)
    } else if o.exit_code != Some(0) {
        format!("{}[Laufzeitfehler, Exit-Code {:?}]", o.stdout, o.exit_code)
    } else {
        o.stdout.clone()
    }
}

/// Short one-line preview of an input.
fn preview(input: &str) -> String {
    let one = input.trim_end().replace('\n', " ⏎ ");
    let short: String = one.chars().take(60).collect();
    if short.len() < one.len() {
        format!("{short} …")
    } else {
        short
    }
}

struct Ctx<'a, E: Executor> {
    req: &'a VariationRequest,
    mode: CompareMode,
    cancel: &'a CancellationToken,
    student: &'a E::Session,
    reference: &'a E::Session,
}

impl<E: Executor> Ctx<'_, E> {
    async fn compile(&self, s: &E::Session, code: &str) -> Result<CompileOutcome, AiError> {
        cancellable(self.cancel, s.compile(code)).await
    }

    async fn run(&self, s: &E::Session, inputs: &[String]) -> Result<Vec<StepOutput>, AiError> {
        cancellable(self.cancel, s.run(inputs)).await
    }

    /// Runs the reference solution twice. Returns the first outputs and,
    /// per input, whether the run is usable: both runs valid and equal (a
    /// program whose output changes between runs – random numbers, the
    /// clock – cannot serve as reference).
    async fn run_reference(
        &self,
        inputs: &[String],
    ) -> Result<(Vec<StepOutput>, Vec<bool>), AiError> {
        let first = self.run(self.reference, inputs).await?;
        let second = self.run(self.reference, inputs).await?;
        let stable = first
            .iter()
            .zip(&second)
            .map(|(a, b)| valid(a) && valid(b) && self.mode.matches(&a.stdout, &b.stdout))
            .collect();
        Ok((first, stable))
    }

    /// Compares the student's output with the reference output.
    fn verdict(&self, reference: &StepOutput, student: &StepOutput) -> Verdict {
        if student.timed_out {
            Verdict::Unclear("Zeitüberschreitung")
        } else if student.exit_code != Some(0) {
            Verdict::Unclear("Laufzeitfehler")
        } else if student.stdout_truncated {
            Verdict::Unclear("Ausgabe abgeschnitten")
        } else if self.mode.matches(&reference.stdout, &student.stdout)
            && fits_templates(&self.req.output_templates, student)
        {
            Verdict::Same
        } else {
            Verdict::Differs
        }
    }

    fn finding(
        &self,
        description: String,
        input: &str,
        reference: &StepOutput,
        student: &StepOutput,
    ) -> VariationFinding {
        VariationFinding {
            description,
            input: input.to_string(),
            expected: reference.stdout.clone(),
            actual: shown_output(student),
            diff: line_diff(
                &self.mode.display_text(&reference.stdout),
                &self.mode.display_text(&student.stdout),
            ),
        }
    }
}

/// Result of one comparison of student and reference.
enum Verdict {
    Same,
    /// Clean run, different output: a logic error.
    Differs,
    /// The student's program crashed or timed out: unclear (German reason).
    Unclear(&'static str),
}

/// Collects the comparisons of a check.
#[derive(Default)]
struct Tally {
    findings: Vec<VariationFinding>,
    unclear: Vec<String>,
    /// Comparisons with a clean run of the student's program.
    clean: usize,
}

impl Tally {
    fn add<E: Executor>(
        &mut self,
        ctx: &Ctx<'_, E>,
        description: String,
        input: &str,
        reference: &StepOutput,
        student: &StepOutput,
    ) {
        match ctx.verdict(reference, student) {
            Verdict::Same => self.clean += 1,
            Verdict::Differs => {
                self.clean += 1;
                self.findings
                    .push(ctx.finding(description, input, reference, student));
            }
            Verdict::Unclear(why) => self.unclear.push(format!("{description}: {why}")),
        }
    }

    fn finish(self, checked: usize, what: &str) -> VariationResult {
        if self.findings.is_empty() && self.clean == 0 {
            let mut r = VariationResult::skipped(format!(
                "Unklar: Der Code bricht mit veränderten Werten ab oder überschreitet das \
                 Zeitlimit ({}). Das ist kein Nachweis eines Logikfehlers.",
                self.unclear.join("; ")
            ));
            r.unclear = self.unclear;
            return r;
        }
        let mut r = outcome(self.findings, checked, what);
        r.unclear = self.unclear;
        r
    }
}

fn outcome(findings: Vec<VariationFinding>, checked: usize, what: &str) -> VariationResult {
    let failed = !findings.is_empty();
    VariationResult {
        state: if failed {
            VariationState::Failed
        } else {
            VariationState::Ok
        },
        note: Some(if failed {
            format!(
                "{} von {checked} {what} liefern ein anderes Ergebnis als die Musterlösung.",
                findings.len().min(checked)
            )
        } else {
            format!(
                "{checked} {what} geprüft – der Code rechnet auch mit veränderten Werten richtig."
            )
        }),
        findings: findings.into_iter().take(MAX_FINDINGS).collect(),
        explanation: None,
        explanation_quote: None,
        checked,
        unclear: Vec::new(),
    }
}

/// Case A: additional inputs from the model.
async fn check_inputs<L: Llm + Sync, E: Executor>(
    llm: &L,
    ctx: &Ctx<'_, E>,
    task: &str,
    on_progress: &(impl Fn(AiProgress) + Sync),
) -> Result<VariationResult, AiError> {
    on_progress(AiProgress::new(
        Phase::Variation,
        "KI schreibt zusätzliche Eingaben …",
    ));
    let existing = ctx.req.test_inputs();
    let answer: Option<InputsAnswer> = ask(
        llm,
        json_request(
            prompts::inputs_messages(task, &ctx.req.input_format, &existing),
            prompts::inputs_schema(),
            INPUTS_TEMPERATURE,
        ),
        ctx.cancel,
    )
    .await?;
    let mut inputs = clean_inputs(&answer.map(|a| a.inputs).unwrap_or_default(), &existing);
    inputs.retain(|i| !i.trim().is_empty());
    inputs.truncate(MAX_VARIATION_INPUTS);
    if inputs.is_empty() {
        return Ok(VariationResult::skipped(
            "Die KI hat keine zusätzlichen Eingaben geliefert.",
        ));
    }
    on_progress(AiProgress::new(
        Phase::Variation,
        format!("Prüfe mit {} veränderten Eingaben …", inputs.len()),
    ));
    let (reference, stable) = ctx.run_reference(&inputs).await?;
    let student = ctx.run(ctx.student, &inputs).await?;
    let mut tally = Tally::default();
    let mut checked = 0;
    for (((input, r), s), stable) in inputs.iter().zip(&reference).zip(&student).zip(stable) {
        // The reference solution must handle the input reproducibly (else
        // the input may be invalid or the program not deterministic).
        if !stable || !fits_templates(&ctx.req.output_templates, r) {
            continue;
        }
        checked += 1;
        tally.add(ctx, format!("Eingabe: {}", preview(input)), input, r, s);
    }
    if checked == 0 {
        return Ok(VariationResult::skipped(
            "Die Musterlösung lief auf keiner der zusätzlichen Eingaben fehlerfrei und \
             reproduzierbar – kein Vergleich möglich.",
        ));
    }
    Ok(tally.finish(checked, "zusätzliche Eingabe(n)"))
}

/// Case B: changed start values in both programs.
async fn check_scenarios<L: Llm + Sync, E: Executor>(
    llm: &L,
    ctx: &Ctx<'_, E>,
    task: &str,
    on_progress: &(impl Fn(AiProgress) + Sync),
) -> Result<VariationResult, AiError> {
    on_progress(AiProgress::new(
        Phase::Variation,
        "KI sucht die vorgegebenen Werte der Aufgabe …",
    ));
    let answer: Option<ScenarioAnswer> = ask(
        llm,
        json_request(
            scenario_messages(task),
            scenario_schema(),
            SCENARIO_TEMPERATURE,
        ),
        ctx.cancel,
    )
    .await?;
    let scenarios = validate_scenarios(
        &ctx.req.task_text,
        &answer.map(|a| a.scenarios).unwrap_or_default(),
    );
    if scenarios.is_empty() {
        return Ok(VariationResult::skipped(
            "In der Aufgabe wurden keine Startwerte gefunden, die sich verändern lassen.",
        ));
    }
    let lang = ctx.req.language.as_str();
    let mut inputs = ctx.req.test_inputs();
    if inputs.is_empty() {
        inputs.push(String::new());
    }
    // Original output of the reference solution (compiled already).
    let (baseline, stable) = ctx.run_reference(&inputs).await?;
    if !baseline.iter().all(valid) {
        return Ok(VariationResult::skipped(
            "Die Musterlösung läuft mit den gegebenen Werten nicht fehlerfrei.",
        ));
    }
    if !stable.iter().all(|s| *s) {
        return Ok(VariationResult::skipped(
            "Die Musterlösung liefert bei gleichen Werten unterschiedliche Ausgaben (z. B. \
             Zufall oder Uhrzeit) – Variationstest nicht möglich.",
        ));
    }
    let mut tally = Tally::default();
    let mut checked = 0;
    let mut skipped: Vec<String> = Vec::new();
    for (n, changes) in scenarios.iter().enumerate() {
        on_progress(AiProgress {
            current: Some(n + 1),
            total: Some(scenarios.len()),
            ..AiProgress::new(
                Phase::Variation,
                format!(
                    "Prüfe mit veränderten Werten (Variante {} von {}) …",
                    n + 1,
                    scenarios.len()
                ),
            )
        });
        let student = substitute(&ctx.req.student_code, lang, changes);
        let reference = substitute(ctx.req.reference(), lang, changes);
        let description = describe(changes, &student.idents);
        let replaceable = student
            .counts
            .iter()
            .zip(&reference.counts)
            .all(|(s, r)| s == r && *s > 0);
        if !replaceable {
            skipped.push(format!(
                "{description}: Werte nicht in beiden Programmen gleich ersetzbar"
            ));
            continue;
        }
        if !ctx.compile(ctx.reference, &reference.code).await?.ok {
            skipped.push(format!("{description}: Musterlösung nicht übersetzbar"));
            continue;
        }
        let (ref_out, ref_stable) = ctx.run_reference(&inputs).await?;
        if !ref_out.iter().all(valid) {
            skipped.push(format!(
                "{description}: Musterlösung läuft nicht fehlerfrei"
            ));
            continue;
        }
        if !ref_stable.iter().all(|s| *s) {
            skipped.push(format!(
                "{description}: Ausgabe der Musterlösung nicht reproduzierbar"
            ));
            continue;
        }
        let unchanged = ref_out
            .iter()
            .zip(&baseline)
            .all(|(r, b)| ctx.mode.matches(&b.stdout, &r.stdout));
        if unchanged {
            skipped.push(format!(
                "{description}: Ergebnis der Musterlösung unverändert"
            ));
            continue;
        }
        if !ctx.compile(ctx.student, &student.code).await?.ok {
            skipped.push(format!("{description}: Code nicht übersetzbar"));
            continue;
        }
        let stu_out = ctx.run(ctx.student, &inputs).await?;
        checked += 1;
        for ((input, r), s) in inputs.iter().zip(&ref_out).zip(&stu_out) {
            let mut d = description.clone();
            if inputs.len() > 1 && !input.trim().is_empty() {
                d.push_str(&format!(", Eingabe: {}", preview(input)));
            }
            tally.add(ctx, d, input, r, s);
        }
    }
    if checked == 0 {
        return Ok(VariationResult::skipped(format!(
            "Keine der vorgeschlagenen Wertänderungen war anwendbar ({}). Ersetzt werden nur \
             einfache Zuweisungen wie „hour = 15“.",
            skipped.join("; ")
        )));
    }
    Ok(tally.finish(checked, "Variante(n)"))
}

/// The check itself, generic over model and sandbox. `llm` is `None` when
/// no model is available. Errors of the model or the sandbox become
/// "skipped"; only a cancel is returned as an error.
pub async fn check<L: Llm + Sync, E: Executor>(
    llm: Option<&L>,
    exec: &E,
    req: &VariationRequest,
    cancel: &CancellationToken,
    on_progress: impl Fn(AiProgress) + Sync,
) -> Result<VariationResult, AiError> {
    if let Some(reason) = req.skip_reason() {
        return Ok(VariationResult::skipped(reason));
    }
    let Some(llm) = llm else {
        return Ok(VariationResult::skipped(
            "Kein KI-Modell verfügbar – der Variationstest braucht das lokale KI-Modell.",
        ));
    };
    skip_on_error(check_inner(llm, exec, req, cancel, &on_progress).await)
}

/// Errors become "skipped" with a German note; a cancel stays an error.
fn skip_on_error(r: Result<VariationResult, AiError>) -> Result<VariationResult, AiError> {
    match r {
        Err(AiError::Cancelled) => Err(AiError::Cancelled),
        Err(e) => Ok(VariationResult::skipped(format!(
            "Variationstest nicht möglich: {e}"
        ))),
        ok => ok,
    }
}

async fn check_inner<L: Llm + Sync, E: Executor>(
    llm: &L,
    exec: &E,
    req: &VariationRequest,
    cancel: &CancellationToken,
    on_progress: &(impl Fn(AiProgress) + Sync),
) -> Result<VariationResult, AiError> {
    let task = shorten_task(&req.task_text, MAX_TASK_PROMPT_CHARS);
    on_progress(AiProgress::new(
        Phase::Variation,
        "Prüfe mit veränderten Werten …",
    ));
    let student = cancellable(cancel, exec.open(&req.language)).await?;
    let reference = match cancellable(cancel, exec.open(&req.language)).await {
        Ok(r) => r,
        Err(e) => {
            student.close().await;
            return Err(e);
        }
    };
    let ctx = Ctx::<E> {
        req,
        mode: req.compare_mode.unwrap_or_default(),
        cancel,
        student: &student,
        reference: &reference,
    };
    let result = async {
        if !ctx.compile(&student, &req.student_code).await?.ok {
            return Ok(VariationResult::skipped(
                "Der Code lässt sich nicht übersetzen.",
            ));
        }
        if !ctx.compile(&reference, req.reference()).await?.ok {
            return Ok(VariationResult::skipped(
                "Die Musterlösung lässt sich nicht übersetzen (andere Sprache?).",
            ));
        }
        if req.has_inputs() {
            check_inputs(llm, &ctx, &task, on_progress).await
        } else {
            check_scenarios(llm, &ctx, &task, on_progress).await
        }
    }
    .await;
    student.close().await;
    reference.close().await;
    let mut result = result?;

    // Explanation only, never a decision.
    if result.state == VariationState::Failed
        && req.student_code.chars().count() <= MAX_CODE_EXPLAIN_CHARS
    {
        on_progress(AiProgress::new(Phase::Variation, "KI erklärt den Befund …"));
        let answer = ask::<L, ExplainAnswer>(
            llm,
            json_request(
                explain_messages(&task, &req.language, &req.student_code, &result.findings),
                explain_schema(),
                EXPLAIN_TEMPERATURE,
            ),
            cancel,
        )
        .await;
        match answer {
            Err(AiError::Cancelled) => return Err(AiError::Cancelled),
            Ok(Some(a)) => {
                if let Some((explanation, quote)) = validate_explanation(&req.student_code, &a) {
                    result.explanation = Some(explanation);
                    result.explanation_quote = Some(quote);
                }
            }
            // A failing explanation does not change the result.
            _ => {}
        }
    }
    Ok(result)
}

/// `check_variations`: takes the AI gate (skipped if test generation
/// runs), makes sure engine and model run and checks against the sandbox.
pub async fn run(
    req: VariationRequest,
    cancel: &CancellationToken,
    on_progress: impl Fn(AiProgress) + Send + Sync + 'static,
) -> Result<VariationResult, AiError> {
    if let Some(reason) = req.skip_reason() {
        return Ok(VariationResult::skipped(reason));
    }
    // A check that was just cancelled (new „Prüfen“) releases the gate
    // shortly after; wait for it a little.
    let Some(_gate) = AiGate::acquire_within(GATE_WAIT).await else {
        return Ok(VariationResult::skipped(
            "Die KI ist gerade beschäftigt (es werden Testfälle erzeugt) – Variationstest \
             übersprungen.",
        ));
    };
    let model = match super::models::selected_model() {
        Ok(m) => m,
        Err(e) => {
            return Ok(VariationResult::skipped(format!(
                "Kein KI-Modell verfügbar: {e}"
            )))
        }
    };
    let on_progress = std::sync::Arc::new(on_progress);
    let result = async {
        let p = on_progress.clone();
        let exec = SandboxExecutor::connect(cancel.clone(), move |m| {
            p(AiProgress::new(Phase::Engine, m))
        })
        .await?;
        on_progress(AiProgress::new(Phase::Model, "KI-Modell wird geladen …"));
        let endpoint = super::llama::ensure_running(
            &model.path,
            model.server_slots(),
            model.chat_template(),
            cancel,
            |m| on_progress(AiProgress::new(Phase::Model, m)),
        )
        .await?;
        let llm = super::llama::LlamaClient::new(endpoint, cancel.clone())?;
        check(Some(&llm), &exec, &req, cancel, |p| on_progress(p)).await
    }
    .await;
    skip_on_error(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::llama::ChatResponse;
    use std::collections::HashMap;
    use std::future::Future;
    use std::sync::{Arc, Mutex};

    fn ch(from: &str, to: &str) -> Change {
        Change {
            from: from.into(),
            to: to.into(),
        }
    }

    // --- Literal replacement ---------------------------------------------------

    #[test]
    fn python_replaces_only_simple_assignments() {
        let code = "hour = 15  # Stunde 15\nminute = 10\nsecond: int = 30\nprint(\"hour = 15\")\n\
                    # hour = 15\nx = hour + 15\ndef f(h = 15):\n    total = 15\n    return h\n\
                    s = '''\nhour = 15\n'''\ny = 150\nz = 15.5\n";
        let s = substitute(code, "python", &[ch("15", "3"), ch("30", "45")]);
        assert_eq!(
            s.code,
            "hour = 3  # Stunde 15\nminute = 10\nsecond: int = 45\nprint(\"hour = 15\")\n\
             # hour = 15\nx = hour + 15\ndef f(h = 15):\n    total = 3\n    return h\n\
             s = '''\nhour = 15\n'''\ny = 150\nz = 15.5\n"
        );
        assert_eq!(s.counts, vec![2, 1]);
        assert_eq!(s.idents, vec![Some("hour".into()), Some("second".into())]);
        // Comparisons and augmented assignments are no assignments.
        let s = substitute("if x == 15:\n    x += 15\n", "python", &[ch("15", "3")]);
        assert_eq!(s.counts, vec![0]);
        // Decimal literals match exactly.
        let s = substitute("rate = 1.5\n", "python", &[ch("1.5", "2.5")]);
        assert_eq!(s.code, "rate = 2.5\n");
    }

    #[test]
    fn java_replaces_declarations_and_keeps_comments_and_strings() {
        let code = "public class Main {\n    static final int START = 15;\n    public static void \
                    main(String[] args) {\n        final int hour = 15, minute = 10;\n        int \
                    second=30;\n        long big = 15L;\n        String s = \"x = 15;\";\n        \
                    // y = 15;\n        /* z = 15; */\n        for (int i = 15; i < 20; i++) {}\n        \
                    hour2 = 15;\n        int w = hour + 15;\n        char c = '1';\n    }\n}\n";
        let s = substitute(code, "java", &[ch("15", "3"), ch("10", "45")]);
        assert!(s.code.contains("static final int START = 3;"));
        assert!(s.code.contains("final int hour = 3, minute = 45;"));
        assert!(s.code.contains("long big = 3L;"));
        assert!(s.code.contains("hour2 = 3;"));
        assert!(s.code.contains("String s = \"x = 15;\";"));
        assert!(s.code.contains("// y = 15;"));
        assert!(s.code.contains("/* z = 15; */"));
        assert!(s.code.contains("for (int i = 15;"));
        assert!(s.code.contains("int w = hour + 15;"));
        assert!(s.code.contains("int second=30;"));
        assert_eq!(s.counts, vec![4, 1]);
        assert_eq!(s.idents[0].as_deref(), Some("START"));
        assert_eq!(s.idents[1].as_deref(), Some("minute"));
    }

    #[test]
    fn fields_negative_literals_and_for_heads() {
        let java = "class Main {\n    int hour;\n    void set() {\n        this.hour = 15;\n        \
                    other.hour = 15;\n        int t = -15;\n        int u = 3 -15;\n        \
                    for (int i = 0, n = 15; i < n; i++) {}\n        for (i = 0; i < 3; i = 15) {}\n        \
                    foo(a, b = 15);\n    }\n}\n";
        let s = substitute(java, "java", &[ch("15", "3"), ch("-15", "-4")]);
        assert!(s.code.contains("this.hour = 3;"), "{}", s.code);
        assert!(s.code.contains("other.hour = 15;"));
        assert!(s.code.contains("int t = -4;"));
        assert!(s.code.contains("int u = 3 -15;"));
        assert!(s.code.contains("for (int i = 0, n = 15; i < n; i++)"));
        assert!(s.code.contains("i = 15) {}"));
        assert!(s.code.contains("foo(a, b = 15);"));
        assert_eq!(s.counts, vec![1, 1]);
        assert_eq!(s.idents[0].as_deref(), Some("this.hour"));
        assert_eq!(s.idents[1].as_deref(), Some("t"));
        let py = "class A:\n    def __init__(self):\n        self.hour = 15\n        \
                  self.temp: int = -15\n        x = -15\n        y = 5 -15\n";
        let s = substitute(py, "python", &[ch("15", "3"), ch("-15", "-4")]);
        assert!(s.code.contains("self.hour = 3\n"));
        assert!(s.code.contains("self.temp: int = -4\n"));
        assert!(s.code.contains("x = -4\n"));
        assert!(s.code.contains("y = 5 -15\n"));
        assert_eq!(s.counts, vec![1, 2]);
        assert_eq!(s.idents[0].as_deref(), Some("self.hour"));
        // An unsigned change does not touch a negative literal.
        let s = substitute("x = -15\n", "python", &[ch("15", "3")]);
        assert_eq!(s.counts, vec![0]);
        assert!(plain_number("-15") && plain_number("-1.5") && !plain_number("--1"));
        assert!(number_in_text("Temperatur -15 Grad", "-15"));
    }

    #[test]
    fn literals_in_text_need_whole_numbers() {
        let text = "Es ist 15:10:30 Uhr, Preis 1.15 Euro, 150 Stück.";
        assert!(number_in_text(text, "15"));
        assert!(number_in_text(text, "10"));
        assert!(number_in_text(text, "30"));
        assert!(number_in_text(text, "1.15"));
        assert!(!number_in_text(text, "50"));
        assert!(!number_in_text(text, "1"));
        let raw = |pairs: &[(&str, &str)]| RawScenario {
            changes: pairs
                .iter()
                .map(|(f, t)| RawChange {
                    from: f.to_string(),
                    to: t.to_string(),
                })
                .collect(),
        };
        let v = validate_scenarios(
            text,
            &[
                raw(&[("15", "3")]),
                raw(&[("15", "3")]),
                raw(&[("16", "3")]),
                raw(&[("15", "x")]),
                raw(&[("15", "15")]),
                raw(&[("15", "3"), ("15", "4")]),
                raw(&[]),
                raw(&[("15", "9"), ("10", "45")]),
                raw(&[("30", "5")]),
                raw(&[("150", "7")]),
            ],
        );
        assert_eq!(
            v,
            vec![
                vec![ch("15", "3")],
                vec![ch("15", "9"), ch("10", "45")],
                vec![ch("30", "5")]
            ]
        );
    }

    #[test]
    fn task_is_shortened_at_both_ends() {
        let text = format!("A{}Z", "x".repeat(10_000));
        let s = shorten_task(&text, 100);
        assert!(s.starts_with('A') && s.ends_with('Z'));
        assert!(s.chars().count() <= 110);
        assert_eq!(shorten_task("kurz", 100), "kurz");
    }

    #[test]
    fn explanation_quote_must_be_in_the_code() {
        let code = "left = 86400 % seconds\nprint(left)\n";
        let a = |q: &str| ExplainAnswer {
            quote: q.into(),
            explanation: "Modulo statt Subtraktion.".into(),
        };
        assert_eq!(
            validate_explanation(code, &a("86400  %  seconds")),
            Some((
                "Modulo statt Subtraktion.".into(),
                "86400  %  seconds".into()
            ))
        );
        assert_eq!(validate_explanation(code, &a("86400 - seconds")), None);
        assert_eq!(validate_explanation(code, &a("")), None);
    }

    // --- Flow with mocks ---------------------------------------------------------

    struct MockLlm<F> {
        answer: F,
        calls: Mutex<Vec<ChatRequest>>,
    }

    impl<F: Fn(&ChatRequest) -> Result<String, AiError> + Sync> Llm for MockLlm<F> {
        async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, AiError> {
            self.calls.lock().unwrap().push(req.clone());
            (self.answer)(req).map(|content| ChatResponse {
                content,
                completion_tokens: Some(1),
                tokens_per_second: None,
                finish_reason: Some("stop".into()),
                stopping_word: None,
            })
        }
    }

    fn llm<F: Fn(&ChatRequest) -> Result<String, AiError> + Sync>(f: F) -> MockLlm<F> {
        MockLlm {
            answer: f,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn step(stdout: &str) -> StepOutput {
        StepOutput {
            stdout: stdout.into(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            exit_code: Some(0),
            timed_out: false,
            duration_ms: 1,
        }
    }

    /// Variables `name = <int>` of a mock program (comments ignored).
    fn vars(code: &str) -> HashMap<String, i64> {
        code.lines()
            .filter_map(|l| {
                let l = l.split('#').next()?;
                let (k, v) = l.split_once('=')?;
                Some((k.trim().to_string(), v.trim().parse().ok()?))
            })
            .collect()
    }

    /// Fake interpreter: `SECONDS` programs print the seconds since and
    /// until midnight (with `86400 % s` if the code says so), `DOUBLE`
    /// doubles the input, `CONST42` always prints 42.
    fn output_for(code: &str, input: &str) -> StepOutput {
        let v = vars(code);
        if code.contains("CRASHMORNING") && v.get("hour").is_some_and(|h| *h < 12) {
            return StepOutput {
                stderr: "ZeroDivisionError".into(),
                exit_code: Some(1),
                ..step("")
            };
        }
        if code.contains("SECONDS") {
            let s = v["hour"] * 3600 + v["minute"] * 60 + v["second"];
            let left = if code.contains("86400 % s") {
                86400 % s
            } else {
                86400 - s
            };
            step(&format!(
                "{s} seconds have passed since midnight.\n{left} seconds are left until midnight.\n"
            ))
        } else if code.contains("DOUBLE") {
            let n: i64 = input.trim().parse().unwrap_or(0);
            step(&format!("{}\n", n * 2))
        } else if code.contains("CONST42") {
            step("42\n")
        } else {
            step("?\n")
        }
    }

    #[derive(Default)]
    struct Counters {
        opened: usize,
        closed: usize,
        runs: usize,
    }

    struct MockExec(Arc<Mutex<Counters>>);
    struct MockSession(Arc<Mutex<Counters>>, Mutex<String>);

    impl Session for MockSession {
        async fn compile(&self, code: &str) -> Result<CompileOutcome, AiError> {
            *self.1.lock().unwrap() = code.to_string();
            Ok(CompileOutcome {
                output: None,
                ok: !code.contains("SYNTAX"),
            })
        }
        async fn run(&self, inputs: &[String]) -> Result<Vec<StepOutput>, AiError> {
            let code = self.1.lock().unwrap().clone();
            let runs = {
                let mut c = self.0.lock().unwrap();
                c.runs += 1;
                c.runs
            };
            Ok(inputs
                .iter()
                .map(|i| {
                    if code.contains("RANDOM") {
                        // Output changes from run to run.
                        step(&format!("{runs}\n"))
                    } else {
                        output_for(&code, i)
                    }
                })
                .collect())
        }
        async fn close(self) {
            self.0.lock().unwrap().closed += 1;
        }
    }

    impl Executor for MockExec {
        type Session = MockSession;
        async fn open(&self, _language: &str) -> Result<MockSession, AiError> {
            self.0.lock().unwrap().opened += 1;
            Ok(MockSession(self.0.clone(), Mutex::new(String::new())))
        }
    }

    fn block_on<F: Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    const TASK: &str = "Es ist 15:10:30 Uhr. a) Speichere die Uhrzeit in hour, minute, second. \
        d) Gib aus, wie viele Sekunden seit Mitternacht vergangen sind und wie viele bis \
        Mitternacht fehlen.";
    const REFERENCE: &str =
        "# SECONDS\nhour = 15\nminute = 10\nsecond = 30\ns = hour * 3600 + minute * 60 + second\nleft = 86400 - s\n";
    const MODULO: &str =
        "# SECONDS\nhour = 15\nminute = 10\nsecond = 30\ns = hour * 3600 + minute * 60 + second\nleft = 86400 % s\n";

    fn request(student: &str) -> VariationRequest {
        VariationRequest {
            task_text: TASK.into(),
            language: "python".into(),
            student_code: student.into(),
            reference_solution: Some(REFERENCE.into()),
            compare_mode: None,
            output_templates: Vec::new(),
            tests: vec![TestCase {
                name: "KI-Test (ohne Eingabe)".into(),
                input: String::new(),
                expected: String::new(),
            }],
            input_format: String::new(),
        }
    }

    fn is_scenarios(r: &ChatRequest) -> bool {
        r.messages[0].content == SCENARIO_SYSTEM
    }
    fn is_explain(r: &ChatRequest) -> bool {
        r.messages[0].content == EXPLAIN_SYSTEM
    }

    fn scenario_model(
        scenarios: &'static str,
        quote: &'static str,
    ) -> MockLlm<impl Fn(&ChatRequest) -> Result<String, AiError> + Sync> {
        llm(move |r: &ChatRequest| {
            if is_scenarios(r) {
                Ok(format!(
                    r#"{{"givenValues": [], "scenarios": {scenarios}}}"#
                ))
            } else if is_explain(r) {
                Ok(json!({"quote": quote, "explanation": "Modulo statt Subtraktion: richtig wäre 86400 - s."}).to_string())
            } else {
                Err(AiError::Other("unerwartet".into()))
            }
        })
    }

    fn run_check<F: Fn(&ChatRequest) -> Result<String, AiError> + Sync>(
        model: Option<&MockLlm<F>>,
        req: &VariationRequest,
    ) -> (VariationResult, (usize, usize)) {
        let counters = Arc::new(Mutex::new(Counters::default()));
        let exec = MockExec(counters.clone());
        let r = block_on(check(model, &exec, req, &CancellationToken::new(), |_| {})).unwrap();
        let c = counters.lock().unwrap();
        (r, (c.opened, c.closed))
    }

    #[test]
    fn modulo_instead_of_subtraction_is_found() {
        let model = scenario_model(
            r#"[{"changes": [{"from": "15", "to": "3"}], "why": "Vormittag"}]"#,
            "86400 % s",
        );
        let (r, (opened, closed)) = run_check(Some(&model), &request(MODULO));
        assert_eq!(r.state, VariationState::Failed, "{r:?}");
        assert_eq!(r.checked, 1);
        assert_eq!(r.findings.len(), 1);
        let f = &r.findings[0];
        assert_eq!(f.description, "hour = 3 statt 15");
        // 03:10:30 = 11430 s: 74970 left, the modulo gives 6390.
        assert!(f.expected.contains("74970 seconds are left"), "{f:?}");
        assert!(f.actual.contains("6390 seconds are left"), "{f:?}");
        assert!(!f.diff.is_empty());
        assert_eq!(r.explanation_quote.as_deref(), Some("86400 % s"));
        assert!(r.explanation.unwrap().contains("Modulo"));
        assert_eq!((opened, closed), (2, 2));
    }

    #[test]
    fn correct_code_is_ok() {
        let model = scenario_model(
            r#"[{"changes": [{"from": "15", "to": "3"}], "why": "a"},
                {"changes": [{"from": "15", "to": "9"}, {"from": "10", "to": "45"}], "why": "b"}]"#,
            "x",
        );
        let student = REFERENCE.replace("left", "rest");
        let (r, _) = run_check(Some(&model), &request(&student));
        assert_eq!(r.state, VariationState::Ok, "{r:?}");
        assert_eq!(r.checked, 2);
        assert!(r.explanation.is_none());
        assert!(!model.calls.lock().unwrap().iter().any(is_explain));
    }

    #[test]
    fn ineffective_scenario_is_skipped() {
        // "30" → "31" changes only the second: the reference output changes.
        // An unused value does not: the scenario has no effect.
        let task = format!("{TASK} Hinweis: 7 Tage pro Woche.");
        let with_unused = |c: &str| format!("{c}unused = 7\n");
        let model = scenario_model(
            r#"[{"changes": [{"from": "7", "to": "8"}], "why": "x"}]"#,
            "x",
        );
        let mut req = request(&with_unused(MODULO));
        req.task_text = task;
        req.reference_solution = Some(with_unused(REFERENCE));
        let (r, _) = run_check(Some(&model), &req);
        assert_eq!(r.state, VariationState::Skipped, "{r:?}");
        assert!(r.note.unwrap().contains("unverändert"));
    }

    #[test]
    fn different_replacement_counts_skip_the_scenario() {
        // The student writes 15 inline: nothing to replace in their code.
        let student = "# SECONDS\nhour = int('15')\nminute = 10\nsecond = 30\nleft = 86400 % s\n";
        let model = scenario_model(
            r#"[{"changes": [{"from": "15", "to": "3"}], "why": "x"}]"#,
            "x",
        );
        let (r, _) = run_check(Some(&model), &request(student));
        assert_eq!(r.state, VariationState::Skipped, "{r:?}");
        assert!(r.note.unwrap().contains("nicht in beiden Programmen"));
        let a = substitute("x = 15\ny = 15\n", "python", &[ch("15", "3")]);
        let b = substitute("x = 15\n", "python", &[ch("15", "3")]);
        assert_ne!(a.counts, b.counts);
    }

    #[test]
    fn quote_not_in_code_gives_no_explanation() {
        let model = scenario_model(
            r#"[{"changes": [{"from": "15", "to": "3"}], "why": "x"}]"#,
            "86400 mod seconds",
        );
        let (r, _) = run_check(Some(&model), &request(MODULO));
        assert_eq!(r.state, VariationState::Failed);
        assert!(r.explanation.is_none());
        assert!(r.explanation_quote.is_none());
    }

    #[test]
    fn no_model_or_no_reference_is_skipped() {
        let (r, (opened, _)) =
            run_check::<fn(&ChatRequest) -> Result<String, AiError>>(None, &request(MODULO));
        assert_eq!(r.state, VariationState::Skipped);
        assert!(r.note.unwrap().contains("Kein KI-Modell"));
        assert_eq!(opened, 0);
        let mut req = request(MODULO);
        req.reference_solution = None;
        let model = scenario_model("[]", "x");
        let (r, _) = run_check(Some(&model), &req);
        assert_eq!(r.state, VariationState::Skipped);
        assert!(r.note.unwrap().contains("Musterlösung"));
        assert!(model.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn model_errors_skip_and_cancel_is_an_error() {
        let model = llm(|_: &ChatRequest| Err(AiError::Http("weg".into())));
        let (r, (opened, closed)) = run_check(Some(&model), &request(MODULO));
        assert_eq!(r.state, VariationState::Skipped);
        assert!(r.note.unwrap().contains("weg"));
        assert_eq!(opened, closed);
        let model = llm(|_: &ChatRequest| Err(AiError::Cancelled));
        let exec = MockExec(Arc::new(Mutex::new(Counters::default())));
        let r = block_on(check(
            Some(&model),
            &exec,
            &request(MODULO),
            &CancellationToken::new(),
            |_| {},
        ));
        assert!(matches!(r, Err(AiError::Cancelled)));
    }

    #[test]
    fn non_deterministic_reference_is_skipped() {
        let model = scenario_model(
            r#"[{"changes": [{"from": "15", "to": "3"}], "why": "x"}]"#,
            "x",
        );
        let mut req = request(MODULO);
        req.reference_solution = Some(format!("{REFERENCE}# RANDOM\n"));
        let (r, _) = run_check(Some(&model), &req);
        assert_eq!(r.state, VariationState::Skipped, "{r:?}");
        assert!(r.note.unwrap().contains("unterschiedliche Ausgaben"));
        // Case A: inputs with changing reference output are not compared.
        let model = llm(|r: &ChatRequest| {
            if r.messages[0].content == prompts::INPUTS_SYSTEM {
                Ok(r#"{"inputs": ["3"]}"#.into())
            } else {
                Err(AiError::Http("x".into()))
            }
        });
        let mut req = request("# CONST42\n");
        req.reference_solution = Some("# RANDOM\n".into());
        req.tests[0].input = "21\n".into();
        let (r, _) = run_check(Some(&model), &req);
        assert_eq!(r.state, VariationState::Skipped, "{r:?}");
        assert!(r.note.unwrap().contains("reproduzierbar"));
    }

    #[test]
    fn student_crash_is_unclear_not_wrong_logic() {
        let model = scenario_model(
            r#"[{"changes": [{"from": "15", "to": "3"}], "why": "x"}]"#,
            "x",
        );
        let student = format!("{REFERENCE}# CRASHMORNING\n");
        let (r, _) = run_check(Some(&model), &request(&student));
        assert_eq!(r.state, VariationState::Skipped, "{r:?}");
        assert!(r.findings.is_empty());
        assert_eq!(
            r.unclear,
            vec!["hour = 3 statt 15: Laufzeitfehler".to_string()]
        );
        assert!(r.note.unwrap().starts_with("Unklar"));
        assert!(!model.calls.lock().unwrap().iter().any(is_explain));
        // Crash in one variant, clean and right in another: ok with a hint.
        let model = scenario_model(
            r#"[{"changes": [{"from": "15", "to": "3"}], "why": "x"},
                {"changes": [{"from": "15", "to": "17"}], "why": "y"}]"#,
            "x",
        );
        let (r, _) = run_check(Some(&model), &request(&student));
        assert_eq!(r.state, VariationState::Ok, "{r:?}");
        assert_eq!(r.unclear.len(), 1);
    }

    #[test]
    fn inputs_case_compares_with_the_reference() {
        let model = llm(|r: &ChatRequest| {
            if r.messages[0].content != prompts::INPUTS_SYSTEM {
                return Err(AiError::Http("keine Erklärung".into()));
            }
            assert!(r.messages[1].content.contains("Input format: eine Zahl n"));
            // The test input is named as existing; duplicates and the
            // empty input are dropped.
            assert!(r.messages[1].content.contains("do not repeat"));
            Ok(r#"{"inputs": ["21", "3", "", "10"]}"#.into())
        });
        let mut req = request("# CONST42\nprint(42)\n");
        req.reference_solution = Some("# DOUBLE\nprint(int(input()) * 2)\n".into());
        req.tests = vec![TestCase {
            name: "t".into(),
            input: "21\n".into(),
            expected: "42\n".into(),
        }];
        req.input_format = "eine Zahl n".into();
        let (r, _) = run_check(Some(&model), &req);
        // The explanation request fails: the result stays.
        assert_eq!(r.state, VariationState::Failed, "{r:?}");
        assert_eq!(r.checked, 2);
        assert_eq!(r.findings[0].description, "Eingabe: 3");
        assert_eq!(r.findings[0].expected, "6\n");
        // A correct program passes.
        req.student_code = "# DOUBLE\nprint(2 * int(input()))\n".into();
        let model = llm(|_: &ChatRequest| Ok(r#"{"inputs": ["3", "10"]}"#.into()));
        let (r, _) = run_check(Some(&model), &req);
        assert_eq!(r.state, VariationState::Ok);
    }

    #[test]
    fn result_serializes_in_camel_case() {
        let r = VariationResult::skipped("x");
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["state"], "skipped");
        assert!(v["explanationQuote"].is_null());
        let req: VariationRequest = serde_json::from_str(
            r#"{"taskText":"t","language":"python","studentCode":"x","referenceSolution":"y"}"#,
        )
        .unwrap();
        assert!(req.tests.is_empty() && req.output_templates.is_empty());
        assert!(req.skip_reason().is_none());
    }
}
