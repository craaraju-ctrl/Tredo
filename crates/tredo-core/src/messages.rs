//! Minimal message types for controlled agent communication and LLM interaction.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMRequest {
    pub request_id: String,
    pub agent_role: crate::role::AgentRole,
    pub prompt: String,
    pub context: serde_json::Value,
    pub max_tokens: u32,
    pub temperature: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMResponse {
    pub content: String,
    pub tokens_used: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentMessage {
    Task(String),
    LLMRequest(LLMRequest),
    LLMResponse(LLMResponse),
    Observation {
        agent: String,
        content: String,
    },
    /// Request to dispatch work to a specific Sub-Agent.
    SubAgentTask {
        target: String,
        input: crate::agent::AgentInput,
    },
    /// Result returned by a Sub-Agent after processing a task.
    SubAgentResult {
        source: String,
        output: crate::agent::AgentOutput,
    },
}
