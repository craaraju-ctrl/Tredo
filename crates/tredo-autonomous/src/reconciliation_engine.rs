//! # ReconciliationEngine — Broker vs Local Portfolio Reconciliation
//!
//! Periodically compares the broker's actual positions (from `BrokerAdapter::get_positions`)
//! against the local `PortfolioState` and reports discrepancies.
//!
//! ## Scenarios Detected
//! - **Phantom Position**: Position exists locally but not on broker (likely filled or cancelled
//!   before TREDO recorded it) → auto-close local position with a warning.
//! - **Ghost Position**: Position exists on broker but not locally (e.g., placed from another app)
//!   → import into local portfolio.
//! - **Size Mismatch**: Different quantities for the same symbol → alert, use broker's count.
//! - **Price Staleness**: Local price significantly different from broker's mark price → update local.
//!
//! ## Alert Flow
//! All discrepancies are logged via COT for real-time TUI display and through
//! the `tredo_core::notifier` for push alerts.

use crate::state::SharedState;
use chrono::Utc;
use tredo_core::paper_engine::{Position, PositionStatus};
use tredo_core::TradeDirection;

// ── Discrepancy Types ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Discrepancy {
    /// Position exists locally but not on broker
    PhantomPosition {
        symbol: String,
        local_qty: i32,
        local_entry: f64,
    },
    /// Position exists on broker but not locally
    GhostPosition {
        symbol: String,
        broker_qty: i32,
        broker_entry: f64,
        broker_current: f64,
    },
    /// Quantity differs between broker and local
    SizeMismatch {
        symbol: String,
        local_qty: i32,
        broker_qty: i32,
        local_entry: f64,
        broker_entry: f64,
    },
    /// Price is significantly stale
    PriceStaleness {
        symbol: String,
        local_price: f64,
        broker_price: f64,
        diff_pct: f64,
    },
}

impl std::fmt::Display for Discrepancy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Discrepancy::PhantomPosition {
                symbol,
                local_qty,
                local_entry,
            } => {
                write!(
                    f,
                    "PHANTOM: {} {}@{} — not on broker, closing local",
                    symbol, local_qty, local_entry
                )
            }
            Discrepancy::GhostPosition {
                symbol,
                broker_qty,
                broker_entry,
                broker_current,
            } => {
                write!(
                    f,
                    "GHOST: {} {}@{} (cur={}) — not in local, importing",
                    symbol, broker_qty, broker_entry, broker_current
                )
            }
            Discrepancy::SizeMismatch {
                symbol,
                local_qty,
                broker_qty,
                local_entry,
                broker_entry,
            } => {
                write!(
                    f,
                    "SIZE: {} local={}@{} vs broker={}@{}",
                    symbol, local_qty, local_entry, broker_qty, broker_entry
                )
            }
            Discrepancy::PriceStaleness {
                symbol,
                local_price,
                broker_price,
                diff_pct,
            } => {
                write!(
                    f,
                    "PRICE: {} local={:.2} vs broker={:.2} ({:+.2}%)",
                    symbol, local_price, broker_price, diff_pct
                )
            }
        }
    }
}

// ── ReconciliationReport ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct ReconciliationReport {
    pub discrepancies: Vec<Discrepancy>,
    pub actions_taken: Vec<String>,
    pub auto_closed: Vec<String>,
    pub auto_imported: Vec<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl ReconciliationReport {
    pub fn has_issues(&self) -> bool {
        !self.discrepancies.is_empty()
    }

    pub fn summary(&self) -> String {
        if !self.has_issues() {
            return "✅ Reconciliation OK — no discrepancies".to_string();
        }
        let mut lines = vec![format!(
            "⚠ Reconciliation: {} discrepancies, {} auto-closed, {} auto-imported",
            self.discrepancies.len(),
            self.auto_closed.len(),
            self.auto_imported.len()
        )];
        for d in &self.discrepancies {
            lines.push(format!("  • {}", d));
        }
        lines.join("\n")
    }
}

// ── ReconciliationEngine ──────────────────────────────────────────────────────

pub struct ReconciliationEngine {
    state: SharedState,
    /// Price staleness threshold (as percentage difference)
    price_staleness_threshold_pct: f64,
}

impl ReconciliationEngine {
    pub fn new(state: SharedState) -> Self {
        Self {
            state,
            price_staleness_threshold_pct: 1.0, // 1% difference triggers alert
        }
    }

    /// Run a full reconciliation cycle: compare broker positions vs local portfolio.
    /// Returns a report of all discrepancies found and any auto-reconciliation actions taken.
    pub async fn reconcile(&self) -> ReconciliationReport {
        let mut report = ReconciliationReport {
            timestamp: Utc::now(),
            ..Default::default()
        };

        // 1. Get broker positions
        let broker = self.state.broker_registry.active_broker().await;
        let broker_positions = match broker.get_positions().await {
            Ok(p) => p,
            Err(e) => {
                let msg = format!("[Reconciliation] ⚠ Failed to fetch broker positions: {}", e);
                eprintln!("{}", msg);
                report.actions_taken.push(msg);
                return report;
            }
        };

        // 2. Get local positions
        let local_positions: Vec<Position> = {
            let portfolio = self.state.portfolio.read().await;
            let mut positions = Vec::new();
            for pos in &portfolio.open_positions {
                positions.push(Position {
                    id: format!("local-{}", pos.symbol),
                    symbol: pos.symbol.clone(),
                    direction: pos.direction,
                    qty: pos.quantity as i32,
                    entry_price: pos.entry_price,
                    current_price: pos.current_price,
                    stop_loss: pos.stop_loss,
                    take_profit: pos.take_profit,
                    unrealized_pnl: pos.unrealized_pnl,
                    unrealized_pnl_pct: pos.unrealized_pnl_pct,
                    status: PositionStatus::Open,
                    opened_at: pos.entry_time,
                    closed_at: None,
                    strategy: Some("tredo-auto".to_string()),
                    order_id: String::new(),
                });
            }
            positions
        };

        // 3. Compare: Find phantom positions (local but not on broker)
        for local_pos in &local_positions {
            let on_broker = broker_positions
                .iter()
                .any(|bp| bp.symbol == local_pos.symbol);

            if !on_broker {
                report.discrepancies.push(Discrepancy::PhantomPosition {
                    symbol: local_pos.symbol.clone(),
                    local_qty: local_pos.qty,
                    local_entry: local_pos.entry_price,
                });

                // Auto-close phantom positions — they were likely filled externally
                // or the position was closed on the exchange without TREDO's knowledge
                let pm = crate::portfolio_manager::PortfolioManagerAgent::new(self.state.clone());
                match pm
                    .close_position(&local_pos.symbol, local_pos.current_price)
                    .await
                {
                    Ok(pnl) => {
                        let msg = format!(
                            "Auto-closed phantom {} @ {:.2} P&L=₹{:.2}",
                            local_pos.symbol, local_pos.current_price, pnl
                        );
                        report.auto_closed.push(msg.clone());
                        report.actions_taken.push(msg);
                    }
                    Err(e) => {
                        let msg =
                            format!("Failed to auto-close phantom {}: {}", local_pos.symbol, e);
                        report.actions_taken.push(msg);
                    }
                }
            }
        }

        // 4. Compare: Find ghost positions (on broker but not local)
        //    and size mismatches / price staleness
        for broker_pos in &broker_positions {
            let local_match = local_positions
                .iter()
                .find(|lp| lp.symbol == broker_pos.symbol);

            match local_match {
                None => {
                    // Ghost position — exists on broker but not in local portfolio
                    report.discrepancies.push(Discrepancy::GhostPosition {
                        symbol: broker_pos.symbol.clone(),
                        broker_qty: broker_pos.qty,
                        broker_entry: broker_pos.entry_price,
                        broker_current: broker_pos.current_price,
                    });

                    // Auto-import ghost positions (conservative: add to local portfolio)
                    let signal = crate::types::TradeSignal {
                        symbol: broker_pos.symbol.clone(),
                        direction: broker_pos.direction,
                        entry_price: broker_pos.entry_price,
                        stop_loss: 0.0,
                        take_profit: 0.0,
                        position_size: broker_pos.qty as f64,
                        confidence_score: 0.5,
                        confluence_score: 0.5,
                        risk_reward_ratio: 0.0,
                        reasoning: format!(
                            "Auto-imported from broker reconciliation (qty={}, entry={})",
                            broker_pos.qty, broker_pos.entry_price
                        ),
                        timestamp: Utc::now(),
                        session_valid: true,
                        risk_check_passed: true,
                    };

                    let pm =
                        crate::portfolio_manager::PortfolioManagerAgent::new(self.state.clone());
                    match pm.add_position(&signal).await {
                        Ok(()) => {
                            let msg = format!(
                                "Auto-imported ghost {} {}@{}",
                                broker_pos.symbol, broker_pos.qty, broker_pos.entry_price
                            );
                            report.auto_imported.push(msg.clone());
                            report.actions_taken.push(msg);
                        }
                        Err(e) => {
                            let msg =
                                format!("Failed to import ghost {}: {}", broker_pos.symbol, e);
                            report.actions_taken.push(msg);
                        }
                    }
                }
                Some(local_pos) => {
                    // Check size mismatch
                    if local_pos.qty != broker_pos.qty {
                        report.discrepancies.push(Discrepancy::SizeMismatch {
                            symbol: broker_pos.symbol.clone(),
                            local_qty: local_pos.qty,
                            broker_qty: broker_pos.qty,
                            local_entry: local_pos.entry_price,
                            broker_entry: broker_pos.entry_price,
                        });

                        // Update local qty to match broker (broker is source of truth)
                        let mut portfolio = self.state.portfolio.write().await;
                        if let Some(lp) = portfolio
                            .open_positions
                            .iter_mut()
                            .find(|p| p.symbol == broker_pos.symbol)
                        {
                            lp.quantity = broker_pos.qty as f64;
                            report.actions_taken.push(format!(
                                "Updated {} qty from {} to {} (broker source of truth)",
                                broker_pos.symbol, local_pos.qty, broker_pos.qty
                            ));
                        }
                        drop(portfolio);
                    }

                    // Check price staleness
                    if local_pos.current_price > 0.0 && broker_pos.current_price > 0.0 {
                        let diff_pct = ((broker_pos.current_price - local_pos.current_price)
                            / local_pos.current_price)
                            .abs()
                            * 100.0;
                        if diff_pct > self.price_staleness_threshold_pct {
                            report.discrepancies.push(Discrepancy::PriceStaleness {
                                symbol: broker_pos.symbol.clone(),
                                local_price: local_pos.current_price,
                                broker_price: broker_pos.current_price,
                                diff_pct,
                            });

                            // Update local price to match broker
                            if let Some(pnl) = self
                                .state
                                .portfolio
                                .write()
                                .await
                                .open_positions
                                .iter_mut()
                                .find(|p| p.symbol == broker_pos.symbol)
                            {
                                pnl.current_price = broker_pos.current_price;
                                pnl.unrealized_pnl = match pnl.direction {
                                    TradeDirection::Long => {
                                        (broker_pos.current_price - pnl.entry_price) * pnl.quantity
                                    }
                                    TradeDirection::Short => {
                                        (pnl.entry_price - broker_pos.current_price) * pnl.quantity
                                    }
                                };
                                pnl.unrealized_pnl_pct = if pnl.entry_price > 0.0 {
                                    (pnl.unrealized_pnl / (pnl.entry_price * pnl.quantity)) * 100.0
                                } else {
                                    0.0
                                };

                                report.actions_taken.push(format!(
                                    "Updated {} price from {:.2} to {:.2} (broker source of truth)",
                                    broker_pos.symbol,
                                    local_pos.current_price,
                                    broker_pos.current_price
                                ));
                            }
                        }
                    }
                }
            }
        }

        // 5. Log discrepancies via COT
        if report.has_issues() {
            let summary = report.summary();
            let _ = self
                .state
                .push_cot(
                    "ReconciliationEngine",
                    "Broker reconciliation cycle",
                    if !report.auto_closed.is_empty() || !report.auto_imported.is_empty() {
                        "AUTO_RECONCILED"
                    } else {
                        "DISCREPANCIES"
                    },
                    &summary,
                    0.5,
                    0,
                    None,
                    None,
                )
                .await;

            // Send push notification for critical issues
            if !report.auto_closed.is_empty() || !report.discrepancies.is_empty() {
                tredo_core::notifier::alert(
                    "Live Broker Reconciliation — Discrepancies Found",
                    &summary,
                )
                .await;
            }
        } else {
            let _ = self
                .state
                .push_cot(
                    "ReconciliationEngine",
                    "Broker reconciliation cycle",
                    "OK",
                    "No discrepancies — local portfolio matches broker",
                    0.95,
                    0,
                    None,
                    None,
                )
                .await;
        }

        report
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Unit tests for the Discrepancy display formatting
    #[test]
    fn test_discrepancy_display_phantom() {
        let d = Discrepancy::PhantomPosition {
            symbol: "BTC".to_string(),
            local_qty: 1,
            local_entry: 50000.0,
        };
        let s = d.to_string();
        assert!(s.contains("PHANTOM"));
        assert!(s.contains("BTC"));
    }

    #[test]
    fn test_discrepancy_display_ghost() {
        let d = Discrepancy::GhostPosition {
            symbol: "ETH".to_string(),
            broker_qty: 2,
            broker_entry: 3000.0,
            broker_current: 3100.0,
        };
        let s = d.to_string();
        assert!(s.contains("GHOST"));
        assert!(s.contains("ETH"));
    }

    #[test]
    fn test_discrepancy_display_size() {
        let d = Discrepancy::SizeMismatch {
            symbol: "SOL".to_string(),
            local_qty: 5,
            broker_qty: 3,
            local_entry: 150.0,
            broker_entry: 155.0,
        };
        let s = d.to_string();
        assert!(s.contains("SIZE"));
        assert!(s.contains("SOL"));
    }

    #[test]
    fn test_report_has_issues() {
        let mut report = ReconciliationReport::default();
        assert!(!report.has_issues());

        report.discrepancies.push(Discrepancy::PhantomPosition {
            symbol: "BTC".to_string(),
            local_qty: 1,
            local_entry: 50000.0,
        });
        assert!(report.has_issues());
    }

    #[test]
    fn test_report_summary_ok() {
        let report = ReconciliationReport::default();
        assert!(report.summary().contains("OK"));
    }

    #[test]
    fn test_report_summary_issues() {
        let mut report = ReconciliationReport::default();
        report.discrepancies.push(Discrepancy::PhantomPosition {
            symbol: "BTC".to_string(),
            local_qty: 1,
            local_entry: 50000.0,
        });
        let summary = report.summary();
        assert!(summary.contains("PHANTOM"));
        assert!(summary.contains("discrepancies"));
    }
}
