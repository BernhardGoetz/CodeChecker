//! Tolerance for input prompts – only for AI-generated tests.
//!
//! Programs often print a prompt before reading stdin (Python
//! `input("Zahl: ")`, Java `System.out.print("Name: ")`). stdin does not
//! appear in stdout, so the prompt ends up directly in front of the next
//! output: as a prefix of a result line ("Zahl: 25") or as a line of its
//! own. The expected outputs of AI tests are produced by AI solutions that
//! were told not to print prompts, so a student's program with prompts
//! would fail although it computes the right values.
//!
//! [`matches_ignoring_prompts`] accepts such an output: guided by the
//! expected output it removes prompt-shaped fragments at the start of lines
//! or whole prompt lines from the **actual** output, at most `budget` of
//! them, and then compares with the normal [`CompareMode::matches`]. The
//! cleaned output is produced by removing text only, and it must match the
//! expected output, so a wrong value is never accepted. Labels that are
//! results ("Ergebnis:", "Summe:", "Result:" …) are never prompts.
//!
//! The caller tries the strict comparison first; handwritten tests never
//! use this tolerance.

use super::compare::{normalize_newlines, CompareMode};
use super::template::MAX_LINE_BYTES;

/// Upper bound for the budget (see [`prompt_budget`]).
pub const MAX_IGNORED_PROMPTS: usize = 64;
/// Longest prompt fragment (characters).
pub const MAX_PROMPT_CHARS: usize = 120;
/// Longest prompt fragment (words).
const MAX_PROMPT_WORDS: usize = 12;
/// Upper bound for the DP tables (bounds time and memory).
const MAX_STATES: usize = 2_000_000;

/// Fragments containing one of these (lowercase) are results, not prompts.
const RESULT_STEMS: &[&str] = &[
    "ergebnis", "resultat", "result", "summe", "sum", "produkt", "product", "ausgabe", "output",
    "lösung", "loesung", "solution", "antwort", "answer", "fehler", "error",
];

/// Words that make a line without terminator a prompt line.
const KEYWORDS: &[&str] = &[
    "eingeben", "eingabe", "enter", "input", "type", "bitte", "please",
];

const TERMINATORS: [char; 4] = [':', '?', '>', '='];

/// Result of [`matches_ignoring_prompts`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptStrip {
    /// The actual output without the ignored prompts.
    pub cleaned: String,
    /// The ignored fragments, in order (shown to the user).
    pub removed: Vec<String>,
}

/// How many prompt fragments may be ignored for `input`: the number of
/// whitespace-separated tokens + 1 (a program may print one prompt per value
/// read, plus one before a read at the end of the input), at most
/// [`MAX_IGNORED_PROMPTS`]. 0 for an input without tokens: a program that
/// reads nothing prints no prompts.
pub fn prompt_budget(input: &str) -> usize {
    match input.split_whitespace().count() {
        0 => 0,
        n => (n + 1).min(MAX_IGNORED_PROMPTS),
    }
}

fn has_result_stem(s: &str) -> bool {
    let lower = s.to_lowercase();
    RESULT_STEMS.iter().any(|w| lower.contains(w))
}

fn short_enough(s: &str) -> bool {
    s.chars().count() <= MAX_PROMPT_CHARS && s.split_whitespace().count() <= MAX_PROMPT_WORDS
}

/// Whether a trimmed text ending with a terminator is a prompt fragment.
fn is_prompt_fragment(frag: &str) -> bool {
    let Some(last) = frag.chars().next_back() else {
        return false;
    };
    if !TERMINATORS.contains(&last) {
        return false;
    }
    if matches!(frag, ">" | ">>" | ">>>" | "?" | ":") {
        return true;
    }
    if !frag.chars().any(char::is_alphabetic) || !short_enough(frag) {
        return false;
    }
    // "n =" is a prompt, "a + b =" labels a result.
    if last == '=' && frag[..frag.len() - 1].split_whitespace().count() > 2 {
        return false;
    }
    !has_result_stem(frag)
}

/// A line without terminator that asks for input ("Bitte Namen eingeben").
fn is_keyword_line(line: &str) -> bool {
    if line.contains(TERMINATORS) || !short_enough(line) || has_result_stem(line) {
        return false;
    }
    let lower = line.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    words.iter().any(|w| KEYWORDS.contains(w))
        || words.windows(2).any(|p| p == ["geben", "sie"])
        || (words.contains(&"gib") && words.contains(&"ein"))
}

/// Prompt fragments at the start of a line and the possible cuts after them.
#[derive(Debug, Default)]
struct LineInfo<'a> {
    /// Consecutive fragments from the start of the line (trimmed).
    frags: Vec<&'a str>,
    /// (byte offset of the rest, number of fragments before it).
    cuts: Vec<(usize, usize)>,
    /// The whole line is a prompt: number of fragments.
    only: Option<usize>,
    /// The whole line is a keyword prompt line (no fragments).
    keyword: bool,
}

/// Fragments at the start of `line`. A fragment ends with a terminator that
/// is followed by whitespace or the line end, or directly preceded by a
/// letter or `)` (`input("Zahl:")` gives "Zahl:25", but "12:30" stays).
fn analyze(line: &str) -> LineInfo<'_> {
    let mut info = LineInfo::default();
    if line.len() > MAX_LINE_BYTES {
        return info;
    }
    let mut start = 0;
    let mut pos = 0;
    while let Some(off) = line[pos..].find(TERMINATORS) {
        let t = pos + off;
        let frag = line[start..=t].trim();
        if !short_enough(frag) {
            break;
        }
        let after = line[t + 1..].chars().next();
        let before = line[..t].chars().next_back();
        let context = after.is_none_or(char::is_whitespace)
            || before.is_some_and(|c| c.is_alphabetic() || c == ')');
        pos = t + 1;
        if !(context && is_prompt_fragment(frag)) {
            continue;
        }
        info.frags.push(frag);
        let k = info.frags.len();
        let end = t + 1;
        let rest = &line[end..];
        let all_ws = end + (rest.len() - rest.trim_start().len());
        if rest.starts_with(' ') && end + 1 < all_ws {
            info.cuts.push((end + 1, k));
        }
        info.cuts.push((all_ws, k));
        start = all_ws;
    }
    if let Some(&(_, k)) = info.cuts.iter().find(|(c, _)| line[*c..].trim().is_empty()) {
        info.only = Some(k);
    } else if !line.trim().is_empty() && is_keyword_line(line.trim()) {
        info.only = Some(1);
        info.keyword = true;
    }
    info
}

/// The removed fragments for a cut with `k` fragments or a whole line.
fn removed_text<'a>(line: &'a str, info: &LineInfo<'a>, k: Option<usize>) -> Vec<&'a str> {
    match k {
        Some(k) => info.frags[..k].to_vec(),
        None if info.keyword => vec![line.trim()],
        None => info.frags.clone(),
    }
}

/// What happens to one line of the actual output.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Keep,
    /// Keep the rest from this byte offset; index into `cuts`.
    Cut(usize),
    Remove,
}

const INF: u32 = u32::MAX;

/// Builds the cleaned output: removed lines disappear with their line
/// break; a removed last line without line break leaves the line break of
/// the line before (the output the program would have written without it).
fn rebuild(lines: &[&str], ops: &[Op], infos: &[LineInfo<'_>]) -> String {
    let mut out = String::new();
    let last = lines.len().saturating_sub(1);
    for (i, line) in lines.iter().enumerate() {
        let text = match ops.get(i).copied().unwrap_or(Op::Keep) {
            Op::Keep => *line,
            Op::Cut(c) => &line[infos[i].cuts[c].0..],
            Op::Remove => continue,
        };
        out.push_str(text);
        if i < last {
            out.push('\n');
        }
    }
    out
}

/// Whether `actual` matches `expected` once at most `budget` prompt
/// fragments are removed from the actual output (see the module docs).
/// `None`: no tolerated reading matches, or nothing had to be removed (then
/// the strict comparison decides).
pub fn matches_ignoring_prompts(
    mode: &CompareMode,
    expected: &str,
    actual: &str,
    budget: usize,
) -> Option<PromptStrip> {
    if budget == 0 {
        return None;
    }
    let budget = budget.min(MAX_IGNORED_PROMPTS);
    let actual_n = normalize_newlines(actual);
    let lines: Vec<&str> = actual_n.split('\n').collect();
    if lines.iter().any(|l| l.len() > MAX_LINE_BYTES) {
        return None;
    }
    let infos: Vec<LineInfo<'_>> = lines.iter().map(|l| analyze(l)).collect();
    let (ops, cost) = match *mode {
        CompareMode::Lines { trim } => lines_path(expected, &lines, &infos, trim, budget)?,
        _ => tokens_path(mode, expected, &lines, &infos, budget)?,
    };
    if cost == 0 || cost as usize > budget {
        return None;
    }
    let cleaned = rebuild(&lines, &ops, &infos);
    if !mode.matches(expected, &cleaned) {
        return None;
    }
    let removed = ops
        .iter()
        .enumerate()
        .flat_map(|(i, op)| match *op {
            Op::Keep => Vec::new(),
            Op::Cut(c) => removed_text(lines[i], &infos[i], Some(infos[i].cuts[c].1)),
            Op::Remove => removed_text(lines[i], &infos[i], None),
        })
        .map(str::to_string)
        .collect();
    Some(PromptStrip { cleaned, removed })
}

/// DP for the line modes. State (i, j, t): actual line i, expected line j,
/// t = fragments already removed after the last non-empty expected line
/// (at most 1). Every actual line is kept, cut or removed; removed lines
/// cost at least 1, so `i - j <= budget` (band).
fn lines_path(
    expected: &str,
    lines: &[&str],
    infos: &[LineInfo<'_>],
    trim: bool,
    budget: usize,
) -> Option<(Vec<Op>, u32)> {
    let expected_n = normalize_newlines(expected);
    let e: Vec<&str> = expected_n.split('\n').collect();
    let blank = |s: &str| {
        if trim {
            s.trim_end().is_empty()
        } else {
            s.is_empty()
        }
    };
    let eq = |x: &str, y: &str| {
        if trim {
            x.trim_end() == y.trim_end()
        } else {
            x == y
        }
    };
    let last_content = |v: &[&str]| v.iter().rposition(|l| !blank(l)).map_or(0, |p| p + 1);
    let m_eff = last_content(&e);
    // With trim, trailing blank lines do not count (they are kept as they are).
    let (n, m) = if trim {
        (last_content(lines), m_eff)
    } else {
        (lines.len(), e.len())
    };
    // Non-trim: a removed last line without line break leaves an empty line.
    let open_end = !trim && lines.last().is_some_and(|l| !l.is_empty());
    if n < m || n > m + budget + 1 {
        return None;
    }
    let width = budget + 1;
    let states = (n + 1).checked_mul(width)?.checked_mul(2)?;
    if states > MAX_STATES {
        return None;
    }
    let idx = |i: usize, d: usize, t: usize| (i * width + d) * 2 + t;
    // choice: 0 none, 1 keep, 2 remove, 3 + c cut c.
    let mut cost = vec![INF; states];
    let mut choice = vec![0u16; states];
    for i in (0..=n).rev() {
        for d in 0..=budget.min(i) {
            let j = i - d;
            for t in 0..2 {
                let k_here = idx(i, d, t);
                if i == n {
                    if j == m {
                        cost[k_here] = 0;
                    }
                    continue;
                }
                let mut best = (INF, 0u16);
                let mut consider = |c: u32, ch: u16| {
                    if c < best.0 {
                        best = (c, ch);
                    }
                };
                let next = |d2: usize, t2: usize| cost[idx(i + 1, d2, t2)];
                // (1) keep.
                if j < m && eq(lines[i], e[j]) {
                    consider(next(d, t), 1);
                }
                // (2) cut the prompt prefix.
                if j < m {
                    for (c, &(off, k)) in infos[i].cuts.iter().enumerate() {
                        let rest = &lines[i][off..];
                        if !blank(rest) && eq(rest, e[j]) {
                            let n2 = next(d, t);
                            if n2 != INF {
                                consider(n2 + k as u32, 3 + c as u16);
                            }
                        }
                    }
                }
                // (3) remove a prompt line.
                if let Some(k) = infos[i].only {
                    let t2 = if j >= m_eff { t + k } else { t };
                    if t2 <= 1 {
                        if open_end && i + 1 == n {
                            // Replaced by the empty last line.
                            if j < m && e[j].is_empty() {
                                let n2 = next(d, t2);
                                if n2 != INF {
                                    consider(n2 + k as u32, 2);
                                }
                            }
                        } else if d < budget {
                            let n2 = next(d + 1, t2);
                            if n2 != INF {
                                consider(n2 + k as u32, 2);
                            }
                        }
                    }
                }
                cost[k_here] = best.0;
                choice[k_here] = best.1;
            }
        }
    }
    let total = cost[idx(0, 0, 0)];
    if total == INF {
        return None;
    }
    let mut ops = Vec::with_capacity(n);
    let (mut i, mut d, mut t) = (0, 0, 0);
    while i < n {
        let j = i - d;
        let ch = choice[idx(i, d, t)];
        match ch {
            1 => ops.push(Op::Keep),
            2 => {
                ops.push(Op::Remove);
                let k = infos[i].only.unwrap_or(1);
                if j >= m_eff {
                    t += k;
                }
                if !(open_end && i + 1 == n) {
                    d += 1;
                }
            }
            c => ops.push(Op::Cut(usize::from(c - 3))),
        }
        i += 1;
    }
    Some((ops, total))
}

/// DP for the token modes: the line structure of the expected output does
/// not matter, the actual lines anchor the prompt positions. State (i, j,
/// t): non-blank actual line i, j expected tokens consumed, t as above.
fn tokens_path(
    mode: &CompareMode,
    expected: &str,
    lines: &[&str],
    infos: &[LineInfo<'_>],
    budget: usize,
) -> Option<(Vec<Op>, u32)> {
    let e: Vec<&str> = expected.split_whitespace().collect();
    let rows: Vec<usize> = (0..lines.len())
        .filter(|&i| !lines[i].trim().is_empty())
        .collect();
    let (n, m) = (rows.len(), e.len());
    if n > m + budget {
        return None;
    }
    let states = (n + 1).checked_mul(m + 1)?.checked_mul(2)?;
    if states > MAX_STATES {
        return None;
    }
    let idx = |i: usize, j: usize, t: usize| (i * (m + 1) + j) * 2 + t;
    let fits = |rest: &str, j: usize| -> Option<usize> {
        let mut j2 = j;
        for tok in rest.split_whitespace() {
            if j2 >= m || !mode.token_matches(e[j2], tok) {
                return None;
            }
            j2 += 1;
        }
        Some(j2)
    };
    let mut cost = vec![INF; states];
    let mut choice = vec![0u16; states];
    // Where a keep/cut leads to (expected tokens consumed afterwards).
    let mut target = vec![0usize; states];
    for i in (0..=n).rev() {
        for j in 0..=m {
            for t in 0..2 {
                let here = idx(i, j, t);
                if i == n {
                    if j == m {
                        cost[here] = 0;
                    }
                    continue;
                }
                let line = lines[rows[i]];
                let info = &infos[rows[i]];
                let mut best = (INF, 0u16, 0usize);
                if let Some(j2) = fits(line, j) {
                    let c = cost[idx(i + 1, j2, t)];
                    if c < best.0 {
                        best = (c, 1, j2);
                    }
                }
                for (c, &(off, k)) in info.cuts.iter().enumerate() {
                    let rest = &line[off..];
                    if rest.trim().is_empty() {
                        continue;
                    }
                    if let Some(j2) = fits(rest, j) {
                        let n2 = cost[idx(i + 1, j2, t)];
                        if n2 != INF && n2 + (k as u32) < best.0 {
                            best = (n2 + k as u32, 3 + c as u16, j2);
                        }
                    }
                }
                if let Some(k) = info.only {
                    let t2 = if j >= m { t + k } else { t };
                    if t2 <= 1 {
                        let n2 = cost[idx(i + 1, j, t2)];
                        if n2 != INF && n2 + (k as u32) < best.0 {
                            best = (n2 + k as u32, 2, j);
                        }
                    }
                }
                cost[here] = best.0;
                choice[here] = best.1;
                target[here] = best.2;
            }
        }
    }
    let total = cost[idx(0, 0, 0)];
    if total == INF {
        return None;
    }
    let mut ops = vec![Op::Keep; lines.len()];
    let (mut j, mut t) = (0, 0);
    for (i, &row) in rows.iter().enumerate() {
        let here = idx(i, j, t);
        match choice[here] {
            1 => j = target[here],
            2 => {
                ops[row] = Op::Remove;
                if j >= m {
                    t += infos[row].only.unwrap_or(1);
                }
            }
            c => {
                ops[row] = Op::Cut(usize::from(c - 3));
                j = target[here];
            }
        }
    }
    Some((ops, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINES: CompareMode = CompareMode::Lines { trim: true };

    fn tol(expected: &str, actual: &str, input: &str) -> Option<PromptStrip> {
        tol_mode(&LINES, expected, actual, input)
    }

    fn tol_mode(
        mode: &CompareMode,
        expected: &str,
        actual: &str,
        input: &str,
    ) -> Option<PromptStrip> {
        assert!(
            !mode.matches(expected, actual),
            "the strict comparison must fail first"
        );
        matches_ignoring_prompts(mode, expected, actual, prompt_budget(input))
    }

    fn removed(s: &Option<PromptStrip>) -> Vec<&str> {
        s.as_ref()
            .unwrap()
            .removed
            .iter()
            .map(String::as_str)
            .collect()
    }

    #[test]
    fn budget_counts_tokens() {
        assert_eq!(prompt_budget(""), 0);
        assert_eq!(prompt_budget("  \n\n"), 0);
        assert_eq!(prompt_budget("5\n"), 2);
        assert_eq!(prompt_budget("3 4\n"), 3);
        assert_eq!(prompt_budget(&"1 ".repeat(500)), MAX_IGNORED_PROMPTS);
    }

    #[test]
    fn python_input_prompt_without_newline() {
        // input("Zahl eingeben: ") / print("Zahl eingeben: ", end="")
        let s = tol("25\n", "Zahl eingeben: 25\n", "5\n");
        assert_eq!(removed(&s), ["Zahl eingeben:"]);
        assert_eq!(s.unwrap().cleaned, "25\n");
        // input("Zahl:") without space.
        assert!(tol("25", "Zahl:25", "5").is_some());
    }

    #[test]
    fn java_print_prompts_before_scanner() {
        // System.out.print("Zahl 1: "); System.out.print("Zahl 2: ");
        let s = tol("5\n", "Zahl 1: Zahl 2: 5\n", "2\n3\n");
        assert_eq!(removed(&s), ["Zahl 1:", "Zahl 2:"]);
        let s = tol("Hallo Anna\n", "Name: Hallo Anna\n", "Anna\n");
        assert_eq!(removed(&s), ["Name:"]);
    }

    #[test]
    fn prompt_lines_of_their_own() {
        let s = tol(
            "Hallo Anna\n",
            "Bitte Namen eingeben:\nHallo Anna\n",
            "Anna\n",
        );
        assert_eq!(removed(&s), ["Bitte Namen eingeben:"]);
        assert_eq!(s.unwrap().cleaned, "Hallo Anna\n");
        let s = tol(
            "Hallo Anna\n",
            "Bitte geben Sie Ihren Namen ein\nHallo Anna\n",
            "Anna\n",
        );
        assert_eq!(removed(&s), ["Bitte geben Sie Ihren Namen ein"]);
        assert!(tol(
            "Hallo Anna\n",
            "Please enter your name\nHallo Anna\n",
            "Anna"
        )
        .is_some());
    }

    #[test]
    fn several_inputs_and_lines() {
        let s = tol(
            "Summe: 7\nProdukt: 12\n",
            "a: b: Summe: 7\nProdukt: 12\n",
            "3\n4\n",
        );
        assert_eq!(removed(&s), ["a:", "b:"]);
        let s = tol("7\n12\n", "Erste Zahl: \nZweite Zahl: 7\n12\n", "3\n4\n");
        assert_eq!(removed(&s), ["Erste Zahl:", "Zweite Zahl:"]);
    }

    #[test]
    fn result_labels_are_no_prompts() {
        // Expected with the label: the strict comparison passes.
        assert!(LINES.matches("Ergebnis: 42", "Ergebnis: 42\n"));
        // Expected without it: the label is not removed.
        assert_eq!(tol("42", "Ergebnis: 42", "6 7"), None);
        for label in ["Summe:", "Ausgabe:", "Result:", "Die Lösung:", "Error:"] {
            assert_eq!(tol("42", &format!("{label} 42"), "6 7"), None, "{label}");
        }
        // Budget 0 without input.
        assert_eq!(tol("42", "Wert: 42", ""), None);
        // Known loosening: an extra label ending with ":" passes for AI tests.
        assert_eq!(removed(&tol("42", "Wert: 42", "6 7")), ["Wert:"]);
    }

    #[test]
    fn wrong_value_after_prompt_fails() {
        assert_eq!(tol("25", "Zahl: 26", "5"), None);
        assert_eq!(tol("25\n36\n", "Zahl: 25\nZahl: 37\n", "5 6"), None);
        // Missing expected line.
        assert_eq!(
            tol("Wie heißt du?\nHallo Anna\n", "Hallo Anna\n", "Anna"),
            None
        );
        // An expected line that looks like a prompt stays.
        let s = tol(
            "Wie heißt du?\nHallo Anna\n",
            "Name: Wie heißt du?\nHallo Anna\n",
            "Anna",
        );
        assert_eq!(removed(&s), ["Name:"]);
    }

    #[test]
    fn limits_of_the_tolerance() {
        // Budget 2 for one token, 3 fragments.
        assert_eq!(tol("25", "A: B: C: 25", "5"), None);
        // At most one fragment after the last expected line.
        let s = tol("25\n", "Zahl: 25\nNochmal? ", "5\n");
        assert_eq!(s.unwrap().cleaned, "25\n");
        assert_eq!(tol("25\n", "Zahl: 25\nA?\nB?\n", "5 6 7"), None);
        // Times, mid-line prompts and result equations stay.
        assert_eq!(tol("30", "12:30", "x"), None);
        assert_eq!(tol("Summe: 5", "Summe: Zahl: 5", "2 3"), None);
        assert_eq!(tol("7", "a + b = 7", "3 4"), None);
        assert!(tol("7", "n = 7", "3").is_some());
        // Overlong lines are never tolerated.
        let long = format!("{}: 25", "x".repeat(MAX_LINE_BYTES));
        assert_eq!(tol("25", &long, "5"), None);
    }

    #[test]
    fn exact_lines_mode() {
        let exact = CompareMode::Lines { trim: false };
        let s = tol_mode(&exact, "25\n", "Zahl: 25\n", "5");
        assert_eq!(s.unwrap().cleaned, "25\n");
        // The final prompt is replaced by the empty last line.
        let s = tol_mode(&exact, "25\n", "Zahl: 25\nNochmal? ", "5");
        assert_eq!(s.unwrap().cleaned, "25\n");
        // The cleaned output must still match exactly.
        assert_eq!(tol_mode(&exact, "25", "Zahl: 25\n", "5"), None);
        // Leading spaces of the expected line are kept.
        assert!(tol_mode(&exact, "  25\n", "Zahl:   25\n", "5").is_some());
    }

    #[test]
    fn tokens_and_float_modes() {
        let tokens = CompareMode::Tokens {
            case_sensitive: true,
        };
        let s = tol_mode(&tokens, "3 4", "a: 3\nb: 4\n", "1 2");
        assert_eq!(removed(&s), ["a:", "b:"]);
        assert_eq!(tol_mode(&tokens, "3 4", "a: 3\nb: 5\n", "1 2"), None);
        assert_eq!(tol_mode(&tokens, "42", "Ergebnis: 42", "6 7"), None);
        let float = CompareMode::Float {
            abs_tol: 1e-6,
            rel_tol: 0.0,
        };
        let s = tol_mode(&float, "0.5", "x = 0.50000001", "1");
        assert_eq!(removed(&s), ["x ="]);
        assert_eq!(tol_mode(&float, "0.5", "x = 0.6", "1"), None);
        // Own prompt line in token mode.
        assert!(tol_mode(&float, "0.5", "Bitte Zahl eingeben\n0.5000000001\nx", "1").is_none());
        assert!(tol_mode(&float, "0.5", "Bitte Zahl eingeben\n0.50000001", "1").is_some());
    }
}
