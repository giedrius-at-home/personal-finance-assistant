//! A heavy forecasting model (TimesFM) behind the daily forecast without
//! slowing the screens or keeping its weights in memory.
//!
//! Screens read the daily forecast through [`ModelRunner::cached`], which
//! answers only from forecasts already computed for exactly the same inputs.
//! The first unanswered request starts one background run on its own
//! thread: load the weights, build the dashboard with the model (recording
//! every answer), keep the answers, drop the weights and tell the app, which
//! reloads. Until then (and for good when the model fails) the forecast falls
//! back to the weekday baseline, as it does without a model. The inputs are
//! the cache key, so new data or a new day starts the next run by itself.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use finance_storage::Store;
use jiff::Zoned;
use sha2::{Digest, Sha256};

use crate::Snapshot;
use crate::daily::SeriesForecaster;
use crate::dashboard;

/// A model the app can load on demand.
pub type LoadModel = dyn Fn() -> Result<Box<dyn SeriesForecaster + Send>, String> + Send + Sync;

/// The model the daily forecast may use, and how the app hears about it.
pub struct ForecastModel {
    /// The name forecasts report (`timesfm-3.0-pytorch`).
    pub name: String,
    pub supports_covariates: bool,
    /// Load the weights; called on the worker thread for each run.
    pub load: Box<LoadModel>,
    /// A background run ended (called on its thread); `true` when it
    /// computed new forecasts the screens should reload for.
    pub on_done: Box<dyn Fn(bool) + Send + Sync>,
}

impl std::fmt::Debug for ForecastModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForecastModel")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

type Key = [u8; 32];
type Answer = Result<Vec<Vec<[f64; 3]>>, String>;

/// The exact request: horizon, contexts and covariates.
fn key(contexts: &[Vec<f64>], horizon: usize, covariates: Option<&[Vec<f64>]>) -> Key {
    let mut hash = Sha256::new();
    hash.update((horizon as u64).to_le_bytes());
    let mut series = |values: &[Vec<f64>]| {
        hash.update((values.len() as u64).to_le_bytes());
        for v in values {
            hash.update((v.len() as u64).to_le_bytes());
            for x in v {
                hash.update(x.to_bits().to_le_bytes());
            }
        }
    };
    series(contexts);
    match covariates {
        Some(c) => series(c),
        None => series(&[]),
    }
    hash.finalize().into()
}

/// What the last background run did, for the settings screen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunStatus {
    pub running: bool,
    /// Load plus forecasts, in seconds.
    pub last_seconds: Option<f64>,
    /// Short description of the last failure.
    pub error: Option<String>,
}

#[derive(Debug, Default)]
struct State {
    answers: HashMap<Key, Answer>,
    running: bool,
    /// The request a failed run could not answer; it is not retried.
    failed: Option<Key>,
    status: RunStatus,
}

/// The model plus the forecasts it computed for the latest inputs.
#[derive(Debug)]
pub struct ModelRunner {
    model: ForecastModel,
    state: Mutex<State>,
}

impl ModelRunner {
    pub fn new(model: ForecastModel) -> Self {
        Self {
            model,
            state: Mutex::new(State::default()),
        }
    }

    pub fn status(&self) -> RunStatus {
        self.state
            .lock()
            .map(|s| RunStatus {
                running: s.running,
                ..s.status.clone()
            })
            .unwrap_or_default()
    }

    /// A forecaster that answers from computed forecasts only.
    pub fn cached(&self) -> Cached<'_> {
        Cached {
            runner: self,
            missed: Cell::new(None),
        }
    }

    /// Compute the forecasts `cached` missed on a worker thread, unless a
    /// run is going or this request already failed. `snapshot` and `now`
    /// must be the ones the missing request came from.
    pub fn start(
        self: &Arc<Self>,
        missed: &Cached<'_>,
        path: PathBuf,
        snapshot: Snapshot,
        now: Zoned,
    ) {
        let Some(request) = missed.missed.get() else {
            return;
        };
        {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            if state.running || state.failed == Some(request) {
                return;
            }
            state.running = true;
        }
        let runner = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("forecast-model".into())
            .spawn(move || runner.run(request, &path, &snapshot, &now));
        if spawned.is_err()
            && let Ok(mut state) = self.state.lock()
        {
            state.running = false;
        }
    }

    fn run(&self, request: Key, path: &std::path::Path, snapshot: &Snapshot, now: &Zoned) {
        let started = Instant::now();
        // The weights live only inside this block.
        let outcome = (|| {
            let model = (self.model.load)()?;
            let recording = Recording {
                model: &*model,
                answers: RefCell::new(HashMap::new()),
            };
            let store = Store::open_read_only(path).map_err(|e| e.to_string())?;
            dashboard::dashboard(&store, snapshot, now, Some(&recording))
                .map_err(|e| e.to_string())?;
            Ok::<_, String>(recording.answers.into_inner())
        })();
        let seconds = started.elapsed().as_secs_f64();
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.running = false;
        state.status.last_seconds = Some(seconds);
        let ready = match outcome {
            Ok(answers) => {
                state.status.error = answers.values().find_map(|a| a.as_ref().err().cloned());
                state.failed = (!answers.contains_key(&request)).then_some(request);
                state.answers = answers;
                true
            }
            Err(error) => {
                state.status.error = Some(error);
                state.failed = Some(request);
                false
            }
        };
        drop(state);
        (self.model.on_done)(ready);
    }
}

/// Answers from a [`ModelRunner`]'s computed forecasts; remembers the first
/// request it could not answer.
#[derive(Debug)]
pub struct Cached<'a> {
    runner: &'a ModelRunner,
    missed: Cell<Option<Key>>,
}

impl Cached<'_> {
    /// Some request had no computed answer.
    pub fn missed(&self) -> bool {
        self.missed.get().is_some()
    }
}

impl SeriesForecaster for Cached<'_> {
    fn name(&self) -> &str {
        &self.runner.model.name
    }

    fn supports_covariates(&self) -> bool {
        self.runner.model.supports_covariates
    }

    fn quantiles(
        &self,
        contexts: &[Vec<f64>],
        horizon: usize,
        covariates: Option<&[Vec<f64>]>,
    ) -> Result<Vec<Vec<[f64; 3]>>, String> {
        let request = key(contexts, horizon, covariates);
        let answer = self
            .runner
            .state
            .lock()
            .ok()
            .and_then(|s| s.answers.get(&request).cloned());
        answer.unwrap_or_else(|| {
            if self.missed.get().is_none() {
                self.missed.set(Some(request));
            }
            Err("forecast not computed yet".into())
        })
    }
}

/// Runs the real model and keeps every answer, failures included, so the
/// screens later fall back exactly where this run did.
#[derive(Debug)]
struct Recording<'a> {
    model: &'a (dyn SeriesForecaster + Send),
    answers: RefCell<HashMap<Key, Answer>>,
}

impl SeriesForecaster for Recording<'_> {
    fn name(&self) -> &str {
        self.model.name()
    }

    fn supports_covariates(&self) -> bool {
        self.model.supports_covariates()
    }

    fn quantiles(
        &self,
        contexts: &[Vec<f64>],
        horizon: usize,
        covariates: Option<&[Vec<f64>]>,
    ) -> Result<Vec<Vec<[f64; 3]>>, String> {
        let answer = self.model.quantiles(contexts, horizon, covariates);
        self.answers
            .borrow_mut()
            .insert(key(contexts, horizon, covariates), answer.clone());
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Forecasts the context's last value for every day.
    #[derive(Debug)]
    struct Last;

    impl SeriesForecaster for Last {
        fn name(&self) -> &str {
            "last"
        }

        fn supports_covariates(&self) -> bool {
            false
        }

        fn quantiles(
            &self,
            contexts: &[Vec<f64>],
            horizon: usize,
            _: Option<&[Vec<f64>]>,
        ) -> Result<Vec<Vec<[f64; 3]>>, String> {
            Ok(contexts
                .iter()
                .map(|c| vec![[0.0, c.last().copied().unwrap_or(0.0), 9.0]; horizon])
                .collect())
        }
    }

    fn runner() -> ModelRunner {
        ModelRunner::new(ForecastModel {
            name: "last".into(),
            supports_covariates: false,
            load: Box::new(|| Ok(Box::new(Last))),
            on_done: Box::new(|_| {}),
        })
    }

    #[test]
    fn cached_answers_only_what_a_run_recorded() {
        let runner = runner();
        let contexts = vec![vec![1.0, 2.0]];
        let cached = runner.cached();
        assert!(cached.quantiles(&contexts, 3, None).is_err());
        assert!(cached.missed());

        let model = Last;
        let recording = Recording {
            model: &model,
            answers: RefCell::new(HashMap::new()),
        };
        let rows = recording.quantiles(&contexts, 3, None).unwrap();
        runner.state.lock().unwrap().answers = recording.answers.into_inner();

        let cached = runner.cached();
        assert_eq!(cached.quantiles(&contexts, 3, None).unwrap(), rows);
        assert!(!cached.missed());
        // Different inputs are a different request.
        assert!(cached.quantiles(&contexts, 4, None).is_err());
        assert!(cached.quantiles(&[vec![1.0, 2.5]], 3, None).is_err());
        assert!(cached.missed());
    }

    #[test]
    fn keys_tell_covariates_apart() {
        let contexts = vec![vec![1.0]];
        assert_ne!(
            key(&contexts, 7, None),
            key(&contexts, 7, Some(&[vec![0.0; 8]]))
        );
        assert_eq!(key(&contexts, 7, None), key(&[vec![1.0]], 7, None));
    }
}
