use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tredo_autonomous::state::SharedState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningStep {
    pub step_type: StepType,
    pub content: String,
    pub confidence: f64,
    pub needs_more_info: bool,
    pub follow_up_questions: Vec<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum StepType {
    Observation,
    Hypothesis,
    Evidence,
    Contradiction,
    Conclusion,
    Uncertainty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningOutcome {
    pub transcript: Vec<ReasoningStep>,
    pub confidence: f64,
    pub iterations_used: u32,
    pub final_recommendation: String,
}

pub struct StreamingReasoner {
    #[allow(dead_code)]
    state: SharedState,
    #[allow(dead_code)]
    llm: Option<Arc<tredo_core::LlmExecutor>>,
    #[allow(dead_code)]
    max_iterations: u32,
}

impl StreamingReasoner {
    pub fn new(state: SharedState) -> Self {
        Self {
            state,
            llm: None,
            max_iterations: 5,
        }
    }

    pub async fn reason(
        &self,
        _symbol: &str,
        _current_price: f64,
    ) -> Result<ReasoningOutcome, String> {
        // Placeholder: when llm is None, return a basic reasoning without LLM calls
        Ok(ReasoningOutcome {
            transcript: vec![ReasoningStep {
                step_type: StepType::Observation,
                content: "StreamingReasoner: no LLM configured, using passive observation"
                    .to_string(),
                confidence: 0.3,
                needs_more_info: true,
                follow_up_questions: vec!["Initialize LLM for full reasoning".to_string()],
                timestamp: chrono::Utc::now(),
            }],
            confidence: 0.3,
            iterations_used: 1,
            final_recommendation: "Insufficient reasoning capacity — deferring to fast pipeline"
                .to_string(),
        })
    }

    #[allow(dead_code)]
    async fn synthesize(&self, transcript: &[ReasoningStep]) -> Result<String, String> {
        let summary = transcript
            .iter()
            .map(|s| format!("{:?}: {}", s.step_type, s.content))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(format!(
            "Final decision based on {} reasoning steps:\n{}",
            transcript.len(),
            summary
        ))
    }
}
