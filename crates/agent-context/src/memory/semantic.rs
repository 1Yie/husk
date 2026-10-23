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
//! Loading is lazy and off the caller's thread: the model is ~240 MB, fetched
//! into the app's data directory on first use, ~1 s on later starts. Every
//! query before it is ready — or after a failed load — simply uses the hash
//! path.
//!
//! The fetch is the one step a user waits on, so its state is observable
//! ([`progress`]). The transfer itself belongs to `semantic_fetch`, which
//! reports real bytes: sized files that passed their check plus what has
//! landed of the one in flight.

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
    /// Bytes verified plus what has landed of the file in flight.
    pub bytes: u64,
    /// The model's download size, from the file manifest.
    pub total: u64,
    /// The file being fetched, while one is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(feature = "semantic")]
mod imp {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
    use std::sync::{Mutex, OnceLock};

    use fastembed::{
        EmbeddingModel, InitOptionsUserDefined, Pooling, QuantizationMode, TextEmbedding,
        TokenizerFiles, UserDefinedEmbeddingModel,
    };

    use super::{Phase, Progress};
    use crate::memory::semantic_fetch::{self, Counters, FetchError};

    /// Multilingual small model: 384 dims, quantized. The identity recorded
    /// with every vector the encoder produces.
    const MODEL: EmbeddingModel = EmbeddingModel::ParaphraseMLMiniLML12V2Q;

    /// The context the encoder was trained on — `fastembed`'s own default, kept
    /// so a model loaded from bytes behaves exactly like one it fetched itself.
    const MAX_LENGTH: usize = 512;

    /// The settings switch, in front of the model. Off means "do not use the
    /// model even though it is loaded" — the weights stay in memory, so
    /// turning it back on costs nothing.
    static ENABLED: AtomicBool = AtomicBool::new(false);

    /// [`Phase`] as a byte — read by the settings poll, written by the loader.
    static PHASE: AtomicU8 = AtomicU8::new(0);
    static ERROR: Mutex<Option<String>> = Mutex::new(None);
    /// Where the weights live. Read once; a later `set_enabled` keeps it.
    static CACHE: OnceLock<PathBuf> = OnceLock::new();

    /// Asks the fetch to stop. Checked between chunks, so a cancelled download
    /// leaves its `.part` file for the next attempt to resume.
    static CANCEL: AtomicBool = AtomicBool::new(false);
    /// A loader thread is alive. A second start while one runs is folded into
    /// [`RESPAWN`] instead of racing it for the same files.
    static RUNNING: AtomicBool = AtomicBool::new(false);
    static RESPAWN: AtomicBool = AtomicBool::new(false);

    /// The weights were deleted by 卸载. A `OnceLock` cannot be emptied, so the
    /// loaded encoder may still sit in memory — it must not keep scoring for a
    /// model the user removed, and the next enable has to fetch again.
    static REMOVED: AtomicBool = AtomicBool::new(false);

    /// One ONNX Runtime per process, built once and reused. `fastembed` would
    /// build its own per load, and the second one fails with "only one
    /// environment" — which made every retry after a failed load impossible.
    static ENVIRONMENT: OnceLock<Result<ort::environment::Environment, String>> = OnceLock::new();

    /// Byte accounting shared with [`super::progress`].
    static COUNTERS: Counters = Counters::new();

    /// The loaded model. `embed` needs `&mut`, and one caller at a time is
    /// fine: recall is a single query per message.
    static MODEL_READY: OnceLock<Mutex<TextEmbedding>> = OnceLock::new();

    /// Start loading the model in the background. `cache_dir` holds the weights
    /// between runs.
    pub fn init(cache_dir: PathBuf) {
        let _ = CACHE.set(cache_dir);
        let Some(dir) = CACHE.get().cloned() else {
            return;
        };
        if MODEL_READY.get().is_some() && !REMOVED.load(Ordering::SeqCst) {
            set_phase(Phase::Ready);
            return;
        }
        if RUNNING.swap(true, Ordering::SeqCst) {
            RESPAWN.store(true, Ordering::SeqCst);
            return;
        }
        CANCEL.store(false, Ordering::SeqCst);
        set_phase(Phase::Downloading);
        let spawned = std::thread::Builder::new()
            .name("semantic-embed".into())
            .spawn(move || run(dir));
        if spawned.is_err() {
            RUNNING.store(false, Ordering::SeqCst);
            fail("无法启动模型加载线程".into());
        }
    }

    fn run(cache_dir: PathBuf) {
        let started = std::time::Instant::now();
        // A panic in the loader used to leave the phase stuck at "downloading"
        // with no error and no retry, which reads exactly like a hang.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| load(&cache_dir)));
        match outcome {
            Ok(Ok(model)) => {
                let _ = MODEL_READY.set(Mutex::new(model));
                REMOVED.store(false, Ordering::SeqCst);
                clear_error();
                set_phase(Phase::Ready);
                tracing::info!(ms = started.elapsed().as_millis(), "semantic recall ready");
            }
            // The user asked it to stop: not a failure, and the next attempt
            // picks up from the bytes already verified.
            Ok(Err(FetchError::Cancelled)) => {
                clear_error();
                set_phase(Phase::Idle);
                tracing::info!("semantic model fetch cancelled");
            }
            Ok(Err(FetchError::Failed(message))) => fail(message),
            Err(panic) => fail(format!("模型加载崩溃: {}", panic_message(&*panic))),
        }
        finish_run();
    }

    /// Clear the run flag, and honour a start that arrived while this one was
    /// winding down (toggling off then on happens faster than a fetch aborts).
    fn finish_run() {
        RUNNING.store(false, Ordering::SeqCst);
        if RESPAWN.swap(false, Ordering::SeqCst) && ENABLED.load(Ordering::Relaxed) {
            if let Some(dir) = CACHE.get() {
                init(dir.clone());
            }
        }
    }

    /// Fetch what is missing, then build the session from the bytes on disk.
    fn load(cache_dir: &std::path::Path) -> Result<TextEmbedding, FetchError> {
        let paths = semantic_fetch::ensure(cache_dir, &COUNTERS, &CANCEL)?;
        set_phase(Phase::Loading);
        let environment = environment().map_err(FetchError::Failed)?;
        let read = |name: &str| {
            let index = semantic_fetch::FILES
                .iter()
                .position(|file| file.name == name)
                .unwrap_or(0);
            std::fs::read(&paths[index])
                .map_err(|e| FetchError::Failed(format!("无法读取 {name}: {e}")))
        };
        let tokenizer = TokenizerFiles {
            tokenizer_file: read("tokenizer.json")?,
            config_file: read("config.json")?,
            special_tokens_map_file: read("special_tokens_map.json")?,
            tokenizer_config_file: read("tokenizer_config.json")?,
        };
        // The pooling and quantization match what `fastembed` picks for this
        // model when it fetches it itself.
        let model = UserDefinedEmbeddingModel::new(read("model_optimized.onnx")?, tokenizer)
            .with_pooling(Pooling::Mean)
            .with_quantization(QuantizationMode::Static);
        let options = InitOptionsUserDefined::new()
            .with_environment(environment)
            .with_max_length(MAX_LENGTH);
        TextEmbedding::try_new_from_user_defined(model, options)
            .map_err(|e| FetchError::Failed(format!("模型加载失败: {e}")))
    }

    fn environment() -> Result<ort::environment::Environment, String> {
        ENVIRONMENT
            .get_or_init(|| ort::environment::init().build().map_err(|e| e.to_string()))
            .clone()
    }

    fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
            .unwrap_or_else(|| "unknown panic".into())
    }

    fn fail(message: String) {
        // Offline, a blocked host, a full disk: recall keeps working on the
        // hash vector. The dialog says why; the chat keeps working.
        tracing::warn!(
            %message,
            "semantic embedding model unavailable; recall stays on the hash vector"
        );
        if let Ok(mut slot) = ERROR.lock() {
            *slot = Some(message);
        }
        set_phase(Phase::Failed);
    }

    fn clear_error() {
        if let Ok(mut slot) = ERROR.lock() {
            *slot = None;
        }
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

    /// Where the fetch currently is. Reads counters only — no directory walk,
    /// so the settings pane can poll it while a download is running.
    pub fn progress() -> Progress {
        let mut phase = phase_of(PHASE.load(Ordering::Relaxed));
        if !ENABLED.load(Ordering::Relaxed) && phase != Phase::Failed {
            phase = Phase::Off;
        }
        Progress {
            phase,
            bytes: COUNTERS.bytes(),
            total: semantic_fetch::total_bytes(),
            file: COUNTERS.file(),
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
        // The slice is handed over as-is: `embed` takes `AsRef<[S]>`, so the
        // batch costs no copy of the texts.
        match model.embed(texts, None) {
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
        ENABLED.load(Ordering::Relaxed)
            && !REMOVED.load(Ordering::SeqCst)
            && MODEL_READY.get().is_some()
    }

    /// Turn the switch on or off. Enabling starts the background load; the
    /// first recall after it completes is the first one scored semantically.
    pub fn set_enabled(on: bool, cache_dir: PathBuf) {
        ENABLED.store(on, Ordering::Relaxed);
        // Remembered even when no load starts: `retry` needs it later.
        let _ = CACHE.set(cache_dir.clone());
        if !on {
            // A download the user just switched off must stop, not finish
            // quietly in the background for another 240 MB.
            CANCEL.store(true, Ordering::SeqCst);
            return;
        }
        // A failure stays a failure until someone asks for a retry. Every
        // session spawn passes through here, and without the guard a machine
        // with no route to the hub re-attempted a quarter-gigabyte download
        // per session — the log filled with the same failure, and the one
        // retry the user did ask for was indistinguishable from the noise.
        if phase_of(PHASE.load(Ordering::Relaxed)) == Phase::Failed {
            return;
        }
        init(cache_dir);
    }

    /// Retry after a failure. Verified files are reused, so a failure in the
    /// load rather than the fetch retries in about a second.
    pub fn retry() {
        let Some(dir) = CACHE.get().cloned() else {
            return;
        };
        clear_error();
        set_phase(Phase::Idle);
        init(dir);
    }

    /// Stop a fetch in progress. Files that already verified stay; the partial
    /// one keeps its `.part` so the next attempt resumes.
    pub fn cancel() {
        CANCEL.store(true, Ordering::SeqCst);
        if !RUNNING.load(Ordering::SeqCst)
            && phase_of(PHASE.load(Ordering::Relaxed)) == Phase::Downloading
        {
            set_phase(Phase::Idle);
        }
    }

    /// Whether this build carries the model at all — the settings pane only
    /// offers the switch when it does.
    pub fn available() -> bool {
        true
    }

    /// The encoder's name, for the settings row. Deliberately NOT [`model_id`]:
    /// that string is stored with every vector, and renaming it would throw
    /// away the whole semantic index.
    pub fn model_name() -> String {
        "paraphrase-multilingual-MiniLM-L12-v2".into()
    }

    /// Whether the weights are already on this machine. The pane asks before
    /// offering a download — a model that is on disk must not be fetched again
    /// just because the switch was turned off and back on.
    pub fn model_cached() -> bool {
        CACHE
            .get()
            .is_some_and(|dir| crate::memory::semantic_fetch::present(dir))
    }

    /// 卸载 — delete the model files and stop using them. The weights are the
    /// quarter gigabyte; the switch is not. Returns whether anything was there
    /// to remove.
    pub fn remove_model() -> bool {
        CANCEL.store(true, Ordering::SeqCst);
        ENABLED.store(false, Ordering::Relaxed);
        REMOVED.store(true, Ordering::SeqCst);
        COUNTERS.reset();
        clear_error();
        set_phase(Phase::Idle);
        let Some(dir) = CACHE.get() else {
            return false;
        };
        // Our own copy, and the old hub client's — both are model weights, and
        // leaving the second one behind would quietly adopt it on the next
        // enable, which is the opposite of what 卸载 means.
        let mut removed = false;
        for path in [
            dir.join(semantic_fetch::REPO),
            dir.join(format!("models--{}", semantic_fetch::REPO.replace('/', "--"))),
        ] {
            if std::fs::remove_dir_all(&path).is_ok() {
                removed = true;
            }
        }
        tracing::info!(removed, "semantic model removed");
        removed
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

    pub fn retry() {}

    pub fn cancel() {}

    pub fn progress() -> super::Progress {
        super::Progress {
            phase: super::Phase::Off,
            bytes: 0,
            total: 0,
            file: None,
            error: None,
        }
    }

    pub fn available() -> bool {
        false
    }

    pub fn model_name() -> String {
        String::new()
    }

    pub fn model_cached() -> bool {
        false
    }

    pub fn remove_model() -> bool {
        false
    }
}

pub use imp::{
    available, cancel, embed, embed_batch, init, is_ready, model_cached, model_id, model_name,
    progress, remove_model, retry, set_enabled,
};

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
    /// *on*: that starts a download.
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
