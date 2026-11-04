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

fn language_rules(language: &str) -> &'static str {
    match language {
        "java" => {
            "Write Java 21. Put everything into `public class Main` with \
             `public static void main(String[] args)`, no package declaration, only the \
             standard library. Read the input with `java.util.Scanner` or \
             `java.io.BufferedReader` from System.in."
        }
        _ => {
            "Write Python 3.12 using only the standard library. Read the input from \
             standard input (`input()` or `sys.stdin`). Never pass a prompt string to \
             `input()` – prompts would end up in the output."
        }
    }
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
template is not an example. Empty list if there is none.\n\
- compareMode: \"float\" if the output contains decimal numbers where tiny rounding \
differences are acceptable, \"tokens\" if only the words/numbers matter and spacing or \
line breaks do not, otherwise \"lines\".\n\
Answer with JSON only.";

pub fn structure_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "inputFormat": { "type": "string" },
            "readsInput": { "type": "boolean" },
            "outputFormat": { "type": "string" },
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
        language_rules(language),
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
    for t in ctx.output_templates {
        let tokens = t
            .placeholders
            .iter()
            .map(|p| format!("`{p}`"))
            .collect::<Vec<_>>()
            .join(", ");
        user.push_str(&format!(
            "\n\nOutput format template (placeholders, not real values). Placeholders: {tokens}. \
             Print exactly this format, replace every placeholder by the real value your program \
             computes, and keep all other text unchanged:\n```\n{}\n```",
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
values, boundaries, special cases mentioned in the task). Each input is the complete text \
given to the program on standard input, with line breaks as \\n. Only inputs, never \
outputs. Answer with JSON only.";

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

pub fn inputs_messages(
    task_text: &str,
    input_format: &str,
    example_inputs: &[String],
) -> Vec<ChatMessage> {
    let mut user = task_block(task_text);
    if !input_format.trim().is_empty() {
        user.push_str(&format!("\n\nInput format: {}", input_format.trim()));
    }
    if !example_inputs.is_empty() {
        user.push_str("\n\nExample inputs from the sheet (do not repeat them):");
        for e in example_inputs {
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
        "java" => text.contains("class Main"),
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
        };
        let m = solution_messages("Verdopple n.", "java", &ctx);
        assert!(m[0].content.contains("public class Main"));
        assert!(m[0].content.contains("```java"));
        assert!(m[1].content.contains("Verdopple n."));
        assert!(m[1].content.contains("Example 1\nInput:\n```\n3\n```"));
        let m = solution_messages("x", "python", &ctx);
        assert!(m[0].content.contains("Never pass a prompt string"));
        let r = repair_messages(m, "```python\nprint(\n```", "SyntaxError: x", "python");
        assert_eq!(r.len(), 4);
        assert!(r[3].content.contains("SyntaxError: x"));
        let i = inputs_messages("x", "n", &["3\n".into()]);
        assert!(i[1].content.contains("do not repeat"));
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
        assert_eq!(inputs_schema()["properties"]["inputs"]["maxItems"], 10);
    }
}
