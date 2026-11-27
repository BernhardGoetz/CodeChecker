//! KI-Benchmark: runs the real test generation (`pipeline::generate`) with
//! a real llama-server and the real sandbox on the tasks in
//! `scripts/ki-bench/tasks/*.json` and writes one JSON line per run to
//! `target/ki-bench/<label>.jsonl`. `scripts/ki-bench/evaluate.py` checks
//! the generated tests against the reference programs.
//!
//! `#[ignore]`d; needs what `ai_integration.rs` needs (llama-server, a GGUF
//! model, Docker with the `:dev` runner images). Environment:
//! - `KI_BENCH_MODEL` (or `CODE_PRUEFER_TEST_MODEL`): path to the GGUF file,
//! - `KI_BENCH_LLAMA_SERVER` (or `CODE_PRUEFER_LLAMA_SERVER`): llama-server,
//! - `KI_BENCH_LABEL` (default `bench`): name of the result file,
//! - `KI_BENCH_RUNS` (default 3): runs per task, seeds `KI_BENCH_SEED`,
//!   `KI_BENCH_SEED + 1`, … (default 1),
//! - `KI_BENCH_TASKS`: comma-separated task names (default: all),
//! - `KI_BENCH_SOLUTIONS` (default 3, as in the catalog),
//! - `KI_BENCH_TIMEOUT_S` (default 3600): limit per run.
//!
//! Runs already in the result file (same task and seed) are skipped, so an
//! interrupted benchmark continues where it stopped.
//!
//! ```sh
//! cd src-tauri && cargo test --test ki_bench -- --ignored --nocapture
//! ```

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use code_pruefer_lib::ai::llama::{
    self, ChatRequest, ChatResponse, ChatTemplate, LlamaClient, Llm,
};
use code_pruefer_lib::ai::pipeline::{self, GenerateRequest, SandboxExecutor, Settings};
use code_pruefer_lib::ai::prompts;
use code_pruefer_lib::ai::AiError;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Deserialize)]
struct Task {
    name: String,
    language: String,
    text: String,
    kind: String,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn env_num<T: std::str::FromStr>(name: &str, default: T) -> T {
    env(name).and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("Repo-Wurzel")
        .to_path_buf()
}

fn load_tasks() -> Vec<Task> {
    let dir = repo_root().join("scripts/ki-bench/tasks");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    let only: Option<HashSet<String>> =
        env("KI_BENCH_TASKS").map(|v| v.split(',').map(|s| s.trim().to_string()).collect());
    files
        .iter()
        .map(|p| {
            let raw = std::fs::read_to_string(p).unwrap();
            serde_json::from_str::<Task>(&raw).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
        })
        .filter(|t| only.as_ref().is_none_or(|o| o.contains(&t.name)))
        .collect()
}

/// Kind of a request, recognised like the scripted models of
/// `ai_integration.rs` do.
fn request_kind(req: &ChatRequest) -> &'static str {
    if prompts::is_step(&req.messages, prompts::STRUCTURE_RULES) {
        "structure"
    } else if prompts::is_step(&req.messages, prompts::INPUTS_RULES) {
        "inputs"
    } else if req.prefill.is_some() && req.messages.len() > 2 {
        "repair"
    } else if req.prefill.is_some() {
        "solution"
    } else {
        "other"
    }
}

/// Wraps the real client: gives every request without a seed one derived
/// from the run's seed and the request itself (independent of the order of
/// concurrent requests; the `n` answers of one request still differ, the
/// server derives theirs from it), and records every answer.
struct BenchLlm {
    inner: LlamaClient,
    seed: u64,
    calls: Mutex<Vec<Value>>,
    started: Instant,
}

impl BenchLlm {
    fn seeded(&self, req: &ChatRequest) -> ChatRequest {
        let mut req = req.clone();
        if req.seed.is_none() {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            self.seed.hash(&mut h);
            serde_json::to_string(&req.messages).unwrap().hash(&mut h);
            req.temperature.to_bits().hash(&mut h);
            // llama-server takes the seed as u32.
            req.seed = Some(h.finish() & 0x7fff_ffff);
        }
        req
    }

    fn record(&self, req: &ChatRequest, at: f64, r: &Result<Vec<ChatResponse>, AiError>) {
        let ms = |s: f64| (s * 1000.0).round() as u64;
        let entry = match r {
            Ok(rs) => json!({
                "kind": request_kind(req),
                "seed": req.seed,
                "startMs": ms(at),
                "endMs": ms(self.started.elapsed().as_secs_f64()),
                "answers": rs.iter().map(|a| json!({
                    "promptTokens": a.prompt_tokens,
                    "cachedTokens": a.cached_tokens,
                    "completionTokens": a.completion_tokens,
                    "finishReason": a.finish_reason,
                    "tokensPerSecond": a.tokens_per_second,
                    "content": a.content,
                })).collect::<Vec<_>>(),
            }),
            Err(e) => json!({
                "kind": request_kind(req),
                "seed": req.seed,
                "startMs": ms(at),
                "error": e.to_string(),
            }),
        };
        self.calls.lock().unwrap().push(entry);
    }
}

impl Llm for BenchLlm {
    async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, AiError> {
        let mut rs = self.chat_n(req, 1).await?;
        rs.pop()
            .ok_or_else(|| AiError::Http("Leere Antwort der KI-Laufzeit".into()))
    }

    async fn chat_n(&self, req: &ChatRequest, n: usize) -> Result<Vec<ChatResponse>, AiError> {
        let req = self.seeded(req);
        let at = self.started.elapsed().as_secs_f64();
        let r = self.inner.chat_n(&req, n).await;
        self.record(&req, at, &r);
        r
    }
}

fn done_runs(path: &Path) -> HashSet<(String, u64)> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return HashSet::new();
    };
    raw.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| Some((v["task"].as_str()?.to_string(), v["seed"].as_u64()?)))
        .collect()
}

#[tokio::test]
#[ignore = "benötigt llama-server, ein GGUF-Modell und Docker mit den Runner-Images"]
async fn ki_bench() {
    for (var, img) in [
        ("CODE_PRUEFER_PYTHON_IMAGE", "code-pruefer-python:dev"),
        ("CODE_PRUEFER_JAVA_IMAGE", "code-pruefer-java:dev"),
    ] {
        if std::env::var(var).is_err() {
            std::env::set_var(var, img);
        }
    }
    if let Some(server) = env("KI_BENCH_LLAMA_SERVER") {
        std::env::set_var(llama::SERVER_ENV, server);
    }
    let model = PathBuf::from(
        env("KI_BENCH_MODEL")
            .or_else(|| env("CODE_PRUEFER_TEST_MODEL"))
            .expect("KI_BENCH_MODEL (Pfad zur GGUF-Datei) setzen"),
    );
    let label = env("KI_BENCH_LABEL").unwrap_or_else(|| "bench".into());
    let runs: u64 = env_num("KI_BENCH_RUNS", 3);
    let first_seed: u64 = env_num("KI_BENCH_SEED", 1);
    let solutions: usize = env_num("KI_BENCH_SOLUTIONS", 3);
    let timeout = Duration::from_secs(env_num("KI_BENCH_TIMEOUT_S", 3600));
    let tasks = load_tasks();
    assert!(!tasks.is_empty(), "keine Aufgaben gefunden");

    let out_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/ki-bench");
    std::fs::create_dir_all(&out_dir).unwrap();
    let out_path = out_dir.join(format!("{label}.jsonl"));
    let done = done_runs(&out_path);
    eprintln!(
        "KI-Benchmark {label}: {} Aufgaben × {runs} Läufe, Modell {}, Ergebnis {} ({} schon vorhanden)",
        tasks.len(),
        model.display(),
        out_path.display(),
        done.len()
    );

    let server_cancel = CancellationToken::new();
    let exec = SandboxExecutor::connect(server_cancel.clone(), |m| eprintln!("  [Engine] {m}"))
        .await
        .expect("Container-Engine");
    let model_name = model
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    let server_version = {
        let p = llama::server_path().ok();
        p.and_then(|p| std::fs::read_to_string(p.parent()?.join("VERSION.txt")).ok())
            .and_then(|v| v.split_whitespace().next().map(String::from))
    };

    let mut by_task: HashMap<String, usize> = HashMap::new();
    for seed in first_seed..first_seed + runs {
        for task in &tasks {
            if done.contains(&(task.name.clone(), seed)) {
                continue;
            }
            // Started per run (the same server is reused when it still
            // runs with the same model and slots).
            let endpoint = llama::ensure_running(
                &model,
                solutions + 1,
                ChatTemplate::FromModel,
                &server_cancel,
                |m| eprintln!("  [Modell] {m}"),
            )
            .await
            .expect("llama-server starten");
            let run_cancel = CancellationToken::new();
            let client = LlamaClient::new(endpoint, run_cancel.clone()).unwrap();
            let llm = BenchLlm {
                inner: client,
                seed,
                calls: Mutex::new(Vec::new()),
                started: Instant::now(),
            };
            let req = GenerateRequest {
                task_text: task.text.clone(),
                language: task.language.clone(),
                compare_mode: None,
            };
            eprintln!("== {} (Seed {seed}) …", task.name);
            let started = Instant::now();
            let settings = Settings {
                solutions,
                repair_rounds: 1,
                speeds: None,
            };
            let canceller = run_cancel.clone();
            let r = tokio::select! {
                r = pipeline::generate(&llm, &exec, &req, settings, &run_cancel, |p| {
                    eprintln!("  [{:?}] {}", p.phase, p.message)
                }) => r,
                _ = tokio::time::sleep(timeout) => {
                    canceller.cancel();
                    Err(AiError::Other(format!("Zeitlimit {} s überschritten", timeout.as_secs())))
                }
            };
            let wall_ms = started.elapsed().as_millis() as u64;
            let calls = llm.calls.into_inner().unwrap();
            let solution_answers: Vec<&Value> = calls
                .iter()
                .filter(|c| matches!(c["kind"].as_str(), Some("solution" | "repair")))
                .flat_map(|c| c["answers"].as_array().into_iter().flatten())
                .collect();
            let finish_reasons: Vec<Value> = solution_answers
                .iter()
                .map(|a| a["finishReason"].clone())
                .collect();
            let mut line = json!({
                "label": label,
                "task": task.name,
                "language": task.language,
                "kind": task.kind,
                "seed": seed,
                "solutions": solutions,
                "model": model_name,
                "llamaServer": server_version,
                "wallMs": wall_ms,
                "solutionFinishReasons": finish_reasons,
                "calls": calls,
            });
            match &r {
                Ok(res) => {
                    let s = &res.stats;
                    eprintln!(
                        "   -> {:?}, {} Tests, {:.0} s, Prompt {} / Cache {} / Ausgabe {} Tokens, {} abgeschnitten\n   Gründe: {:?}",
                        res.confidence.level,
                        res.tests.len(),
                        wall_ms as f64 / 1000.0,
                        s.prompt_tokens,
                        s.cached_tokens,
                        s.completion_tokens,
                        s.solutions_truncated,
                        res.confidence.reasons
                    );
                    line["level"] = serde_json::to_value(res.confidence.level).unwrap();
                    line["unsupported"] = serde_json::to_value(res.unsupported).unwrap();
                    line["ioAssumptionNote"] = json!(res.io_assumption_note);
                    line["variationSuggested"] = json!(res.variation_suggested);
                    line["reasons"] = json!(res.confidence.reasons);
                    line["testCount"] = json!(res.tests.len());
                    line["tests"] = json!(res
                        .tests
                        .iter()
                        .map(|t| json!({
                            "name": t.name,
                            "input": t.input,
                            "expected": t.expected,
                            "source": t.source,
                            "agreement": t.agreement,
                        }))
                        .collect::<Vec<_>>());
                    line["compareMode"] = serde_json::to_value(res.compare_mode).unwrap();
                    line["referenceSolution"] = json!(res.reference_solution);
                    line["phaseMs"] = serde_json::to_value(&s.phase_ms).unwrap();
                    line["promptTokens"] = json!(s.prompt_tokens);
                    line["cachedTokens"] = json!(s.cached_tokens);
                    line["completionTokens"] = json!(s.completion_tokens);
                    line["solutionTokens"] = json!(s.solution_tokens);
                    line["solutionsTruncated"] = json!(s.solutions_truncated);
                    line["stats"] = serde_json::to_value(s).unwrap();
                }
                Err(e) => {
                    eprintln!("   -> Fehler: {e}");
                    line["error"] = json!(e.to_string());
                }
            }
            // Appended and flushed per run: an interrupted benchmark keeps
            // everything measured so far.
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&out_path)
                .unwrap();
            writeln!(f, "{line}").unwrap();
            f.sync_all().unwrap();
            *by_task.entry(task.name.clone()).or_default() += 1;
        }
    }
    eprintln!("Fertig: {by_task:?} -> {}", out_path.display());
    llama::shutdown().await;
}
