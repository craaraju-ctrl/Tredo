//! # Broker — Paper & Live Trading Engine
//!
//! This module has been superseded by [`crate::paper_engine`] which provides:
//! - [`crate::paper_engine::PaperEngine`] — Virtual portfolio & order matching engine
//! - [`crate::paper_engine::BrokerAdapter`] trait — Unified interface for paper & live brokers
//! - [`crate::paper_engine::PaperBroker`] — Paper trading implementation
//!
//! ## Usage
//! ```
//! use tredo_core::paper_engine::*;
//! ```
//!
//! ## Paper/Live Parity
//! The exact same code path is used for both paper and live trading.
//! The only difference is which `BrokerAdapter` implementation handles execution:
//! - `PaperBroker` → virtual money via `PaperEngine`
//! - Zerodha Kite broker → real money via Kite API
//! - Angel One broker → real money via Angel One API

pub use crate::paper_engine::*;

/// Legacy — use [`PaperEngine::new`] with [`PaperEngineConfig::default`] instead.
#[deprecated(
    since = "0.2.0",
    note = "Use PaperBroker::new(PaperEngineConfig::default()) instead"
)]
pub fn create_paper_adapter() -> PaperBroker {
    PaperBroker::new(PaperEngineConfig::default())
}
