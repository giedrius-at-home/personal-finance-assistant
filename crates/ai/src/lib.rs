//! The app side of AI suggestions: HTTP clients for local Ollama, TypeSafe
//! and public research ([`clients`]), the TypeSafe key in the secret store, and
//! [`Assistant`], which runs one suggestion pass at a time for the
//! scheduler, the bank sync and the owner's "Suggest categories". The
//! prompts, parsing and storage are `finance_core::ai`.

pub mod clients;

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use finance_core::ai::groups::{self, GroupRanker};
use finance_core::ai::runner::{Backends, Runner};
use finance_core::ai::{BackendError, research};
use finance_core::service::Service;
use finance_secrets::{Item, Secret};
use finance_types::ai::{
    AiRun, AiSettings, AiStatus, Classifier, GroupSuggestion, MerchantResearchResult,
    SuggestionPass,
};
use jiff::Timestamp;

use clients::{Ollama, TypeSafe, Web};

/// Generation can take a while on a laptop; Python allowed 120 s.
const GENERATE_TIMEOUT: Duration = Duration::from_secs(120);
/// `suggest_spending_group` used 45 s and a single attempt.
const GROUP_TIMEOUT: Duration = Duration::from_secs(45);

/// `model` is among Ollama's `api/tags` names (`qwen3` means `qwen3:latest`).
pub fn installed(models: &[String], model: &str) -> bool {
    models
        .iter()
        .any(|m| m == model || (!model.contains(':') && *m == format!("{model}:latest")))
}

/// Runs one AI pass at a time and remembers the last one.
#[derive(Debug)]
pub struct Assistant {
    runner: Mutex<Runner>,
    last_run: Mutex<Option<AiRun>>,
    group: Mutex<()>,
    /// Whether the TypeSafe key may be read from the secret store (not in the
    /// development server).
    keychain: bool,
    wake: (Mutex<bool>, Condvar),
}

impl Default for Assistant {
    fn default() -> Self {
        Self::new(true)
    }
}

fn message(error: &BackendError) -> String {
    match error {
        BackendError::Unavailable => "The model could not be reached.".into(),
        BackendError::Status(401 | 403) => "The API key was rejected.".into(),
        BackendError::Status(code) => format!("The model returned HTTP {code}."),
        BackendError::Protocol => "The model returned an invalid answer.".into(),
        BackendError::Invalid(reason) => (*reason).to_owned(),
    }
}

impl Assistant {
    pub fn new(keychain: bool) -> Self {
        Self {
            runner: Mutex::new(Runner::default()),
            last_run: Mutex::new(None),
            group: Mutex::new(()),
            keychain,
            wake: (Mutex::new(false), Condvar::new()),
        }
    }

    fn key(&self) -> Option<Secret> {
        if !self.keychain {
            return None;
        }
        finance_secrets::get(Item::TypeSafeApiKey).ok().flatten()
    }

    /// Store the owner's TypeSafe key; it is never returned to the UI.
    pub fn set_typesafe_key(&self, key: &str) -> Result<(), String> {
        if !self.keychain {
            return Err("TypeSafe keys are stored by the app only.".into());
        }
        let key = Secret::new(key.trim().to_owned());
        if !clients::valid_key(key.expose()) {
            return Err("The TypeSafe API key is not valid.".into());
        }
        finance_secrets::set(Item::TypeSafeApiKey, &key).map_err(|e| e.to_string())
    }

    pub fn forget_typesafe_key(&self) -> Result<(), String> {
        if !self.keychain {
            return Ok(());
        }
        finance_secrets::delete(Item::TypeSafeApiKey).map_err(|e| e.to_string())
    }

    /// The settings card.
    pub fn status(&self, service: &Service) -> Result<AiStatus, String> {
        let settings = service.ai_settings()?;
        let models = Ollama::installed(&settings.ollama_url);
        Ok(AiStatus {
            ollama_detected: models.is_some(),
            model_available: models
                .as_deref()
                .is_some_and(|m| installed(m, &settings.ollama_model)),
            embedding_available: settings
                .embedding()
                .is_none_or(|e| models.as_deref().is_some_and(|m| installed(m, e))),
            ollama_models: models.unwrap_or_default(),
            typesafe_key_set: self.key().is_some(),
            running: self.runner.try_lock().is_err(),
            last_run: self.last_run.lock().ok().and_then(|r| r.clone()),
            settings,
        })
    }

    pub fn save_settings(
        &self,
        service: &Service,
        settings: AiSettings,
    ) -> Result<AiStatus, String> {
        service.save_ai_settings(settings)?;
        self.wake();
        self.status(service)
    }

    /// Ask the scheduler to run now (after a bank sync wrote transactions).
    pub fn wake(&self) {
        if let Ok(mut flag) = self.wake.0.lock() {
            *flag = true;
            self.wake.1.notify_all();
        }
    }

    /// Sleep until `timeout` passes or [`Assistant::wake`] is called.
    pub fn wait(&self, timeout: Duration) {
        let Ok(mut flag) = self.wake.0.lock() else {
            std::thread::sleep(timeout);
            return;
        };
        let deadline = Instant::now() + timeout;
        while !*flag {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match self.wake.1.wait_timeout(flag, left) {
                Ok((guard, _)) => flag = guard,
                Err(_) => return,
            }
        }
        *flag = false;
    }

    /// One pass: keep the daily inbox current, propose categories for new
    /// or stale transactions, then apply confident Jev suggestions when the
    /// owner allowed it. `Ok(None)` when everything is off or a pass is
    /// already running.
    pub fn tick(&self, service: &Service) -> Result<Option<AiRun>, String> {
        let settings = service.ai_settings()?;
        if settings.classifier == Classifier::Off && !settings.auto_apply {
            return Ok(None);
        }
        let Ok(mut runner) = self.runner.try_lock() else {
            return Ok(None);
        };
        let started = Instant::now();
        let mut run = AiRun::default();
        let outcome = self.pass(service, &settings, &mut runner, &mut run);
        run.error = outcome.err();
        run.millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        run.finished_at = Timestamp::now().to_string();
        if let Ok(mut last) = self.last_run.lock() {
            *last = Some(run.clone());
        }
        Ok(Some(run))
    }

    fn pass(
        &self,
        service: &Service,
        settings: &AiSettings,
        runner: &mut Runner,
        run: &mut AiRun,
    ) -> Result<(), String> {
        if settings.classifier != Classifier::Off {
            service.daily_review_pass(false)?;
            let embed = settings
                .embedding()
                .map(|model| Ollama::new(&settings.ollama_url, model, GENERATE_TIMEOUT, 3))
                .transpose()
                .map_err(|e| message(&e))?;
            match settings.classifier {
                Classifier::Ollama => {
                    let Some(models) = Ollama::installed(&settings.ollama_url) else {
                        return Err("Ollama is not running.".into());
                    };
                    if !installed(&models, &settings.ollama_model) {
                        return Err(format!(
                            "{} is not available on this Ollama.",
                            settings.ollama_model
                        ));
                    }
                    let ollama = Ollama::new(
                        &settings.ollama_url,
                        &settings.ollama_model,
                        GENERATE_TIMEOUT,
                        3,
                    )
                    .map_err(|e| message(&e))?;
                    run.suggested = runner.run(
                        service,
                        Backends {
                            settings,
                            generate: Some(&ollama),
                            embed: embed.as_ref().map(|e| e as _),
                            choose: None,
                        },
                    )?;
                }
                Classifier::TypeSafe => {
                    let key = self
                        .key()
                        .ok_or_else(|| "Add the TypeSafe API key in Settings.".to_owned())?;
                    let typesafe =
                        TypeSafe::new(key, &settings.typesafe_model, 3).map_err(|e| message(&e))?;
                    run.suggested = runner.run(
                        service,
                        Backends {
                            settings,
                            generate: None,
                            embed: embed.as_ref().map(|e| e as _),
                            choose: Some(&typesafe),
                        },
                    )?;
                }
                Classifier::Off => {}
            }
        }
        if settings.auto_apply {
            run.applied = service.daily_review_pass(true)?;
        }
        Ok(())
    }

    /// The owner's "Suggest categories": a pass now, waiting for it.
    pub fn suggest_now(&self, service: &Service) -> Result<SuggestionPass, String> {
        let settings = service.ai_settings()?;
        if settings.classifier == Classifier::Off {
            return Err("Turn on AI suggestions in Settings first.".into());
        }
        match self.tick(service)? {
            None => {
                Err("Suggestions are already running; the inbox updates when they finish.".into())
            }
            Some(AiRun { error: Some(e), .. }) => Err(e),
            Some(run) => Ok(SuggestionPass {
                suggested: run.suggested,
                applied: run.applied,
            }),
        }
    }

    fn research_on(
        &self,
        service: &Service,
        settings: &AiSettings,
        name: &str,
    ) -> MerchantResearchResult {
        research::merchant(
            settings.research,
            service,
            &Web,
            name,
            None,
            Timestamp::now(),
        )
    }

    /// `POST /spending-groups/suggest`.
    pub fn suggest_group(
        &self,
        service: &Service,
        category: &str,
    ) -> Result<GroupSuggestion, String> {
        let settings = service.ai_settings()?;
        let mut input = service.group_input(category)?;
        let Ok(_one) = self.group.try_lock() else {
            return Err("A suggestion is already running.".into());
        };
        let unavailable =
            |_| "AI group suggestions are unavailable. You can choose a group manually.".to_owned();
        match settings.classifier {
            Classifier::Off => Err("Turn on AI suggestions in Settings first.".into()),
            Classifier::TypeSafe => {
                let key = self
                    .key()
                    .ok_or_else(|| "Add the TypeSafe API key in Settings.".to_owned())?;
                if settings.research
                    && let Ok(name) = research::public_name(&input.category)
                {
                    input.public_research = Some(self.research_on(service, &settings, &name));
                }
                let client =
                    TypeSafe::new(key, &settings.typesafe_model, 1).map_err(unavailable)?;
                groups::TypeSafe { client: &client }
                    .suggest(&input)
                    .map_err(unavailable)
            }
            Classifier::Ollama => {
                let ollama = Ollama::new(
                    &settings.ollama_url,
                    &settings.ollama_model,
                    GROUP_TIMEOUT,
                    1,
                )
                .map_err(unavailable)?;
                groups::Ollama { backend: &ollama }
                    .suggest(&input)
                    .map_err(unavailable)
            }
        }
    }

    /// `POST /review/{id}/research`: only the public name leaves the Mac.
    pub fn research_merchant(
        &self,
        service: &Service,
        proposal_id: &str,
        public_name: &str,
    ) -> Result<MerchantResearchResult, String> {
        let settings = service.ai_settings()?;
        if !settings.research {
            return Err("Public merchant research is off. Turn it on in Settings.".into());
        }
        if !service.proposal_pending(proposal_id)? {
            return Err("This review changed. Refresh and try again.".into());
        }
        let name = research::public_name(public_name).map_err(|e| e.to_string())?;
        Ok(self.research_on(service, &settings, &name))
    }
}
