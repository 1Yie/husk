//! `semantic` — local sentence embeddings for recall (`semantic` feature).
//!
//! The hash embedder in `store` is a bag of hashed tokens: deterministic,
//! dependency-free, and blind to a re-phrasing that shares no tokens — a
//! Chinese sentence and its English translation measure 0.08. A small
//! multilingual sentence encoder scores the same pair at 0.71–0.80, which is
//! what makes recall work across languages.
//!
//! It is **not** a drop-in replacement for the hash vector: a sentence and its
//! reordering ("先跑 fmt 再 test" / "先跑 test 再 fmt") embed at 0.97, so
//! dedupe and persona folding stay in the hash space. Here it only scores
//! recall — see `MemoryStore::recall_scored`.
//!
//! Loading is lazy and off the caller's thread: the model is ~250 MB, fetched
//! from HuggingFace on first use into the app's data directory, ~35 s once and
//! ~1 s on later starts. Every query before it is ready — or after a failed
//! load — simply uses the hash path.
//!
//! The fetch is the one step a user waits on, so its state is observable
//! ([`progress`]): the loader counts bytes that actually landed in the cache
//! directory — `fastembed` reports no callback, but the files it writes do
//! have a size — and the settings dialog draws that number.

/// Where the fetch/load stands, as the settings pane shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// The switch is off.
    Off,
    /// On, but the loader has not started (nothing asked for a vector yet).
    Idle,
    /// Fetching the weights.
    Downloading,
    /// Weights on disk, building the session.
    Loading,
    /// Usable.
    Ready,
    /// The fetch or the load failed; `error` says why.
    Failed,
}

/// One observation of the fetch.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Progress {
    pub phase: Phase,
    /// Bytes of the model present in the cache directory.
    pub bytes: u64,
    /// The finished model's size — an estimate for the bar, never a promise.
    pub total: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(feature = "semantic")]
mod imp {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
    use std::sync::{Mutex, OnceLock};

    use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};

    use super::{Phase, Progress};

    /// Multilingual small model: 384 dims, quantized.
    const MODEL: EmbeddingModel = EmbeddingModel::ParaphraseMLMiniLML12V2Q;

    /// The finished model's size on disk, measured after a first fetch. Only
    /// drives the progress bar — a wrong estimate shows as a bar that fills
    /// early or late, never as a wrong number in the log.
    const MODEL_BYTES: u64 = 241 * 1024 * 1024;

    /// Below this fraction of [`MODEL_BYTES`] the weights are still arriving;
    /// above it `try_new` is building the session.
    const FETCHED: f64 = 0.98;

    /// The settings switch, in front of the model. Off means "do not use the
    /// model even though it is loaded" — the weights stay in memory, so
    /// turning it back on costs nothing.
    static ENABLED: AtomicBool = AtomicBool::new(false);

    /// [`Phase`] as a byte — read by the settings poll, written by the loader.
    static PHASE: AtomicU8 = AtomicU8::new(0);
    static ERROR: Mutex<Option<String>> = Mutex::new(None);
    /// Where the weights live. `fastembed` also honours `HF_HOME`, so the byte
    /// counter resolves the same directory it writes to.
    static CACHE: OnceLock<PathBuf> = OnceLock::new();

    /// Set once a load has been started — a failed load is not retried, or
    /// every recall would pay for the same download attempt.
    static STARTED: AtomicBool = AtomicBool::new(false);
    /// The loaded model. `embed` needs `&mut`, and one caller at a time is
    /// fine: recall is a single query per message.
    static MODEL_READY: OnceLock<Mutex<TextEmbedding>> = OnceLock::new();

    /// Start loading the model in the background, once per process. `cache_dir`
    /// holds the weights between runs.
    pub fn init(cache_dir: PathBuf) {
        if STARTED.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = CACHE.set(cache_dir.clone());
        set_phase(Phase::Downloading);
        std::thread::Builder::new()
            .name("semantic-embed".into())
            .spawn(move || {
                let started = std::time::Instant::now();
                let options = TextInitOptions::new(MODEL).with_cache_dir(cache_dir);
                match TextEmbedding::try_new(options) {
                    Ok(model) => {
                        let _ = MODEL_READY.set(Mutex::new(model));
                        set_phase(Phase::Ready);
                        tracing::info!(ms = started.elapsed().as_millis(), "semantic recall ready");
                    }
                    // Offline, a blocked host, a full disk: recall keeps
                    // working on the hash vector. The dialog says why; the
                    // chat keeps working.
                    Err(error) => {
                        if let Ok(mut slot) = ERROR.lock() {
                            *slot = Some(error.to_string());
                        }
                        set_phase(Phase::Failed);
                        tracing::warn!(
                            %error,
                            "semantic embedding model unavailable; recall stays on the hash vector"
                        );
                    }
                }
            })
            .ok();
    }

    fn set_phase(phase: Phase) {
        let byte = match phase {
            Phase::Off => 0,
            Phase::Idle => 1,
            Phase::Downloading => 2,
            Phase::Loading => 3,
            Phase::Ready => 4,
            Phase::Failed => 5,
        };
        PHASE.store(byte, Ordering::Relaxed);
    }

    fn phase_of(byte: u8) -> Phase {
        match byte {
            1 => Phase::Idle,
            2 => Phase::Downloading,
            3 => Phase::Loading,
            4 => Phase::Ready,
            5 => Phase::Failed,
            _ => Phase::Off,
        }
    }

    /// Bytes of the model present in the cache directory. `hf-hub` writes into
    /// the target directory as it streams, so this counts what actually
    /// arrived rather than what was requested.
    ///
    /// Symlinks are skipped: the hub keeps the weights in `blobs/` and links
    /// them from `snapshots/`, so following either the link or its metadata
    /// (which does follow) counts the same file twice and pins the bar at 200%.
    fn bytes_on_disk(dir: &Path) -> u64 {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return 0;
        };
        let mut total = 0;
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                total += bytes_on_disk(&entry.path());
            } else {
                total += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
        total
    }

    /// Where the fetch currently is. Reads no model state: the phase says
    /// whether the loader is running, the directory says how far it got.
    pub fn progress() -> Progress {
        let mut phase = phase_of(PHASE.load(Ordering::Relaxed));
        if !ENABLED.load(Ordering::Relaxed) && phase != Phase::Failed {
            phase = Phase::Off;
        }
        let bytes = CACHE.get().map(|d| bytes_on_disk(d)).unwrap_or(0);
        // The weights are all there and `try_new` is still working — say so,
        // instead of pinning the bar at 100% while nothing moves.
        if phase == Phase::Downloading && bytes as f64 >= MODEL_BYTES as f64 * FETCHED {
            phase = Phase::Loading;
        }
        Progress {
            phase,
            bytes,
            total: MODEL_BYTES,
            error: ERROR.lock().ok().and_then(|e| e.clone()),
        }
    }

    /// The vector for `text`, or `None` while the model is still loading (or
    /// after a load that failed).
    pub fn embed(text: &str) -> Option<Vec<f32>> {
        if !ENABLED.load(Ordering::Relaxed) {
            return None;
        }
        let model = MODEL_READY.get()?;
        let mut model = model.lock().unwrap_or_else(|e| e.into_inner());
        model
            .embed(vec![text], None)
            .ok()
            .and_then(|mut v| v.drain(..).next())
    }

    /// Vectors for `texts`, one encoder call for the batch. This is what the
    /// store's catch-up pass uses: a call per fact turned the first recall
    /// after the model landed into seconds of inference.
    ///
    /// The answer is shaped like the question — an entry is `None` when the
    /// model is not usable or that one text failed — so the caller can book
    /// exactly what landed.
    pub fn embed_batch(texts: &[String]) -> Vec<Option<Vec<f32>>> {
        let mut out = vec![None; texts.len()];
        if texts.is_empty() || !ENABLED.load(Ordering::Relaxed) {
            return out;
        }
        let Some(model) = MODEL_READY.get() else {
            return out;
        };
        let mut model = model.lock().unwrap_or_else(|e| e.into_inner());
        match model.embed(texts.to_vec(), None) {
            Ok(vectors) => {
                for (slot, vector) in out.iter_mut().zip(vectors) {
                    *slot = Some(vector);
                }
                out
            }
            Err(error) => {
                tracing::warn!(%error, "semantic batch embedding failed");
                out
            }
        }
    }

    /// Identifies the encoder behind [`embed`]. Stored with every vector: two
    /// models do not score into the same space, and they may well agree on the
    /// width, so the dimensionality is no test of compatibility.
    pub fn model_id() -> String {
        format!("{MODEL:?}")
    }

    /// Whether a query right now would be scored semantically.
    pub fn is_ready() -> bool {
        ENABLED.load(Ordering::Relaxed) && MODEL_READY.get().is_some()
    }

    /// Turn the switch on or off. Enabling starts the background load; the
    /// first recall after it completes is the first one scored semantically.
    pub fn set_enabled(on: bool, cache_dir: PathBuf) {
        ENABLED.store(on, Ordering::Relaxed);
        if !on {
            return;
        }
        // A failed fetch is not permanent: flipping the switch back on retries
        // it, so the dialog can show a fresh attempt instead of a dead switch.
        if phase_of(PHASE.load(Ordering::Relaxed)) == Phase::Failed {
            STARTED.store(false, Ordering::SeqCst);
            if let Ok(mut slot) = ERROR.lock() {
                *slot = None;
            }
        }
        init(cache_dir);
    }

    /// Whether this build carries the model at all — the settings pane only
    /// offers the switch when it does.
    pub fn available() -> bool {
        true
    }
}

/// Without the feature there is nothing to load: every entry point reports
/// "unavailable" and the store keeps to its hash vector.
#[cfg(not(feature = "semantic"))]
mod imp {
    use std::path::PathBuf;

    pub fn init(_cache_dir: PathBuf) {}

    pub fn embed(_text: &str) -> Option<Vec<f32>> {
        None
    }

    pub fn embed_batch(texts: &[String]) -> Vec<Option<Vec<f32>>> {
        vec![None; texts.len()]
    }

    /// No model, no identity: every stored vector is "from no model", which is
    /// exactly what a build that cannot score should record.
    pub fn model_id() -> String {
        String::new()
    }

    pub fn is_ready() -> bool {
        false
    }

    pub fn set_enabled(_on: bool, _cache_dir: PathBuf) {}

    pub fn progress() -> super::Progress {
        super::Progress {
            phase: super::Phase::Off,
            bytes: 0,
            total: 0,
            error: None,
        }
    }

    pub fn available() -> bool {
        false
    }
}

pub use imp::{available, embed, embed_batch, init, is_ready, model_id, progress, set_enabled};

#[cfg(test)]
mod tests {
    use super::*;

    /// The switch is off until something turns it on — a process that never
    /// reads a preference never touches the model.
    #[test]
    fn recall_is_off_until_the_switch_is_set() {
        assert!(!is_ready());
        assert_eq!(embed("用中文回答"), None);
    }

    /// `available` reports the build, not the switch: it is what tells the
    /// settings pane whether a toggle can do anything at all.
    #[test]
    fn availability_follows_the_build_feature() {
        assert_eq!(available(), cfg!(feature = "semantic"));
    }

    /// Turning it off stops semantic answers at once. Nothing here turns it
    /// *on*: that starts a ~250 MB download.
    #[test]
    fn turning_the_switch_off_stops_the_model() {
        set_enabled(false, std::env::temp_dir());
        assert!(!is_ready());
        assert_eq!(embed("用中文回答"), None);
    }

    /// The dialog reads this: off means off, and a process that never enabled
    /// the model reports no fetch in flight.
    #[test]
    fn progress_reports_off_until_the_switch_is_set() {
        set_enabled(false, std::env::temp_dir());
        let p = progress();
        assert_eq!(p.phase, Phase::Off);
        assert_eq!(p.bytes, 0);
        assert!(p.error.is_none());
    }
}
