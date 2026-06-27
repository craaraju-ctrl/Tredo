//! tredo-runtime — Unified runtime layer for the tredo agentic trading system.
//!
//! This crate provides the event-driven architecture, multi-mode trading,
//! introspection, goal management, world model, and all the other pieces
//! that transform tredo from a batch pipeline into a truly agentic system.

pub mod active_learner;
pub mod api_clients;
pub mod backtest_feed;
pub mod broker;
pub mod data_feed;
pub mod engine;
pub mod event_bus;
pub mod goal_manager;
pub mod introspector;
pub mod live_broker;
// pub mod live_feed; // Removed: deprecated after event-driven refactor (engine uses api_clients directly)
pub mod mode;
pub mod paper_broker;
pub mod policy_cache;
pub mod portfolio_reasoner;
pub mod resilient_pipeline;
pub mod risk_manager;
pub mod strategy;
pub mod streaming_reasoner;
pub mod world_model;

pub use engine::RuntimeEngine;
pub use event_bus::{AgentEvent, EventBus};
pub use mode::TradingMode;
