//! Data-driven language table. Adding a language = adding one entry here
//! plus a Dockerfile under `images/`.

use serde::Serialize;
use std::time::Duration;

use super::java;

/// Placeholder in [`LanguageConfig::source_file`], `compile_cmd` and
/// `run_cmd` for the file stem of the source (Java: public class).
pub const FILE_PLACEHOLDER: &str = "{file}";
/// Placeholder in `run_cmd` for the (qualified) class to start (Java).
pub const MAIN_PLACEHOLDER: &str = "{main}";

/// Values for the placeholders, determined from the code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryPoint {
    /// Replaces [`FILE_PLACEHOLDER`].
    pub file: String,
    /// Replaces [`MAIN_PLACEHOLDER`].
    pub main: String,
}

/// The code has no usable entry point: German `hint`, plus the placeholder
/// values used anyway (compiling still shows the compiler's messages).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryError {
    pub hint: String,
    pub fallback: EntryPoint,
}

/// Entry point of languages without placeholders (fixed file names).
fn fixed_entry(_code: &str) -> Result<EntryPoint, EntryError> {
    Ok(EntryPoint {
        file: "main".into(),
        main: "main".into(),
    })
}

/// Java: the top-level class with `public static void main(String[])`
/// (any name); the file is named after the public class.
fn java_entry(code: &str) -> Result<EntryPoint, EntryError> {
    let program = java::analyze(code);
    let file = program.file_stem().to_string();
    match java::main_class(&program) {
        Ok(main) => Ok(EntryPoint {
            main: program.qualified(main),
            file,
        }),
        Err(hint) => Err(EntryError {
            hint,
            fallback: EntryPoint {
                main: program.qualified(&file),
                file,
            },
        }),
    }
}

/// Registry namespace of the published runner images (GHCR names must be
/// lowercase). `.github/workflows/release.yml` checks that it matches the
/// repository owner it pushes to.
pub const IMAGE_REGISTRY: &str = "ghcr.io/bernhardgoetz";

/// Tag of the runner images = app version (`Cargo.toml`, kept equal to
/// `tauri.conf.json` and `package.json`). The release workflow pushes the
/// images as `<IMAGE_REGISTRY>/code-pruefer-<id>:<version>` for tag `v<version>`.
pub const IMAGE_TAG: &str = env!("CARGO_PKG_VERSION");

/// Developer overrides (`CODE_PRUEFER_<LANG>_IMAGE`, the local `:dev`
/// images) only work in debug builds; release builds run nothing but the
/// embedded or published images.
pub const DEV_OVERRIDES: bool = cfg!(debug_assertions);

#[derive(Debug, Clone)]
pub struct LanguageConfig {
    /// Stable identifier used by the frontend (`"python"`, `"java"`).
    pub id: &'static str,
    /// Display name for the UI.
    pub name: &'static str,
    /// Image reference used when no override env var is set
    /// (`<IMAGE_REGISTRY>/code-pruefer-<id>:<IMAGE_TAG>`).
    pub default_image: &'static str,
    /// Environment variable that overrides the image reference (debug
    /// builds only, see [`DEV_OVERRIDES`]).
    pub image_env: &'static str,
    /// Tag of a locally built image (`docker build -t <local_image> images/<id>`),
    /// used when the default image is neither present nor pullable (debug
    /// builds only).
    pub local_image: &'static str,
    /// File name the source code is stored under inside `/work`; may
    /// contain [`FILE_PLACEHOLDER`].
    pub source_file: &'static str,
    /// File extensions accepted by the upload dialog.
    pub extensions: &'static [&'static str],
    /// Optional compile/syntax-check command (run once before the tests).
    pub compile_cmd: Option<&'static [&'static str]>,
    /// Command executed once per test case, stdin = test input. Both
    /// commands may contain [`FILE_PLACEHOLDER`] and [`MAIN_PLACEHOLDER`].
    pub run_cmd: &'static [&'static str],
    /// Determines the placeholder values from the code.
    pub entry_point: fn(&str) -> Result<EntryPoint, EntryError>,
    pub compile_timeout: Duration,
    pub run_timeout: Duration,
    /// Starter code shown in the editor.
    pub template: &'static str,
}

/// The override `value` of an image env var, if overrides are `allowed`
/// and it differs from `default`. Pure, unit-tested.
pub fn override_from(value: Option<String>, default: &str, allowed: bool) -> Option<String> {
    value
        .filter(|_| allowed)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != default)
}

impl LanguageConfig {
    /// Image set via env var (e.g. `CODE_PRUEFER_PYTHON_IMAGE=code-pruefer-python:dev`),
    /// if it differs from the default image; always `None` in release builds.
    pub fn image_override(&self) -> Option<String> {
        override_from(
            std::env::var(self.image_env).ok(),
            self.default_image,
            DEV_OVERRIDES,
        )
    }

    /// Image reference, overridable via env var.
    pub fn image(&self) -> String {
        self.image_override()
            .unwrap_or_else(|| self.default_image.to_string())
    }

    /// Placeholder values for `code` and, if the code cannot be started,
    /// the German hint (see [`precheck`]).
    pub fn entry(&self, code: &str) -> (EntryPoint, Option<String>) {
        match (self.entry_point)(code) {
            Ok(e) => (e, None),
            Err(e) => (e.fallback, Some(e.hint)),
        }
    }

    fn fill(&self, part: &str, entry: &EntryPoint) -> String {
        part.replace(FILE_PLACEHOLDER, &entry.file)
            .replace(MAIN_PLACEHOLDER, &entry.main)
    }

    /// Source file name for `entry` (e.g. `Time.java`).
    pub fn source_file_for(&self, entry: &EntryPoint) -> String {
        self.fill(self.source_file, entry)
    }

    /// `cmd` (compile or run command) with the placeholders filled in.
    pub fn command(&self, cmd: &[&str], entry: &EntryPoint) -> Vec<String> {
        cmd.iter().map(|p| self.fill(p, entry)).collect()
    }

    /// File name of the bundled image archive for an OCI architecture
    /// (`amd64` / `arm64`), see `runner::images`.
    pub fn archive_name(&self, arch: &str) -> String {
        format!("code-pruefer-{}-{arch}.tar.gz", self.id)
    }
}

pub static LANGUAGES: &[LanguageConfig] = &[
    LanguageConfig {
        id: "python",
        name: "Python 3.12",
        default_image: concat!(
            "ghcr.io/bernhardgoetz/code-pruefer-python:",
            env!("CARGO_PKG_VERSION")
        ),
        image_env: "CODE_PRUEFER_PYTHON_IMAGE",
        local_image: "code-pruefer-python:dev",
        source_file: "main.py",
        extensions: &["py"],
        compile_cmd: Some(&["python", "-m", "py_compile", "main.py"]),
        run_cmd: &["python", "-u", "main.py"],
        entry_point: fixed_entry,
        compile_timeout: Duration::from_secs(30),
        run_timeout: Duration::from_secs(5),
        template: "# Liest eine Zeile von der Standardeingabe und gibt sie aus\nname = input()\nprint(f\"Hallo, {name}!\")\n",
    },
    LanguageConfig {
        id: "java",
        name: "Java 21",
        default_image: concat!(
            "ghcr.io/bernhardgoetz/code-pruefer-java:",
            env!("CARGO_PKG_VERSION")
        ),
        image_env: "CODE_PRUEFER_JAVA_IMAGE",
        local_image: "code-pruefer-java:dev",
        source_file: "{file}.java",
        extensions: &["java"],
        compile_cmd: Some(&[
            "javac",
            "-J-Xmx256m",
            "-J-XX:+UseSerialGC",
            "-J-XX:TieredStopAtLevel=1",
            "-J-XX:-UsePerfData",
            "-encoding",
            "UTF-8",
            // Class files go to package directories (`package a.b;`).
            "-d",
            ".",
            "{file}.java",
        ]),
        run_cmd: &[
            "java",
            "-Xmx256m",
            "-Xss8m",
            "-XX:+UseSerialGC",
            "-XX:TieredStopAtLevel=1",
            "-XX:-UsePerfData",
            "-Dfile.encoding=UTF-8",
            "-Dstdout.encoding=UTF-8",
            "{main}",
        ],
        entry_point: java_entry,
        compile_timeout: Duration::from_secs(30),
        run_timeout: Duration::from_secs(8),
        template: "import java.util.Scanner;\n\n// Der Klassenname ist frei wählbar; gestartet wird die Klasse mit main.\npublic class Main {\n    public static void main(String[] args) {\n        Scanner sc = new Scanner(System.in);\n        String name = sc.nextLine();\n        System.out.println(\"Hallo, \" + name + \"!\");\n    }\n}\n",
    },
];

pub fn get(id: &str) -> Option<&'static LanguageConfig> {
    LANGUAGES.iter().find(|l| l.id == id)
}

/// Language-specific sanity check before running; returns a German hint
/// if the code has no usable entry point (Java: no or several classes with
/// `main`).
pub fn precheck(lang: &LanguageConfig, code: &str) -> Option<String> {
    lang.entry(code).1
}

/// Serializable summary for the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageInfo {
    pub id: String,
    pub name: String,
    pub source_file: String,
    pub extensions: Vec<String>,
    pub template: String,
    pub image: String,
    pub run_timeout_secs: u64,
}

pub fn infos() -> Vec<LanguageInfo> {
    LANGUAGES
        .iter()
        .map(|l| LanguageInfo {
            id: l.id.into(),
            name: l.name.into(),
            // Display only: the real name depends on the code.
            source_file: l.source_file.replace(FILE_PLACEHOLDER, "<Klassenname>"),
            extensions: l.extensions.iter().map(|e| e.to_string()).collect(),
            template: l.template.into(),
            image: l.image(),
            run_timeout_secs: l.run_timeout.as_secs(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_and_defaults() {
        let py = get("python").unwrap();
        assert_eq!(py.source_file, "main.py");
        assert_eq!(py.run_timeout, Duration::from_secs(5));
        let java = get("java").unwrap();
        assert_eq!(java.source_file, "{file}.java");
        assert_eq!(java.run_timeout, Duration::from_secs(8));
        assert!(get("cobol").is_none());
    }

    #[test]
    fn default_images_follow_release_scheme() {
        assert_eq!(IMAGE_REGISTRY, IMAGE_REGISTRY.to_lowercase());
        for l in LANGUAGES {
            assert_eq!(
                l.default_image,
                format!("{IMAGE_REGISTRY}/code-pruefer-{}:{IMAGE_TAG}", l.id)
            );
            assert_eq!(
                l.archive_name("arm64"),
                format!("code-pruefer-{}-arm64.tar.gz", l.id)
            );
        }
    }

    /// The image tag is the Cargo version; the release workflow tags by the
    /// app version, so all three version fields must agree.
    #[test]
    fn versions_are_in_sync() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let version_of = |file: &str| -> String {
            let raw = std::fs::read_to_string(root.join(file)).unwrap();
            let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
            v["version"].as_str().unwrap().to_string()
        };
        assert_eq!(version_of("tauri.conf.json"), IMAGE_TAG);
        assert_eq!(version_of("../package.json"), IMAGE_TAG);
    }

    #[test]
    fn local_images_match_build_instructions() {
        for l in LANGUAGES {
            assert_eq!(l.local_image, format!("code-pruefer-{}:dev", l.id));
        }
    }

    #[test]
    fn overrides_only_in_debug_builds() {
        let default = get("python").unwrap().default_image;
        let set = |v: &str| Some(v.to_string());
        assert_eq!(
            override_from(set(" mein:x "), default, true),
            Some("mein:x".into())
        );
        assert_eq!(override_from(set("mein:x"), default, false), None);
        assert_eq!(override_from(set(""), default, true), None);
        assert_eq!(override_from(set(default), default, true), None);
        assert_eq!(override_from(None, default, true), None);
        // `cargo test` builds with debug assertions, `--release` without.
        assert_eq!(DEV_OVERRIDES, cfg!(debug_assertions));
    }

    #[test]
    fn java_entry_point_from_code() {
        let java = get("java").unwrap();
        let code = "public class Time { public static void main(String... args) { } }";
        let (e, hint) = java.entry(code);
        assert_eq!(hint, None);
        assert_eq!(java.source_file_for(&e), "Time.java");
        let run = java.command(java.run_cmd, &e);
        assert_eq!(run.last().unwrap(), "Time");
        let compile = java.command(java.compile_cmd.unwrap(), &e);
        assert_eq!(compile.last().unwrap(), "Time.java");
        assert!(compile.windows(2).any(|w| w == ["-d", "."]));
        assert!(precheck(java, code).is_none());
        assert!(precheck(java, "class Main{ public static void main(String[] a){} }").is_none());

        // Package: class files in the package directory, qualified start.
        let (e, hint) = java
            .entry("package de.uni;\npublic class Time { public static void main(String[] a) {} }");
        assert_eq!(hint, None);
        assert_eq!(java.source_file_for(&e), "Time.java");
        assert_eq!(
            java.command(java.run_cmd, &e).last().unwrap(),
            "de.uni.Time"
        );

        // No main: hint, file still named after the public class (javac
        // then reports real syntax errors instead of a file-name error).
        let (e, hint) = java.entry("public class Aufgabe1 { }");
        assert!(hint.unwrap().contains("main-Methode"));
        assert_eq!(java.source_file_for(&e), "Aufgabe1.java");
        let two = "class A { public static void main(String[] a) {} }\n\
                   class B { public static void main(String[] a) {} }";
        assert!(precheck(java, two).unwrap().contains("Mehrere Klassen"));
        assert_eq!(java.source_file_for(&java.entry("").0), "Main.java");

        // Python has fixed names.
        let py = get("python").unwrap();
        let (e, hint) = py.entry("print(1)");
        assert_eq!(hint, None);
        assert_eq!(py.source_file_for(&e), "main.py");
        assert_eq!(py.command(py.run_cmd, &e), vec!["python", "-u", "main.py"]);
        assert!(precheck(py, "print(1)").is_none());
    }

    #[test]
    fn template_and_infos() {
        let java = get("java").unwrap();
        assert_eq!(java.entry(java.template).1, None);
        assert_eq!(
            java.source_file_for(&java.entry(java.template).0),
            "Main.java"
        );
        let info = infos();
        let j = info.iter().find(|i| i.id == "java").unwrap();
        assert_eq!(j.source_file, "<Klassenname>.java");
    }
}
