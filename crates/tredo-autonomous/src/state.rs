use crate::behavioral_psychology::BehavioralPsychologyEngine;
use crate::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
use crate::episode_store::EpisodeStore;
use crate::live_order_manager::LiveOrderManager;
use crate::types::{CotEntry, MarketRegime, PortfolioState, TradeSignal};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use tredo_core::paper_engine::{BrokerRegistry, PaperEngineConfig};
use tredo_core::{
    AdvancedPattern, CalendarEvent, Config, DisciplineRules, KnowledgeGraph, LlmExecutor,
    MemoryStore, NewsContext, OhlcvBar, PivotLevels, ServiceManager, SkillVote, TradingGoals,
    VectorMemory,
};
use tredo_core::{CandlestickPattern, MultiTfPatternConfirmation};

/// Maximum COT entries kept in RAM before flushing to SQLite.
/// Reduced from 50 to 20 to keep RAM footprint smaller.
const MAX_COT_RAM: usize = 20;

/// Flush COT entries to SQLite only every N pushes (batch flush).
/// This reduces SQLite writes by 100× compared to flushing on every push.
const COT_FLUSH_INTERVAL: usize = 100;

/// Auto-prune COT entries older than this many days from SQLite.
const COT_PRUNE_DAYS: u64 = 7;

/// Multi-timeframe market data for a single symbol
#[derive(Debug, Clone)]
pub struct TimeframeData {
    pub timeframe: String, // "1m", "5m", "15m", "30m", "1h", "2h", "4h", "8h", "12h", "1d", "1w"
    pub ohlcv: Vec<OhlcvBar>,
    pub pivots: Option<PivotLevels>,
    pub confluence: f64,
    pub last_updated: DateTime<Utc>,
}

/// Per-timeframe complete analysis snapshot — indicators + patterns + skills
#[derive(Debug, Clone)]
pub struct TimeframeAnalysis {
    pub timeframe: String,
    pub metrics: crate::market_metrics_meter::MetricsSnapshot,
    pub patterns: Vec<CandlestickPattern>,
    pub confluence: f64,
    pub aggregated_direction: String, // "bullish" | "bearish" | "neutral"
    pub aggregated_conviction: f64,   // 0.0 to 1.0
    pub last_updated: DateTime<Utc>,
}

/// Aggregated multi-timeframe signal — weighted combination across all 11 TFs
#[derive(Debug, Clone)]
pub struct MultiTfAggregate {
    pub symbol: String,
    /// Per-timeframe analysis snapshots
    pub tf_analyses: HashMap<String, TimeframeAnalysis>,
    /// Number of timeframes with valid data
    pub tf_count: usize,
    /// Weighted aggregate signal (-1.0 to 1.0), where higher weights on longer TFs
    pub aggregate_signal: f64,
    /// Aggregate direction
    pub aggregate_direction: String,
    /// How many TFs agree with the aggregate direction (0.0 to 1.0)
    pub agreement_pct: f64,
    /// Confluence-weighted average score
    pub weighted_confluence: f64,
    /// Last updated timestamp
    pub last_updated: DateTime<Utc>,
}

/// A scheduled task for the agent to execute at specific intervals
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentTask {
    pub name: String,
    pub interval_secs: u64,
    pub last_run: Option<DateTime<Utc>>,
    pub enabled: bool,
}

impl AgentTask {
    pub fn new(name: &str, interval_secs: u64) -> Self {
        Self {
            name: name.to_string(),
            interval_secs,
            last_run: None,
            enabled: true,
        }
    }

    pub fn should_run(&self, now: &DateTime<Utc>) -> bool {
        if !self.enabled {
            return false;
        }
        match self.last_run {
            Some(last) => (*now - last).num_seconds() as u64 >= self.interval_secs,
            None => true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SharedState {
    pub portfolio: Arc<RwLock<PortfolioState>>,
    pub memory: Arc<MemoryStore>,
    pub rules: Arc<RwLock<DisciplineRules>>,
    pub config: Arc<Config>,
    pub last_signals: Arc<RwLock<Vec<TradeSignal>>>,
    pub market_regime: Arc<RwLock<Option<MarketRegime>>>,
    pub llm: Arc<LlmExecutor>,
    /// Kronos forecast stored by MarketIntelligenceAgent (Phase 2) for use in StrategyDecisionAgent (Phase 5).
    pub last_forecast: Arc<RwLock<Option<serde_json::Value>>>,
    /// LLM reasoning from last cycle — stored for debugging / UI display.
    pub last_llm_reason: Arc<RwLock<String>>,
    /// Historical OHLCV data per symbol (1m) — capped at MAX_OHLCV_BARS per symbol.
    pub ohlcv_history: Arc<RwLock<HashMap<String, Vec<OhlcvBar>>>>,

    // ── 24/7 Agentic AI System Additions ──
    /// Economic calendar — upcoming high-impact events the agent should know about.
    pub calendar_events: Arc<RwLock<Vec<CalendarEvent>>>,
    /// Trading goals — targets and behavior mode the agent references for decisions.
    pub trading_goals: Arc<RwLock<TradingGoals>>,
    /// Dynamic watchlist — symbols the agent is currently monitoring.
    pub watchlist: Arc<RwLock<Vec<String>>>,
    /// Multi-timeframe OHLCV data (15m, 1h, 1d) per symbol.
    pub multi_timeframe_data: Arc<RwLock<HashMap<String, Vec<TimeframeData>>>>,
    /// Scheduled agent tasks for different workflows.
    pub agent_tasks: Arc<RwLock<Vec<AgentTask>>>,
    /// Last full-scan timestamp for the watchlist scanner.
    pub last_watchlist_scan: Arc<RwLock<Option<DateTime<Utc>>>>,
    /// The agent's latest summary of market conditions (generated by LLM reflection).
    pub agent_market_summary: Arc<RwLock<String>>,

    /// Latest episode ID per symbol — used to update outcomes when trades close.
    pub latest_episode: Arc<RwLock<HashMap<String, String>>>,

    /// Latest news context per symbol — fetched by NewsFetcher, injected into LLM prompts.
    pub latest_news: Arc<RwLock<HashMap<String, NewsContext>>>,

    /// Vector memory for semantic similarity search across episodes (now LanceDB backed for production).
    pub vector_memory: Arc<tokio::sync::RwLock<tredo_core::VectorMemory>>,

    /// Knowledge graph for relationship-based recall (symbol→regime→outcome).
    /// Built lazily from closed episodes on first query, then cached.
    pub knowledge_graph: Arc<RwLock<KnowledgeGraph>>,

    /// Latest detected candlestick patterns per symbol — populated by MarketIntelligenceAgent.
    pub last_patterns: Arc<RwLock<HashMap<String, Vec<CandlestickPattern>>>>,

    /// Multi-timeframe pattern confirmation per symbol — cross-references patterns across timeframes.
    pub last_mtf_patterns: Arc<RwLock<HashMap<String, MultiTfPatternConfirmation>>>,

    /// Advanced chart patterns (H&S, double tops, wedges, flags) per symbol.
    pub last_advanced_patterns: Arc<RwLock<HashMap<String, Vec<AdvancedPattern>>>>,

    /// Latest tri-level parallel verdict (rules + LLM + Kronos) per symbol.
    pub last_tri_level_verdict:
        Arc<RwLock<HashMap<String, crate::tri_level_validator::TriLevelVerdict>>>,

    /// Trust weights for tri-level layers — upgraded after each trade close.
    pub layer_trust_weights: Arc<RwLock<crate::tri_level_validator::LayerTrustWeights>>,

    /// Chain-of-thought store — capped at MAX_COT_RAM entries in RAM; older entries flushed to SQLite.
    pub cot_store: Arc<RwLock<Vec<CotEntry>>>,
    /// Atomic counter for generating unique COT entry IDs.
    pub cot_id_counter: Arc<AtomicU64>,

    /// Broadcast channel for real-time WS updates (COT, signals, prices, memory recalls) - connects TUI and clients.
    pub update_tx: Arc<tokio::sync::broadcast::Sender<String>>,
    /// SQLite-backed persistent store for closed trade episodes, regret events, COT logs.
    pub episode_store: Arc<EpisodeStore>,
    /// The most recent skill votes, captured by MarketIntelligenceAgent.
    /// Consumed by OutcomeProcessor when a trade closes.
    pub last_skill_votes: Arc<RwLock<Vec<SkillVote>>>,

    /// The aggregated signal from skills (produced by SkillAggregator in MI).
    /// Now wired into strategy decision for real use of ensemble (previously only in COT).
    pub last_aggregated_signal: Arc<RwLock<Option<tredo_core::AggregatedSignal>>>,

    /// Latest rich metrics snapshot per symbol from MarketMetricsMeter tool (RSI/MACD/ATR/BB/Stoch/vol/regime/fib/confluence).
    /// Connected to pipeline, debate, strategy (autonomous levels), aggregator (via skill), memory recall, WS price updates.
    pub latest_metrics: Arc<RwLock<HashMap<String, crate::market_metrics_meter::MetricsSnapshot>>>,

    /// Broker registry for live trading — routes orders through PaperBroker or Zerodha/other.
    /// When mode is Live, trades are executed on the real exchange via the registered broker adapter.
    pub broker_registry: Arc<BrokerRegistry>,

    /// Behavioral Psychology Engine — tracks emotional state, detects biases,
    /// and adjusts position sizing based on psychological health.
    /// Thread-safe via Arc<RwLock<>>.
    pub behavioral_psychology: Arc<RwLock<BehavioralPsychologyEngine>>,

    /// Service Manager — monitors health of external servers (LLM, Kronos, etc.)
    /// via periodic health checks. Status is broadcast via WebSocket for TUI display.
    pub service_manager: Arc<tredo_core::ServiceManager>,

    /// Circuit Breaker — anomaly detection for live trading. Monitors rejections,
    /// slippage, connection drops, and P&L drawdowns. Automatically halts trading
    /// when thresholds are breached.
    pub circuit_breaker: Arc<CircuitBreaker>,

    /// Live Order Manager — SQLite-backed order lifecycle tracker. Persists every
    /// live order placed through the broker for crash recovery, fill confirmation,
    /// and rejection tracking.
    pub live_order_manager: Arc<LiveOrderManager>,

    // ═══════════════════════════════════════════════════════════════════════
    // Multi-Timeframe Analysis (11 TFs: 1m/5m/15m/30m/1h/2h/4h/8h/12h/1d/1w)
    // ═══════════════════════════════════════════════════════════════════════
    /// Per-symbol, per-timeframe full analysis snapshots (all indicators + patterns)
    pub multi_tf_analyses: Arc<RwLock<HashMap<String, HashMap<String, TimeframeAnalysis>>>>,

    /// Per-symbol aggregated multi-timeframe signal (weighted across all 11 TFs)
    pub multi_tf_aggregate: Arc<RwLock<HashMap<String, MultiTfAggregate>>>,
}

impl SharedState {
    /// Get current skill weights (for FSM coordinator).
    pub fn get_skill_weights(&self) -> std::collections::HashMap<String, f64> {
        let mut weights = std::collections::HashMap::new();
        weights.insert("news_analyser".to_string(), 0.25);
        weights.insert("market_metrics_meter".to_string(), 0.25);
        weights.insert("sentiment_analyzer".to_string(), 0.25);
        weights.insert("on_chain_data".to_string(), 0.25);
        weights
    }

    /// Get current risk config (for FSM coordinator).
    pub fn get_risk_config(&self) -> crate::risk_guardian::RiskGuardianConfig {
        crate::risk_guardian::RiskGuardianConfig::default_fallback()
    }
    pub fn new(
        memory: MemoryStore,
        rules: DisciplineRules,
        config: Config,
        db_path: &str,
    ) -> Result<Self, rusqlite::Error> {
        // Open (or create) SQLite history database (with built-in recovery for locks/WAL)
        let episode_store = Arc::new(EpisodeStore::open(db_path)?);
        let portfolio = PortfolioState {
            cash_balance: config.initial_balance,
            total_equity: config.initial_balance,
            daily_pnl: 0.0,
            daily_pnl_pct: 0.0,
            open_positions: Vec::new(),
            total_trades_today: 0,
            winning_trades_today: 0,
            losing_trades_today: 0,
            consecutive_losses: 0,
            max_drawdown_today: 0.0,
            last_trade_time: None,
            last_trade_symbol: None,
            last_trade_by_symbol: HashMap::new(),
            trading_enabled: true,
        };

        // Generate economic calendar at startup
        let calendar = tredo_core::generate_economic_calendar();

        // Default watchlist
        let default_watchlist = Vec::new();

        // Default agent tasks for 24/7 operation
        let tasks = vec![
            AgentTask::new("price_scan", 5),           // Every 5 seconds
            AgentTask::new("position_monitor", 10),    // Every 10 seconds
            AgentTask::new("market_scan", 300),        // Every 5 minutes
            AgentTask::new("portfolio_review", 3600),  // Every hour
            AgentTask::new("goal_review", 43200),      // Every 12 hours
            AgentTask::new("daily_reflection", 86400), // Once per day
        ];

        let (update_tx, _) = tokio::sync::broadcast::channel(256);

        let llm = LlmExecutor::from_config(&config);

        Ok(Self {
            portfolio: Arc::new(RwLock::new(portfolio)),
            memory: Arc::new(memory),
            rules: Arc::new(RwLock::new(rules)),
            config: Arc::new(config),
            last_signals: Arc::new(RwLock::new(Vec::new())),
            market_regime: Arc::new(RwLock::new(None)),
            llm: Arc::new(llm),
            last_forecast: Arc::new(RwLock::new(None)),
            last_llm_reason: Arc::new(RwLock::new(String::new())),
            ohlcv_history: Arc::new(RwLock::new(HashMap::new())),
            calendar_events: Arc::new(RwLock::new(calendar)),
            trading_goals: Arc::new(RwLock::new(TradingGoals::default())),
            watchlist: Arc::new(RwLock::new(default_watchlist)),
            multi_timeframe_data: Arc::new(RwLock::new(HashMap::new())),
            agent_tasks: Arc::new(RwLock::new(tasks)),
            last_watchlist_scan: Arc::new(RwLock::new(None)),
            agent_market_summary: Arc::new(RwLock::new(String::new())),
            latest_episode: Arc::new(RwLock::new(HashMap::new())),
            latest_news: Arc::new(RwLock::new(HashMap::new())),
            vector_memory: Arc::new(tokio::sync::RwLock::new(VectorMemory::new(
                "tredo_vectors.json",
            ))), // With `lancedb` feature on tredo-core, first store() lazily creates sibling tredo_vectors.lance/ + migrates JSON (see vector_memory.rs)
            knowledge_graph: Arc::new(RwLock::new(KnowledgeGraph::new())),
            last_patterns: Arc::new(RwLock::new(HashMap::new())),
            last_mtf_patterns: Arc::new(RwLock::new(HashMap::new())),
            last_advanced_patterns: Arc::new(RwLock::new(HashMap::new())),
            last_tri_level_verdict: Arc::new(RwLock::new(HashMap::new())),
            layer_trust_weights: Arc::new(RwLock::new(
                crate::tri_level_validator::LayerTrustWeights::default(),
            )),
            cot_store: Arc::new(RwLock::new(Vec::new())),
            cot_id_counter: Arc::new(AtomicU64::new(1)),
            episode_store,
            last_skill_votes: Arc::new(RwLock::new(Vec::new())),
            last_aggregated_signal: Arc::new(RwLock::new(None)),
            latest_metrics: Arc::new(RwLock::new(HashMap::new())),
            update_tx: Arc::new(update_tx),
            circuit_breaker: Arc::new(CircuitBreaker::new(CircuitBreakerConfig::default())),
            live_order_manager: Arc::new(
                LiveOrderManager::open(Some("tredo_orders.db")).unwrap_or_else(|e| {
                    eprintln!("[LiveOrderManager] ⚠ Failed to open order DB: {}", e);
                    LiveOrderManager::open(Some(":memory:")).expect("In-memory fallback failed")
                }),
            ),
            broker_registry: Arc::new(BrokerRegistry::new(PaperEngineConfig::default())),
            behavioral_psychology: Arc::new(RwLock::new(BehavioralPsychologyEngine::new())),
            service_manager: Arc::new(ServiceManager::new()),
            multi_tf_analyses: Arc::new(RwLock::new(HashMap::new())),
            multi_tf_aggregate: Arc::new(RwLock::new(HashMap::new())),
        })
    }
}

impl SharedState {
    /// Refresh economic calendar from live API (FMP/AlphaVantage) or built-in fallback.
    pub async fn refresh_calendar(&self) {
        let events = tredo_core::fetch_economic_calendar_live().await;
        *self.calendar_events.write().await = events;
    }

    /// Push a chain-of-thought entry into the store and return its unique ID.
    #[allow(clippy::too_many_arguments)]
    pub async fn push_cot(
        &self,
        agent: &str,
        input: &str,
        action: &str,
        reason: &str,
        confidence: f64,
        chain_id: u64,
        parent_id: Option<u64>,
        symbol: Option<String>,
    ) -> u64 {
        self.push_cot_with_persist(
            agent, input, action, reason, confidence, chain_id, parent_id, symbol, true,
        )
        .await
    }

    /// Push a COT entry with an explicit persist flag.
    /// When `persist` is false, the entry is broadcast via WebSocket for real-time TUI display
    /// but is NOT flushed to SQLite. This is useful for per-agent pipeline steps that are
    /// only relevant for real-time monitoring, not historical analysis.
    /// Only summary entries (with `persist=true`) get stored in SQLite.
    #[allow(clippy::too_many_arguments)]
    pub async fn push_cot_with_persist(
        &self,
        agent: &str,
        input: &str,
        action: &str,
        reason: &str,
        confidence: f64,
        chain_id: u64,
        parent_id: Option<u64>,
        symbol: Option<String>,
        persist: bool,
    ) -> u64 {
        let id = self.cot_id_counter.fetch_add(1, Ordering::Relaxed);
        let entry = CotEntry {
            id,
            chain_id,
            parent_id,
            agent: agent.to_string(),
            input: input.to_string(),
            action: action.to_string(),
            reason: reason.to_string(),
            confidence,
            timestamp: Utc::now().to_rfc3339(),
            symbol,
        };
        let mut store = self.cot_store.write().await;
        store.push(entry.clone());
        let store_len = store.len();

        // Only flush to SQLite when `persist` is true. Per-agent COT entries
        // (persist=false) are only kept in RAM for real-time TUI display and
        // are dropped when RAM is full — they never touch SQLite.
        // This is the key fix for COT explosion: 17 entries per pipeline run
        // → only 1 summary entry per run persists to SQLite.
        if persist && store_len > MAX_COT_RAM + COT_FLUSH_INTERVAL {
            let drain_count = store_len - MAX_COT_RAM;
            let overflow: Vec<_> = store.drain(0..drain_count).collect();
            let rows: Vec<crate::episode_store::CotLogRow> = overflow
                .iter()
                .map(|e| crate::episode_store::CotLogRow {
                    chain_id: e.chain_id,
                    agent: e.agent.clone(),
                    action: e.action.clone(),
                    reason: e.reason.clone(),
                    confidence: e.confidence,
                    symbol: e.symbol.clone(),
                    ts: e.timestamp.clone(),
                })
                .collect();
            let _ = self.episode_store.flush_cot_batch(&rows);
        }

        // Broadcast for WS real-time (connects to TUI/clients with debate/trained data)
        // Include all fields the COT renderer expects: timestamp, input, id, chain_id, etc.
        let update = serde_json::json!({
            "type": "cot",
            "id": entry.id,
            "chain_id": entry.chain_id,
            "parent_id": entry.parent_id,
            "agent": entry.agent,
            "input": entry.input,
            "action": entry.action,
            "reason": entry.reason,
            "confidence": entry.confidence,
            "timestamp": entry.timestamp,
            "symbol": entry.symbol
        })
        .to_string();
        let _ = self.update_tx.send(update);

        id
    }

    /// Prune old COT entries from SQLite (older than COT_PRUNE_DAYS).
    /// Call this periodically (e.g., once per day or on startup) to prevent unbounded SQLite growth.
    pub async fn prune_old_cot_entries(&self) {
        use chrono::Duration;
        let cutoff = (Utc::now() - Duration::days(COT_PRUNE_DAYS as i64)).to_rfc3339();
        match self.episode_store.prune_cot_entries(&cutoff) {
            Ok(deleted) => {
                if deleted > 0 {
                    println!(
                        "[COT] 🧹 Pruned {} COT entries older than {} days from SQLite",
                        deleted, COT_PRUNE_DAYS
                    );
                }
            }
            Err(e) => eprintln!("[COT] ⚠ Failed to prune old COT entries: {}", e),
        }
    }

    /// Hierarchical memory recall for "smarter" agents: combines 3 layers:
    ///   1. Knowledge Graph (relationship-based: symbol→regime→outcome paths)
    ///   2. Vector RAG (semantic similarity on recent trained episodes)
    ///   3. AgentMemory (long-term shared trained lessons across sessions)
    ///
    ///   Returns formatted string for injection into reasoning/prompts/COT.
    pub async fn recall_trained_memory(
        &self,
        query_context: &str, // e.g. "proposed BUY on high vol low corr for BTC"
        top_k: usize,
    ) -> String {
        let mut parts = vec!["── HIERARCHICAL TRAINED MEMORY RECALL ──".to_string()];

        // Layer 1: Knowledge Graph (relationship-based recall — symbol→regime→outcome)
        {
            let kg = self.knowledge_graph.read().await;
            if kg.is_built() {
                // Extract symbol and regime from query context for targeted graph traversal
                let graph_result = self.graph_recall_from_context(&kg, query_context);
                if graph_result.total_episodes > 0 {
                    parts.push(graph_result.summary);
                }
            }
        }

        // Layer 2: Local vector RAG (fast, in-process, recent episodes with regret/lessons)
        {
            let vm = self.vector_memory.read().await;
            if !vm.is_empty() {
                match vm.search(query_context, top_k, &self.llm).await {
                    Ok(results) if !results.is_empty() => {
                        parts.push("LOCAL VECTOR (recent trained episodes):".to_string());
                        for r in results {
                            let regret = r
                                .regret_score
                                .map(|s| format!(" regret={:.2}", s))
                                .unwrap_or_default();
                            parts.push(format!(
                                "  - {} (sim {:.0}%{}): {}",
                                r.timestamp.format("%m/%d"),
                                r.similarity * 100.0,
                                regret,
                                r.summary_text
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }

        // Layer 3: Long-term agentmemory (shared, cross-session trained intelligence)
        {
            let mem = tredo_core::AgentMemoryClient::new();
            match mem
                .recall(&format!("trained lesson OR past action {}", query_context))
                .await
            {
                Ok(past) if !past.is_empty() => {
                    parts.push("LONG-TERM AGENTMEMORY (trained lessons across time):".to_string());
                    for p in past.iter().take(top_k) {
                        parts.push(format!("  - {}", p));
                    }
                }
                _ => {}
            }
        }

        if parts.len() == 1 {
            parts.push(
                "No strong trained memory match – proceeding with current rules + data only."
                    .to_string(),
            );
        }

        parts.join("\n")
    }

    /// Build the knowledge graph from closed episode store data.
    /// Called lazily on first recall or explicitly on startup.
    pub async fn rebuild_knowledge_graph(&self) {
        let episodes = match self.episode_store.fetch_closed_episodes_lite() {
            Ok(ep) => ep,
            Err(e) => {
                eprintln!("[GraphRAG] ⚠ Failed to fetch episodes for graph: {}", e);
                return;
            }
        };
        if episodes.is_empty() {
            return;
        }
        let mut kg = self.knowledge_graph.write().await;
        kg.build_from_episodes(&episodes);
    }

    /// Extract symbol/regime/direction from query context and run targeted graph traversal.
    fn graph_recall_from_context(
        &self,
        kg: &KnowledgeGraph,
        query_context: &str,
    ) -> tredo_core::graph_rag::GraphRecallResult {
        let qc = query_context.to_uppercase();

        // Try to extract symbol from the graph's own nodes (dynamic, not hardcoded)
        let symbol_nodes = kg.symbol_nodes();
        let found_symbol = symbol_nodes.iter().find(|s| qc.contains(s.as_str()));

        // Try to extract regime
        let known_regimes = ["TRENDINGBULL", "TRENDINGBEAR", "RANGING", "VOLATILE"];
        let found_regime = known_regimes.iter().find(|r| qc.contains(*r));

        match (found_symbol, found_regime) {
            (Some(sym), Some(reg)) => {
                // Most specific: symbol + regime
                kg.query_symbol_regime(sym, reg)
            }
            (Some(sym), None) => {
                // Symbol only — 2-hop traversal from symbol node
                let start = tredo_core::graph_rag::GraphNode::Symbol(sym.to_string());
                kg.query_relationship(&start, None, 2)
            }
            (None, Some(reg)) => {
                // Regime only — 2-hop traversal from regime node
                let start = tredo_core::graph_rag::GraphNode::Regime(reg.to_string());
                kg.query_relationship(&start, None, 2)
            }
            (None, None) => {
                // No specific entity found — return empty
                tredo_core::graph_rag::GraphRecallResult {
                    relationships: vec![],
                    total_episodes: 0,
                    aggregate_win_rate: 0.0,
                    aggregate_avg_pnl: 0.0,
                    summary: String::new(),
                }
            }
        }
    }

    /// Start a new COT chain (root node) — creates an entry with chain_id = own id.
    pub async fn start_cot_chain(
        &self,
        agent: &str,
        input: &str,
        action: &str,
        reason: &str,
        confidence: f64,
    ) -> u64 {
        let id = self.cot_id_counter.fetch_add(1, Ordering::Relaxed);
        let entry = CotEntry {
            id,
            chain_id: id,
            parent_id: None,
            agent: agent.to_string(),
            input: input.to_string(),
            action: action.to_string(),
            reason: reason.to_string(),
            confidence,
            timestamp: Utc::now().to_rfc3339(),
            symbol: None,
        };
        let mut store = self.cot_store.write().await;
        store.push(entry);
        let store_len = store.len();
        if store_len > MAX_COT_RAM + COT_FLUSH_INTERVAL {
            let drain_count = store_len - MAX_COT_RAM;
            let overflow: Vec<_> = store.drain(0..drain_count).collect();
            let rows: Vec<crate::episode_store::CotLogRow> = overflow
                .iter()
                .map(|e| crate::episode_store::CotLogRow {
                    chain_id: e.chain_id,
                    agent: e.agent.clone(),
                    action: e.action.clone(),
                    reason: e.reason.clone(),
                    confidence: e.confidence,
                    symbol: e.symbol.clone(),
                    ts: e.timestamp.clone(),
                })
                .collect();
            let _ = self.episode_store.flush_cot_batch(&rows);
        }
        id
    }

    /// Push a summary COT entry that embeds multiple layer results as a single entry.
    /// This is the "summary mode" — instead of 17 per-agent COT entries per pipeline run,
    /// the pipeline emits ONE entry with all layer data embedded in the reason field as JSON.
    #[allow(clippy::too_many_arguments)]
    pub async fn push_summary_cot(
        &self,
        chain_id: u64,
        symbol: &str,
        layers: Vec<(&str, &str, f64, &str)>, // (layer_name, action, confidence, reason)
        final_action: &str,
        final_reason: &str,
    ) -> u64 {
        let summary_json = serde_json::json!({
            "type": "pipeline_summary",
            "layers": layers.iter().map(|(name, action, conf, reason)| serde_json::json!({
                "agent": name,
                "action": action,
                "confidence": conf,
                "reason": reason
            })).collect::<Vec<_>>(),
            "final_action": final_action,
            "final_reason": final_reason
        });
        self.push_cot(
            "PipelineSummary",
            &format!("Full pipeline for {}", symbol),
            final_action,
            &summary_json.to_string(),
            1.0,
            chain_id,
            Some(chain_id),
            Some(symbol.to_string()),
        )
        .await
    }

    /// Register external services (LLM, Kronos) with the ServiceManager
    /// and spawn the background health check loop with WS status broadcasts.
    pub async fn register_and_monitor_services(&self) {
        // Register LLM server
        let llm_endpoint = self.config.llm_endpoint.clone();
        let llm_name = format!("llm_{}", self.config.llm_provider);
        self.service_manager
            .register_service(&llm_name, &llm_endpoint)
            .await;

        // Register Kronos forecast server
        let kronos_endpoint = self.config.kronos_service_url.clone();
        self.service_manager
            .register_service("kronos", &kronos_endpoint)
            .await;

        // Register Broker API — determine endpoint from env vars
        let (broker_id, broker_endpoint) = detect_broker_endpoint();
        self.service_manager
            .register_service(&broker_id, &broker_endpoint)
            .await;

        // Clone the service manager and update_tx for the background loop
        let mgr = self.service_manager.clone();
        let tx = self.update_tx.clone();

        // Spawn background health check loop (every 30 seconds)
        tokio::spawn(async move {
            loop {
                // Run health checks
                mgr.run_all_health_checks().await;

                // Broadcast status via WebSocket
                let statuses = mgr.get_all_statuses().await;
                let msg = serde_json::json!({
                    "type": "service_status",
                    "services": statuses,
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                })
                .to_string();
                let _ = tx.send(msg);

                // Wait 30 seconds
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
        });

        // Print initial status
        self.service_manager.print_status_board().await;
    }

    /// Broadcast current service status via WebSocket.
    pub async fn broadcast_service_status(&self) {
        let statuses = self.service_manager.get_all_statuses().await;
        let msg = serde_json::json!({
            "type": "service_status",
            "services": statuses,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        })
        .to_string();
        let _ = self.update_tx.send(msg);
    }

    /// Build a JSON portfolio snapshot for HTTP status and WebSocket clients.
    pub fn portfolio_snapshot_json(portfolio: &crate::types::PortfolioState) -> serde_json::Value {
        let mr = portfolio.total_trades_today.max(1);
        serde_json::json!({
            "total_equity": portfolio.total_equity,
            "cash_balance": portfolio.cash_balance,
            "daily_pnl": portfolio.daily_pnl,
            "daily_pnl_pct": portfolio.daily_pnl_pct,
            "open_positions_count": portfolio.open_positions.len(),
            "open_positions": portfolio.open_positions,
            "total_trades_today": portfolio.total_trades_today,
            "trades_today": portfolio.total_trades_today,
            "winning_trades_today": portfolio.winning_trades_today,
            "losing_trades_today": portfolio.losing_trades_today,
            "consecutive_losses": portfolio.consecutive_losses,
            "win_rate": portfolio.winning_trades_today as f64 / mr as f64,
            "max_drawdown_today": portfolio.max_drawdown_today,
            "trading_enabled": portfolio.trading_enabled,
        })
    }

    /// Push a portfolio snapshot to all WebSocket subscribers.
    pub async fn broadcast_portfolio_snapshot(&self) {
        let portfolio = self.portfolio.read().await;
        let mut snapshot = Self::portfolio_snapshot_json(&portfolio);
        if let Some(obj) = snapshot.as_object_mut() {
            obj.insert("type".to_string(), serde_json::json!("portfolio"));
        }
        let _ = self.update_tx.send(snapshot.to_string());
    }

    /// Add a step to an existing COT chain.
    /// Uses persist=false so per-agent pipeline entries are broadcast to the TUI
    /// for real-time display but are NOT stored in SQLite.
    /// Only summary entries (via push_summary_cot) persist to SQLite.
    ///
    /// NOTE: `quiet` flag — when true, the COT entry is skipped entirely.
    /// This eliminates per-agent write-lock contention on `cot_store` during
    /// automated pipeline runs (the summary entry at the end is still emitted).
    /// Use `quiet=false` for manual/interactive pipeline runs where TUI display matters.
    #[allow(clippy::too_many_arguments)]
    pub async fn add_cot_step_quiet(
        &self,
        chain_id: u64,
        agent: &str,
        input: &str,
        action: &str,
        reason: &str,
        confidence: f64,
        symbol: Option<String>,
        quiet: bool,
    ) -> u64 {
        if quiet {
            // Skip entirely — no lock acquired, no WS broadcast.
            // The summary COT entry at the end of the pipeline still fires.
            return 0;
        }
        self.push_cot_with_persist(
            agent,
            input,
            action,
            reason,
            confidence,
            chain_id,
            Some(chain_id),
            symbol,
            false, // persist=false — real-time TUI display only, no SQLite
        )
        .await;
        self.cot_id_counter.load(Ordering::Relaxed) - 1
    }

    /// Legacy non-quiet wrapper — calls add_cot_step_quiet(quiet=false).
    #[allow(clippy::too_many_arguments)]
    pub async fn add_cot_step(
        &self,
        chain_id: u64,
        agent: &str,
        input: &str,
        action: &str,
        reason: &str,
        confidence: f64,
        symbol: Option<String>,
    ) -> u64 {
        self.add_cot_step_quiet(
            chain_id, agent, input, action, reason, confidence, symbol, false,
        )
        .await
    }

    /// Broadcast a transient live agent communication event directly to the WebSocket channel.
    /// Does not write to DB. Used for live Ollama and Kronos API call streams in the TUI.
    pub async fn push_live_comm(
        &self,
        from: &str,
        to: &str,
        action: &str,
        reason: &str,
        symbol: Option<String>,
    ) {
        let update = serde_json::json!({
            "type": "cot",
            "agent": from,
            "to": to,
            "action": action,
            "reason": reason,
            "confidence": 0.0,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "symbol": symbol
        })
        .to_string();
        let _ = self.update_tx.send(update);
    }
}

/// Detect which broker is configured and return its API endpoint.
/// Checks env vars for Alpaca and Zerodha credentials.
/// Returns (service_name, endpoint_url) — endpoint is empty if no external broker is configured.
fn detect_broker_endpoint() -> (String, String) {
    // Check Alpaca first
    let alpaca_key = std::env::var("ALPACA_API_KEY_ID").ok();
    let zerodha_key = std::env::var("ZERODHA_API_KEY").ok();

    if let Some(_key) = alpaca_key {
        let paper_mode = std::env::var("ALPACA_PAPER")
            .map(|v| v == "true")
            .unwrap_or(true);
        let endpoint = if paper_mode {
            "https://paper-api.alpaca.markets".to_string()
        } else {
            "https://api.alpaca.markets".to_string()
        };
        ("broker_alpaca".to_string(), endpoint)
    } else if let Some(_key) = zerodha_key {
        (
            "broker_zerodha".to_string(),
            "https://api.kite.trade".to_string(),
        )
    } else {
        // No live broker configured — register with empty endpoint
        // The ServiceManager will treat it as always healthy (no external ping needed).
        ("broker_paper".to_string(), String::new())
    }
}

pub async fn initialize_autonomous_system(
) -> Result<crate::AutonomousOrchestrator, Box<dyn std::error::Error + Send + Sync>> {
    let memory = MemoryStore::new("tredo.redb")?;
    let rules = DisciplineRules::default();
    let config = Config::default();
    let state = SharedState::new(memory, rules, config, "tredo_history.db")
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;

    // Load live economic calendar (falls back to built-in events)
    state.refresh_calendar().await;

    // Register external services with the ServiceManager
    state.register_and_monitor_services().await;

    // Build knowledge graph from closed episodes immediately (so recall has graph data from start)
    state.rebuild_knowledge_graph().await;

    Ok(crate::AutonomousOrchestrator::new(state))
}
