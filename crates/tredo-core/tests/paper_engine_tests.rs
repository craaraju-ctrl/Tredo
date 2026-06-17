//! Integration tests for tredo-core's PaperEngine.
//!
//! Tests cover: order matching, P&L calculations, risk checks, SL/TP
//! auto-close, commission deduction, portfolio equity, trade history,
//! and edge cases.

use tredo_core::paper_engine::*;
use tredo_core::TradeDirection;

// ── Helpers ────────────────────────────────────────────────────────────────

fn default_config() -> PaperEngineConfig {
    PaperEngineConfig {
        initial_balance: 100_000.0,
        max_position_size_pct: 5.0,
        max_daily_loss_pct: 3.0,
        max_drawdown_pct: 10.0,
        max_concentration_pct: 20.0,
        max_portfolio_heat_pct: 30.0,
        max_leverage: 1.0,
        slippage_model: SlippageModel::None,
        commission_pct: 0.0, // zero commission for clean P&L math
    }
}

fn commission_config() -> PaperEngineConfig {
    PaperEngineConfig {
        commission_pct: 0.03, // 0.03% like Zerodha
        ..default_config()
    }
}

fn market_order(symbol: &str, direction: TradeDirection, qty: i32) -> OrderRequest {
    OrderRequest {
        symbol: symbol.to_string(),
        direction,
        order_type: OrderType::Market,
        qty,
        price: None,
        stop_loss: None,
        take_profit: None,
        strategy: None,
        client_order_id: None,
    }
}

fn market_order_with_sl_tp(
    symbol: &str,
    direction: TradeDirection,
    qty: i32,
    sl: f64,
    tp: f64,
) -> OrderRequest {
    OrderRequest {
        symbol: symbol.to_string(),
        direction,
        order_type: OrderType::Market,
        qty,
        price: None,
        stop_loss: Some(sl),
        take_profit: Some(tp),
        strategy: None,
        client_order_id: None,
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// ORDER MATCHING
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_market_order_long_fills() {
    let engine = PaperEngine::new(default_config());
    let req = market_order("BTC", TradeDirection::Long, 1);
    let _order_id = engine.place_order(req, 50_000.0).await.unwrap();

    let positions = engine.get_positions().await;
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0].symbol, "BTC");
    assert_eq!(positions[0].direction, TradeDirection::Long);
    assert_eq!(positions[0].qty, 1);
    assert_eq!(positions[0].entry_price, 50_000.0);
    assert_eq!(positions[0].status, PositionStatus::Open);
}

#[tokio::test]
async fn test_market_order_short_fills() {
    let engine = PaperEngine::new(default_config());
    let req = market_order("ETH", TradeDirection::Short, 2);
    engine.place_order(req, 3_000.0).await.unwrap();

    let positions = engine.get_positions().await;
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0].direction, TradeDirection::Short);
    assert_eq!(positions[0].qty, 2);
    assert_eq!(positions[0].entry_price, 3_000.0);
}

#[tokio::test]
async fn test_slippage_fixed_buy_pays_more() {
    let config = PaperEngineConfig {
        slippage_model: SlippageModel::Fixed(1.0),
        ..default_config()
    };
    let engine = PaperEngine::new(config);
    let req = market_order("BTC", TradeDirection::Long, 1);
    engine.place_order(req, 50_000.0).await.unwrap();

    let positions = engine.get_positions().await;
    // Long buy with fixed slippage: fill = market + slippage
    assert_eq!(positions[0].entry_price, 50_001.0);
}

#[tokio::test]
async fn test_slippage_fixed_sell_receives_less() {
    let config = PaperEngineConfig {
        slippage_model: SlippageModel::Fixed(1.0),
        ..default_config()
    };
    let engine = PaperEngine::new(config);
    let req = market_order("BTC", TradeDirection::Short, 1);
    engine.place_order(req, 50_000.0).await.unwrap();

    let positions = engine.get_positions().await;
    // Short sell with fixed slippage: fill = market - slippage
    assert_eq!(positions[0].entry_price, 49_999.0);
}

#[tokio::test]
async fn test_slippage_percentage() {
    let config = PaperEngineConfig {
        slippage_model: SlippageModel::Percentage(0.01), // 0.01%
        ..default_config()
    };
    let engine = PaperEngine::new(config);
    let req = market_order("BTC", TradeDirection::Long, 1);
    engine.place_order(req, 50_000.0).await.unwrap();

    let positions = engine.get_positions().await;
    // 0.01% of 50000 = 5.0
    assert!((positions[0].entry_price - 50_005.0).abs() < 0.01);
}

#[tokio::test]
async fn test_order_rejected_zero_qty() {
    let engine = PaperEngine::new(default_config());
    let req = OrderRequest {
        symbol: "BTC".to_string(),
        direction: TradeDirection::Long,
        order_type: OrderType::Market,
        qty: 0,
        price: None,
        stop_loss: None,
        take_profit: None,
        strategy: None,
        client_order_id: None,
    };
    let result = engine.place_order(req, 50_000.0).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_order_rejected_insufficient_cash() {
    let config = PaperEngineConfig {
        initial_balance: 100.0,
        ..default_config()
    };
    let engine = PaperEngine::new(config);
    // Try to buy 1 BTC at 50000 with only 100 cash
    let req = market_order("BTC", TradeDirection::Long, 1);
    let result = engine.place_order(req, 50_000.0).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_commission_deducted_on_entry() {
    let engine = PaperEngine::new(commission_config());
    let summary_before = engine.get_summary().await;
    let cash_before = summary_before.cash;

    let req = market_order("BTC", TradeDirection::Long, 1);
    engine.place_order(req, 50_000.0).await.unwrap();

    let summary_after = engine.get_summary().await;
    let cash_after = summary_after.cash;
    let expected_commission = 50_000.0 * 0.0003; // 0.03%
    let expected_cost = 50_000.0 + expected_commission;

    assert!(
        (cash_before - cash_after - expected_cost).abs() < 0.01,
        "Cash should decrease by cost + commission: expected {:.2}, got {:.2}",
        expected_cost,
        cash_before - cash_after
    );
}

#[tokio::test]
async fn test_multiple_positions_same_symbol() {
    let engine = PaperEngine::new(default_config());
    engine
        .place_order(market_order("BTC", TradeDirection::Long, 1), 50_000.0)
        .await
        .unwrap();
    engine
        .place_order(market_order("BTC", TradeDirection::Long, 1), 50_000.0)
        .await
        .unwrap();

    let positions = engine.get_positions().await;
    assert_eq!(positions.len(), 2);
    assert!(positions.iter().all(|p| p.symbol == "BTC"));
}

// ══════════════════════════════════════════════════════════════════════════════
// P&L CALCULATIONS
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_long_position_pnl_positive() {
    let engine = PaperEngine::new(default_config());
    engine
        .place_order(market_order("BTC", TradeDirection::Long, 1), 50_000.0)
        .await
        .unwrap();

    // Price goes up to 51000
    let closed = engine.update_price("BTC", 51_000.0).await;
    assert!(closed.is_empty()); // no SL/TP hit

    let positions = engine.get_positions().await;
    assert_eq!(positions[0].unrealized_pnl, 1_000.0);
    assert!(positions[0].unrealized_pnl_pct > 0.0);
}

#[tokio::test]
async fn test_long_position_pnl_negative() {
    let engine = PaperEngine::new(default_config());
    engine
        .place_order(market_order("BTC", TradeDirection::Long, 1), 50_000.0)
        .await
        .unwrap();

    // Price drops to 49000
    engine.update_price("BTC", 49_000.0).await;

    let positions = engine.get_positions().await;
    assert_eq!(positions[0].unrealized_pnl, -1_000.0);
}

#[tokio::test]
async fn test_short_position_pnl_positive() {
    let engine = PaperEngine::new(default_config());
    engine
        .place_order(market_order("BTC", TradeDirection::Short, 1), 50_000.0)
        .await
        .unwrap();

    // Price drops to 49000 (good for short)
    engine.update_price("BTC", 49_000.0).await;

    let positions = engine.get_positions().await;
    assert_eq!(positions[0].unrealized_pnl, 1_000.0);
}

#[tokio::test]
async fn test_short_position_pnl_negative() {
    let engine = PaperEngine::new(default_config());
    engine
        .place_order(market_order("BTC", TradeDirection::Short, 1), 50_000.0)
        .await
        .unwrap();

    // Price rises to 51000 (bad for short)
    engine.update_price("BTC", 51_000.0).await;

    let positions = engine.get_positions().await;
    assert_eq!(positions[0].unrealized_pnl, -1_000.0);
}

#[tokio::test]
async fn test_pnl_with_multiple_qty() {
    let engine = PaperEngine::new(default_config());
    engine
        .place_order(market_order("ETH", TradeDirection::Long, 10), 3_000.0)
        .await
        .unwrap();

    // Price goes up by 100
    engine.update_price("ETH", 3_100.0).await;

    let positions = engine.get_positions().await;
    assert_eq!(positions[0].unrealized_pnl, 1_000.0); // 10 * 100
}

// ══════════════════════════════════════════════════════════════════════════════
// STOP-LOSS / TAKE-PROFIT MONITORING
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_stop_loss_long_triggers() {
    let engine = PaperEngine::new(default_config());
    let req = market_order_with_sl_tp("BTC", TradeDirection::Long, 1, 49_000.0, 52_000.0);
    engine.place_order(req, 50_000.0).await.unwrap();

    // Price drops to SL
    let closed = engine.update_price("BTC", 49_000.0).await;
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].close_reason, CloseReason::StopLoss);
    assert_eq!(closed[0].exit_price, 49_000.0);

    let positions = engine.get_positions().await;
    assert!(positions.is_empty(), "Position should be closed after SL");
}

#[tokio::test]
async fn test_take_profit_long_triggers() {
    let engine = PaperEngine::new(default_config());
    let req = market_order_with_sl_tp("BTC", TradeDirection::Long, 1, 48_000.0, 52_000.0);
    engine.place_order(req, 50_000.0).await.unwrap();

    // Price rises to TP
    let closed = engine.update_price("BTC", 52_000.0).await;
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].close_reason, CloseReason::TakeProfit);
    assert_eq!(closed[0].exit_price, 52_000.0);

    let positions = engine.get_positions().await;
    assert!(positions.is_empty());
}

#[tokio::test]
async fn test_stop_loss_short_triggers() {
    let engine = PaperEngine::new(default_config());
    // Short: SL above entry, TP below entry
    let req = market_order_with_sl_tp("BTC", TradeDirection::Short, 1, 51_000.0, 48_000.0);
    engine.place_order(req, 50_000.0).await.unwrap();

    // Price rises to SL
    let closed = engine.update_price("BTC", 51_000.0).await;
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].close_reason, CloseReason::StopLoss);
}

#[tokio::test]
async fn test_take_profit_short_triggers() {
    let engine = PaperEngine::new(default_config());
    let req = market_order_with_sl_tp("BTC", TradeDirection::Short, 1, 51_000.0, 48_000.0);
    engine.place_order(req, 50_000.0).await.unwrap();

    // Price drops to TP
    let closed = engine.update_price("BTC", 48_000.0).await;
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].close_reason, CloseReason::TakeProfit);
}

#[tokio::test]
async fn test_no_close_when_price_between_sl_tp() {
    let engine = PaperEngine::new(default_config());
    let req = market_order_with_sl_tp("BTC", TradeDirection::Long, 1, 49_000.0, 52_000.0);
    engine.place_order(req, 50_000.0).await.unwrap();

    // Price moves but stays within SL/TP range
    let closed = engine.update_price("BTC", 50_500.0).await;
    assert!(closed.is_empty());

    let closed = engine.update_price("BTC", 49_500.0).await;
    assert!(closed.is_empty());
}

#[tokio::test]
async fn test_sl_tp_not_set_no_auto_close() {
    let engine = PaperEngine::new(default_config());
    engine
        .place_order(market_order("BTC", TradeDirection::Long, 1), 50_000.0)
        .await
        .unwrap();

    // No SL/TP set — price can go anywhere without auto-close
    let closed = engine.update_price("BTC", 1.0).await;
    assert!(closed.is_empty());
}

#[tokio::test]
async fn test_commission_deducted_on_close() {
    let engine = PaperEngine::new(commission_config());
    let req = market_order_with_sl_tp("BTC", TradeDirection::Long, 1, 48_000.0, 52_000.0);
    engine.place_order(req, 50_000.0).await.unwrap();

    let closed = engine.update_price("BTC", 52_000.0).await;
    assert_eq!(closed.len(), 1);

    // PnL = 2000, but commission is deducted on both entry and exit
    // The realized_pnl should be less than raw 2000 due to exit commission
    let raw_pnl = 52_000.0 - 50_000.0;
    let exit_commission = 52_000.0 * 0.0003;
    assert!(
        closed[0].realized_pnl < raw_pnl,
        "realized_pnl ({}) should be less than raw P&L ({}) due to exit commission",
        closed[0].realized_pnl,
        raw_pnl
    );
    assert!(
        closed[0].realized_pnl > raw_pnl - exit_commission - 1.0,
        "realized_pnl ({}) should be approximately raw P&L minus exit commission",
        closed[0].realized_pnl
    );
}

// ══════════════════════════════════════════════════════════════════════════════
// PORTFOLIO EQUITY
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_equity_initial_equals_balance() {
    let engine = PaperEngine::new(default_config());
    let summary = engine.get_summary().await;
    assert_eq!(summary.cash, 100_000.0);
    assert_eq!(summary.equity, 100_000.0);
}

#[tokio::test]
async fn test_equity_after_buy_reflects_market_value() {
    let engine = PaperEngine::new(default_config());
    engine
        .place_order(market_order("BTC", TradeDirection::Long, 1), 50_000.0)
        .await
        .unwrap();

    let summary = engine.get_summary().await;
    // Cash = 100000 - 50000 = 50000, Position market value = 50000
    // Equity = 50000 + 50000 = 100000
    assert_eq!(summary.cash, 50_000.0);
    assert_eq!(summary.equity, 100_000.0);
}

#[tokio::test]
async fn test_equity_updates_with_price() {
    let engine = PaperEngine::new(default_config());
    engine
        .place_order(market_order("BTC", TradeDirection::Long, 1), 50_000.0)
        .await
        .unwrap();

    engine.update_price("BTC", 55_000.0).await;

    let summary = engine.get_summary().await;
    // Cash = 50000, Position market value = 55000
    assert_eq!(summary.equity, 105_000.0);
    // daily_pnl is only updated on position close, not price update
    assert_eq!(summary.daily_pnl, 0.0);
}

#[tokio::test]
async fn test_equity_after_close() {
    let engine = PaperEngine::new(default_config());
    engine
        .place_order(market_order("BTC", TradeDirection::Long, 1), 50_000.0)
        .await
        .unwrap();

    engine.update_price("BTC", 55_000.0).await;
    engine.close_position("POS-000001", 55_000.0).await.unwrap();

    let summary = engine.get_summary().await;
    // Cash restored: 50000 + 55000 = 105000, no open positions
    assert_eq!(summary.cash, 105_000.0);
    assert_eq!(summary.equity, 105_000.0);
    assert_eq!(summary.open_positions, 0);
}

// ══════════════════════════════════════════════════════════════════════════════
// RISK CHECKS
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_risk_check_passes_normal_order() {
    let engine = PaperEngine::new(default_config());
    let result = engine.check_risk("BTC", 5_000.0).await;
    assert!(result.passed);
}

#[tokio::test]
async fn test_risk_check_blocks_oversized_position() {
    let engine = PaperEngine::new(default_config());
    // Max position = 5% of 100000 = 5000
    let result = engine.check_risk("BTC", 6_000.0).await;
    assert!(!result.passed);
    assert!(!result.max_position_size_ok);
}

#[tokio::test]
async fn test_risk_check_blocks_excessive_concentration() {
    let config = PaperEngineConfig {
        max_concentration_pct: 10.0,
        ..default_config()
    };
    let engine = PaperEngine::new(config);

    // First position: 8000 (8% — under 10% limit)
    engine
        .place_order(market_order("BTC", TradeDirection::Long, 1), 8_000.0)
        .await
        .unwrap();

    // Second position in same symbol would push concentration above 10%
    let result = engine.check_risk("BTC", 3_000.0).await;
    assert!(!result.passed);
    assert!(!result.concentration_ok);
}

#[tokio::test]
async fn test_risk_check_blocks_excessive_heat() {
    let config = PaperEngineConfig {
        max_position_size_pct: 100.0,       // allow large positions
        max_portfolio_heat_pct: 0.5,         // very tight heat limit (0.5%)
        ..default_config()
    };
    let engine = PaperEngine::new(config);

    // Buy with a tight stop loss to create portfolio heat
    // Risk per unit = 50000 - 49000 = 1000, total risk = 1000
    // Max heat = 0.5% of 100000 = 500, so 1000 > 500 triggers block
    let req = market_order_with_sl_tp("BTC", TradeDirection::Long, 1, 49_000.0, 55_000.0);
    engine.place_order(req, 50_000.0).await.unwrap();

    let result = engine.check_risk("ETH", 1_000.0).await;
    assert!(!result.passed);
    assert!(!result.portfolio_heat_ok);
}

#[tokio::test]
async fn test_risk_check_drawdown() {
    let config = PaperEngineConfig {
        max_drawdown_pct: 5.0,
        ..default_config()
    };
    let engine = PaperEngine::new(config);

    // Buy 1 BTC at 50000 (50% of equity), then price drops to 44000.
    // equity = cash(50000) + position_value(44000) = 94000
    // drawdown = (100000 - 94000) / 100000 * 100 = 6% > 5% limit
    engine
        .place_order(market_order("BTC", TradeDirection::Long, 1), 50_000.0)
        .await
        .unwrap();
    engine.update_price("BTC", 44_000.0).await;

    let result = engine.check_risk("ETH", 1_000.0).await;
    assert!(!result.passed);
    assert!(!result.drawdown_ok);
}

// ══════════════════════════════════════════════════════════════════════════════
// TRADE HISTORY & JOURNAL
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_trade_history_recorded_on_close() {
    let engine = PaperEngine::new(default_config());
    engine
        .place_order(market_order("BTC", TradeDirection::Long, 1), 50_000.0)
        .await
        .unwrap();

    engine.close_position("POS-000001", 55_000.0).await.unwrap();

    let history = engine.get_trade_history().await;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].symbol, "BTC");
    assert_eq!(history[0].entry_price, 50_000.0);
    assert_eq!(history[0].exit_price, 55_000.0);
    assert_eq!(history[0].realized_pnl, 5_000.0);
    assert_eq!(history[0].close_reason, CloseReason::Manual);
}

#[tokio::test]
async fn test_trade_history_records_stop_loss() {
    let engine = PaperEngine::new(default_config());
    let req = market_order_with_sl_tp("BTC", TradeDirection::Long, 1, 48_000.0, 52_000.0);
    engine.place_order(req, 50_000.0).await.unwrap();

    engine.update_price("BTC", 48_000.0).await;

    let history = engine.get_trade_history().await;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].close_reason, CloseReason::StopLoss);
    assert_eq!(history[0].realized_pnl, -2_000.0);
}

#[tokio::test]
async fn test_trade_history_records_take_profit() {
    let engine = PaperEngine::new(default_config());
    let req = market_order_with_sl_tp("BTC", TradeDirection::Long, 1, 48_000.0, 52_000.0);
    engine.place_order(req, 50_000.0).await.unwrap();

    engine.update_price("BTC", 52_000.0).await;

    let history = engine.get_trade_history().await;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].close_reason, CloseReason::TakeProfit);
    assert_eq!(history[0].realized_pnl, 2_000.0);
}

#[tokio::test]
async fn test_trade_history_most_recent_first() {
    let engine = PaperEngine::new(default_config());

    // Trade 1: Buy and close
    engine
        .place_order(market_order("BTC", TradeDirection::Long, 1), 50_000.0)
        .await
        .unwrap();
    engine.close_position("POS-000001", 55_000.0).await.unwrap();

    // Trade 2: Buy and close
    engine
        .place_order(market_order("ETH", TradeDirection::Long, 1), 3_000.0)
        .await
        .unwrap();
    engine.close_position("POS-000002", 3_500.0).await.unwrap();

    let history = engine.get_trade_history().await;
    assert_eq!(history.len(), 2);
    // Most recent first
    assert_eq!(history[0].symbol, "ETH");
    assert_eq!(history[1].symbol, "BTC");
}

#[tokio::test]
async fn test_get_recent_trades_limits() {
    let engine = PaperEngine::new(default_config());
    for i in 0..5 {
        engine
            .place_order(
                market_order("BTC", TradeDirection::Long, 1),
                50_000.0 + i as f64,
            )
            .await
            .unwrap();
        engine.close_position(&format!("POS-{:06}", i + 1), 55_000.0).await.unwrap();
    }

    let recent = engine.get_recent_trades(3).await;
    assert_eq!(recent.len(), 3);
}

// ══════════════════════════════════════════════════════════════════════════════
// PORTFOLIO STATISTICS
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_win_rate_calculation() {
    let engine = PaperEngine::new(default_config());

    // Win 1
    engine
        .place_order(market_order("BTC", TradeDirection::Long, 1), 50_000.0)
        .await
        .unwrap();
    engine.close_position("POS-000001", 51_000.0).await.unwrap();

    // Loss 1
    engine
        .place_order(market_order("ETH", TradeDirection::Long, 1), 3_000.0)
        .await
        .unwrap();
    engine.close_position("POS-000002", 2_900.0).await.unwrap();

    // Win 2
    engine
        .place_order(market_order("SOL", TradeDirection::Long, 1), 100.0)
        .await
        .unwrap();
    engine.close_position("POS-000003", 110.0).await.unwrap();

    let summary = engine.get_summary().await;
    assert_eq!(summary.total_trades, 3);
    assert_eq!(summary.winning_trades, 2);
    assert_eq!(summary.losing_trades, 1);
    assert!((summary.win_rate - 66.67).abs() < 0.1);
}

#[tokio::test]
async fn test_consecutive_losses_tracking() {
    let engine = PaperEngine::new(default_config());

    // 3 consecutive losses
    for i in 0..3 {
        engine
            .place_order(
                market_order("BTC", TradeDirection::Long, 1),
                50_000.0,
            )
            .await
            .unwrap();
        engine
            .close_position(&format!("POS-{:06}", i + 1), 49_000.0)
            .await
            .unwrap();
    }

    let summary = engine.get_summary().await;
    assert_eq!(summary.consecutive_losses, 3);

    // A win resets the counter
    engine
        .place_order(market_order("BTC", TradeDirection::Long, 1), 50_000.0)
        .await
        .unwrap();
    engine
        .close_position("POS-000004", 51_000.0)
        .await
        .unwrap();

    let summary = engine.get_summary().await;
    assert_eq!(summary.consecutive_losses, 0);
}

// ══════════════════════════════════════════════════════════════════════════════
// RESET
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_reset_clears_everything() {
    let engine = PaperEngine::new(default_config());

    // Do some trading
    engine
        .place_order(market_order("BTC", TradeDirection::Long, 1), 50_000.0)
        .await
        .unwrap();
    engine.close_position("POS-000001", 55_000.0).await.unwrap();

    assert_eq!(engine.get_positions().await.len(), 0);
    assert!(!engine.get_trade_history().await.is_empty());

    engine.reset().await;

    let summary = engine.get_summary().await;
    assert_eq!(summary.cash, 100_000.0);
    assert_eq!(summary.equity, 100_000.0);
    assert_eq!(summary.total_trades, 0);
    assert!(engine.get_positions().await.is_empty());
}

// ══════════════════════════════════════════════════════════════════════════════
// MANUAL CLOSE
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_manual_close_position() {
    let engine = PaperEngine::new(default_config());
    engine
        .place_order(market_order("BTC", TradeDirection::Long, 1), 50_000.0)
        .await
        .unwrap();

    let result = engine.close_position("POS-000001", 52_000.0).await;
    assert!(result.is_ok());

    let trade = result.unwrap();
    assert_eq!(trade.close_reason, CloseReason::Manual);
    assert_eq!(trade.exit_price, 52_000.0);
    assert_eq!(trade.realized_pnl, 2_000.0);
}

#[tokio::test]
async fn test_close_nonexistent_position_fails() {
    let engine = PaperEngine::new(default_config());
    let result = engine.close_position("POS-999999", 50_000.0).await;
    assert!(result.is_err());
}

// ══════════════════════════════════════════════════════════════════════════════
// BROKER ADAPTER (PaperBroker)
// ══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_paper_broker_connect_disconnect() {
    let broker = PaperBroker::new(PaperEngineConfig::default());
    broker.connect().await.unwrap();
    broker.disconnect().await.unwrap();
    assert_eq!(broker.mode(), TradingMode::Paper);
    assert_eq!(broker.broker_name(), "Paper Trading");
}

#[tokio::test]
async fn test_broker_registry_mode_switching() {
    let registry = BrokerRegistry::new(PaperEngineConfig::default());
    registry.set_mode(TradingMode::Paper).await.unwrap();
    assert_eq!(registry.current_mode().await, TradingMode::Paper);
    assert_eq!(registry.current_broker_name().await, "Paper Trading");

    // Switching to live without a registered broker should fail
    let result = registry.set_mode(TradingMode::Live).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_broker_registry_paper_engine_access() {
    let registry = BrokerRegistry::new(PaperEngineConfig::default());
    let engine = registry.paper_engine();

    // Use the engine directly to place an order
    engine
        .place_order(market_order("BTC", TradeDirection::Long, 1), 50_000.0)
        .await
        .unwrap();

    let positions = engine.get_positions().await;
    assert_eq!(positions.len(), 1);
}
