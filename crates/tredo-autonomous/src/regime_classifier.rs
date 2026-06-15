//! RegimeClassifier — Cognitive Core (Layer 2)
//!
//! Re-exports and wraps regime detection so that Layer 1 (data ingestion)
//! remains pure collection, while regime understanding lives in the
//! Cognitive Intelligence layer alongside skills and debate.
//!
//! This satisfies the separation of concerns critique.

pub use crate::regime_detector::RegimeDetector;
pub use crate::types::MarketRegime; // re-export for Layer 2 consumers

/// Thin cognitive wrapper. Future extensions can add HMM, macro data fusion, etc.
pub type RegimeClassifier = RegimeDetector;
