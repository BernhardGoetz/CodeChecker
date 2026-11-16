//! Finds the entry point of a Java program: the top-level class with
//! `public static void main(String[] args)`, the public top-level class
//! (which decides the file name) and the package. A small tokenizer skips
//! comments, string/char literals and text blocks, so `// class Fake` or
//! `"void main("` never count. Pure, unit-tested.

/// Name used when the code names no usable class (starter template, AI
/// prompts without a class name from the task).
pub const DEFAULT_CLASS: &str = "Main";

/// What the analysis found in the code.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JavaProgram {
    /// `package a.b;` → `Some("a.b")`.
    pub package: Option<String>,
    /// Public top-level types in source order (javac allows only one).
    pub public_types: Vec<String>,
    /// Top-level types with a valid `main` method, in source order.
    pub main_types: Vec<String>,
}

impl JavaProgram {
    /// Fully qualified name of a top-level type (with the package).
    pub fn qualified(&self, name: &str) -> String {
        match &self.package {
            Some(p) => format!("{p}.{name}"),
            None => name.to_string(),
        }
    }

    /// File stem the source must be stored under: the public top-level
    /// type (javac requires `<Name>.java`), else the main type, else
    /// [`DEFAULT_CLASS`].
    pub fn file_stem(&self) -> &str {
        self.public_types
            .first()
            .or(self.main_types.first())
            .map(String::as_str)
            .unwrap_or(DEFAULT_CLASS)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok<'a> {
    Ident(&'a str),
    Sym(char),
    /// `...` (varargs).
    Ellipsis,
}

/// Splits Java source into identifiers (also keywords and numbers) and
/// symbols; comments, string, char and text-block literals are dropped.
fn tokenize(code: &str) -> Vec<Tok<'_>> {
    let mut toks = Vec::new();
    let bytes = code.as_bytes();
    let mut chars = code.char_indices().peekable();
    let starts = |i: usize, s: &str| code[i..].starts_with(s);
    while let Some((i, c)) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        if starts(i, "//") {
            while let Some(&(_, n)) = chars.peek() {
                if n == '\n' {
                    break;
                }
                chars.next();
            }
        } else if starts(i, "/*") {
            chars.next();
            let mut prev = ' ';
            for (_, n) in chars.by_ref() {
                if prev == '*' && n == '/' {
                    break;
                }
                prev = n;
            }
        } else if starts(i, "\"\"\"") {
            chars.next();
            chars.next();
            // Text block: ends at the next unescaped `"""`.
            while let Some((j, n)) = chars.next() {
                if n == '\\' {
                    chars.next();
                } else if starts(j, "\"\"\"") {
                    chars.next();
                    chars.next();
                    break;
                }
            }
        } else if c == '"' || c == '\'' {
            while let Some((_, n)) = chars.next() {
                if n == '\\' {
                    chars.next();
                } else if n == c || n == '\n' {
                    break;
                }
            }
        } else if c.is_alphanumeric() || c == '_' || c == '$' {
            let mut end = i + c.len_utf8();
            while let Some(&(j, n)) = chars.peek() {
                if n.is_alphanumeric() || n == '_' || n == '$' {
                    end = j + n.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            toks.push(Tok::Ident(&code[i..end]));
        } else if c == '.' && bytes.get(i + 1) == Some(&b'.') && bytes.get(i + 2) == Some(&b'.') {
            chars.next();
            chars.next();
            toks.push(Tok::Ellipsis);
        } else {
            toks.push(Tok::Sym(c));
        }
    }
    toks
}

#[derive(Debug)]
struct Scope<'a> {
    name: &'a str,
    /// Brace depth inside the type body.
    body_depth: usize,
    top_level: bool,
    interface: bool,
}

struct Pending<'a> {
    name: &'a str,
    depth: usize,
    public: bool,
    interface: bool,
}

fn has(toks: &[Tok], word: &str) -> bool {
    toks.contains(&Tok::Ident(word))
}

/// Index of the `)` matching the `(` at `open`.
fn matching_paren(toks: &[Tok], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, t) in toks.iter().enumerate().skip(open) {
        match t {
            Tok::Sym('(') => depth += 1,
            Tok::Sym(')') => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Drops annotations (`@A`, `@a.B(...)`) and `final` from a parameter list.
fn strip_param_modifiers<'a>(toks: &[Tok<'a>]) -> Vec<Tok<'a>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        match toks[i] {
            Tok::Sym('@') => {
                i += 1;
                // Qualified annotation name.
                while matches!(toks.get(i), Some(Tok::Ident(_))) {
                    i += 1;
                    if toks.get(i) == Some(&Tok::Sym('.')) {
                        i += 1;
                    } else {
                        break;
                    }
                }
                if toks.get(i) == Some(&Tok::Sym('(')) {
                    i = matching_paren(toks, i).map_or(toks.len(), |c| c + 1);
                }
            }
            Tok::Ident("final") => i += 1,
            t => {
                out.push(t);
                i += 1;
            }
        }
    }
    out
}

/// `String[] a`, `String... a`, `String a[]`, `java.lang.String[] a`.
fn is_main_params(toks: &[Tok]) -> bool {
    let t = strip_param_modifiers(toks);
    let rest = match t.as_slice() {
        [Tok::Ident("String"), rest @ ..] => rest,
        [Tok::Ident("java"), Tok::Sym('.'), Tok::Ident("lang"), Tok::Sym('.'), Tok::Ident("String"), rest @ ..] => {
            rest
        }
        _ => return false,
    };
    matches!(
        rest,
        [Tok::Sym('['), Tok::Sym(']'), Tok::Ident(_)]
            | [Tok::Ellipsis, Tok::Ident(_)]
            | [Tok::Ident(_), Tok::Sym('['), Tok::Sym(']')]
    )
}

/// Analyses Java source code (see [`JavaProgram`]).
pub fn analyze(code: &str) -> JavaProgram {
    let toks = tokenize(code);
    let mut program = JavaProgram::default();
    let mut stack: Vec<Scope> = Vec::new();
    let mut pending: Option<Pending> = None;
    let mut depth = 0usize;
    // First token of the current declaration/statement.
    let mut start = 0usize;
    let mut i = 0usize;
    while i < toks.len() {
        match toks[i] {
            Tok::Sym('{') => {
                depth += 1;
                if let Some(p) = pending.take() {
                    if p.depth + 1 == depth {
                        let top_level = stack.is_empty() && p.depth == 0;
                        if top_level && p.public {
                            program.public_types.push(p.name.to_string());
                        }
                        stack.push(Scope {
                            name: p.name,
                            body_depth: depth,
                            top_level,
                            interface: p.interface,
                        });
                    }
                }
                start = i + 1;
            }
            Tok::Sym('}') => {
                if stack.last().is_some_and(|s| s.body_depth == depth) {
                    stack.pop();
                }
                depth = depth.saturating_sub(1);
                pending = None;
                start = i + 1;
            }
            Tok::Sym(';') => {
                if depth == 0 && toks.get(start) == Some(&Tok::Ident("package")) {
                    let parts = &toks[start + 1..i];
                    let valid = !parts.is_empty()
                        && parts.iter().enumerate().all(|(k, t)| {
                            if k % 2 == 0 {
                                matches!(t, Tok::Ident(_))
                            } else {
                                *t == Tok::Sym('.')
                            }
                        })
                        && parts.len() % 2 == 1;
                    if valid {
                        let name: String = parts
                            .iter()
                            .map(|t| match t {
                                Tok::Ident(s) => *s,
                                _ => ".",
                            })
                            .collect();
                        program.package = Some(name);
                    }
                }
                start = i + 1;
            }
            Tok::Ident(kw @ ("class" | "interface" | "enum" | "record")) => {
                let after_dot = i > 0 && toks[i - 1] == Tok::Sym('.');
                let next = toks.get(i + 1);
                let is_decl = match (next, toks.get(i + 2)) {
                    (Some(Tok::Ident(_)), after) => {
                        kw != "record" || matches!(after, Some(Tok::Sym('(' | '<')))
                    }
                    _ => false,
                };
                if !after_dot && is_decl {
                    if let Some(Tok::Ident(name)) = next {
                        pending = Some(Pending {
                            name,
                            depth,
                            public: has(&toks[start.min(i)..i], "public"),
                            interface: kw == "interface",
                        });
                        i += 1;
                    }
                }
            }
            Tok::Ident("main") => {
                let member_of_top_level = stack
                    .last()
                    .filter(|s| s.body_depth == depth && s.top_level && stack.len() == 1);
                if let Some(scope) = member_of_top_level {
                    let mods = &toks[start.min(i)..i];
                    let ok_head = i > 0
                        && toks[i - 1] == Tok::Ident("void")
                        && toks.get(i + 1) == Some(&Tok::Sym('('))
                        && has(mods, "static")
                        && (has(mods, "public") || scope.interface);
                    if ok_head {
                        if let Some(close) = matching_paren(&toks, i + 1) {
                            if is_main_params(&toks[i + 2..close])
                                && !program.main_types.iter().any(|m| m == scope.name)
                            {
                                program.main_types.push(scope.name.to_string());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    program
}

/// The class to start: the only top-level type with `main`, or a German
/// explanation why there is none.
pub fn main_class(program: &JavaProgram) -> Result<&str, String> {
    match program.main_types.as_slice() {
        [one] => Ok(one),
        [] => Err(
            "Keine Klasse mit einer main-Methode gefunden. Java-Code braucht eine Klasse mit \
             `public static void main(String[] args)`, z. B. `public class Main { public static \
             void main(String[] args) { ... } }`. Der Klassenname ist frei wählbar."
                .to_string(),
        ),
        many => Err(format!(
            "Mehrere Klassen enthalten eine main-Methode ({}). Bitte nur in einer Klasse \
             `public static void main(String[] args)` verwenden, damit klar ist, welche \
             gestartet wird.",
            many.join(", ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mains(code: &str) -> Vec<String> {
        analyze(code).main_types
    }

    #[test]
    fn finds_main_in_any_class() {
        let p = analyze("public class Main { public static void main(String[] args) {} }");
        assert_eq!(p.main_types, vec!["Main"]);
        assert_eq!(p.public_types, vec!["Main"]);
        assert_eq!(p.file_stem(), "Main");
        let p = analyze(
            "import java.util.*;\n\npublic class Time {\n    private int h;\n    \
             public static void main(String[] args) {\n        System.out.println(1);\n    }\n}\n",
        );
        assert_eq!(p.main_types, vec!["Time"]);
        assert_eq!(p.file_stem(), "Time");
        assert_eq!(main_class(&p), Ok("Time"));
        // Package-private class: the file is named after it, too.
        let p = analyze("class Aufgabe1 { public static void main(String[] a) { } }");
        assert_eq!(p.file_stem(), "Aufgabe1");
        assert!(p.public_types.is_empty());
    }

    #[test]
    fn accepts_parameter_variants() {
        for params in [
            "String[] args",
            "String... args",
            "String args[]",
            "final String[] args",
            "@SuppressWarnings(\"unused\") final String... a",
            "java.lang.String[] args",
            "String []args",
        ] {
            let code = format!("public class A {{ public static void main({params}) {{ }} }}");
            assert_eq!(mains(&code), vec!["A"], "{params}");
        }
        // Modifier order, annotations and `final` on the method.
        assert_eq!(
            mains("class A { @Deprecated static public final void main(String[] x) {} }"),
            vec!["A"]
        );
        for wrong in [
            "class A { public static void main(int[] args) {} }",
            "class A { public static void main(String args) {} }",
            "class A { public static void main() {} }",
            "class A { public static void main(String[] a, int b) {} }",
            "class A { public void main(String[] args) {} }",
            "class A { static int main(String[] args) { return 0; } }",
            "class A { private static void main(String[] args) {} }",
        ] {
            assert!(mains(wrong).is_empty(), "{wrong}");
        }
        // Interface members are implicitly public.
        assert_eq!(
            mains("interface I { static void main(String[] a) {} }"),
            vec!["I"]
        );
    }

    #[test]
    fn ignores_comments_and_strings() {
        let code = "// public class Fake { public static void main(String[] a) {} }\n\
                    /* class Fake2 { public static void main(String[] a) {} } */\n\
                    public class Time {\n\
                        String s = \"class Fake3 { public static void main(String[] a) {} }\";\n\
                        char c = '{';\n\
                        String t = \"\"\"\n  } class Fake4 { \\\"\"\" \n  \"\"\";\n\
                        public static void main(String... args) { System.out.println(\"}\"); }\n\
                    }\n";
        let p = analyze(code);
        assert_eq!(p.main_types, vec!["Time"]);
        assert_eq!(p.public_types, vec!["Time"]);
        // `Foo.class` and a variable called `record` are no declarations.
        let p = analyze(
            "public class A { Object o = String.class; int record = 1; \
             public static void main(String[] a) {} }",
        );
        assert_eq!(p.main_types, vec!["A"]);
    }

    #[test]
    fn several_classes() {
        // The public class decides the file name, the main class is started.
        let p = analyze(
            "public class Time { int h; }\nclass Main { public static void main(String[] a) {} }",
        );
        assert_eq!(p.main_types, vec!["Main"]);
        assert_eq!(p.file_stem(), "Time");
        // Helper class without main next to the main class.
        let p = analyze(
            "class Helper { static int f() { return 1; } }\n\
             public class Rechner { public static void main(String[] a) {} }",
        );
        assert_eq!(main_class(&p), Ok("Rechner"));
        assert_eq!(p.file_stem(), "Rechner");
        // Two classes with main: German error naming both.
        let p = analyze(
            "class A { public static void main(String[] a) {} }\n\
             class B { public static void main(String[] a) {} }",
        );
        let e = main_class(&p).unwrap_err();
        assert!(e.contains("Mehrere Klassen") && e.contains("A, B"), "{e}");
        // Nested classes, anonymous classes and lambdas do not count.
        let p = analyze(
            "public class Outer {\n  static class Inner { public static void main(String[] a) {} }\n  \
             Runnable r = new Runnable() { public void run() {} };\n  \
             public static void main(String[] a) { Runnable q = () -> {}; }\n}",
        );
        assert_eq!(p.main_types, vec!["Outer"]);
    }

    #[test]
    fn no_main() {
        let p = analyze("public class Aufgabe { }");
        let e = main_class(&p).unwrap_err();
        assert!(e.starts_with("Keine Klasse mit einer main-Methode"), "{e}");
        assert_eq!(p.file_stem(), "Aufgabe");
        assert_eq!(analyze("").file_stem(), DEFAULT_CLASS);
        // Broken code does not panic.
        for broken in [
            "class",
            "class A {",
            "}}}",
            "public static void main(",
            "\"",
            "/*",
        ] {
            let _ = analyze(broken);
        }
    }

    #[test]
    fn package_declaration() {
        let p = analyze(
            "package de.hs.uebung3;\n\nimport java.util.Scanner;\n\npublic class Time { \
             public static void main(String[] args) {} }",
        );
        assert_eq!(p.package.as_deref(), Some("de.hs.uebung3"));
        assert_eq!(p.qualified("Time"), "de.hs.uebung3.Time");
        assert_eq!(
            analyze("package x;\nclass A {}").package.as_deref(),
            Some("x")
        );
        assert_eq!(analyze("class A {}").qualified("A"), "A");
        // A package inside a comment does not count.
        assert_eq!(analyze("// package x;\nclass A {}").package, None);
    }
}
