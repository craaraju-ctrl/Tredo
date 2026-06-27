//! Resilient pipeline — every step has intelligent fallbacks so a single error
//! never kills the agent or loses a valid opportunity.

#![allow(rustdoc::invalid_html_tags)]

use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
use tracing::{error, info, warn};

/// The outcome of a single pipeline step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StepOutcome<T> {
    /// Step succeeded with the given output.
    Success(T),
    /// Step failed but was handled by a fallback.
    Degraded { output: T, reason: String },
    /// Step failed entirely and cannot proceed.
    Fatal { reason: String },
}

impl<T> StepOutcome<T> {
    pub fn into_inner(self) -> Option<T> {
        match self {
            StepOutcome::Success(val) | StepOutcome::Degraded { output: val, .. } => Some(val),
            StepOutcome::Fatal { .. } => None,
        }
    }

    pub fn is_ok(&self) -> bool {
        matches!(self, StepOutcome::Success(_) | StepOutcome::Degraded { .. })
    }

    pub fn is_degraded(&self) -> bool {
        matches!(self, StepOutcome::Degraded { .. })
    }
}

/// Configuration for pipeline resilience.
#[derive(Debug, Clone)]
pub struct ResilienceConfig {
    /// Maximum number of retries per step.
    pub max_retries: u32,
    /// Initial backoff between retries (doubles each attempt).
    pub initial_backoff_ms: u64,
    /// Whether to allow degraded fallback paths (stale data, cached predictions).
    pub allow_degraded: bool,
    /// Whether to log and continue on non-critical failures.
    pub continue_on_non_critical: bool,
}

impl Default for ResilienceConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff_ms: 100,
            allow_degraded: true,
            continue_on_non_critical: true,
        }
    }
}

/// Convenient type alias for the complex PipelineStep return type.
pub type StepResult<O> = Result<O, Box<dyn std::error::Error + Send + Sync>>;

/// A pipeline step that can be wrapped with resilience logic.
/// Uses explicit Pin<Box<dyn Future>> return types to avoid #[async_trait] lifetime issues.
pub trait PipelineStep<I, O>: Send + Sync {
    /// Execute this step with the given input.
    fn execute<'a>(&'a self, input: I) -> Pin<Box<dyn Future<Output = StepResult<O>> + Send + 'a>>;

    /// Optional degraded fallback that runs when the primary execution fails.
    fn degraded<'a>(
        &'a self,
        _input: I,
        _error: &str,
    ) -> Pin<Box<dyn Future<Output = StepResult<O>> + Send + 'a>> {
        Box::pin(async { Err("No degraded fallback available".into()) })
    }
}

/// Run a single pipeline step with retry + fallback.
pub async fn run_step<I, O>(
    step: &dyn PipelineStep<I, O>,
    input: I,
    config: &ResilienceConfig,
    step_name: &str,
) -> StepOutcome<O>
where
    I: Clone + Send + Sync + 'static,
    O: Send + Sync + 'static,
{
    let mut last_error = String::new();
    let backoff = Duration::from_millis(config.initial_backoff_ms);

    for attempt in 1..=config.max_retries {
        match step.execute(input.clone()).await {
            Ok(output) => {
                if attempt > 1 {
                    info!("{} succeeded after {} retries", step_name, attempt - 1);
                }
                return StepOutcome::Success(output);
            }
            Err(e) => {
                last_error = e.to_string();
                if attempt < config.max_retries {
                    let wait = backoff * (2u32.pow(attempt - 1));
                    warn!(
                        "{} attempt {}/{} failed: {}. Retrying in {:?}",
                        step_name, attempt, config.max_retries, last_error, wait
                    );
                    tokio::time::sleep(wait).await;
                }
            }
        }
    }

    // All retries exhausted → try degraded fallback
    if config.allow_degraded {
        match step.degraded(input, &last_error).await {
            Ok(output) => {
                warn!(
                    "{} all retries failed, using degraded fallback: {}",
                    step_name, last_error
                );
                return StepOutcome::Degraded {
                    output,
                    reason: format!("retries exhausted: {}", last_error),
                };
            }
            Err(fallback_err) => {
                error!(
                    "{} failed after {} retries AND degraded fallback failed: {} / {}",
                    step_name, config.max_retries, last_error, fallback_err
                );
            }
        }
    } else {
        error!(
            "{} failed after {} retries (degraded disabled): {}",
            step_name, config.max_retries, last_error
        );
    }

    StepOutcome::Fatal {
        reason: format!("{} failed: {}", step_name, last_error),
    }
}

/// Shortcut: wrap a closure into a PipelineStep.
pub struct FnStep<I, O, F> {
    f: F,
    _phantom: std::marker::PhantomData<(I, O)>,
}

pub fn fn_step<I, O, F, Fut>(f: F) -> FnStep<I, O, F>
where
    F: Fn(I) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = StepResult<O>> + Send,
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
{
    FnStep {
        f,
        _phantom: std::marker::PhantomData,
    }
}

impl<I, O, F, Fut> PipelineStep<I, O> for FnStep<I, O, F>
where
    F: Fn(I) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = StepResult<O>> + Send,
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
{
    fn execute<'a>(&'a self, input: I) -> Pin<Box<dyn Future<Output = StepResult<O>> + Send + 'a>> {
        Box::pin(async move { (self.f)(input).await })
    }
}
