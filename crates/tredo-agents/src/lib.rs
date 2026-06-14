// DEPRECATED SHIM — see tredo-autonomous for the active two-tier agents, debate, skills, and orchestrator logic.
// This crate is retained only because some older paths (orchestrator) still list it as a dependency.
// All new development MUST target tredo-autonomous and tredo-core.

pub mod main_agents;
pub mod sub_agents;

#[deprecated(note = "Use tredo_autonomous instead. This shim will be removed after migration.")]
pub fn deprecated_shim_notice() {
    eprintln!("[tredo-agents] DEPRECATED — migrate to tredo-autonomous");
}

// NOTE (duplication fix): This crate (tredo-agents) appears to be a parallel/legacy implementation
// of the agent hierarchy also present in tredo-autonomous (with main_agents/sub_agents mirroring
// the Tredo structure). The active code path used by tredo-orchestrator and Tauri is tredo-autonomous.
// This crate is kept for now but should be consolidated in a future refactor to avoid maintenance burden.
// (deprecated stub removed — tredo-agents duplicates tredo-autonomous; see comment above)

// Re-export common agents
pub use main_agents::*;
pub use sub_agents::*;
