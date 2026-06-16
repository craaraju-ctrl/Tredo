//! Broker plugin framework — plug-and-play broker connections.
//!
//! Users add brokers by dropping a `.toml` file in `~/.tredo/plugins/brokers/`.
//! Built-in plugins: paper, backtest, zerodha (kite connect v3).

pub mod plugin_registry;
pub mod sandbox;

pub use plugin_registry::{BrokerPluginManager, BrokerPlugin, BrokerConfig, ConfigField};
pub use sandbox::BrokerSandbox;
