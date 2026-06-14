//! # Broker — Paper & Live Trading Engine
//!
//! This module has been superseded by [`paper_engine`] which provides:
//! - [`PaperEngine`] — Virtual portfolio & order matching engine
//! - [`BrokerAdapter`] trait — Unified interface for paper & live brokers
//! - [`PaperBroker`] — Paper trading implementation
//! - [`ZerodhaKiteBroker`] — Live Zerodha Kite integration
//! - [`AngelOneBroker`] — Live Angel One integration
//! - [`BrokerRegistry`] — Routes orders to the active broker
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
//! - `ZerodhaKiteBroker` → real money via Kite API
//! - `AngelOneBroker` → real money via Angel One API

pub use crate::paper_engine::*;

/// Legacy — use [`PaperEngine::new`] with [`PaperEngineConfig::default`] instead.
#[deprecated(
    since = "0.2.0",
    note = "Use PaperBroker::new(PaperEngineConfig::default()) instead"
)]
pub fn create_paper_adapter() -> PaperBroker {
    PaperBroker::new(PaperEngineConfig::default())
}
