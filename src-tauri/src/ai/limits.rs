//! Input limits the task states (the `inputConstraints` of the structure
//! step, e.g. „1 <= n <= 1000“ or „positive whole number“), read by rules.
//! Generated inputs outside them are dropped: the task does not require the
//! program to handle them, and their expected output may be meaningless.
//!
//! The model writes the constraints, so a bound only counts if the task text
//! backs it: a numeric bound must occur in the text as a number, „positive“
//! or „non-negative“ needs „positiv“ or „negativ“ in the text. Anything
//! unclear is not checked (no input is dropped because of a guess): several
//! variables with different limits, an input with several numbers for the
//! limit of one variable.

/// One end of a range: the value and whether it belongs to the range.
type End = (f64, bool);

/// Limits for the numbers of an input.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Limits {
    pub lo: Option<End>,
    pub hi: Option<End>,
    /// The limits apply to every number of the input („1 <= a, b <= 100“,
    /// „each number“); otherwise only to an input that is one number.
    pub joint: bool,
}

impl Limits {
    fn contains(&self, x: f64) -> bool {
        let lo_ok = self
            .lo
            .is_none_or(|(v, incl)| if incl { x >= v } else { x > v });
        let hi_ok = self
            .hi
            .is_none_or(|(v, incl)| if incl { x <= v } else { x < v });
        lo_ok && hi_ok
    }

    fn tighten_lo(&mut self, e: End) {
        if self
            .lo
            .is_none_or(|(v, incl)| e.0 > v || (e.0 == v && incl && !e.1))
        {
            self.lo = Some(e);
        }
    }

    fn tighten_hi(&mut self, e: End) {
        if self
            .hi
            .is_none_or(|(v, incl)| e.0 < v || (e.0 == v && incl && !e.1))
        {
            self.hi = Some(e);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64, String),
    Op(&'static str),
    Word(String),
    Comma,
}

fn number(s: &str) -> Option<f64> {
    let t = s.replace('_', "");
    let ok = !t.is_empty()
        && t.trim_start_matches(['-', '+'])
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
        && t.chars()
            .skip(1)
            .all(|c| c.is_ascii_digit() || c == '.' || c == ',');
    if !ok {
        return None;
    }
    // „1.000“ / „1,000“ may be a thousands separator: ambiguous, refused.
    let t = t.replace(',', ".");
    match t.split_once('.') {
        Some((_, frac)) if frac.contains('.') => return None,
        Some((int, frac)) if frac.len() == 3 && int.trim_start_matches(['-', '+']) != "0" => {
            return None
        }
        _ => {}
    }
    t.parse().ok()
}

fn tokens(text: &str) -> Vec<Tok> {
    let text = text
        .replace('≤', " <= ")
        .replace('≥', " >= ")
        .replace("=<", " <= ")
        .replace("=>", " >= ")
        .replace('−', "-");
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() || "()[]{};:".contains(c) {
            i += 1;
        } else if c == ',' {
            out.push(Tok::Comma);
            i += 1;
        } else if c == '<' || c == '>' {
            let eq = chars.get(i + 1) == Some(&'=');
            out.push(Tok::Op(match (c, eq) {
                ('<', true) => "<=",
                ('<', false) => "<",
                ('>', true) => ">=",
                _ => ">",
            }));
            i += 1 + usize::from(eq);
        } else {
            let start = i;
            let numeric = c.is_ascii_digit()
                || (c == '-' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()));
            i += 1;
            while i < chars.len() {
                let d = chars[i];
                let cont = if numeric {
                    d.is_ascii_digit()
                        || ((d == '.' || d == ',')
                            && chars.get(i + 1).is_some_and(|e| e.is_ascii_digit()))
                } else {
                    d.is_alphanumeric() || d == '_' || d == '-'
                };
                if !cont {
                    break;
                }
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            // „10^9“, „2*10^5“, „1e9“, „5ms“: not a plain number.
            let glued = numeric
                && chars
                    .get(i)
                    .is_some_and(|d| d.is_alphabetic() || "^*·".contains(*d));
            match number(&s).filter(|_| numeric && !glued) {
                Some(v) => out.push(Tok::Num(v, s)),
                None => {
                    let w = s
                        .trim_matches(|c: char| !c.is_alphanumeric())
                        .to_lowercase();
                    if !w.is_empty() {
                        out.push(Tok::Word(w));
                    }
                }
            }
        }
    }
    out
}

/// A clause found in the constraints.
struct Clause {
    lo: Option<End>,
    hi: Option<End>,
    /// Variables it names (empty: none, e.g. „positive“, „1 to 100“).
    vars: Vec<String>,
    /// Numbers it uses (raw text, for the check against the task).
    numbers: Vec<String>,
    /// Word it needs in the task text instead („positiv“, „negativ“).
    needs_word: Option<&'static str>,
}

fn is_var(w: &str) -> bool {
    w.chars().count() <= 12
        && w.chars().next().is_some_and(char::is_alphabetic)
        && w.chars().all(|c| c.is_alphanumeric() || c == '_')
}

const JOINT_WORDS: &[&str] = &[
    "each",
    "every",
    "all",
    "jede",
    "jeder",
    "jedes",
    "alle",
    "sämtliche",
];

fn clauses(toks: &[Tok]) -> Vec<Clause> {
    let mut out = Vec::new();
    let num = |i: usize| match toks.get(i) {
        Some(Tok::Num(v, s)) => Some((*v, s.clone())),
        _ => None,
    };
    let op = |i: usize| match toks.get(i) {
        Some(Tok::Op(o)) => Some(*o),
        _ => None,
    };
    let word = |i: usize| match toks.get(i) {
        Some(Tok::Word(w)) => Some(w.as_str()),
        _ => None,
    };
    // Variables from i on: „a“ or „a, b, c“; returns them and the next index.
    let vars_at = |mut i: usize| -> (Vec<String>, usize) {
        let mut v = Vec::new();
        while let Some(w) = word(i).filter(|w| is_var(w)) {
            v.push(w.to_string());
            i += 1;
            if toks.get(i) == Some(&Tok::Comma) && word(i + 1).is_some_and(is_var) {
                i += 1;
            } else {
                break;
            }
        }
        (v, i)
    };
    let mut i = 0;
    while i < toks.len() {
        // NUM op VARS op NUM, or NUM op VARS.
        if let (Some((a, sa)), Some(o1)) = (num(i), op(i + 1)) {
            let (vars, j) = vars_at(i + 2);
            if !vars.is_empty() {
                let mut c = Clause {
                    lo: None,
                    hi: None,
                    vars,
                    numbers: vec![sa],
                    needs_word: None,
                };
                match o1 {
                    "<" | "<=" => c.lo = Some((a, o1 == "<=")),
                    _ => c.hi = Some((a, o1 == ">=")),
                }
                let mut next = j;
                if let (Some(o2), Some((b, sb))) = (op(j), num(j + 1)) {
                    if matches!(
                        (o1, o2),
                        ("<" | "<=", "<" | "<=") | (">" | ">=", ">" | ">=")
                    ) {
                        match o2 {
                            "<" | "<=" => c.hi = Some((b, o2 == "<=")),
                            _ => c.lo = Some((b, o2 == ">=")),
                        }
                        c.numbers.push(sb);
                        next = j + 2;
                    }
                }
                out.push(c);
                i = next;
                continue;
            }
        }
        // VARS op NUM.
        let (vars, j) = vars_at(i);
        if !vars.is_empty() {
            if let (Some(o), Some((b, sb))) = (op(j), num(j + 1)) {
                let mut c = Clause {
                    lo: None,
                    hi: None,
                    vars,
                    numbers: vec![sb],
                    needs_word: None,
                };
                match o {
                    "<" | "<=" => c.hi = Some((b, o == "<=")),
                    _ => c.lo = Some((b, o == ">=")),
                }
                out.push(c);
                i = j + 2;
                continue;
            }
        }
        // „from A to B“, „between A and B“, „von A bis B“, „A to B“.
        if let (Some((a, sa)), Some(w), Some((b, sb))) = (num(i), word(i + 1), num(i + 2)) {
            let before = if i > 0 { word(i - 1) } else { None };
            let range_word = matches!(w, "to" | "bis")
                || (w == "and" && before == Some("between"))
                || (w == "und" && before == Some("zwischen"));
            if range_word && a <= b {
                out.push(Clause {
                    lo: Some((a, true)),
                    hi: Some((b, true)),
                    vars: Vec::new(),
                    numbers: vec![sa, sb],
                    needs_word: None,
                });
                i += 3;
                continue;
            }
        }
        if let Some(w) = word(i) {
            let negated = |k: usize| {
                k > 0 && matches!(word(k - 1), Some("not" | "nicht" | "no" | "kein" | "keine"))
            };
            let clause = |lo: End, needs: &'static str| Clause {
                lo: Some(lo),
                hi: None,
                vars: Vec::new(),
                numbers: Vec::new(),
                needs_word: Some(needs),
            };
            if matches!(
                w,
                "non-negative" | "nonnegative" | "nichtnegativ" | "nichtnegative"
            ) || (w.starts_with("negativ") || w == "negative") && negated(i)
            {
                out.push(clause((0.0, true), "negativ"));
            } else if (w.starts_with("positiv") || w == "positive") && !negated(i) {
                out.push(clause((0.0, false), "positiv"));
            }
        }
        i += 1;
    }
    out
}

/// Number tokens of a text (raw), for the check against the task.
fn numbers_in(text: &str) -> Vec<f64> {
    tokens(text)
        .into_iter()
        .filter_map(|t| match t {
            Tok::Num(v, _) => Some(v),
            _ => None,
        })
        .collect()
}

/// The limits of `constraints` that the task text backs; `None` if there
/// are none or they are unclear (several variables with own limits).
pub fn parse(constraints: &str, task_text: &str) -> Option<Limits> {
    let task_lower = task_text.to_lowercase();
    let task_numbers = numbers_in(task_text);
    let toks = tokens(constraints);
    let joint_word = toks
        .iter()
        .any(|t| matches!(t, Tok::Word(w) if JOINT_WORDS.contains(&w.as_str())));
    let backed: Vec<Clause> = clauses(&toks)
        .into_iter()
        .filter(|c| {
            c.numbers.iter().all(|n| {
                number(n).is_some_and(|v| task_numbers.iter().any(|t| (t - v).abs() < 1e-9))
            }) && c.needs_word.is_none_or(|w| task_lower.contains(w))
        })
        .collect();
    if backed.is_empty() {
        return None;
    }
    let mut names: Vec<&Vec<String>> = backed
        .iter()
        .map(|c| &c.vars)
        .filter(|v| !v.is_empty())
        .collect();
    names.dedup();
    if names.len() > 1 {
        return None;
    }
    let mut limits = Limits {
        joint: joint_word || names.first().is_some_and(|v| v.len() > 1),
        ..Limits::default()
    };
    for c in &backed {
        if let Some(e) = c.lo {
            limits.tighten_lo(e);
        }
        if let Some(e) = c.hi {
            limits.tighten_hi(e);
        }
    }
    if let (Some((lo, li)), Some((hi, hi_incl))) = (limits.lo, limits.hi) {
        if lo > hi || (lo == hi && !(li && hi_incl)) {
            return None;
        }
    }
    Some(limits)
}

/// Whether an input keeps the limits. Only checked where it is clear: every
/// number for joint limits, else an input that is exactly one number.
pub fn fits(input: &str, limits: &Limits) -> bool {
    let numbers: Vec<f64> = input
        .split_whitespace()
        .filter_map(|w| number(w).filter(|v| v.is_finite()))
        .collect();
    if numbers.is_empty() || (!limits.joint && numbers.len() != 1) {
        return true;
    }
    numbers.iter().all(|x| limits.contains(*x))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lim(c: &str, task: &str) -> Option<Limits> {
        parse(c, task)
    }

    #[test]
    fn ranges_from_the_constraints() {
        let task = "Lesen Sie eine ganze Zahl n (1 <= n <= 20) ein.";
        let l = lim("1 <= n <= 20, whole number", task).unwrap();
        assert_eq!(l.lo, Some((1.0, true)));
        assert_eq!(l.hi, Some((20.0, true)));
        assert!(!l.joint);
        assert!(fits("1\n", &l) && fits("20\n", &l) && fits(" 7 ", &l));
        assert!(!fits("0\n", &l) && !fits("21\n", &l) && !fits("-3\n", &l));
        // Strict bounds and the sign written by the model.
        let task = "Lesen Sie eine ganze Zahl n (0 <= n < 86400) ein.";
        let l = lim("0 ≤ n < 86400", task).unwrap();
        assert!(fits("0", &l) && fits("86399", &l) && !fits("86400", &l));
        // Words.
        let task = "Lesen Sie die Parkdauer in Minuten ein (ganze Zahl, 1 bis 1440).";
        let l = lim("ganze Zahl von 1 bis 1440", task).unwrap();
        assert!(fits("1440", &l) && !fits("1441", &l) && !fits("0", &l));
        let l = lim("whole number between 1 and 1440", task).unwrap();
        assert!(!fits("2000", &l));
        let task = "Lesen Sie eine Punktzahl p (ganze Zahl von 0 bis 100) ein.";
        let l = lim("p is an integer from 0 to 100", task).unwrap();
        assert!(fits("100", &l) && !fits("101", &l));
        // „positive“ only with the word in the task.
        let task = "Lesen Sie eine positive ganze Zahl n ein.";
        let l = lim("positive whole number", task).unwrap();
        assert!(fits("5000", &l) && !fits("0", &l) && !fits("-1", &l));
        assert_eq!(lim("positive whole number", "Lies n ein."), None);
        let l = lim("non-negative integer", "Die Zahl ist nicht negativ.").unwrap();
        assert!(fits("0", &l) && !fits("-1", &l));
        assert_eq!(lim("not negative", "Lies n.").map(|l| l.lo), None);
    }

    #[test]
    fn several_numbers_only_with_joint_limits() {
        let task = "Lesen Sie zwei Zahlen a und b ein (1 <= a, b <= 100).";
        let l = lim("1 <= a, b <= 100", task).unwrap();
        assert!(l.joint);
        assert!(fits("3 100", &l) && !fits("3 101", &l) && !fits("0 5", &l));
        let task = "Jede Zahl liegt zwischen 0 und 9.";
        let l = lim("each number between 0 and 9", task).unwrap();
        assert!(l.joint && !fits("1 2 10", &l));
        // The limit of one variable: an input with several numbers is not checked.
        let task = "Lesen Sie n (1 <= n <= 5) und danach n Zahlen ein.";
        let l = lim("1 <= n <= 5", task).unwrap();
        assert!(fits("3\n100 200 300\n", &l));
        assert!(!fits("9\n", &l));
    }

    #[test]
    fn unclear_or_invented_limits_are_not_used() {
        for (c, task) in [
            ("", "Lies n."),
            ("none", "Lies n."),
            // Bound not in the task: invented by the model.
            ("1 <= n <= 1000", "Lies eine Zahl n ein."),
            ("n <= 100", "Lies n ein (n <= 10)."),
            // Different variables with their own limits.
            ("1 <= n <= 10, 1 <= m <= 5", "1 <= n <= 10 und 1 <= m <= 5"),
            // Contradiction.
            ("10 <= n <= 1", "10 <= n <= 1"),
            ("words only", "Lies Wörter."),
        ] {
            assert_eq!(lim(c, task), None, "{c}");
        }
        // Several clauses of one variable are joined.
        let l = lim("n >= 1, n <= 50, positive", "positiv, 1 bis 50").unwrap();
        assert_eq!((l.lo, l.hi), (Some((1.0, true)), Some((50.0, true))));
        // Powers and thousands separators are not misread as small bounds.
        assert_eq!(lim("1 <= n <= 10^9", "1 <= n <= 10^9").unwrap().hi, None);
        assert_eq!(lim("1 <= n <= 1e9", "1 <= n <= 1e9").unwrap().hi, None);
        assert_eq!(
            lim("1 <= n <= 100.000", "1 <= n <= 100.000").unwrap().hi,
            None
        );
        assert_eq!(
            lim("0 <= x <= 0,5", "0 <= x <= 0,5").unwrap().hi,
            Some((0.5, true))
        );
        // Inputs without numbers or with text are not judged by numbers.
        let l = lim("1 <= n <= 5", "1 <= n <= 5").unwrap();
        assert!(fits("abc\n", &l) && fits("", &l));
    }
}
