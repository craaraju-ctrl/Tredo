// graph_rag.rs — Knowledge Graph for relationship-based recall.
//
// Builds a directed graph from closed trade episodes:
//
//   Symbol ──TRADED_IN──▶ Episode ──IN_REGIME──▶ Regime
//                             │
//                             ├──HAS_DIRECTION──▶ Direction
//                             │
//                             ├──RESULTED_IN──▶ Outcome (WIN/LOSS)
//                             │
//                             └──IN_CONFLUENCE──▶ ConfluenceBucket
//
// Graph traversal enables queries like:
//   "What happened to BTC in TrendingBull regime?"
//   "What's the win rate for Long trades in Ranging markets?"
//   "How does ETH perform in Volatile regimes at high confluence?"
//
// The graph is built lazily from EpisodeStore on first query, then cached.
// Persistence: serialized to JSON for fast startup without re-querying SQLite.

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ═══════════════════════════════════════════════════════════════════════════════
// Node Types
// ═══════════════════════════════════════════════════════════════════════════════

/// Every node in the knowledge graph is one of these entity types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum GraphNode {
    /// A trading symbol (e.g. "BTC", "ETH", "NIFTY")
    Symbol(String),
    /// A market regime (e.g. "TrendingBull", "Volatile", "Ranging")
    Regime(String),
    /// Trade direction (e.g. "Long", "Short")
    Direction(String),
    /// Trade outcome (e.g. "WIN", "LOSS", "BREAKEVEN")
    Outcome(String),
    /// Confluence bucket (e.g. "LOW", "MED", "HIGH")
    ConfluenceBucket(String),
}

impl std::fmt::Display for GraphNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphNode::Symbol(s) => write!(f, "Symbol({})", s),
            GraphNode::Regime(r) => write!(f, "Regime({})", r),
            GraphNode::Direction(d) => write!(f, "Dir({})", d),
            GraphNode::Outcome(o) => write!(f, "Outcome({})", o),
            GraphNode::ConfluenceBucket(c) => write!(f, "Conf({})", c),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Edge Weights
// ═══════════════════════════════════════════════════════════════════════════════

/// Edge weight tracks aggregate statistics for a relationship.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EdgeWeight {
    /// Number of episodes that traversed this edge.
    pub frequency: u32,
    /// Win rate for this specific relationship path (0.0–1.0).
    pub win_rate: f64,
    /// Average P&L% for episodes on this edge.
    pub avg_pnl_pct: f64,
    /// Average regret score (0.0 = good, 1.0 = bad).
    pub avg_regret: f64,
}

impl EdgeWeight {
    /// Merge another observation into this edge weight (running average).
    pub fn merge(&mut self, pnl_pct: f64, was_win: bool, regret: f64) {
        let n = self.frequency as f64;
        let new_n = n + 1.0;
        self.avg_pnl_pct = (self.avg_pnl_pct * n + pnl_pct) / new_n;
        self.avg_regret = (self.avg_regret * n + regret) / new_n;
        let win_count = self.win_rate * n + if was_win { 1.0 } else { 0.0 };
        self.win_rate = win_count / new_n;
        self.frequency += 1;
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Graph Query Results
// ═══════════════════════════════════════════════════════════════════════════════

/// A relationship found by graph traversal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphRelationship {
    pub from: GraphNode,
    pub to: GraphNode,
    pub weight: EdgeWeight,
}

/// Summary of graph recall for a specific query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphRecallResult {
    /// Relationships found by traversal.
    pub relationships: Vec<GraphRelationship>,
    /// Total episodes observed in the traversed subgraph.
    pub total_episodes: u32,
    /// Aggregate win rate across all traversed relationships.
    pub aggregate_win_rate: f64,
    /// Aggregate avg P&L across all traversed relationships.
    pub aggregate_avg_pnl: f64,
    /// Human-readable summary for LLM injection.
    pub summary: String,
}

// ═══════════════════════════════════════════════════════════════════════════════
// Knowledge Graph
// ═══════════════════════════════════════════════════════════════════════════════

/// In-memory knowledge graph built from closed trade episodes.
///
/// Uses petgraph::DiGraph for relationship storage and BFS traversal.
/// The graph is built lazily on first query, then cached in memory.
/// Persistence via JSON serialization for fast startup.
#[derive(Debug, Clone)]
pub struct KnowledgeGraph {
    graph: DiGraph<GraphNode, EdgeWeight>,
    /// Fast lookup: node value → NodeIndex
    node_index: HashMap<GraphNode, NodeIndex>,
    /// Whether the graph has been built from episode data
    built: bool,
}

impl KnowledgeGraph {
    /// Create an empty knowledge graph.
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            node_index: HashMap::new(),
            built: false,
        }
    }

    /// Get or create a node, returning its index.
    fn get_or_create_node(&mut self, node: GraphNode) -> NodeIndex {
        if let Some(&idx) = self.node_index.get(&node) {
            idx
        } else {
            let idx = self.graph.add_node(node.clone());
            self.node_index.insert(node, idx);
            idx
        }
    }

    /// Add or update an edge between two nodes with a new observation.
    fn add_observation(
        &mut self,
        from: GraphNode,
        to: GraphNode,
        pnl_pct: f64,
        was_win: bool,
        regret: f64,
    ) {
        let from_idx = self.get_or_create_node(from);
        let to_idx = self.get_or_create_node(to);

        // Check if edge already exists
        if let Some(edge_idx) = self.graph.find_edge(from_idx, to_idx) {
            let weight = self.graph.edge_weight_mut(edge_idx).unwrap();
            weight.merge(pnl_pct, was_win, regret);
        } else {
            let mut weight = EdgeWeight::default();
            weight.merge(pnl_pct, was_win, regret);
            self.graph.add_edge(from_idx, to_idx, weight);
        }
    }

    /// Build the knowledge graph from a list of closed episodes.
    ///
    /// Each episode creates edges:
    ///   Episode → Symbol (TRADED_IN)
    ///   Episode → Regime (IN_REGIME)
    ///   Episode → Direction (HAS_DIRECTION)
    ///   Episode → Outcome (RESULTED_IN)
    ///   Episode → ConfluenceBucket (IN_CONFLUENCE)
    ///
    /// For simplicity, we connect entity nodes directly (Symbol ↔ Regime ↔ Direction etc.)
    /// rather than creating intermediate episode nodes, since the graph is used for
    /// aggregate relationship queries, not individual episode lookup.
    pub fn build_from_episodes(&mut self, episodes: &[ClosedEpisodeLite]) {
        for ep in episodes {
            let pnl_pct = ep.pnl_pct;
            let was_win = ep.was_correct;
            let regret = ep.regret_score;

            let sym = GraphNode::Symbol(ep.symbol.clone());
            let regime = GraphNode::Regime(ep.market_regime.clone());
            let dir = GraphNode::Direction(ep.direction.clone());
            let outcome = GraphNode::Outcome(ep.outcome.clone());
            let conf_bucket = GraphNode::ConfluenceBucket(confluence_label(ep.confluence_score));

            // Symbol ↔ Regime
            self.add_observation(sym.clone(), regime.clone(), pnl_pct, was_win, regret);
            // Symbol ↔ Direction
            self.add_observation(sym.clone(), dir.clone(), pnl_pct, was_win, regret);
            // Symbol ↔ Outcome
            self.add_observation(sym.clone(), outcome.clone(), pnl_pct, was_win, regret);
            // Symbol ↔ ConfluenceBucket
            self.add_observation(sym.clone(), conf_bucket.clone(), pnl_pct, was_win, regret);
            // Regime ↔ Direction
            self.add_observation(regime.clone(), dir.clone(), pnl_pct, was_win, regret);
            // Regime ↔ Outcome
            self.add_observation(regime.clone(), outcome.clone(), pnl_pct, was_win, regret);
            // Direction ↔ Outcome
            self.add_observation(dir, outcome, pnl_pct, was_win, regret);
        }

        self.built = true;
        println!(
            "[GraphRAG] ✅ Built graph: {} nodes, {} edges from {} episodes",
            self.graph.node_count(),
            self.graph.edge_count(),
            episodes.len()
        );
    }

    /// Query: "What happens to {symbol} in {regime} regime?"
    ///
    /// Traverses 2 hops from the symbol node, collecting all reachable
    /// relationships with aggregate statistics.
    pub fn query_symbol_regime(&self, symbol: &str, regime: &str) -> GraphRecallResult {
        let sym_node = GraphNode::Symbol(symbol.to_string());
        self.query_relationship(&sym_node, Some(&GraphNode::Regime(regime.to_string())), 2)
    }

    /// Query: "What's the win rate for {direction} trades in {regime} markets?"
    pub fn query_direction_regime(&self, direction: &str, regime: &str) -> GraphRecallResult {
        let dir_node = GraphNode::Direction(direction.to_string());
        self.query_relationship(&dir_node, Some(&GraphNode::Regime(regime.to_string())), 2)
    }

    /// Generic 2-hop relationship query from a starting node.
    ///
    /// If `target` is provided, only includes relationships that connect
    /// to or through the target node type. Otherwise, collects all 2-hop
    /// relationships from the start node.
    pub fn query_relationship(
        &self,
        start: &GraphNode,
        target: Option<&GraphNode>,
        max_depth: usize,
    ) -> GraphRecallResult {
        let &start_idx = match self.node_index.get(start) {
            Some(idx) => idx,
            None => {
                return GraphRecallResult {
                    relationships: vec![],
                    total_episodes: 0,
                    aggregate_win_rate: 0.0,
                    aggregate_avg_pnl: 0.0,
                    summary: format!("GraphRAG: No historical data for {}.", start),
                };
            }
        };

        let mut relationships = Vec::new();
        let mut visited: std::collections::HashSet<NodeIndex> = std::collections::HashSet::new();
        let mut queue: Vec<(NodeIndex, usize)> = vec![(start_idx, 0)];
        visited.insert(start_idx);

        while let Some((current, depth)) = queue.pop() {
            if depth >= max_depth {
                continue;
            }

            // Walk outgoing edges
            for edge in self.graph.edges(current) {
                let target_idx = edge.target();
                let weight = edge.weight();

                let from_node = self.graph[current].clone();
                let to_node = self.graph[target_idx].clone();

                // Filter by target if provided
                let matches_target = match target {
                    Some(t) => to_node == *t || from_node == *t,
                    None => true,
                };

                if matches_target {
                    relationships.push(GraphRelationship {
                        from: from_node,
                        to: to_node,
                        weight: weight.clone(),
                    });
                }

                // Continue BFS if not visited and within depth
                if visited.insert(target_idx) && depth + 1 < max_depth {
                    queue.push((target_idx, depth + 1));
                }
            }

            // Walk incoming edges too (bidirectional traversal)
            for edge in self
                .graph
                .edges_directed(current, petgraph::Direction::Incoming)
            {
                let source_idx = edge.source();
                let weight = edge.weight();

                let from_node = self.graph[source_idx].clone();
                let to_node = self.graph[current].clone();

                let matches_target = match target {
                    Some(t) => to_node == *t || from_node == *t,
                    None => true,
                };

                if matches_target
                    && !relationships
                        .iter()
                        .any(|r| r.from == from_node && r.to == to_node)
                {
                    relationships.push(GraphRelationship {
                        from: from_node,
                        to: to_node,
                        weight: weight.clone(),
                    });
                }

                if visited.insert(source_idx) && depth + 1 < max_depth {
                    queue.push((source_idx, depth + 1));
                }
            }
        }

        // Compute aggregates
        let total_episodes: u32 = relationships.iter().map(|r| r.weight.frequency).sum();
        let aggregate_win_rate = if total_episodes > 0 {
            let total_wins: f64 = relationships
                .iter()
                .map(|r| r.weight.win_rate * r.weight.frequency as f64)
                .sum();
            total_wins / total_episodes as f64
        } else {
            0.0
        };
        let aggregate_avg_pnl = if total_episodes > 0 {
            relationships
                .iter()
                .map(|r| r.weight.avg_pnl_pct * r.weight.frequency as f64)
                .sum::<f64>()
                / total_episodes as f64
        } else {
            0.0
        };

        // Build human-readable summary
        let summary = self.format_summary(
            start,
            &relationships,
            total_episodes,
            aggregate_win_rate,
            aggregate_avg_pnl,
        );

        GraphRecallResult {
            relationships,
            total_episodes,
            aggregate_win_rate,
            aggregate_avg_pnl,
            summary,
        }
    }

    /// Format a human-readable summary for LLM injection.
    fn format_summary(
        &self,
        start: &GraphNode,
        relationships: &[GraphRelationship],
        total: u32,
        win_rate: f64,
        avg_pnl: f64,
    ) -> String {
        if total == 0 {
            return format!("GraphRAG: No historical data for {}.", start);
        }

        let mut lines = vec![format!(
            "── GRAPH RAG: {} ({} observations) ──",
            start, total
        )];

        for rel in relationships {
            lines.push(format!(
                "  {} → {}: {} trades, {:.0}% win, avg P&L {:+.2}%, avg regret {:.2}",
                rel.from,
                rel.to,
                rel.weight.frequency,
                rel.weight.win_rate * 100.0,
                rel.weight.avg_pnl_pct,
                rel.weight.avg_regret
            ));
        }

        lines.push(format!(
            "  Aggregate: {:.0}% win rate, avg P&L {:+.2}%",
            win_rate * 100.0,
            avg_pnl
        ));

        lines.join("\n")
    }

    /// Check if the graph has been built.
    pub fn is_built(&self) -> bool {
        self.built
    }

    /// Return all symbol node labels in the graph (for dynamic query extraction).
    pub fn symbol_nodes(&self) -> Vec<String> {
        self.node_index
            .keys()
            .filter_map(|node| match node {
                GraphNode::Symbol(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    }

    /// Number of nodes in the graph.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Number of edges in the graph.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    // ── Serialization ───────────────────────────────────────────────────

    /// Serialize the graph to JSON for persistence.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        let serializable = SerializableGraph {
            nodes: self
                .graph
                .node_indices()
                .map(|idx| self.graph[idx].clone())
                .collect(),
            edges: self
                .graph
                .edge_indices()
                .map(|idx| {
                    let (from, to) = self.graph.edge_endpoints(idx).unwrap();
                    (
                        self.graph[from].clone(),
                        self.graph[to].clone(),
                        self.graph[idx].clone(),
                    )
                })
                .collect(),
        };
        serde_json::to_string(&serializable)
    }

    /// Deserialize the graph from JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let data: SerializableGraph = serde_json::from_str(json)?;
        let mut kg = KnowledgeGraph::new();

        for node in data.nodes {
            kg.get_or_create_node(node);
        }

        for (from, to, weight) in data.edges {
            let from_idx = kg.get_or_create_node(from);
            let to_idx = kg.get_or_create_node(to);
            kg.graph.add_edge(from_idx, to_idx, weight);
        }

        kg.built = true;
        Ok(kg)
    }

    /// Save the graph to a JSON file.
    pub fn save_to_file(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let json = self.to_json()?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load the graph from a JSON file.
    pub fn load_from_file(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let json = std::fs::read_to_string(path)?;
        Ok(Self::from_json(&json)?)
    }
}

impl Default for KnowledgeGraph {
    fn default() -> Self {
        Self::new()
    }
}

// ── Serialization helper ─────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct SerializableGraph {
    nodes: Vec<GraphNode>,
    edges: Vec<(GraphNode, GraphNode, EdgeWeight)>,
}

// ── Data types for building from episodes ─────────────────────────────────

/// Lightweight episode data needed for graph construction.
/// Avoids pulling the full ClosedEpisode from SQLite just for graph building.
#[derive(Debug, Clone)]
pub struct ClosedEpisodeLite {
    pub symbol: String,
    pub direction: String,
    pub outcome: String,
    pub pnl_pct: f64,
    pub regret_score: f64,
    pub was_correct: bool,
    pub market_regime: String,
    pub confluence_score: f64,
}

/// Map a confluence score to a bucket label.
fn confluence_label(score: f64) -> String {
    if score < 0.55 {
        "LOW".to_string()
    } else if score < 0.70 {
        "MED".to_string()
    } else {
        "HIGH".to_string()
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn make_episode(
        symbol: &str,
        direction: &str,
        outcome: &str,
        regime: &str,
        pnl_pct: f64,
        was_correct: bool,
        regret: f64,
        confluence: f64,
    ) -> ClosedEpisodeLite {
        ClosedEpisodeLite {
            symbol: symbol.to_string(),
            direction: direction.to_string(),
            outcome: outcome.to_string(),
            pnl_pct,
            regret_score: regret,
            was_correct,
            market_regime: regime.to_string(),
            confluence_score: confluence,
        }
    }

    #[test]
    fn test_build_graph_from_episodes() {
        let episodes = vec![
            make_episode("BTC", "Long", "WIN", "TrendingBull", 2.5, true, 0.1, 0.75),
            make_episode("BTC", "Long", "WIN", "TrendingBull", 1.8, true, 0.0, 0.80),
            make_episode("BTC", "Short", "LOSS", "Volatile", -1.2, false, 0.6, 0.50),
            make_episode("ETH", "Long", "WIN", "TrendingBull", 3.1, true, 0.05, 0.85),
            make_episode("ETH", "Short", "LOSS", "Ranging", -0.8, false, 0.4, 0.45),
        ];

        let mut kg = KnowledgeGraph::new();
        kg.build_from_episodes(&episodes);

        assert!(kg.is_built());
        assert!(kg.node_count() > 0);
        assert!(kg.edge_count() > 0);
    }

    #[test]
    fn test_query_symbol_regime() {
        let episodes = vec![
            make_episode("BTC", "Long", "WIN", "TrendingBull", 2.5, true, 0.1, 0.75),
            make_episode("BTC", "Long", "WIN", "TrendingBull", 1.8, true, 0.0, 0.80),
            make_episode("BTC", "Short", "LOSS", "Volatile", -1.2, false, 0.6, 0.50),
            make_episode("ETH", "Long", "WIN", "TrendingBull", 3.1, true, 0.05, 0.85),
        ];

        let mut kg = KnowledgeGraph::new();
        kg.build_from_episodes(&episodes);

        let result = kg.query_symbol_regime("BTC", "TrendingBull");
        assert!(result.total_episodes > 0);
        assert!(!result.relationships.is_empty());
        assert!(!result.summary.is_empty());
        // BTC in TrendingBull should have good stats
        assert!(result.aggregate_win_rate > 0.5);
    }

    #[test]
    fn test_query_direction_regime() {
        let episodes = vec![
            make_episode("BTC", "Long", "WIN", "TrendingBull", 2.5, true, 0.1, 0.75),
            make_episode("ETH", "Long", "WIN", "TrendingBull", 3.1, true, 0.05, 0.85),
            make_episode("SOL", "Short", "LOSS", "Ranging", -0.8, false, 0.4, 0.45),
        ];

        let mut kg = KnowledgeGraph::new();
        kg.build_from_episodes(&episodes);

        let result = kg.query_direction_regime("Long", "TrendingBull");
        assert!(result.total_episodes > 0);
        assert!(result.aggregate_win_rate > 0.5);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let episodes = vec![
            make_episode("BTC", "Long", "WIN", "TrendingBull", 2.5, true, 0.1, 0.75),
            make_episode("BTC", "Short", "LOSS", "Volatile", -1.2, false, 0.6, 0.50),
        ];

        let mut kg = KnowledgeGraph::new();
        kg.build_from_episodes(&episodes);

        let json = kg.to_json().unwrap();
        let kg2 = KnowledgeGraph::from_json(&json).unwrap();

        assert_eq!(kg.node_count(), kg2.node_count());
        assert_eq!(kg.edge_count(), kg2.edge_count());

        let result1 = kg.query_symbol_regime("BTC", "TrendingBull");
        let result2 = kg2.query_symbol_regime("BTC", "TrendingBull");
        assert_eq!(result1.total_episodes, result2.total_episodes);
    }

    #[test]
    fn test_empty_graph_query() {
        let kg = KnowledgeGraph::new();
        let result = kg.query_symbol_regime("BTC", "TrendingBull");
        assert_eq!(result.total_episodes, 0);
        assert!(result.summary.contains("No historical data"));
    }

    #[test]
    fn test_edge_weight_merge() {
        let mut w = EdgeWeight::default();
        w.merge(2.0, true, 0.1);
        assert_eq!(w.frequency, 1);
        assert!((w.win_rate - 1.0).abs() < 0.001);
        assert!((w.avg_pnl_pct - 2.0).abs() < 0.001);

        w.merge(-1.0, false, 0.5);
        assert_eq!(w.frequency, 2);
        assert!((w.win_rate - 0.5).abs() < 0.001);
        assert!((w.avg_pnl_pct - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_confluence_label() {
        assert_eq!(confluence_label(0.3), "LOW");
        assert_eq!(confluence_label(0.6), "MED");
        assert_eq!(confluence_label(0.85), "HIGH");
    }

    #[test]
    fn test_graph_persistence_file() {
        let episodes = vec![make_episode(
            "BTC",
            "Long",
            "WIN",
            "TrendingBull",
            2.5,
            true,
            0.1,
            0.75,
        )];

        let mut kg = KnowledgeGraph::new();
        kg.build_from_episodes(&episodes);

        let path = "_test_graph_rag.json";
        kg.save_to_file(path).unwrap();
        let kg2 = KnowledgeGraph::load_from_file(path).unwrap();

        assert_eq!(kg.node_count(), kg2.node_count());
        let _ = std::fs::remove_file(path);
    }
}
