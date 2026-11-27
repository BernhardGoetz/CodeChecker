//! Output templates: a task sheet may show the output format with
//! placeholder values ("12345678 seconds have passed since midnight." where
//! the sheet says 12345678 is only a placeholder). Such a template is not
//! an example – it only fixes the format. [`template_matches`] checks an
//! output against a template: the fixed text must match, every placeholder
//! stands for a non-empty value: an integer if the placeholder consists of
//! digits only (`12345678` → `54630`, not `54630.0`), a number if it is a
//! decimal number, any text otherwise. Templates the model split into one
//! template per line are joined again ([`verify_templates`]).
//!
//! Matching is done line by line (lines trimmed, whitespace runs collapsed,
//! empty lines at the start and end ignored) with a small backtracking
//! matcher, no regular expressions.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::compare::{is_number, normalize_for_match, normalize_newlines};

/// Upper bound for templates taken from one task.
pub const MAX_TEMPLATES: usize = 3;
/// Upper bound for placeholders of one template.
pub const MAX_PLACEHOLDERS: usize = 6;
/// Output lines longer than this never match a template (bounds the
/// matching time; templates are short lines of text).
pub const MAX_LINE_BYTES: usize = 4096;

/// What a placeholder stands for, derived from the token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceholderKind {
    /// Digits only (`12345678`): a whole number, optionally signed, without
    /// decimal point or exponent.
    Integer,
    /// Another number (`3.14`): any number (see [`is_number`]).
    Decimal,
    /// Anything else (`<n>`, `X`): any non-empty text.
    Text,
}

pub fn placeholder_kind(token: &str) -> PlaceholderKind {
    let digits = token.strip_prefix(['+', '-']).unwrap_or(token);
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        PlaceholderKind::Integer
    } else if is_number(token) {
        PlaceholderKind::Decimal
    } else {
        PlaceholderKind::Text
    }
}

fn is_integer(value: &str) -> bool {
    placeholder_kind(value) == PlaceholderKind::Integer
}

impl PlaceholderKind {
    /// Whether `value` may stand for a placeholder of this kind.
    fn accepts(self, value: &str) -> bool {
        match self {
            PlaceholderKind::Integer => is_integer(value),
            PlaceholderKind::Decimal => is_number(value),
            PlaceholderKind::Text => !value.is_empty(),
        }
    }

    /// Characters a value of this kind may consist of (`None`: any).
    fn allows(self, c: char) -> Option<bool> {
        match self {
            PlaceholderKind::Integer => Some(c.is_ascii_digit() || matches!(c, '+' | '-')),
            PlaceholderKind::Decimal => {
                Some(c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E'))
            }
            PlaceholderKind::Text => None,
        }
    }
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
pub fn contains_token(s: &str, token: &str) -> bool {
    s.char_indices().any(|(i, _)| token_at(s, i, token))
}

/// Occurrences considered per template when joining (bounds the search;
/// the last ones are kept: a final block wins over single lines before it).
const MAX_OCCURRENCES: usize = 16;
/// Accepted templates considered for joining (the rest stays unjoined).
const MAX_MERGE_CANDIDATES: usize = 8;

/// Result of [`verify_templates_with`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TemplateReport {
    /// The accepted templates in text order (at most [`MAX_TEMPLATES`]).
    pub templates: Vec<OutputTemplate>,
    /// Per template: whether it contains a template of the model (`false`:
    /// found only by [`detect_templates`]).
    pub from_model: Vec<bool>,
    /// Rejected by the checks (not verbatim, no placeholder, …) or cut off
    /// at [`MAX_TEMPLATES`].
    pub discarded: usize,
    /// Model templates joined into another one or subsumed by a longer
    /// template (e.g. three one-line templates that form one block).
    pub merged: usize,
}

/// Accepted templates and the number of discarded ones (see
/// [`verify_templates_with`]; without detected templates).
pub fn verify_templates(task_text: &str, raw: &[OutputTemplate]) -> (Vec<OutputTemplate>, usize) {
    let r = verify_templates_with(task_text, raw, &[]);
    (r.templates, r.discarded)
}

/// Checks the model's templates (`raw`) and the ones found without the
/// model (`detected`, see [`detect_templates`]) against the task text. A
/// template is discarded if its text is not verbatim in the task text (see
/// [`normalize_for_match`]), it has no placeholders, or a placeholder is
/// empty, contains whitespace or does not occur in the template text.
/// Duplicates are removed.
///
/// Templates that are whole, directly consecutive lines of the task text
/// are joined into one template in text order (the model sometimes splits
/// a template of several lines into one template per line; an output of
/// several lines would then never match). Every whole-line occurrence
/// counts, not only the first: a sheet often shows the lines one by one in
/// the subtasks and then together as the final block. A template whose
/// lines are part of a longer template is dropped (subsumed). At most
/// [`MAX_TEMPLATES`] remain (longest first), in text order.
pub fn verify_templates_with(
    task_text: &str,
    raw: &[OutputTemplate],
    detected: &[OutputTemplate],
) -> TemplateReport {
    let text = normalize_for_match(task_text);
    let (model, mut discarded) = accept_templates(&text, raw);
    let model_count = model.len();
    let (found, _) = accept_templates(&text, detected);
    let mut cands: Vec<(OutputTemplate, bool)> = model.into_iter().map(|t| (t, true)).collect();
    for t in found {
        if !cands.iter().any(|(k, _)| k.text == t.text) {
            cands.push((t, false));
        }
    }
    let placed = subsume(merge_chains(&text, cands));
    let before_cap = placed.len();
    let model_left = placed.iter().filter(|p| p.from_model).count();
    let mut placed = placed;
    if placed.len() > MAX_TEMPLATES {
        // Keep the longest, then the ones after an output intro, then the
        // later ones; afterwards text order again.
        placed.sort_by_key(|p| std::cmp::Reverse((p.t.text.lines().count(), p.intro, p.pos)));
        placed.truncate(MAX_TEMPLATES);
        placed.sort_by_key(|p| p.pos);
        discarded += before_cap - MAX_TEMPLATES;
    }
    TemplateReport {
        from_model: placed.iter().map(|p| p.from_model).collect(),
        templates: placed.into_iter().map(|p| p.t).collect(),
        discarded,
        merged: model_count.saturating_sub(model_left),
    }
}

/// A template with its position in the task text.
struct Placed {
    t: OutputTemplate,
    /// Byte offset in the normalized task text (sort key: text order).
    pos: usize,
    /// Directly after an output intro ("Die Ausgabe sollte so aussehen:").
    intro: bool,
    from_model: bool,
}

/// All 0-based start lines where `body` occurs as whole lines of the text.
fn line_starts(text_lines: &[&str], body_lines: &[&str]) -> Vec<usize> {
    if body_lines.is_empty() || body_lines.len() > text_lines.len() {
        return Vec::new();
    }
    (0..=text_lines.len() - body_lines.len())
        .filter(|&s| {
            body_lines
                .iter()
                .enumerate()
                .all(|(r, l)| text_lines[s + r] == *l)
        })
        .collect()
}

/// Whether two templates may be joined: no placeholder of one occurs as
/// fixed text in the other (joining would turn that text into a hole).
fn compatible(a: &OutputTemplate, b: &OutputTemplate) -> bool {
    let fixed_in = |x: &OutputTemplate, y: &OutputTemplate| {
        x.placeholders
            .iter()
            .any(|p| !y.placeholders.contains(p) && contains_token(&y.text, p))
    };
    !fixed_in(a, b) && !fixed_in(b, a)
}

/// Whether the line before `start` (an empty line or a code fence may be
/// in between) is an output intro.
fn intro_before(text_lines: &[&str], start: usize) -> bool {
    let mut i = start;
    while i > 0 && start - i < 2 {
        i -= 1;
        let l = text_lines[i];
        if l.is_empty() || l.starts_with("```") {
            continue;
        }
        return is_output_intro(l);
    }
    false
}

/// Joins templates on directly consecutive lines of the task text (longest
/// chains first, every whole-line occurrence considered); returns all
/// templates in text order.
fn merge_chains(text: &str, cands: Vec<(OutputTemplate, bool)>) -> Vec<Placed> {
    let text_lines: Vec<&str> = text.split('\n').collect();
    // Byte offset of every line.
    let mut line_pos = Vec::with_capacity(text_lines.len());
    let mut p = 0;
    for l in &text_lines {
        line_pos.push(p);
        p += l.len() + 1;
    }
    let starts: Vec<Vec<usize>> = cands
        .iter()
        .enumerate()
        .map(|(i, (t, _))| {
            if i >= MAX_MERGE_CANDIDATES {
                return Vec::new();
            }
            let body: Vec<&str> = t.text.split('\n').collect();
            let mut s = line_starts(&text_lines, &body);
            if s.len() > MAX_OCCURRENCES {
                s.drain(..s.len() - MAX_OCCURRENCES);
            }
            s
        })
        .collect();
    let search = Search {
        starts: &starts,
        heights: cands
            .iter()
            .map(|(t, _)| t.text.split('\n').count())
            .collect(),
        cands: &cands,
    };
    let mut remaining: Vec<bool> = vec![true; cands.len()];
    let mut out: Vec<Placed> = Vec::new();
    loop {
        let mut memo: HashMap<(usize, u32), Vec<usize>> = HashMap::new();
        // (templates, lines, intro, start) – the largest key wins.
        type Key = (usize, usize, bool, usize);
        let mut best: Option<(Key, Vec<usize>)> = None;
        for (i, ss) in starts.iter().enumerate() {
            if !remaining[i] {
                continue;
            }
            for &s in ss {
                let chain = search.extend(s, 0, &remaining, &mut memo);
                let lines: usize = chain.iter().map(|&k| search.heights[k]).sum();
                let key = (chain.len(), lines, intro_before(&text_lines, s), s);
                if best.as_ref().is_none_or(|(b, _)| key > *b) {
                    best = Some((key, chain));
                }
            }
        }
        let Some(((len, _, intro, start), chain)) = best else {
            break;
        };
        if len < 2 {
            break;
        }
        let mut t = OutputTemplate::default();
        let mut from_model = false;
        for &k in &chain {
            remaining[k] = false;
            from_model |= cands[k].1;
            let c = &cands[k].0;
            t.text = if t.text.is_empty() {
                c.text.clone()
            } else {
                format!("{}\n{}", t.text, c.text)
            };
            for ph in &c.placeholders {
                if !t.placeholders.contains(ph) {
                    t.placeholders.push(ph.clone());
                }
            }
        }
        out.push(Placed {
            t,
            pos: line_pos[start],
            intro,
            from_model,
        });
    }
    for (i, (t, from_model)) in cands.iter().enumerate() {
        if !remaining[i] {
            continue;
        }
        let (pos, intro) = match starts[i].first() {
            Some(&s) => (line_pos[s], intro_before(&text_lines, s)),
            None => (text.find(&t.text).unwrap_or(text.len()), false),
        };
        out.push(Placed {
            t: t.clone(),
            pos,
            intro,
            from_model: *from_model,
        });
    }
    out.sort_by_key(|p| p.pos);
    out
}

/// Chain search over the whole-line occurrences of the templates.
struct Search<'a> {
    starts: &'a [Vec<usize>],
    heights: Vec<usize>,
    cands: &'a [(OutputTemplate, bool)],
}

impl Search<'_> {
    /// Longest chain of remaining templates starting at `line`
    /// (depth-first, memoized over (line, used set)); every template at
    /// most once, all members pairwise [`compatible`].
    fn extend(
        &self,
        line: usize,
        used: u32,
        remaining: &[bool],
        memo: &mut HashMap<(usize, u32), Vec<usize>>,
    ) -> Vec<usize> {
        if let Some(c) = memo.get(&(line, used)) {
            return c.clone();
        }
        let n = self.starts.len().min(MAX_MERGE_CANDIDATES);
        let mut best: Vec<usize> = Vec::new();
        let mut best_lines = 0;
        for i in 0..n {
            if !remaining[i] || used & (1 << i) != 0 || !self.starts[i].contains(&line) {
                continue;
            }
            let fits = (0..n)
                .filter(|k| used & (1 << k) != 0)
                .all(|k| compatible(&self.cands[k].0, &self.cands[i].0));
            if !fits {
                continue;
            }
            let mut chain = vec![i];
            chain.extend(self.extend(line + self.heights[i], used | (1 << i), remaining, memo));
            let lines: usize = chain.iter().map(|&k| self.heights[k]).sum();
            if (chain.len(), lines) > (best.len(), best_lines) {
                best_lines = lines;
                best = chain;
            }
        }
        memo.insert((line, used), best.clone());
        best
    }
}

/// Whether the lines of `small` map in order onto consecutive lines of
/// `big`, each equal to the line of `big` or contained in it at word
/// boundaries.
fn subsumes(big: &OutputTemplate, small: &OutputTemplate) -> bool {
    if big.text == small.text {
        return false;
    }
    let b: Vec<&str> = big.text.split('\n').collect();
    let s: Vec<&str> = small.text.split('\n').collect();
    if s.len() > b.len() {
        return false;
    }
    (0..=b.len() - s.len()).any(|o| {
        s.iter()
            .enumerate()
            .all(|(r, l)| b[o + r] == *l || contains_token(b[o + r], l))
    })
}

/// Drops templates that are part of a longer one or equal to another one
/// (e.g. a joined chain equal to a block of the model); their placeholders
/// go to the remaining one where [`compatible`].
fn subsume(mut placed: Vec<Placed>) -> Vec<Placed> {
    let mut i = 0;
    while i < placed.len() {
        let big = (0..placed.len()).find(|&k| {
            k != i && (placed[k].t.text == placed[i].t.text || subsumes(&placed[k].t, &placed[i].t))
        });
        match big {
            Some(k) => {
                let small = placed.remove(i);
                let k = if k > i { k - 1 } else { k };
                let b = &mut placed[k];
                b.from_model |= small.from_model;
                if compatible(&b.t, &small.t) {
                    for p in small.t.placeholders {
                        if !b.t.placeholders.contains(&p) && contains_token(&b.t.text, &p) {
                            b.t.placeholders.push(p);
                        }
                    }
                }
                i = 0;
            }
            None => i += 1,
        }
    }
    placed
}

/// The templates that pass the checks of [`verify_templates`] (not joined,
/// not limited) and the number discarded.
fn accept_templates(text: &str, raw: &[OutputTemplate]) -> (Vec<OutputTemplate>, usize) {
    let mut kept: Vec<OutputTemplate> = Vec::new();
    let mut discarded = 0;
    let declared: Vec<String> = {
        let lines: Vec<&str> = text.split('\n').collect();
        declared_placeholders(&lines)
            .into_iter()
            .map(|d| d.token)
            .collect()
    };
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
            })
            && !example_sentence(text, &body, &tokens, &declared);
        if !ok {
            discarded += 1;
            continue;
        }
        if !kept.iter().any(|k| k.text == body) {
            kept.push(OutputTemplate {
                text: body,
                placeholders: tokens,
            });
        }
    }
    (kept, discarded)
}

/// Phrases that introduce an example value in prose („z. B. “Your current
/// balance is €100.0”“).
const EXAMPLE_INTROS: &[&str] = &[
    "z. b.",
    "z.b.",
    "zb.",
    "e.g.",
    "e. g.",
    "zum beispiel",
    "for example",
    "beispielsweise",
    "bspw.",
];

/// Characters between an example intro and the example (quotes, colon).
const EXAMPLE_GAP: &[char] = &[
    ' ', ':', '"', '\'', '`', '„', '“', '”', '‚', '‘', '’', '»', '«', '(',
];

/// Whether `body` is only an example sentence of the task text: every
/// occurrence directly follows an example intro ([`EXAMPLE_INTROS`]) and no
/// placeholder is declared (by the sheet, see [`declared_placeholders`], or
/// by its form, see [`syntactic_placeholders`]). Such a sentence („z.B.
/// “Your current balance is €100.0”“) shows one possible output with real
/// values; the model's "placeholder" `100.0` is a guess (regression
/// „Aufgabe 6.2 BankAccount“).
fn example_sentence(text: &str, body: &str, tokens: &[String], declared: &[String]) -> bool {
    let explicit = tokens.iter().any(|t| declared.contains(t))
        || body
            .split('\n')
            .any(|l| tokens.iter().any(|t| syntactic_placeholders(l).contains(t)));
    if explicit {
        return false;
    }
    let mut seen = false;
    for (p, _) in text.match_indices(body) {
        seen = true;
        let before = text[..p].trim_end_matches(EXAMPLE_GAP).to_lowercase();
        if !EXAMPLE_INTROS.iter().any(|i| before.ends_with(i)) {
            return false;
        }
    }
    seen
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
    /// A placeholder of the given kind.
    Hole {
        kind: PlaceholderKind,
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
                kind: placeholder_kind(tok),
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
        Segment::Hole { kind } => {
            let rest = &s[pos..];
            // A number value only extends over number characters.
            let limit = pos
                + rest
                    .find(|c: char| kind.allows(c) == Some(false))
                    .unwrap_or(rest.len());
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
                    kind.accepts(&s[pos..end]) && match_from(segs, i + 1, s, end, failed)
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
    line_matches_with(template_line, tokens, line, false)
}

/// `relax_integers`: integer placeholders accept any number (to tell the
/// user that only the decimal places are wrong).
fn line_matches_with(
    template_line: &str,
    tokens: &[&str],
    line: &str,
    relax_integers: bool,
) -> bool {
    if line.len() > MAX_LINE_BYTES {
        return false;
    }
    let mut segs = segments(template_line, tokens);
    if relax_integers {
        for s in &mut segs {
            if let Segment::Hole { kind } = s {
                if *kind == PlaceholderKind::Integer {
                    *kind = PlaceholderKind::Decimal;
                }
            }
        }
    }
    // Every hole takes at least one byte.
    let min_len: usize = segs
        .iter()
        .map(|s| match s {
            Segment::Text(t) => t.len(),
            Segment::Hole { .. } => 1,
        })
        .sum();
    // Cheap pre-check: the fixed texts occur in this order.
    let mut from = 0;
    for seg in &segs {
        if let Segment::Text(t) = seg {
            match line[from..].find(t.as_str()) {
                Some(k) => from += k + t.len(),
                None => return false,
            }
        }
    }
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

/// Whether `output` is an ordered sequence of distinct templates: its lines
/// split into consecutive sections, each matching another template, in
/// the stored order (= text order); templates may be left out, none is
/// repeated, empty lines between two sections are allowed. `Err` carries
/// the number of output lines covered before the match got stuck.
fn sequence_matches(templates: &[OutputTemplate], output: &str) -> Result<(), usize> {
    let out = lines(output);
    let n = out.len();
    if n == 0 {
        return Err(0);
    }
    let tl: Vec<Vec<String>> = templates.iter().map(|t| lines(&t.text)).collect();
    let tokens: Vec<Vec<&str>> = templates.iter().map(sorted_tokens).collect();
    let mut reach = vec![false; n + 1];
    reach[0] = true;
    let mut furthest = 0;
    for (k, t) in tl.iter().enumerate() {
        let mut next = reach.clone(); // template k left out
        for i in (0..=n).filter(|&i| reach[i]) {
            let mut i2 = i;
            if i > 0 {
                while i2 < n && out[i2].is_empty() {
                    i2 += 1;
                }
            }
            let h = t.len();
            if h > 0 && i2 + h <= n && (0..h).all(|r| line_matches(&t[r], &tokens[k], &out[i2 + r]))
            {
                next[i2 + h] = true;
                furthest = furthest.max(i2 + h);
            }
        }
        reach = next;
    }
    if reach[n] {
        Ok(())
    } else {
        Err(furthest)
    }
}

/// German error message if `output` matches none of the templates (`None`
/// if there are no templates, one matches, or the output is an ordered
/// sequence of several of them – see [`sequence_matches`]). Names the line
/// and the expected template line of the first template.
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
    if templates.len() > 1 {
        match sequence_matches(templates, output) {
            Ok(()) => return None,
            Err(reach) if reach > 0 => {
                let actual = lines(output);
                // Only the decimal places are wrong in the next line?
                let decimals_only = actual.get(reach).is_some_and(|a| {
                    templates.iter().any(|t| {
                        lines(&t.text)
                            .first()
                            .is_some_and(|e| line_matches_with(e, &sorted_tokens(t), a, true))
                    })
                });
                let hint = if decimals_only {
                    " Ganzzahl-Platzhalter erwarten Ganzzahlen ohne Nachkommastellen."
                } else {
                    ""
                };
                return Some(format!(
                    "Ausgabeformat weicht von den Vorlagen des Aufgabenblatts ab: Zeile {} passt \
                     zu keiner weiteren Vorlage.{hint}",
                    reach + 1
                ));
            }
            Err(_) => {}
        }
    }
    let expected = lines(&t.text);
    let actual = lines(output);
    let integers: Vec<String> = t
        .placeholders
        .iter()
        .filter(|p| placeholder_kind(p) == PlaceholderKind::Integer)
        .map(|p| format!("„{p}“"))
        .collect();
    // Only the decimal places are wrong (54630.0 for an integer placeholder)?
    let decimals_only = !integers.is_empty()
        && match (expected.get(line - 1), actual.get(line - 1)) {
            (Some(e), Some(a)) => line_matches_with(e, &sorted_tokens(t), a, true),
            _ => false,
        };
    let integer_hint = if decimals_only {
        format!(
            " Platzhalter {} erwartet eine Ganzzahl ohne Nachkommastellen.",
            integers.join(", ")
        )
    } else {
        String::new()
    };
    let placeholders = t
        .placeholders
        .iter()
        .map(|p| format!("„{p}“"))
        .collect::<Vec<_>>()
        .join(", ");
    Some(match expected.get(line - 1) {
        Some(e) => format!(
            "Ausgabeformat weicht von der Vorlage des Aufgabenblatts ab: Zeile {line} passt nicht \
             zu „{e}“ (Platzhalter: {placeholders}).{integer_hint}"
        ),
        None => format!(
            "Ausgabeformat weicht von der Vorlage des Aufgabenblatts ab: Zeile {line} ist \
             überzählig – die Vorlage hat nur {} Zeile(n).",
            expected.len()
        ),
    })
}

// --- Detection without the model ---------------------------------------------

/// Longest line of a detected template.
const MAX_DETECTED_LINE_CHARS: usize = 200;
/// Longest block of a detected template.
const MAX_DETECTED_LINES: usize = 12;

fn lower_words(line: &str) -> Vec<String> {
    line.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | ',' | ';' | ':' | '.'))
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn has_output_stem(line: &str) -> bool {
    lower_words(line).iter().any(|w| {
        w.contains("ausgabe")
            || w.contains("ausgeb")
            || w.starts_with("aussehen")
            || w.starts_with("output")
            || w.starts_with("print")
            || w == "gib"
    })
}

fn has_input_stem(line: &str) -> bool {
    lower_words(line).iter().any(|w| {
        w.contains("eingabe")
            || w.starts_with("input")
            || w.starts_with("einles")
            || w.starts_with("eingeles")
            || w == "lies"
            || w.starts_with("read")
            || w == "ein-"
    })
}

/// "Die Ausgabe sollte folgendermaßen aussehen:", "Beispielausgabe:",
/// "Output:" – a line ending with a colon about the output, not the input
/// ("Ein- und Ausgabeformat:" is neither).
pub fn is_output_intro(line: &str) -> bool {
    line.trim_end().ends_with(':') && has_output_stem(line) && !has_input_stem(line)
}

/// Characters that end a word around a token (not part of the token).
fn trim_token(w: &str) -> &str {
    w.trim_matches(|c: char| {
        matches!(
            c,
            '(' | ')'
                | ','
                | ';'
                | ':'
                | '.'
                | '!'
                | '?'
                | '"'
                | '\''
                | '`'
                | '„'
                | '“'
                | '”'
                | '‚'
                | '‘'
                | '’'
                | '»'
                | '«'
        )
    })
}

/// A token that the sheet explicitly calls a placeholder.
#[derive(Debug, Clone, PartialEq)]
struct Declared {
    token: String,
    /// Line of the declaration (normalized text, 0-based).
    line: usize,
    /// Short number or single letter: only near the declaration or after
    /// an output intro.
    weak: bool,
}

/// Whether an unquoted word may be a placeholder token: it contains a
/// digit or another non-letter, or it is at most three capital letters or
/// a single letter ("Die Zahl ist ein Platzhalter" yields nothing).
fn plausible_token(t: &str, quoted: bool) -> bool {
    let n = t.chars().count();
    if n == 0 || n > 20 || t.chars().any(char::is_whitespace) {
        return false;
    }
    if quoted {
        return true;
    }
    t.chars().any(|c| !c.is_alphabetic())
        || (n <= 3 && t.chars().all(|c| c.is_uppercase()))
        || (n == 1 && t.chars().all(|c| c.is_lowercase()))
}

fn is_weak(t: &str) -> bool {
    let digits = t.chars().filter(char::is_ascii_digit).count();
    (digits > 0 && digits == t.chars().count() && digits < 4) || t.chars().count() == 1
}

/// Sentences of a line (split after `.`, `!`, `?` followed by whitespace).
fn sentences(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let b = line.as_bytes();
    for (i, c) in line.char_indices() {
        if matches!(c, '.' | '!' | '?') && b.get(i + 1).is_some_and(|n| n.is_ascii_whitespace()) {
            out.push(&line[start..=i]);
            start = i + 1;
        }
    }
    out.push(&line[start..]);
    out
}

/// Tokens in quotes or backticks.
fn quoted_tokens(s: &str) -> Vec<String> {
    let pairs = [
        ('„', '“'),
        ('"', '"'),
        ('\'', '\''),
        ('`', '`'),
        ('“', '”'),
        ('‚', '‘'),
        ('»', '«'),
    ];
    let mut out = Vec::new();
    for (open, close) in pairs {
        let mut rest = s;
        while let Some(a) = rest.find(open) {
            let after = &rest[a + open.len_utf8()..];
            let Some(b) = after.find(close) else { break };
            out.push(after[..b].to_string());
            rest = &after[b + close.len_utf8()..];
        }
    }
    out
}

/// Tokens the sheet explicitly declares as placeholders ("12345678 ist nur
/// ein Platzhalter", "N is a placeholder", "Platzhalter „XX“").
fn declared_placeholders(text_lines: &[&str]) -> Vec<Declared> {
    const VERBS: [&str; 6] = ["ist", "sind", "dient", "is", "are", "serves"];
    const FILLERS: [&str; 14] = [
        "nur",
        "lediglich",
        "hier",
        "dabei",
        "just",
        "only",
        "merely",
        "ein",
        "eine",
        "a",
        "als",
        "as",
        "jeweils",
        "bloß",
    ];
    const CONJ: [&str; 4] = ["und", "and", "oder", "or"];
    let mut out: Vec<Declared> = Vec::new();
    for (ln, line) in text_lines.iter().enumerate() {
        for sentence in sentences(line) {
            let lower = sentence.to_lowercase();
            if !(lower.contains("platzhalter") || lower.contains("placeholder"))
                || sentence.chars().count() > 160
            {
                continue;
            }
            let words: Vec<&str> = sentence.split_whitespace().collect();
            let lw: Vec<String> = words.iter().map(|w| trim_token(w).to_lowercase()).collect();
            if lw
                .iter()
                .any(|w| matches!(w.as_str(), "kein" | "keine" | "keinen" | "no"))
                || lower.contains("not a")
            {
                continue;
            }
            let mut found: Vec<(String, bool)> = quoted_tokens(sentence)
                .into_iter()
                .map(|t| (t, true))
                .collect();
            for (k, w) in lw.iter().enumerate() {
                if !(w.starts_with("platzhalter") || w.starts_with("placeholder")) {
                    continue;
                }
                // TOKEN VERB FILLER* KEYWORD, more tokens before via "und"/",".
                let mut j = k;
                while j > 0 && FILLERS.contains(&lw[j - 1].as_str()) {
                    j -= 1;
                }
                if j >= 2 && VERBS.contains(&lw[j - 1].as_str()) {
                    let mut t = j - 2;
                    let mut taken = 0;
                    loop {
                        found.push((trim_token(words[t]).to_string(), false));
                        taken += 1;
                        if taken >= 4 || t == 0 {
                            break;
                        }
                        if CONJ.contains(&lw[t - 1].as_str()) && t >= 2 {
                            t -= 2;
                        } else if words[t - 1].ends_with(',') {
                            t -= 1;
                        } else {
                            break;
                        }
                    }
                }
                // KEYWORD TOKEN ("Platzhalter XX").
                if let Some(next) = words.get(k + 1) {
                    let n = trim_token(next);
                    let nl = n.to_lowercase();
                    if !matches!(nl.as_str(), "für" | "for" | "ist" | "is" | "sind" | "are")
                        && !words[k].ends_with([':', ')', ','])
                    {
                        found.push((n.to_string(), false));
                    }
                }
            }
            for (t, quoted) in found {
                let t = t.trim().to_string();
                let tl = t.to_lowercase();
                if tl.starts_with("platzhalter")
                    || tl.starts_with("placeholder")
                    || !plausible_token(&t, quoted)
                    || out.iter().any(|d| d.token == t)
                {
                    continue;
                }
                out.push(Declared {
                    weak: is_weak(&t),
                    token: t,
                    line: ln,
                });
            }
        }
    }
    out
}

/// Placeholder tokens that can never be real values: `<n>`, `___`, `XX.XX`,
/// `[Name]`.
fn syntactic_placeholders(line: &str) -> Vec<String> {
    const HTML: [&str; 7] = ["br", "p", "b", "i", "u", "div", "span"];
    let mut out: Vec<String> = Vec::new();
    let mut push = |t: &str| {
        if !out.iter().any(|o| o == t) {
            out.push(t.to_string());
        }
    };
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    for (ci, &(i, c)) in chars.iter().enumerate() {
        let before_word = ci > 0 && is_word_char(chars[ci - 1].1);
        match c {
            '<' | '[' if !before_word => {
                let close = if c == '<' { '>' } else { ']' };
                let rest = &line[i + 1..];
                if let Some(e) = rest.find(close) {
                    let inner = &rest[..e];
                    let n = inner.chars().count();
                    let ok = if c == '<' {
                        (1..=20).contains(&n)
                            && inner
                                .chars()
                                .all(|x| x.is_alphanumeric() || x == '_' || x == '-')
                            && !HTML.contains(&inner.to_lowercase().as_str())
                    } else {
                        (1..=20).contains(&n) && inner.chars().all(char::is_alphabetic)
                    };
                    if ok {
                        push(&line[i..i + 1 + e + 1]);
                    }
                }
            }
            '_' if ci == 0 || chars[ci - 1].1 != '_' => {
                let run = chars[ci..].iter().take_while(|x| x.1 == '_').count();
                let before_ok = ci == 0 || !is_word_char(chars[ci - 1].1);
                let after_ok = chars.get(ci + run).is_none_or(|x| !is_word_char(x.1));
                if run >= 3 && before_ok && after_ok {
                    push(&"_".repeat(run));
                }
            }
            _ => {}
        }
    }
    // X runs as whole words: "XX", "XX.XX", "xx:xx" (not "XXL").
    for w in line.split_whitespace() {
        let t = w.trim_end_matches(['.', ',', ';', '!', '?', ')']);
        let t = t.trim_start_matches('(');
        let xs = t.chars().filter(|&c| c == 'X' || c == 'x').count();
        let one_case = t.chars().all(|c| matches!(c, 'X' | '.' | ':' | ','))
            || t.chars().all(|c| matches!(c, 'x' | '.' | ':' | ','));
        if xs >= 2
            && one_case
            && t.starts_with(['X', 'x'])
            && t.ends_with(['X', 'x'])
            && t.contains("XX") | t.contains("xx")
        {
            push(t);
        }
    }
    out
}

fn is_ellipsis_line(l: &str) -> bool {
    let t = l.trim();
    t == "..." || t == "…" || t == "⋮"
}

/// Prose rather than output: explanations of the placeholder or long
/// sentences.
fn is_prose(line: &str) -> bool {
    let l = line.to_lowercase();
    l.contains("platzhalter")
        || l.contains("placeholder")
        || l.contains("ersetze")
        || l.contains("replace")
        || l.contains("steht für")
        || l.contains("stands for")
        || line.split_whitespace().count() > 25
}

/// Builds a template from a block of lines and the tokens found in it.
fn block_template(lines: &[&str], tokens: &[String]) -> Option<OutputTemplate> {
    if lines.is_empty()
        || lines.iter().any(|l| is_ellipsis_line(l))
        || lines
            .iter()
            .any(|l| l.chars().count() > MAX_DETECTED_LINE_CHARS)
    {
        return None;
    }
    let text = lines.join("\n");
    let placeholders: Vec<String> = tokens
        .iter()
        .filter(|t| contains_token(&text, t))
        .cloned()
        .collect();
    (!placeholders.is_empty() && placeholders.len() <= MAX_PLACEHOLDERS)
        .then_some(OutputTemplate { text, placeholders })
}

/// Tokens the task text explicitly declares as placeholders ("12345678
/// ist nur ein Platzhalter").
pub fn declared_placeholder_tokens(task_text: &str) -> Vec<String> {
    let text = normalize_for_match(task_text);
    let lines: Vec<&str> = text.split('\n').collect();
    declared_placeholders(&lines)
        .into_iter()
        .map(|d| d.token)
        .collect()
}

/// Output templates found in the task text without the model, by two
/// rules:
///
/// - R1: a token the sheet explicitly declares as a placeholder ("12345678
///   ist ein Platzhalter"); every block of consecutive lines containing it
///   (not the declaration itself, not prose) becomes a template. A plain
///   number is never a placeholder by its pattern alone.
/// - R2: syntactic placeholders (`<n>`, `___`, `XX.XX`, `[Name]`) in the
///   block right after an output intro ("Ausgabe:") or inline after such a
///   label ("Ausgabe: Ergebnis: ___").
///
/// A block with an ellipsis line ("...") is discarded (lines are missing).
/// The result still has to pass [`verify_templates_with`].
pub fn detect_templates(task_text: &str) -> Vec<OutputTemplate> {
    let text = normalize_for_match(task_text);
    let lines: Vec<&str> = text.split('\n').collect();
    let declared = declared_placeholders(&lines);
    let mut found: Vec<OutputTemplate> = Vec::new();
    let mut add = |t: OutputTemplate| {
        if !found.iter().any(|f| f.text == t.text) {
            found.push(t);
        }
    };

    // Blocks right after an output intro (R1 and R2 tokens).
    let mut in_intro_block = vec![false; lines.len()];
    for (i, l) in lines.iter().enumerate() {
        if !is_output_intro(l) {
            continue;
        }
        let mut s = i + 1;
        while s < lines.len() && s <= i + 2 && (lines[s].is_empty() || lines[s].starts_with("```"))
        {
            s += 1;
        }
        let mut e = s;
        while e < lines.len()
            && e - s < MAX_DETECTED_LINES
            && !lines[e].is_empty()
            && !lines[e].starts_with("```")
            && !is_output_intro(lines[e])
            && !is_prose(lines[e])
        {
            e += 1;
        }
        let tok_of = |l: &str| -> Vec<String> {
            let mut t = syntactic_placeholders(l);
            for d in &declared {
                if contains_token(l, &d.token) && !t.contains(&d.token) {
                    t.push(d.token.clone());
                }
            }
            t
        };
        // Trailing lines without a token do not belong to the template.
        while e > s && tok_of(lines[e - 1]).is_empty() {
            e -= 1;
        }
        if e == s {
            continue;
        }
        let block = &lines[s..e];
        let mut tokens: Vec<String> = Vec::new();
        for l in block {
            for t in tok_of(l) {
                if !tokens.contains(&t) {
                    tokens.push(t);
                }
            }
        }
        for flag in &mut in_intro_block[s..e] {
            *flag = true;
        }
        if let Some(t) = block_template(block, &tokens) {
            add(t);
        }
    }

    // Inline form: "Ausgabe: Ergebnis: ___".
    for l in &lines {
        let Some(colon) = l.find(": ") else { continue };
        let (label, rest) = (&l[..colon], l[colon + 2..].trim());
        if label.split_whitespace().count() > 3
            || !has_output_stem(label)
            || has_input_stem(label)
            || is_prose(l)
        {
            continue;
        }
        let tokens = syntactic_placeholders(rest);
        if let Some(t) = block_template(&[rest], &tokens) {
            add(t);
        }
    }

    // R1: blocks of lines with a declared token anywhere in the text.
    let mut i = 0;
    while i < lines.len() {
        let usable = |k: usize| -> Vec<String> {
            if in_intro_block[k] || lines[k].is_empty() || is_prose(lines[k]) {
                return Vec::new();
            }
            declared
                .iter()
                .filter(|d| contains_token(lines[k], &d.token))
                .filter(|d| !d.weak || k.abs_diff(d.line) <= 5)
                .map(|d| d.token.clone())
                .collect()
        };
        if usable(i).is_empty() {
            i += 1;
            continue;
        }
        let s = i;
        let mut tokens: Vec<String> = Vec::new();
        while i < lines.len() && i - s < MAX_DETECTED_LINES {
            let t = usable(i);
            if t.is_empty() {
                break;
            }
            for x in t {
                if !tokens.contains(&x) {
                    tokens.push(x);
                }
            }
            i += 1;
        }
        if let Some(t) = block_template(&lines[s..i], &tokens) {
            add(t);
        }
    }

    // Blocks that are part of a longer detected block add nothing.
    let all = found.clone();
    found.retain(|t| !all.iter().any(|b| subsumes(b, t)));
    found
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
        // Decimal placeholder: negative and decimal numbers are numbers.
        let tpl = t("Ergebnis: 0.0", &["0.0"]);
        assert_eq!(template_matches(&tpl, "Ergebnis: -2.5"), Ok(()));
        assert_eq!(template_matches(&tpl, "Ergebnis: 3"), Ok(()));
        assert_eq!(template_matches(&tpl, "Ergebnis: zwei"), Err(1));
        // Integer placeholder: whole numbers only.
        let tpl = t("Ergebnis: 0", &["0"]);
        assert_eq!(template_matches(&tpl, "Ergebnis: -25"), Ok(()));
        assert_eq!(template_matches(&tpl, "Ergebnis: -2.5"), Err(1));
        assert_eq!(template_matches(&tpl, "Ergebnis: 2e3"), Err(1));
    }

    #[test]
    fn placeholder_kinds() {
        assert_eq!(placeholder_kind("12345678"), PlaceholderKind::Integer);
        assert_eq!(placeholder_kind("-1"), PlaceholderKind::Integer);
        assert_eq!(placeholder_kind("3.14"), PlaceholderKind::Decimal);
        assert_eq!(placeholder_kind("1e5"), PlaceholderKind::Decimal);
        assert_eq!(placeholder_kind("<n>"), PlaceholderKind::Text);
        assert_eq!(placeholder_kind("X"), PlaceholderKind::Text);
        // 54630.0 does not fit an integer placeholder; the message says why.
        let out = "54630.0 seconds have passed since midnight.\n31770 seconds are left until \
                   midnight.\nSince I started with the task 441 seconds have passed.";
        assert_eq!(template_matches(&sheet_template(), out), Err(1));
        let msg = check_output(&[sheet_template()], out).unwrap();
        assert!(
            msg.contains("„12345678“ erwartet eine Ganzzahl ohne Nachkommastellen"),
            "{msg}"
        );
        // Other errors get no integer hint.
        let msg = check_output(&[sheet_template()], "x\ny\nz").unwrap();
        assert!(!msg.contains("Ganzzahl"), "{msg}");
    }

    #[test]
    fn split_templates_are_joined() {
        let lines: Vec<OutputTemplate> = sheet_template()
            .text
            .lines()
            .map(|l| t(l, &["12345678"]))
            .collect();
        // Given in any order: joined in text order.
        let shuffled = vec![lines[2].clone(), lines[0].clone(), lines[1].clone()];
        let (kept, discarded) = verify_templates(SHEET, &shuffled);
        assert_eq!(discarded, 0);
        assert_eq!(kept, vec![sheet_template()]);
        let out = "54630 seconds have passed since midnight.\n31770 seconds are left until \
                   midnight.\nSince I started with the task 441 seconds have passed.\n";
        assert_eq!(check_output(&kept, out), None);
        // Lines that are not consecutive (an empty line between) stay apart.
        let text = "A: 1\n\nB: 2\nC: 3\n";
        let (kept, _) = verify_templates(
            text,
            &[t("A: 1", &["1"]), t("B: 2", &["2"]), t("C: 3", &["3"])],
        );
        assert_eq!(kept, vec![t("A: 1", &["1"]), t("B: 2\nC: 3", &["2", "3"])]);
        // Parts of a line are not joined.
        let (kept, _) = verify_templates(
            "Wert 1 und 2\nSumme 3\n",
            &[t("Wert 1", &["1"]), t("Summe 3", &["3"])],
        );
        assert_eq!(kept.len(), 2);
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
                Segment::Hole {
                    kind: PlaceholderKind::Integer
                },
                Segment::Text(" und ".into()),
                Segment::Hole {
                    kind: PlaceholderKind::Integer
                }
            ]
        );
        assert_eq!(template_matches(&tpl, "5 und 6"), Ok(()));
    }

    #[test]
    fn several_templates_match_alone_or_in_order() {
        let a = t("Summe: 0", &["0"]);
        let b = t("Keine Zahlen", &["Zahlen"]);
        let ab = [a.clone(), b.clone()];
        assert_eq!(check_output(&ab, "Keine Werte"), None);
        assert_eq!(check_output(&ab, "Summe: 4"), None);
        assert!(check_output(std::slice::from_ref(&a), "Produkt: 4").is_some());
        assert_eq!(check_output(&[], "irgendwas"), None);
        // An ordered sequence of different templates (deliberately allowed).
        assert_eq!(check_output(&ab, "Summe: 4\nKeine Werte"), None);
        // Wrong order, repetition, empty output.
        assert!(check_output(&ab, "Keine Werte\nSumme: 4").is_some());
        assert!(check_output(&ab, "Summe: 4\nSumme: 5").is_some());
        assert!(check_output(&ab, "").is_some());
    }

    fn time_lines() -> Vec<OutputTemplate> {
        sheet_template()
            .text
            .lines()
            .map(|l| t(l, &["12345678"]))
            .collect()
    }

    const L_OUT: [&str; 3] = [
        "54630 seconds have passed since midnight.",
        "31770 seconds are left until midnight.",
        "Since I started with the task 441 seconds have passed.",
    ];

    #[test]
    fn split_templates_match_in_order() {
        let ls = time_lines();
        assert_eq!(check_output(&ls, &L_OUT.join("\n")), None);
        // A subsequence is fine, an empty line between sections as well.
        assert_eq!(
            check_output(&ls, &format!("{}\n{}", L_OUT[0], L_OUT[2])),
            None
        );
        assert_eq!(
            check_output(&ls[..2], &format!("{}\n\n{}", L_OUT[0], L_OUT[1])),
            None
        );
        // Line 2 before line 1: stuck after the first line.
        let msg = check_output(&ls, &format!("{}\n{}", L_OUT[1], L_OUT[0])).unwrap();
        assert!(
            msg.contains("Zeile 2 passt zu keiner weiteren Vorlage"),
            "{msg}"
        );
        // Decimal places in the first line: the old message with its hint.
        let out = format!("54630.0 seconds have passed since midnight.\n{}", L_OUT[1]);
        let msg = check_output(&ls, &out).unwrap();
        assert!(msg.contains("Ganzzahl ohne Nachkommastellen"), "{msg}");
        // An extra line after all three.
        let msg = check_output(&ls, &format!("{}\nmehr", L_OUT.join("\n"))).unwrap();
        assert!(
            msg.contains("Zeile 4 passt zu keiner weiteren Vorlage"),
            "{msg}"
        );
    }

    /// The task sheet of the regression case: the lines one by one in c),
    /// d), e), then the final block.
    const TIME_SHEET: &str = "Aufgabe 2.2 Time\n\
        c) Geben Sie die Sekunden seit Mitternacht in folgendem Format aus:\n\
        12345678 seconds have passed since midnight.\n\
        d) Berechnen Sie die Sekunden bis Mitternacht und geben Sie sie so aus:\n\
        12345678 seconds are left until midnight.\n\
        e) Berechnen Sie die vergangenen Sekunden:\n\
        Since I started with the task 12345678 seconds have passed.\n\
        Die Ausgabe sollte folgendermaßen aussehen (12345678 ist ein Platzhalter):\n\
        12345678 seconds have passed since midnight.\n\
        12345678 seconds are left until midnight.\n\
        Since I started with the task 12345678 seconds have passed.\n";

    #[test]
    fn repeated_lines_are_joined_where_they_are_consecutive() {
        let ls = time_lines();
        for order in [[0, 1, 2], [2, 0, 1]] {
            let raw: Vec<OutputTemplate> = order.iter().map(|&i| ls[i].clone()).collect();
            let r = verify_templates_with(TIME_SHEET, &raw, &[]);
            assert_eq!(r.templates, vec![sheet_template()]);
            assert_eq!((r.merged, r.discarded), (2, 0));
            assert_eq!(r.from_model, vec![true]);
        }
        // Block and single lines from the model: the block remains.
        let mut raw = vec![sheet_template()];
        raw.extend(ls.iter().cloned());
        let r = verify_templates_with(TIME_SHEET, &raw, &[]);
        assert_eq!(r.templates, vec![sheet_template()]);
        assert_eq!(r.merged, 3);
        // A shortened line of the model is subsumed by the detected block.
        let raw = vec![
            ls[0].clone(),
            t("12345678 seconds are left", &["12345678"]),
            ls[2].clone(),
        ];
        let detected = detect_templates(TIME_SHEET);
        assert_eq!(detected, vec![sheet_template()]);
        let r = verify_templates_with(TIME_SHEET, &raw, &detected);
        assert_eq!(r.templates, vec![sheet_template()]);
        assert_eq!(r.from_model, vec![true]);
    }

    #[test]
    fn joining_keeps_fixed_text_and_prefers_the_block_after_an_intro() {
        // "1" is a placeholder of A but fixed text in B: not joined.
        let r = verify_templates_with(
            "A: 1\nB: 2 von 1\n",
            &[t("A: 1", &["1"]), t("B: 2 von 1", &["2"])],
            &[],
        );
        assert_eq!(r.templates.len(), 2);
        assert_eq!(check_output(&r.templates, "A: 7\nB: 9 von 1"), None);
        assert!(check_output(&r.templates, "A: 7\nB: 9 von 3").is_some());
        // Equal chains: the one after "Ausgabe:" wins.
        let r = verify_templates_with(
            "X: 1\nY: 2\nText dazwischen\nAusgabe:\nY: 2\nX: 1\n",
            &[t("X: 1", &["1"]), t("Y: 2", &["2"])],
            &[],
        );
        assert_eq!(r.templates, vec![t("Y: 2\nX: 1", &["2", "1"])]);
        // Otherwise the later one (final block); each template only once.
        let r = verify_templates_with(
            "A: 1\nB: 2\nA: 1\n",
            &[t("A: 1", &["1"]), t("B: 2", &["2"])],
            &[],
        );
        assert_eq!(r.templates, vec![t("B: 2\nA: 1", &["2", "1"])]);
        // Many occurrences: the last 16 are kept, the final block joins.
        let text = format!("{}Z: 0\nW: 0\n", "Z: 0\nText\n".repeat(40));
        let r = verify_templates_with(&text, &[t("Z: 0", &["0"]), t("W: 0", &["0"])], &[]);
        assert_eq!(r.templates, vec![t("Z: 0\nW: 0", &["0"])]);
        // Five lines that are not neighbours: three remain, in text order.
        let text = "V: 1\nx\nW: 2\nx\nX: 3\nx\nY: 4\nx\nZ: 5\n";
        let raw: Vec<OutputTemplate> = [
            ("V: 1", "1"),
            ("W: 2", "2"),
            ("X: 3", "3"),
            ("Y: 4", "4"),
            ("Z: 5", "5"),
        ]
        .iter()
        .map(|(a, b)| t(a, &[b]))
        .collect();
        let r = verify_templates_with(text, &raw, &[]);
        assert_eq!(r.templates.len(), 3);
        assert_eq!(r.discarded, 2);
        let pos: Vec<usize> = r
            .templates
            .iter()
            .map(|x| text.find(&x.text).unwrap())
            .collect();
        assert!(pos.windows(2).all(|w| w[0] < w[1]), "{:?}", r.templates);
    }

    #[test]
    fn detects_placeholder_block_after_intro() {
        let d = detect_templates("Ausgabeformat:\nSumme: <n>\nMittel: <m>\n");
        assert_eq!(d, vec![t("Summe: <n>\nMittel: <m>", &["<n>", "<m>"])]);
        let d = detect_templates("Ausgabe:\nPreis: XX.XX Euro\n");
        assert_eq!(d, vec![t("Preis: XX.XX Euro", &["XX.XX"])]);
        let d = detect_templates("Ausgabe: Ergebnis: ___\n");
        assert_eq!(d, vec![t("Ergebnis: ___", &["___"])]);
        let d = detect_templates("Ausgabe:\nHallo, [Name]!\n");
        assert_eq!(d, vec![t("Hallo, [Name]!", &["[Name]"])]);
        let d = detect_templates("Ausgabe (N ist ein Platzhalter):\nEs sind N Tage.\n");
        assert_eq!(d, vec![t("Es sind N Tage.", &["N"])]);
        // The QAS sheet: declared placeholder, no output intro.
        let d = detect_templates(SHEET);
        assert_eq!(d, vec![sheet_template()]);
    }

    #[test]
    fn ignores_numbers_without_placeholder_hint() {
        for text in [
            "Eingabe: 5\nAusgabe: 25\n",
            "Beispielausgabe:\n12345678 Sekunden\n",
            "Eingabeformat:\n<n>\n<a1> <a2>\n",
            "Die Zahl ist ein Platzhalter.\nDie Zahl 7 ist prim.\n",
            "Gib die Größe XXL aus.\n",
            "Ausgabe:\n1\n2\n...\n10\n",
            "0 ist kein Platzhalter, sondern ein echter Wert.\nAusgabe:\nSumme: 0\n",
            "Gib arr[i] aus:\nWert\n",
            "Gib List<String> aus:\nWert <br> fett\n",
        ] {
            assert_eq!(detect_templates(text), vec![], "{text}");
        }
    }

    #[test]
    fn output_intros() {
        assert!(is_output_intro(
            "Die Ausgabe sollte folgendermaßen aussehen:"
        ));
        assert!(is_output_intro("Beispielausgabe:"));
        assert!(is_output_intro("Output:"));
        assert!(!is_output_intro("Eingabe:"));
        assert!(!is_output_intro("Ein- und Ausgabeformat:"));
        assert!(!is_output_intro("Die Ausgabe ist eine Zahl."));
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
    fn example_sentences_are_no_templates() {
        // Regression 1.1.2 („Aufgabe 6.2 BankAccount“): the model took the
        // example sentence as a template with the placeholder `100.0`.
        let text = "Wenn die Einzahlung erfolgreich ist, soll eine formatierte Nachricht \
            zurückgegeben werden, die den aktuellen Kontostand angibt, z.B. “Your current \
            balance is €100.0”.\nBei erfolgreicher Abhebung ebenso, z. B. „Your current \
            balance is €100.0“.";
        let raw = [t("Your current balance is €100.0", &["100.0"])];
        assert_eq!(verify_templates(text, &raw), (vec![], 1));
        for intro in ["e.g.", "zum Beispiel", "for example:", "beispielsweise"] {
            let text = format!("Gib den Kontostand aus, {intro} Kontostand: 100.0");
            let raw = [t("Kontostand: 100.0", &["100.0"])];
            assert_eq!(verify_templates(&text, &raw).0, vec![], "{intro}");
        }
        // A declared or syntactic placeholder keeps the template.
        let text = "Gib den Kontostand aus, z.B. „Kontostand: 100.0“ (100.0 ist ein Platzhalter).";
        let raw = [t("Kontostand: 100.0", &["100.0"])];
        assert_eq!(verify_templates(text, &raw).0.len(), 1);
        let text = "Gib den Kontostand aus, z. B. Kontostand: <betrag>";
        let raw = [t("Kontostand: <betrag>", &["<betrag>"])];
        assert_eq!(verify_templates(text, &raw).0.len(), 1);
        // Also shown outside an example sentence: still a template.
        let text = "Ausgabe:\nKontostand: 100.0\nz. B. Kontostand: 100.0";
        let raw = [t("Kontostand: 100.0", &["100.0"])];
        assert_eq!(verify_templates(text, &raw).0.len(), 1);
    }

    #[test]
    fn serde_uses_camel_case_and_defaults() {
        let t: OutputTemplate = serde_json::from_str(r#"{"text":"x 1"}"#).unwrap();
        assert!(t.placeholders.is_empty());
        let v = serde_json::to_value(sheet_template()).unwrap();
        assert!(v["placeholders"].is_array());
    }
}
