// Strong "Skills" set (AgentSkill trait) – tells agents/sub-agents *how* to do things.
// Skills are pluggable tools/capabilities: "how to analyze sentiment", "how to compute vol", "how to recall trained memory", "how to debate", etc.
// Agents and sub-agents already "know what to do" (their roles in the Tredo hierarchy: Identifier scans, Verifier validates, Executer decides, Guardian protects).
// Skills give them the "how". Rules (DisciplinedCore + trained adjustments) tell "what to do / not to do".
// Combined with hierarchical trained memory (RAG+ vector + agentmemory), this makes every agent/sub-agent smarter: it remembers exactly what *it* did in past similar situations, what the outcome/lesson was, and executes its role better over time with far less hallucination.

use crate::agent::{AgentInput, AgentOutput};
use async_trait::async_trait;
use std::error::Error;

#[async_trait]
pub trait AgentSkill: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn execute(
        &self,
        input: &AgentInput,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>>;
    fn is_available(&self) -> bool {
        true
    }
}

// Example skill wrapper to turn sub-agents into skills if needed.
pub struct SkillWrapper<S: AgentSkill + ?Sized> {
    inner: Box<S>,
}

impl<S: AgentSkill> SkillWrapper<S> {
    pub fn new(skill: S) -> Self {
        Self {
            inner: Box::new(skill),
        }
    }
}

#[async_trait]
impl<S: AgentSkill> AgentSkill for SkillWrapper<S> {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    async fn execute(
        &self,
        input: &AgentInput,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        self.inner.execute(input).await
    }
}

// TrainedMemorySkill – a first-class "how" for long-term self-understanding.
// Uses the hierarchical recall (local vector RAG for recent trained episodes + agentmemory for long-term lessons).
// Every agent/sub-agent can "execute" this skill to ground its decision in "exactly what I did before and what happened".
#[allow(clippy::type_complexity)]
pub struct TrainedMemorySkill {
    pub recall_fn: Box<dyn Fn(&str, usize) -> String + Send + Sync>, // injected from state.recall_trained_memory
}

impl TrainedMemorySkill {
    pub fn new(recall_fn: impl Fn(&str, usize) -> String + Send + Sync + 'static) -> Self {
        Self {
            recall_fn: Box::new(recall_fn),
        }
    }
}

#[async_trait]
impl AgentSkill for TrainedMemorySkill {
    fn name(&self) -> &str {
        "TrainedMemorySkill"
    }
    fn description(&self) -> &str {
        "Recalls hierarchical trained memory (past actions by this/similar agents, outcomes, regret, lessons) for the given context. This is how an agent 'remembers exactly what it was doing' and improves long-term while reducing hallucinations."
    }

    async fn execute(
        &self,
        input: &AgentInput,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        // Extract a query from input (simple heuristic; in real use, agents pass rich context).
        let query = match input {
            AgentInput::ConfluenceRequest { context } => format!(
                "{} price {:.2} confluence",
                context.symbol, context.current_price
            ),
            _ => "general trading decision".to_string(),
        };
        let recall = (self.recall_fn)(&query, 3);
        println!(
            "[Skill] {} executed: {}",
            self.name(),
            &recall[..recall.len().min(120)]
        );
        // Return the recall string as a structured SkillResult so calling agents
        // can aggregate it into their reasoning / COT.
        let has_data = !recall.contains("No strong trained memory match");
        Ok(AgentOutput::SkillResult {
            name: self.name().to_string(),
            score: if has_data { 0.7 } else { 0.3 },
            note: recall,
            confidence: if has_data { 0.8 } else { 0.2 },
            direction: crate::agent::SkillDirection::Neutral,
            weight: 0.2,
        })
    }
}
