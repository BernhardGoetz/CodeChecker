//! Output templates: a task sheet may show the output format with
//! placeholder values ("12345678 seconds have passed since midnight." where
//! the sheet says 12345678 is only a placeholder). Such a template is not
//! an example – it only fixes the format. [`template_matches`] checks an
//! output against a template: the fixed text must match, every placeholder
//! stands for a non-empty value (a number if the placeholder is a number).
//!
//! Matching is done line by line (lines trimmed, whitespace runs collapsed,
//! empty lines at the start and end ignored) with a small backtracking
//! matcher, no regular expressions.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::compare::{is_number, normalize_for_match, normalize_newlines};

/// Upper bound for templates taken from one task.
pub const MAX_TEMPLATES: usize = 3;
/// Upper bound for placeholders of one template.
pub const MAX_PLACEHOLDERS: usize = 6;
/// Output lines longer than this never match a template (bounds the
/// matching time; templates are short lines of text).
pub const MAX_LINE_BYTES: usize = 4096;

/// Characters a number value may consist of (see [`is_number`]).
fn number_char(c: char) -> bool {
    c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E')
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OutputTemplate {
    /// The template as printed in the task sheet.
    pub text: String,
    /// Placeholder tokens exactly as they appear in `text`.
    pub placeholders: Vec<String>,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether `token` occurs in `s` at byte offset `pos` with word boundaries
/// (like `\b` in regular expressions: only checked where the token itself
/// starts/ends with a word character).
fn token_at(s: &str, pos: usize, token: &str) -> bool {
    if token.is_empty() || !s[pos..].starts_with(token) {
        return false;
    }
    let first = token.chars().next().is_some_and(is_word_char);
    let last = token.chars().next_back().is_some_and(is_word_char);
    let before = s[..pos].chars().next_back();
    let after = s[pos + token.len()..].chars().next();
    !(first && before.is_some_and(is_word_char)) && !(last && after.is_some_and(is_word_char))
}

/// Whether `token` occurs in `s` at a word boundary.
fn contains_token(s: &str, token: &str) -> bool {
    s.char_indices().any(|(i, _)| token_at(s, i, token))
}

/// Accepted templates and the number of discarded ones. A template is
/// discarded if its text is not verbatim in the task text (see
/// [`normalize_for_match`]), it has no placeholders, or a placeholder is
/// empty, contains whitespace or does not occur in the template text. At
/// most [`MAX_TEMPLATES`], duplicates removed.
pub fn verify_templates(task_text: &str, raw: &[OutputTemplate]) -> (Vec<OutputTemplate>, usize) {
    let text = normalize_for_match(task_text);
    let mut kept: Vec<OutputTemplate> = Vec::new();
    let mut discarded = 0;
    for t in raw {
        let body = normalize_for_match(&t.text);
        let mut tokens: Vec<String> = Vec::new();
        for p in &t.placeholders {
            let p = p.trim().to_string();
            if !tokens.contains(&p) {
                tokens.push(p);
            }
        }
        let ok = !body.is_empty()
            && text.contains(&body)
            && !tokens.is_empty()
            && tokens.len() <= MAX_PLACEHOLDERS
            && tokens.iter().all(|p| {
                !p.is_empty() && !p.chars().any(char::is_whitespace) && contains_token(&body, p)
            });
        if !ok {
            discarded += 1;
            continue;
        }
        if kept.len() < MAX_TEMPLATES && !kept.iter().any(|k| k.text == body) {
            kept.push(OutputTemplate {
                text: body,
                placeholders: tokens,
            });
        }
    }
    (kept, discarded)
}

/// Lines for the comparison: LF, trimmed, whitespace runs as one space,
/// no empty lines at the start and end.
fn lines(s: &str) -> Vec<String> {
    let all: Vec<String> = normalize_newlines(s)
        .split('\n')
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    let start = all.iter().position(|l| !l.is_empty());
    let end = all.iter().rposition(|l| !l.is_empty());
    match (start, end) {
        (Some(a), Some(b)) => all[a..=b].to_vec(),
        _ => Vec::new(),
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Segment {
    Text(String),
    /// A placeholder; `numeric`: the value must be a number.
    Hole {
        numeric: bool,
    },
}

/// Splits a template line into fixed text and holes (longest tokens first,
/// only at word boundaries).
fn segments(line: &str, tokens: &[&str]) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut pos = 0;
    while pos < line.len() {
        if let Some(tok) = tokens.iter().find(|t| token_at(line, pos, t)) {
            if !text.is_empty() {
                out.push(Segment::Text(std::mem::take(&mut text)));
            }
            out.push(Segment::Hole {
                numeric: is_number(tok),
            });
            pos += tok.len();
        } else {
            let c = line[pos..].chars().next().expect("in bounds");
            text.push(c);
            pos += c.len_utf8();
        }
    }
    if !text.is_empty() {
        out.push(Segment::Text(text));
    }
    out
}

/// Backtracking match of `segs[i..]` against `s[pos..]`; `failed` memoizes
/// (segment, position) pairs that cannot match.
fn match_from(
    segs: &[Segment],
    i: usize,
    s: &str,
    pos: usize,
    failed: &mut HashSet<(usize, usize)>,
) -> bool {
    if i == segs.len() {
        return pos == s.len();
    }
    if failed.contains(&(i, pos)) {
        return false;
    }
    let ok = match &segs[i] {
        Segment::Text(t) => {
            s[pos..].starts_with(t.as_str()) && match_from(segs, i + 1, s, pos + t.len(), failed)
        }
        Segment::Hole { numeric } => {
            let rest = &s[pos..];
            // A number value only extends over number characters.
            let limit = if *numeric {
                pos + rest.find(|c: char| !number_char(c)).unwrap_or(rest.len())
            } else {
                s.len()
            };
            // The value is not empty: it ends after its first character.
            let first = pos + rest.chars().next().map_or(0, char::len_utf8);
            if first == pos || first > limit {
                false
            } else {
                // Possible ends: before the next fixed text, at the end of
                // the line for a final hole, else every character boundary.
                let ends: Vec<usize> = match segs.get(i + 1) {
                    None => vec![s.len()].into_iter().filter(|&e| e <= limit).collect(),
                    Some(Segment::Text(t)) => s[first..]
                        .match_indices(t.as_str())
                        .map(|(k, _)| first + k)
                        .take_while(|&e| e <= limit)
                        .collect(),
                    Some(Segment::Hole { .. }) => {
                        (first..=limit).filter(|&e| s.is_char_boundary(e)).collect()
                    }
                };
                ends.into_iter().any(|end| {
                    (!numeric || is_number(&s[pos..end])) && match_from(segs, i + 1, s, end, failed)
                })
            }
        }
    };
    if !ok {
        failed.insert((i, pos));
    }
    ok
}

fn line_matches(template_line: &str, tokens: &[&str], line: &str) -> bool {
    if line.len() > MAX_LINE_BYTES {
        return false;
    }
    let segs = segments(template_line, tokens);
    // Every hole takes at least one byte.
    let min_len: usize = segs
        .iter()
        .map(|s| match s {
            Segment::Text(t) => t.len(),
            Segment::Hole { .. } => 1,
        })
        .sum();
    min_len <= line.len() && match_from(&segs, 0, line, 0, &mut HashSet::new())
}

fn sorted_tokens(t: &OutputTemplate) -> Vec<&str> {
    let mut tokens: Vec<&str> = t
        .placeholders
        .iter()
        .map(|p| p.as_str())
        .filter(|p| !p.is_empty())
        .collect();
    tokens.sort_by_key(|t| std::cmp::Reverse(t.len()));
    tokens
}

/// Whether `output` has the format of template `t`. `Err` carries the
/// first line (1-based) that does not match – a missing or an extra line
/// counts as well.
pub fn template_matches(t: &OutputTemplate, output: &str) -> Result<(), usize> {
    let tokens = sorted_tokens(t);
    let expected = lines(&t.text);
    let actual = lines(output);
    for (i, (e, a)) in expected.iter().zip(&actual).enumerate() {
        if !line_matches(e, &tokens, a) {
            return Err(i + 1);
        }
    }
    if expected.len() != actual.len() {
        return Err(expected.len().min(actual.len()) + 1);
    }
    Ok(())
}

/// German error message if `output` matches none of the templates (`None`
/// if there are no templates or one matches). Names the line and the
/// expected template line of the first template.
pub fn check_output(templates: &[OutputTemplate], output: &str) -> Option<String> {
    let mut first: Option<(&OutputTemplate, usize)> = None;
    for t in templates {
        match template_matches(t, output) {
            Ok(()) => return None,
            Err(line) => {
                first.get_or_insert((t, line));
            }
        }
    }
    let (t, line) = first?;
    let expected = lines(&t.text);
    let placeholders = t
        .placeholders
        .iter()
        .map(|p| format!("„{p}“"))
        .collect::<Vec<_>>()
        .join(", ");
    Some(match expected.get(line - 1) {
        Some(e) => format!(
            "Ausgabeformat weicht von der Vorlage des Aufgabenblatts ab: Zeile {line} passt nicht \
             zu „{e}“ (Platzhalter: {placeholders})."
        ),
        None => format!(
            "Ausgabeformat weicht von der Vorlage des Aufgabenblatts ab: Zeile {line} ist \
             überzählig – die Vorlage hat nur {} Zeile(n).",
            expected.len()
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHEET: &str = "Aufgabe 2\nGib Folgendes aus (12345678 ist nur ein Platzhalter):\n\n  \
        12345678 seconds have passed since midnight.\n  12345678 seconds are left until \
        midnight.\n  Since I started with the task 12345678 seconds have passed.\n";

    fn sheet_template() -> OutputTemplate {
        OutputTemplate {
            text: "12345678 seconds have passed since midnight.\n12345678 seconds are left until \
                   midnight.\nSince I started with the task 12345678 seconds have passed."
                .into(),
            placeholders: vec!["12345678".into()],
        }
    }

    fn t(text: &str, placeholders: &[&str]) -> OutputTemplate {
        OutputTemplate {
            text: text.into(),
            placeholders: placeholders.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn sheet_example_matches_real_values() {
        let out = "54630 seconds have passed since midnight.\n31770 seconds are left until \
                   midnight.\nSince I started with the task 441 seconds have passed.\n";
        assert_eq!(template_matches(&sheet_template(), out), Ok(()));
        assert_eq!(check_output(&[sheet_template()], out), None);
        // CRLF, extra spaces and empty lines at the end do not matter.
        let out = "54630  seconds have passed since midnight. \r\n31770 seconds are left until \
                   midnight.\r\nSince I started with the task 441 seconds have passed.\r\n\r\n";
        assert_eq!(template_matches(&sheet_template(), out), Ok(()));
        // The template itself matches as well.
        assert_eq!(
            template_matches(&sheet_template(), &sheet_template().text),
            Ok(())
        );
    }

    #[test]
    fn wrong_text_missing_line_and_non_numbers_fail() {
        let wrong = "54630 seconds passed since midnight.\n31770 seconds are left until \
                     midnight.\nSince I started with the task 441 seconds have passed.";
        assert_eq!(template_matches(&sheet_template(), wrong), Err(1));
        let missing = "54630 seconds have passed since midnight.\n31770 seconds are left until \
                       midnight.";
        assert_eq!(template_matches(&sheet_template(), missing), Err(3));
        let msg = check_output(&[sheet_template()], missing).unwrap();
        assert!(msg.contains("Zeile 3"), "{msg}");
        assert!(
            msg.contains("Since I started with the task 12345678"),
            "{msg}"
        );
        let text = "abc seconds have passed since midnight.\n31770 seconds are left until \
                    midnight.\nSince I started with the task 441 seconds have passed.";
        assert_eq!(template_matches(&sheet_template(), text), Err(1));
        let extra = format!("{}\nmehr", sheet_template().text);
        assert_eq!(template_matches(&sheet_template(), &extra), Err(4));
        assert!(check_output(&[sheet_template()], &extra)
            .unwrap()
            .contains("überzählig"));
        // Empty output: the first line is missing.
        assert_eq!(template_matches(&sheet_template(), ""), Err(1));
    }

    #[test]
    fn placeholders_only_at_word_boundaries() {
        // "1" is a placeholder, but the "1" inside "12" is not.
        let tpl = t("Wert 1 von 12", &["1"]);
        assert_eq!(template_matches(&tpl, "Wert 7 von 12"), Ok(()));
        assert_eq!(template_matches(&tpl, "Wert 7 von 72"), Err(1));
        assert_eq!(template_matches(&tpl, "Wert 7 von 7"), Err(1));
        // "X" inside a word is no placeholder.
        let tpl = t("X Xylophone", &["X"]);
        assert_eq!(template_matches(&tpl, "Anna Xylophone"), Ok(()));
        assert_eq!(template_matches(&tpl, "Anna Bylophone"), Err(1));
    }

    #[test]
    fn non_numeric_placeholders_take_any_non_empty_text() {
        let tpl = t("Hallo, <name>! Du bist <n> Jahre alt.", &["<name>", "<n>"]);
        assert_eq!(
            template_matches(&tpl, "Hallo, Anna Maria! Du bist 17 Jahre alt."),
            Ok(())
        );
        assert_eq!(
            template_matches(&tpl, "Hallo, ! Du bist 17 Jahre alt."),
            Err(1)
        );
        // <n> is not a number: any value is fine.
        assert_eq!(
            template_matches(&tpl, "Hallo, A! Du bist viele Jahre alt."),
            Ok(())
        );
        // Numeric placeholder: negative and decimal numbers are numbers.
        let tpl = t("Ergebnis: 0", &["0"]);
        assert_eq!(template_matches(&tpl, "Ergebnis: -2.5"), Ok(()));
        assert_eq!(template_matches(&tpl, "Ergebnis: zwei"), Err(1));
    }

    #[test]
    fn long_lines_are_fast() {
        let tpl = t("Wert: 0 und X Ende", &["0", "X"]);
        let started = std::time::Instant::now();
        // 64 KB in one line: rejected right away.
        let huge = format!("Wert: {} und a Ende", "1".repeat(64 * 1024));
        assert_eq!(template_matches(&tpl, &huge), Err(1));
        let huge = "x ".repeat(32 * 1024);
        assert_eq!(template_matches(&tpl, &huge), Err(1));
        // Close to the limit: still matched, and quickly.
        let long = format!("Wert: {} und {} Ende", "7".repeat(1500), "b ".repeat(1000));
        assert!(long.len() <= MAX_LINE_BYTES);
        assert_eq!(template_matches(&tpl, &long), Ok(()));
        let long = format!("Wert: {} und {} Endx", "7".repeat(1500), "b ".repeat(1000));
        assert_eq!(template_matches(&tpl, &long), Err(1));
        // Adjacent holes over a long line.
        let adj = t("<a><b>!", &["<a>", "<b>"]);
        let line = "c".repeat(4000);
        assert_eq!(template_matches(&adj, &format!("{line}!")), Ok(()));
        assert_eq!(template_matches(&adj, &format!("{line}?")), Err(1));
        assert!(
            started.elapsed() < std::time::Duration::from_millis(500),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn longest_token_wins() {
        let tpl = t("12 und 123", &["12", "123"]);
        assert_eq!(
            segments(&lines(&tpl.text)[0], &sorted_tokens(&tpl)),
            vec![
                Segment::Hole { numeric: true },
                Segment::Text(" und ".into()),
                Segment::Hole { numeric: true }
            ]
        );
        assert_eq!(template_matches(&tpl, "5 und 6"), Ok(()));
    }

    #[test]
    fn several_templates_need_one_match() {
        let a = t("Summe: 0", &["0"]);
        let b = t("Keine Zahlen", &["Zahlen"]);
        assert_eq!(check_output(&[a.clone(), b.clone()], "Keine Werte"), None);
        assert_eq!(check_output(&[a.clone(), b], "Summe: 4"), None);
        assert!(check_output(&[a], "Produkt: 4").is_some());
        assert_eq!(check_output(&[], "irgendwas"), None);
    }

    #[test]
    fn templates_are_verified_against_the_task() {
        let (kept, discarded) = verify_templates(
            SHEET,
            &[
                sheet_template(),
                // Duplicate (whitespace differs).
                t(
                    "  12345678 seconds have passed since midnight.\n12345678 seconds are left \
                     until midnight.\nSince I started with the task 12345678 seconds have passed.",
                    &[" 12345678 "],
                ),
                // Not in the text.
                t("99 seconds", &["99"]),
                // No placeholders.
                t("seconds have passed since midnight.", &[]),
                // Placeholder not in the template text.
                t("seconds have passed since midnight.", &["12345678"]),
                // Placeholder with whitespace.
                t("12345678 seconds have passed", &["12345678 seconds"]),
                // Only inside a word: "1234" is no token of the text.
                t("12345678 seconds have passed", &["1234"]),
            ],
        );
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].placeholders, vec!["12345678".to_string()]);
        assert_eq!(discarded, 5);
        let many: Vec<OutputTemplate> = ["Aufgabe", "Gib", "Platzhalter", "seconds"]
            .iter()
            .map(|w| t(w, &[w]))
            .collect();
        assert_eq!(verify_templates(SHEET, &many).0.len(), MAX_TEMPLATES);
    }

    #[test]
    fn serde_uses_camel_case_and_defaults() {
        let t: OutputTemplate = serde_json::from_str(r#"{"text":"x 1"}"#).unwrap();
        assert!(t.placeholders.is_empty());
        let v = serde_json::to_value(sheet_template()).unwrap();
        assert!(v["placeholders"].is_array());
    }
}
