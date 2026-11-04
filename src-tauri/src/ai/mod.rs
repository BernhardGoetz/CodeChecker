//! Local AI: llama.cpp sidecar (`llama`), model management (`models`), the
//! test generation pipeline (`pipeline`, `prompts`) and corrections of
//! OCR text (`ocr_fix`).
//!
//! Everything runs on this machine; the only network access is the model
//! download from huggingface.co (`models`).

pub mod gguf;
pub mod llama;
pub mod models;
pub mod ocr_fix;
pub mod pipeline;
pub mod prompts;
pub mod variation;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::Serialize;
use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error(
        "KI-Laufzeit nicht gefunden ({0}). Bitte den Code-Prüfer neu installieren; in \
         Entwickler-Builds llama-server mit „scripts/fetch-llama-server.sh“ holen oder \
         CODE_PRUEFER_LLAMA_SERVER setzen."
    )]
    RuntimeMissing(String),
    #[error("Die KI-Laufzeit konnte nicht gestartet werden: {0}")]
    Start(String),
    #[error("Fehler bei der Anfrage an die KI-Laufzeit: {0}")]
    Http(String),
    #[error("Abgebrochen.")]
    Cancelled,
    #[error("{0}")]
    Other(String),
}

impl Serialize for AiError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

/// Kind of an AI operation: `cancel_variations` cancels only variation
/// checks, `cancel_ai` everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    General,
    Variation,
}

/// Cancellation tokens of the AI operations in flight (test generation,
/// OCR correction, model download, variation check); `cancel_ai` cancels
/// all of them.
static OPERATIONS: Mutex<Vec<(u64, OperationKind, CancellationToken)>> = Mutex::new(Vec::new());
static NEXT_OP: AtomicU64 = AtomicU64::new(1);

/// A registered operation; deregisters itself when dropped.
pub struct Operation {
    id: u64,
    pub token: CancellationToken,
}

impl Operation {
    pub fn start() -> Self {
        Self::start_kind(OperationKind::General)
    }

    pub fn start_kind(kind: OperationKind) -> Self {
        let id = NEXT_OP.fetch_add(1, Ordering::Relaxed);
        let token = CancellationToken::new();
        if let Ok(mut ops) = OPERATIONS.lock() {
            ops.push((id, kind, token.clone()));
        }
        Operation { id, token }
    }
}

impl Drop for Operation {
    fn drop(&mut self) {
        if let Ok(mut ops) = OPERATIONS.lock() {
            ops.retain(|(id, _, _)| *id != self.id);
        }
    }
}

/// Cancels every running AI operation. Returns how many were running.
pub fn cancel_all() -> usize {
    cancel_where(|_| true)
}

/// Cancels the running operations of one kind (e.g. only variation checks,
/// llama-server keeps running). Returns how many were running.
pub fn cancel_kind(kind: OperationKind) -> usize {
    cancel_where(|k| k == kind)
}

fn cancel_where(pred: impl Fn(OperationKind) -> bool) -> usize {
    OPERATIONS
        .lock()
        .map(|ops| {
            let mut n = 0;
            for (_, _, t) in ops.iter().filter(|(_, k, _)| pred(*k)) {
                t.cancel();
                n += 1;
            }
            n
        })
        .unwrap_or(0)
}

/// Lower-case hex encoding.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The operation tests cancel globally; they must not overlap.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn operations_register_and_cancel() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let a = Operation::start();
        let b = Operation::start();
        assert!(cancel_all() >= 2);
        assert!(a.token.is_cancelled() && b.token.is_cancelled());
        let id = a.id;
        drop(a);
        assert!(!OPERATIONS.lock().unwrap().iter().any(|(i, _, _)| *i == id));
    }

    #[test]
    fn cancel_kind_only_hits_that_kind() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let general = Operation::start();
        let variation = Operation::start_kind(OperationKind::Variation);
        assert!(cancel_kind(OperationKind::Variation) >= 1);
        assert!(variation.token.is_cancelled());
        assert!(!general.token.is_cancelled());
    }

    #[test]
    fn hex_is_lowercase() {
        assert_eq!(hex(&[0, 0xab, 0x10]), "00ab10");
    }

    #[test]
    fn errors_are_german_strings() {
        let v = serde_json::to_value(AiError::Cancelled).unwrap();
        assert_eq!(v, "Abgebrochen.");
        assert!(AiError::RuntimeMissing("x".into())
            .to_string()
            .starts_with("KI-Laufzeit nicht gefunden"));
    }
}
