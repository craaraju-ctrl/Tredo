use std::collections::HashMap;

use tredo_autonomous::episode_store::{ClosedEpisode, EpisodeStore, RuleSnapshot};
use tredo_autonomous::meta_control::EvolvedMetaControl;
use tredo_autonomous::outcome_processor::{OutcomeProcessor, PreTradeSnapshot};
use tredo_autonomous::risk_guardian::RiskGuardianConfig;
use tredo_autonomous::types::MarketRegime;

#[tokio::test]
async fn test_automated_rule_revert_on_degraded_performance() {
    // 1. Setup in-memory sqlite engine
    let store = EpisodeStore::open("file::memory:?cache=shared")
        .expect("Failed to create in-memory EpisodeStore");

    // Seed genesis rule version v1
    let genesis_snapshot = RuleSnapshot {
        version: 1,
        config_json: serde_json::to_string(&RiskGuardianConfig::default_fallback()).unwrap(),
        baseline_win_rate: 0.65,
        baseline_avg_regret: 0.15,
        timestamp: 100000,
    };
    store
        .insert_rule_snapshot(genesis_snapshot)
        .expect("Genesis seed must succeed");

    // 2. Initialize EvolvedMetaControl on active version v2 (simulating a prior rule adaptation)
    let meta_control = EvolvedMetaControl::new(store.clone(), 0.05, 2);
    let weight_tuner = tredo_autonomous::weight_tuner::AttributionEngine::new(0.10);
    let processor = OutcomeProcessor::new(store.clone(), weight_tuner, meta_control);

    // Seed v2 rules snapshot in the database to establish adaptation baselines
    let v2_rules = RiskGuardianConfig {
        absolute_max_drawdown_pct: 0.10,
        absolute_max_leverage: 2,
        max_risk_per_trade_pct: 0.01,
        hard_min_stop_loss_pct: 0.005,
        hard_max_stop_loss_pct: 0.08,
        ..RiskGuardianConfig::default_fallback()
    };
    let v2_snapshot = RuleSnapshot {
        version: 2,
        config_json: serde_json::to_string(&v2_rules).unwrap(),
        baseline_win_rate: 0.60,
        baseline_avg_regret: 0.20,
        timestamp: 101000,
    };
    store
        .insert_rule_snapshot(v2_snapshot)
        .expect("v2 snapshot must succeed");

    // 3. Register 15 consecutively losing trades under version v2 to trigger high regret limits
    for i in 0..15 {
        let ep = ClosedEpisode {
            id: format!("ep_loss_{}", i),
            symbol: "BTCUSDT".to_string(),
            direction: "Long".to_string(),
            entry_price: 60000.0,
            exit_price: 54000.0,
            stop_loss: 54000.0,
            take_profit: 66000.0,
            position_size: 1.0,
            pnl: -6000.0,
            pnl_pct: -0.10,
            outcome: "LOSS".to_string(),
            exit_reason: "stop_loss".to_string(),
            regret_score: 0.85,
            lesson: "Overfitted parameters".to_string(),
            confluence_score: 0.80,
            portfolio_heat: 0.05,
            market_regime: "TrendingBull".to_string(),
            session: "Normal".to_string(),
            agent_reasoning: "Incorrect trend persistence".to_string(),
            consecutive_losses_at_entry: i as u32,
            entry_time: (102000 + i * 1000).to_string(),
            exit_time: (102500 + i * 1000).to_string(),
            rule_version: 2,
            was_correct: false,
        };
        store
            .insert_closed_trade(&ep)
            .expect("Must insert losing trace");
    }

    // Register active pre-trade snapshot
    let mut initial_weights = HashMap::new();
    initial_weights.insert("news_analyser".to_string(), 0.25);
    initial_weights.insert("market_metrics_meter".to_string(), 0.25);

    let mut predictions = HashMap::new();
    predictions.insert("news_analyser".to_string(), 0.80);
    predictions.insert("market_metrics_meter".to_string(), 0.80);

    let pending = PreTradeSnapshot {
        episode_id: "trigger_episode".to_string(),
        symbol: "BTCUSDT".to_string(),
        direction: "BUY".to_string(),
        entry_price: 60000.0,
        rule_version: 2,
        active_weights: initial_weights,
        skill_predictions: predictions,
        layer_predictions: HashMap::new(),
    };
    processor.register_pending_trade(pending).await;

    // 4. Resolve the trade, which triggers the evaluation rollback checks
    let (_weights, evolved_config) = processor
        .process_trade_close(
            "trigger_episode",
            54000.0,
            MarketRegime::TrendingBull,
            10,
            &v2_rules,
            None,
        )
        .await
        .expect("Outcome evaluation must run successfully");

    // 5. Verification: Check that the rollback activated and restored v1 rules
    assert!(
        evolved_config.is_some(),
        "Rollback protocol must trigger and return restored config parameters"
    );

    let restored = evolved_config.unwrap();
    assert_eq!(restored.absolute_max_drawdown_pct, 0.15);
    assert_eq!(restored.absolute_max_leverage, 3);
    assert_eq!(restored.max_risk_per_trade_pct, 0.02);

    println!(
        "[TEST PASSED] Automated RULE_REVERT successfully triggered, falling back to version 1."
    );
}
