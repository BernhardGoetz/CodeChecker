//! Remaining time of a test generation ("noch ca. 4–6 Min."), adaptive and
//! rather pessimistic.
//!
//! The estimate is work divided by measured speed:
//!
//! ```text
//! remaining = prompt tokens left / prompt speed
//!           + output tokens left / generation speed
//!           + sandbox seconds left
//! ```
//!
//! - The speeds are the **throughput of the whole server** (all parallel
//!   slots together), not the speed of one slot multiplied by the number of
//!   slots: a [`Meter`] measures them on a processor-sharing clock (while
//!   `c` requests run at once, each one is charged `1/c` of the wall time),
//!   so the charged times of all requests add up to the busy wall time.
//! - Before anything is measured, the speeds of the last run of the same
//!   model are used (stored by the caller), else conservative defaults
//!   ([`default_speeds`]). Measurements replace them gradually
//!   ([`Meter::blended`]).
//! - The running step is never estimated shorter than the time it already
//!   took ([`OVERRUN_SHARE`]).
//! - The range is `[estimate, estimate × SAFETY_FACTOR]`.

use serde::{Deserialize, Serialize};

/// Upper end of the range: estimate × this.
pub const SAFETY_FACTOR: f64 = 1.3;
/// A step that already ran longer than estimated is assumed to need
/// this share of its elapsed time more.
pub const OVERRUN_SHARE: f64 = 0.15;
/// Output tokens after which measured speeds fully replace the prior.
pub const BLEND_TOKENS: f64 = 300.0;
/// Least charged time and output for a measurement to count.
pub const MIN_MEASURED_SECONDS: f64 = 2.0;
pub const MIN_MEASURED_TOKENS: f64 = 50.0;

/// Throughput of llama-server in tokens per second (all slots together).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Speeds {
    pub prompt_tps: f64,
    pub gen_tps: f64,
}

impl Speeds {
    fn valid(self) -> bool {
        self.prompt_tps.is_finite()
            && self.gen_tps.is_finite()
            && self.prompt_tps > 0.0
            && self.gen_tps > 0.0
    }
}

/// Conservative speeds without a previous run: slow CPUs without AVX-512,
/// several slots at once. Small models (1.5B and below) are faster.
pub fn default_speeds(model_id: &str) -> Speeds {
    let id = model_id.to_lowercase();
    let small = ["0.5b", "1b", "1.5b", "-1.5", "0.5-", "1.5-"]
        .iter()
        .any(|k| id.contains(k));
    if small {
        Speeds {
            prompt_tps: 18.0,
            gen_tps: 4.0,
        }
    } else {
        Speeds {
            prompt_tps: 6.0,
            gen_tps: 1.5,
        }
    }
}

/// Work of a step: tokens the server has to process and sandbox time.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Work {
    pub prompt_tokens: f64,
    pub gen_tokens: f64,
    pub sandbox_seconds: f64,
}

impl Work {
    pub fn seconds(&self, s: Speeds) -> f64 {
        self.prompt_tokens / s.prompt_tps + self.gen_tokens / s.gen_tps + self.sandbox_seconds
    }

    pub fn plus(self, o: Work) -> Work {
        Work {
            prompt_tokens: self.prompt_tokens + o.prompt_tokens,
            gen_tokens: self.gen_tokens + o.gen_tokens,
            sandbox_seconds: self.sandbox_seconds + o.sandbox_seconds,
        }
    }

    pub fn times(self, k: f64) -> Work {
        Work {
            prompt_tokens: self.prompt_tokens * k,
            gen_tokens: self.gen_tokens * k,
            sandbox_seconds: self.sandbox_seconds * k,
        }
    }
}

/// Remaining time in seconds, `low ≤ high`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EtaRange {
    pub low: u64,
    pub high: u64,
}

/// Remaining time: the rest of the running step (`current`, running for
/// `elapsed` seconds) plus the `later` steps. The running step takes at
/// least its elapsed time plus [`OVERRUN_SHARE`] of it. Pure.
pub fn eta_range(current: Work, elapsed: f64, later: Work, speeds: Speeds) -> Option<EtaRange> {
    if !speeds.valid() {
        return None;
    }
    let elapsed = elapsed.max(0.0);
    let step = current.seconds(speeds).max(elapsed * (1.0 + OVERRUN_SHARE));
    let rest = (step - elapsed) + later.seconds(speeds);
    let low = rest.max(1.0);
    Some(EtaRange {
        // Tolerance against rounding errors (115 % of 100 s is 115 s).
        low: (low + 1e-6).floor() as u64,
        high: (low * SAFETY_FACTOR - 1e-6).ceil() as u64,
    })
}

/// Measures the server throughput from finished requests (see the module
/// docs). Times are seconds since any fixed start; pure, no clock inside.
#[derive(Debug, Clone, Default)]
pub struct Meter {
    /// Requests in flight.
    active: usize,
    /// Processor-sharing clock: advances by `dt / active` while busy.
    virtual_s: f64,
    last_s: f64,
    /// Totals over the finished requests.
    charged_s: f64,
    prompt_tokens: f64,
    gen_tokens: f64,
    /// Per-slot timings (for the ratio of prompt to generation speed).
    slot_prompt_ms: f64,
    slot_prompt_tokens: f64,
    slot_gen_s: f64,
    slot_gen_tokens: f64,
}

/// What a finished request reports (llama-server `timings`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Sample {
    pub prompt_tokens: Option<u64>,
    pub prompt_ms: Option<f64>,
    pub gen_tokens: Option<u64>,
    /// Generation speed of the slot (`predicted_per_second`).
    pub gen_tps: Option<f64>,
}

impl Sample {
    /// One request with `n` answers (llama-server `"n"`): the prompt was
    /// processed once (every child slot reports the same prompt timings,
    /// counted once here), the answers were generated at the same time
    /// (output tokens and slot speeds add up). `None` for no answers. Pure.
    pub fn combine(samples: &[Sample]) -> Option<Sample> {
        let max_u = |f: fn(&Sample) -> Option<u64>| samples.iter().filter_map(f).max();
        let sum_u = |f: fn(&Sample) -> Option<u64>| {
            let v: Vec<u64> = samples.iter().filter_map(f).collect();
            (!v.is_empty()).then(|| v.iter().sum())
        };
        let prompt_ms = samples
            .iter()
            .filter_map(|s| s.prompt_ms)
            .fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.max(x))));
        let tps: Vec<f64> = samples.iter().filter_map(|s| s.gen_tps).collect();
        (!samples.is_empty()).then(|| Sample {
            prompt_tokens: max_u(|s| s.prompt_tokens),
            prompt_ms,
            gen_tokens: sum_u(|s| s.gen_tokens),
            gen_tps: (!tps.is_empty()).then(|| tps.iter().sum()),
        })
    }
}

/// Wall time covered by the intervals `(start, end)` (overlaps counted
/// once): requests running at the same time process their prompts in the
/// same batches, so their prompt times must not be added up. Pure.
pub fn covered_seconds(spans: &[(f64, f64)]) -> f64 {
    let mut v: Vec<(f64, f64)> = spans
        .iter()
        .copied()
        .filter(|(a, b)| a.is_finite() && b.is_finite() && b > a)
        .collect();
    v.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut total = 0.0;
    let mut current: Option<(f64, f64)> = None;
    for (a, b) in v {
        match current {
            Some((ca, cb)) if a <= cb => current = Some((ca, cb.max(b))),
            Some((ca, cb)) => {
                total += cb - ca;
                current = Some((a, b));
            }
            None => current = Some((a, b)),
        }
    }
    total + current.map_or(0.0, |(a, b)| b - a)
}

impl Meter {
    fn advance(&mut self, now: f64) {
        let dt = (now - self.last_s).max(0.0);
        if self.active > 0 {
            self.virtual_s += dt / self.active as f64;
        }
        self.last_s = self.last_s.max(now);
    }

    /// A request starts; returns its start on the clock.
    pub fn start(&mut self, now: f64) -> f64 {
        self.advance(now);
        self.active += 1;
        self.virtual_s
    }

    /// A request started at `started` (from [`Meter::start`]) ends;
    /// `sample` is `None` for a failed or cancelled request.
    pub fn finish(&mut self, now: f64, started: f64, sample: Option<Sample>) {
        self.advance(now);
        self.active = self.active.saturating_sub(1);
        let Some(s) = sample else {
            return;
        };
        let prompt = s.prompt_tokens.unwrap_or(0) as f64;
        let gen = s.gen_tokens.unwrap_or(0) as f64;
        self.charged_s += (self.virtual_s - started).max(0.0);
        self.prompt_tokens += prompt;
        self.gen_tokens += gen;
        if let Some(ms) = s.prompt_ms.filter(|m| *m > 0.0 && prompt > 0.0) {
            self.slot_prompt_ms += ms;
            self.slot_prompt_tokens += prompt;
        }
        if let Some(tps) = s.gen_tps.filter(|t| *t > 0.0 && gen > 0.0) {
            self.slot_gen_s += gen / tps;
            self.slot_gen_tokens += gen;
        }
    }

    /// Measured throughput, `None` until enough was measured. The charged
    /// time is split into prompt and generation by the ratio of the two
    /// per-slot speeds (`prior` gives the ratio without prompt timings):
    /// `charged = P / (r · g) + G / g`.
    pub fn measured(&self, prior: Speeds) -> Option<Speeds> {
        if self.charged_s < MIN_MEASURED_SECONDS || self.gen_tokens < MIN_MEASURED_TOKENS {
            return None;
        }
        let ratio = if self.slot_prompt_ms > 0.0 && self.slot_gen_s > 0.0 {
            (self.slot_prompt_tokens / (self.slot_prompt_ms / 1000.0))
                / (self.slot_gen_tokens / self.slot_gen_s)
        } else {
            prior.prompt_tps / prior.gen_tps
        };
        if !ratio.is_finite() || ratio <= 0.0 {
            return None;
        }
        let gen_tps = (self.prompt_tokens / ratio + self.gen_tokens) / self.charged_s;
        let s = Speeds {
            prompt_tps: ratio * gen_tps,
            gen_tps,
        };
        s.valid().then_some(s)
    }

    /// The prior, replaced by the measurement step by step (fully after
    /// [`BLEND_TOKENS`] output tokens).
    pub fn blended(&self, prior: Speeds) -> Speeds {
        let Some(m) = self.measured(prior) else {
            return prior;
        };
        let w = (self.gen_tokens / BLEND_TOKENS).min(1.0);
        Speeds {
            prompt_tps: w * m.prompt_tps + (1.0 - w) * prior.prompt_tps,
            gen_tps: w * m.gen_tps + (1.0 - w) * prior.gen_tps,
        }
    }
}

/// "ca. 4–6 Min." / "ca. 40–55 s" (German, as in the frontend). Pure.
pub fn format_range(r: EtaRange) -> String {
    if r.high < 90 {
        let low = (r.low / 5) * 5;
        let high = r.high.div_ceil(5) * 5;
        if low == high || low == 0 {
            format!("ca. {high} s")
        } else {
            format!("ca. {low}–{high} s")
        }
    } else {
        let low = (r.low / 60).max(1);
        let high = r.high.div_ceil(60).max(low);
        if low == high {
            format!("ca. {low} Min.")
        } else {
            format!("ca. {low}–{high} Min.")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLOW: Speeds = Speeds {
        prompt_tps: 6.0,
        gen_tps: 1.5,
    };

    #[test]
    fn answers_of_one_request_are_combined() {
        // Three answers of one request: prompt once, outputs added up.
        let one = |gen: u64| Sample {
            prompt_tokens: Some(600),
            prompt_ms: Some(30_000.0),
            gen_tokens: Some(gen),
            gen_tps: Some(4.0),
        };
        let c = Sample::combine(&[one(300), one(250), one(350)]).unwrap();
        assert_eq!(c.prompt_tokens, Some(600));
        assert_eq!(c.prompt_ms, Some(30_000.0));
        assert_eq!(c.gen_tokens, Some(900));
        assert_eq!(c.gen_tps, Some(12.0));
        assert!(Sample::combine(&[]).is_none());
        let empty = Sample::combine(&[Sample::default()]).unwrap();
        assert_eq!(
            (
                empty.prompt_tokens,
                empty.prompt_ms,
                empty.gen_tokens,
                empty.gen_tps
            ),
            (None, None, None, None)
        );
        // The meter then measures the server throughput: 600 prompt
        // tokens and 900 output tokens in 105 s of wall time.
        let mut m = Meter::default();
        let s = m.start(0.0);
        m.finish(105.0, s, Some(c));
        let v = m.measured(SLOW).unwrap();
        let wall = 600.0 / v.prompt_tps + 900.0 / v.gen_tps;
        assert!((wall - 105.0).abs() < 1e-6, "{wall}");
    }

    #[test]
    fn overlapping_prompt_times_count_once() {
        // Three requests processing their prompts in the same batches.
        assert_eq!(
            covered_seconds(&[(0.0, 30.0), (0.0, 30.0), (1.0, 31.0)]),
            31.0
        );
        assert_eq!(covered_seconds(&[(10.0, 20.0), (0.0, 5.0)]), 15.0);
        assert_eq!(covered_seconds(&[(0.0, 10.0), (2.0, 4.0)]), 10.0);
        assert_eq!(covered_seconds(&[]), 0.0);
        assert_eq!(covered_seconds(&[(5.0, 5.0), (3.0, 1.0)]), 0.0);
    }

    #[test]
    fn defaults_by_model_size() {
        assert_eq!(default_speeds("qwen2.5-coder-7b"), SLOW);
        assert_eq!(default_speeds("qwen2.5-coder-1.5b").gen_tps, 4.0);
        assert_eq!(default_speeds("qwen2.5-coder-1.5b").prompt_tps, 18.0);
        // Unknown imported models: the slow assumption.
        assert_eq!(default_speeds("import-abc"), SLOW);
    }

    #[test]
    fn range_from_work_and_speeds() {
        // 600 prompt tokens at 6/s + 150 output tokens at 1.5/s + 10 s.
        let w = Work {
            prompt_tokens: 600.0,
            gen_tokens: 150.0,
            sandbox_seconds: 10.0,
        };
        let r = eta_range(w, 0.0, Work::default(), SLOW).unwrap();
        assert_eq!(
            r,
            EtaRange {
                low: 210,
                high: 273
            }
        );
        // Elapsed time is subtracted from the running step only.
        let r = eta_range(w, 60.0, w, SLOW).unwrap();
        assert_eq!(r.low, 150 + 210);
        assert!(eta_range(
            w,
            0.0,
            w,
            Speeds {
                prompt_tps: 0.0,
                gen_tps: 1.0
            }
        )
        .is_none());
    }

    #[test]
    fn running_step_is_never_shorter_than_its_elapsed_time() {
        let w = Work {
            gen_tokens: 15.0,
            ..Work::default()
        };
        // Estimated 10 s, already 100 s: 15 s more (never 0 or negative).
        let r = eta_range(w, 100.0, Work::default(), SLOW).unwrap();
        assert_eq!(r.low, 15);
        assert_eq!(r.high, 20);
        // Tiny rest: at least one second.
        let r = eta_range(Work::default(), 0.0, Work::default(), SLOW).unwrap();
        assert_eq!(r, EtaRange { low: 1, high: 2 });
    }

    #[test]
    fn parallel_requests_share_the_measured_time() {
        // Three requests of 300 output tokens run together from 0 s to
        // 100 s: the server did 900 tokens in 100 s → 9 tokens/s in total
        // (not 3 × the per-slot speed of one request measured alone).
        let mut m = Meter::default();
        let starts: Vec<f64> = (0..3).map(|_| m.start(0.0)).collect();
        for s in starts {
            m.finish(
                100.0,
                s,
                Some(Sample {
                    prompt_tokens: Some(0),
                    prompt_ms: None,
                    gen_tokens: Some(300),
                    gen_tps: Some(3.0),
                }),
            );
        }
        let prior = Speeds {
            prompt_tps: 30.0,
            gen_tps: 10.0,
        };
        let s = m.measured(prior).unwrap();
        assert!((s.gen_tps - 9.0).abs() < 1e-9, "{s:?}");
        // Prompt speed from the prior ratio (no prompt timings).
        assert!((s.prompt_tps - 27.0).abs() < 1e-9);
        // 900 output tokens: fully measured.
        assert_eq!(m.blended(prior), s);
    }

    #[test]
    fn prompt_and_generation_are_split_by_the_slot_ratio() {
        // One request: 400 prompt tokens in 20 s (20/s), 100 output tokens at
        // 5/s (20 s) → 40 s wall time.
        let mut m = Meter::default();
        let s0 = m.start(10.0);
        m.finish(
            50.0,
            s0,
            Some(Sample {
                prompt_tokens: Some(400),
                prompt_ms: Some(20_000.0),
                gen_tokens: Some(100),
                gen_tps: Some(5.0),
            }),
        );
        let s = m.measured(SLOW).unwrap();
        assert!((s.prompt_tps - 20.0).abs() < 1e-9, "{s:?}");
        assert!((s.gen_tps - 5.0).abs() < 1e-9);
        // 100 of 300 tokens: one third measured, two thirds prior.
        let b = m.blended(SLOW);
        assert!((b.gen_tps - (5.0 / 3.0 + 1.0)).abs() < 1e-9, "{b:?}");
    }

    #[test]
    fn overlapping_requests_are_charged_by_share() {
        // A runs 0–10 s alone and 10–20 s with B; B runs 10–30 s.
        let mut m = Meter::default();
        let a = m.start(0.0);
        let b = m.start(10.0);
        let sample = Sample {
            gen_tokens: Some(100),
            gen_tps: Some(5.0),
            ..Sample::default()
        };
        m.finish(20.0, a, Some(sample));
        // A was charged 10 + 5 = 15 s.
        assert!((m.charged_s - 15.0).abs() < 1e-9);
        m.finish(30.0, b, Some(sample));
        // Both together: the 30 s of wall time.
        assert!((m.charged_s - 30.0).abs() < 1e-9);
        // Failed requests end without data.
        let c = m.start(30.0);
        m.finish(40.0, c, None);
        assert!((m.charged_s - 30.0).abs() < 1e-9);
        assert_eq!(m.active, 0);
    }

    #[test]
    fn too_little_data_keeps_the_prior() {
        let mut m = Meter::default();
        let s = m.start(0.0);
        m.finish(
            0.01,
            s,
            Some(Sample {
                gen_tokens: Some(10),
                gen_tps: Some(1000.0),
                ..Sample::default()
            }),
        );
        assert_eq!(m.measured(SLOW), None);
        assert_eq!(m.blended(SLOW), SLOW);
    }

    #[test]
    fn range_text() {
        assert_eq!(
            format_range(EtaRange {
                low: 240,
                high: 330
            }),
            "ca. 4–6 Min."
        );
        assert_eq!(
            format_range(EtaRange {
                low: 300,
                high: 300
            }),
            "ca. 5 Min."
        );
        assert_eq!(format_range(EtaRange { low: 42, high: 55 }), "ca. 40–55 s");
        assert_eq!(format_range(EtaRange { low: 1, high: 2 }), "ca. 5 s");
    }
}
