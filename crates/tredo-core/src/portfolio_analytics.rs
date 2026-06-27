//! # Portfolio-Level Analytics
//!
//! Advanced quantitative portfolio management tools:
//! - **Kelly Criterion** — Optimal position sizing based on edge/odds
//! - **Mean-Variance Optimization** — Markowitz portfolio optimization
//! - **Efficient Frontier** — Generate frontier points for visualization
//! - **Value at Risk (VaR)** — Historical and parametric VaR estimation
//!
//! All calculations are pure Rust — no external dependencies beyond `serde`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Kelly Criterion ─────────────────────────────────────────────────────────

/// Result of a Kelly Criterion calculation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KellyAllocation {
    /// The Kelly fraction (0.0 to 1.0) — what fraction of capital to risk
    pub fraction: f64,
    /// Half-Kelly (more conservative, recommended for most traders)
    pub half_kelly: f64,
    /// Quarter-Kelly (very conservative)
    pub quarter_kelly: f64,
    /// Optimal position size in dollars
    pub position_size_dollars: f64,
    /// Optimal position size in units
    pub position_size_units: f64,
    /// Probability of winning (0.0 to 1.0)
    pub win_probability: f64,
    /// Win/loss ratio (average win / average loss)
    pub win_loss_ratio: f64,
    /// Edge = win_prob * win_loss_ratio - (1 - win_prob)
    pub edge: f64,
    /// Maximum recommended risk as % of portfolio
    pub max_risk_pct: f64,
}

/// Calculate the Kelly Criterion fraction for position sizing.
///
/// Formula: f* = (p * b - q) / b
/// where:
///   p = probability of winning
///   b = win/loss ratio (average win / average loss)
///   q = probability of losing (1 - p)
///
/// Returns 0 (no bet) when edge is negative.
///
/// # Arguments
/// * `win_probability` — Historical win rate (0.0 to 1.0)
/// * `avg_win` — Average profit on winning trades (dollars)
/// * `avg_loss` — Average loss on losing trades (dollars, positive)
/// * `account_balance` — Current account equity
/// * `entry_price` — Current entry price of the asset
/// * `conservative` — If true, use half-kelly (recommended)
pub fn kelly_criterion_fraction(
    win_probability: f64,
    avg_win: f64,
    avg_loss: f64,
    account_balance: f64,
    entry_price: f64,
    conservative: bool,
) -> KellyAllocation {
    let p = win_probability.clamp(0.0, 1.0);
    let avg_win = avg_win.max(0.01);
    let avg_loss = avg_loss.max(0.01);

    let b = avg_win / avg_loss; // win/loss ratio
    let q = 1.0 - p;
    let edge = p * b - q;

    let fraction = if edge > 0.0 && b > 0.0 {
        (edge / b).clamp(0.0, 0.25) // Cap at 25% of capital
    } else {
        0.0
    };

    let half_kelly = fraction * 0.5;
    let quarter_kelly = fraction * 0.25;

    let use_fraction = if conservative { half_kelly } else { fraction };
    let max_risk_pct = use_fraction;

    let max_risk_dollars = account_balance * max_risk_pct;

    // Position size in dollars = max_risk_dollars (full Kelly on risk), then
    // position size in units = max_risk_dollars / entry_price
    let position_size_dollars = max_risk_dollars;
    let position_size_units = if entry_price > 0.0 {
        max_risk_dollars / entry_price
    } else {
        0.0
    };

    KellyAllocation {
        fraction,
        half_kelly,
        quarter_kelly,
        position_size_dollars,
        position_size_units,
        win_probability: p,
        win_loss_ratio: b,
        edge,
        max_risk_pct,
    }
}

/// Allocate capital across multiple assets using the Kelly-optimal portfolio.
/// Uses a simplified approach: compute Kelly fraction per asset, normalize.
pub fn optimal_kelly_portfolio(
    assets: &[(&str, f64, f64, f64)], // (symbol, win_rate, avg_win, avg_loss)
    total_capital: f64,
    conservative: bool,
) -> Vec<(String, f64, f64)> {
    let mut allocations = Vec::new();
    let mut total_edge = 0.0;
    let mut per_asset = Vec::new();

    for &(symbol, win_rate, avg_win, avg_loss) in assets {
        let p = win_rate.clamp(0.0, 1.0);
        let b = avg_win.max(0.01) / avg_loss.max(0.01);
        let edge = p * b - (1.0 - p);
        if edge > 0.0 {
            let fraction = (edge / b).clamp(0.0, 0.25);
            let use_fraction = if conservative {
                fraction * 0.5
            } else {
                fraction
            };
            total_edge += edge.max(0.0);
            per_asset.push((symbol.to_string(), use_fraction, edge));
        }
    }

    if total_edge <= 0.0 || per_asset.is_empty() {
        return allocations;
    }

    // Normalize fractions to sum to 1.0, then apply total Kelly exposure
    let total_fraction: f64 = per_asset.iter().map(|(_, f, _)| f).sum();
    let overall_exposure = total_fraction.min(1.0);

    for (symbol, fraction, _edge) in &per_asset {
        let weight = if total_fraction > 0.0 {
            fraction / total_fraction
        } else {
            0.0
        };
        let allocation_pct = weight * overall_exposure;
        let dollars = total_capital * allocation_pct;
        allocations.push((symbol.clone(), allocation_pct, dollars));
    }

    allocations
}

// ── Mean-Variance Optimization ──────────────────────────────────────────────

/// Result of a mean-variance optimization run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeanVarianceResult {
    /// Asset weights that maximize Sharpe ratio
    pub optimal_weights: HashMap<String, f64>,
    /// Expected portfolio return
    pub expected_return: f64,
    /// Expected portfolio volatility (standard deviation)
    pub portfolio_volatility: f64,
    /// Sharpe ratio of the optimal portfolio (assumes risk-free rate = 0)
    pub sharpe_ratio: f64,
    /// Diversification ratio (1.0 = same as single asset, >1.0 = diversified)
    pub diversification_ratio: f64,
}

/// Run mean-variance optimization to find the maximum Sharpe ratio portfolio.
///
/// Uses a simplified grid search approach since we don't have a proper
/// quadratic optimizer. For 2-5 assets this is sufficient.
///
/// # Arguments
/// * `expected_returns` — Map of asset symbol to expected annual return
/// * `volatilities` — Map of asset symbol to annual volatility (standard deviation)
/// * `correlations` — Correlation matrix as (asset_i, asset_j) -> correlation
/// * `risk_free_rate` — Risk-free rate (default 0.05 for 5%)
pub fn mean_variance_optimize(
    expected_returns: &HashMap<String, f64>,
    volatilities: &HashMap<String, f64>,
    correlations: &HashMap<(String, String), f64>,
    risk_free_rate: f64,
) -> MeanVarianceResult {
    let symbols: Vec<&String> = expected_returns.keys().collect();
    let n = symbols.len();

    if n == 1 {
        // Single asset case
        let sym = symbols[0].clone();
        let ret = *expected_returns.get(&sym).unwrap_or(&0.0);
        let vol = *volatilities.get(&sym).unwrap_or(&0.0);
        let sharpe = if vol > 0.0 {
            (ret - risk_free_rate) / vol
        } else {
            0.0
        };
        let mut weights = HashMap::new();
        weights.insert(sym.clone(), 1.0);
        return MeanVarianceResult {
            optimal_weights: weights,
            expected_return: ret,
            portfolio_volatility: vol,
            sharpe_ratio: sharpe,
            diversification_ratio: 1.0,
        };
    }

    if n == 0 {
        return MeanVarianceResult {
            optimal_weights: HashMap::new(),
            expected_return: 0.0,
            portfolio_volatility: 0.0,
            sharpe_ratio: 0.0,
            diversification_ratio: 1.0,
        };
    }

    // Grid search over weight combinations for up to 5 assets
    let mut best_sharpe = -f64::INFINITY;
    let mut best_weights = HashMap::new();
    let mut best_ret = 0.0;
    let mut best_vol = 0.0;
    let step = if n <= 3 { 0.05 } else { 0.1 };

    // Recursive weight generation
    let mut current = vec![0.0; n];
    let mut idx = 0;

    loop {
        if idx == 0 {
            current[0] += step;
            if current[0] > 1.0 + 0.001 {
                break;
            }
        }

        // Compute sum up to idx
        let sum: f64 = current.iter().take(idx + 1).sum();
        if sum > 1.0 + 0.001 {
            // Reset this level, backtrack
            current[idx] = 0.0;
            if idx == 0 {
                break;
            }
            idx -= 1;
            continue;
        }

        if idx < n - 1 {
            idx += 1;
            continue;
        }

        // Set the last weight to balance
        let remaining = 1.0 - current.iter().take(n - 1).sum::<f64>();
        if remaining < 0.0 {
            idx = n.saturating_sub(2);
            continue;
        }
        current[n - 1] = remaining;

        // Compute portfolio return, variance, sharpe
        let mut port_ret = 0.0;
        let mut port_var = 0.0;

        for i in 0..n {
            let sym_i = symbols[i];
            let wi = current[i];
            let ri = *expected_returns.get(sym_i.as_str()).unwrap_or(&0.0);
            let vi = *volatilities.get(sym_i.as_str()).unwrap_or(&0.0);
            port_ret += wi * ri;

            for j in 0..n {
                let sym_j = symbols[j];
                let wj = current[j];
                let vj = *volatilities.get(sym_j.as_str()).unwrap_or(&0.0);
                let corr = correlations
                    .get(&(sym_i.to_string(), sym_j.to_string()))
                    .or_else(|| correlations.get(&(sym_j.to_string(), sym_i.to_string())))
                    .copied()
                    .unwrap_or(if i == j { 1.0 } else { 0.0 });
                port_var += wi * wj * vi * vj * corr;
            }
        }

        let port_vol = port_var.sqrt();
        let sharpe = if port_vol > 0.0 {
            (port_ret - risk_free_rate) / port_vol
        } else {
            0.0
        };

        if sharpe > best_sharpe {
            best_sharpe = sharpe;
            best_weights = symbols
                .iter()
                .enumerate()
                .map(|(i, s)| ((*s).clone(), current[i]))
                .collect();
            best_ret = port_ret;
            best_vol = port_vol;
        }

        // Backtrack
        current[n - 1] = 0.0;
        idx = n.saturating_sub(2);
    }

    // Diversification ratio = weighted vol / port_vol
    let weighted_vol: f64 = symbols
        .iter()
        .map(|s| {
            let w = best_weights.get(s.as_str()).copied().unwrap_or(0.0);
            let v = volatilities.get(s.as_str()).copied().unwrap_or(0.0);
            w * v
        })
        .sum();
    let div_ratio = if best_vol > 0.0 {
        weighted_vol / best_vol
    } else {
        1.0
    };

    MeanVarianceResult {
        optimal_weights: best_weights,
        expected_return: best_ret,
        portfolio_volatility: best_vol,
        sharpe_ratio: best_sharpe,
        diversification_ratio: div_ratio,
    }
}

// ── Efficient Frontier ─────────────────────────────────────────────────────

/// A single point on the efficient frontier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrontierPoint {
    pub expected_return: f64,
    pub volatility: f64,
    pub sharpe_ratio: f64,
    pub weights: HashMap<String, f64>,
}

/// Generate points along the efficient frontier by optimizing for Sharpe
/// at various expected return levels.
///
/// # Arguments
/// * `expected_returns` — Map of asset symbol to expected annual return
/// * `volatilities` — Map of asset symbol to annual volatility
/// * `correlations` — Correlation matrix
/// * `num_points` — Number of frontier points to generate (default 20)
pub fn efficient_frontier_points(
    expected_returns: &HashMap<String, f64>,
    volatilities: &HashMap<String, f64>,
    correlations: &HashMap<(String, String), f64>,
    num_points: usize,
) -> Vec<FrontierPoint> {
    let points = num_points.clamp(5, 100);
    let mut frontier = Vec::new();

    // Find min and max return from individual assets
    let min_ret = expected_returns.values().cloned().fold(f64::MAX, f64::min);
    let max_ret = expected_returns.values().cloned().fold(f64::MIN, f64::max);

    if min_ret.is_infinite() || max_ret.is_infinite() || max_ret <= min_ret {
        return frontier;
    }

    let step = (max_ret - min_ret) / points as f64;

    for i in 0..=points {
        let target_return = min_ret + step * i as f64;

        // Find portfolio with minimum variance at this return level
        let (vol, sharpe, weights) =
            min_variance_at_return(expected_returns, volatilities, correlations, target_return);

        frontier.push(FrontierPoint {
            expected_return: target_return,
            volatility: vol,
            sharpe_ratio: sharpe,
            weights,
        });
    }

    frontier
}

/// Find the minimum variance portfolio for a given target return.
fn min_variance_at_return(
    expected_returns: &HashMap<String, f64>,
    volatilities: &HashMap<String, f64>,
    correlations: &HashMap<(String, String), f64>,
    target_return: f64,
) -> (f64, f64, HashMap<String, f64>) {
    let symbols: Vec<&String> = expected_returns.keys().collect();
    let n = symbols.len();
    let mut best_var = f64::INFINITY;
    let mut best_weights = HashMap::new();
    let step = if n <= 3 { 0.05 } else { 0.1 };

    if n == 0 {
        return (0.0, 0.0, HashMap::new());
    }

    if n == 1 {
        let mut w = HashMap::new();
        w.insert(symbols[0].clone(), 1.0);
        let v = volatilities.get(symbols[0]).copied().unwrap_or(0.0);
        let r = expected_returns.get(symbols[0]).copied().unwrap_or(0.0);
        let sharpe = if v > 0.0 { r / v } else { 0.0 };
        return (v, sharpe, w);
    }

    let mut current = vec![0.0; n];
    let mut idx = 0;

    loop {
        if idx == 0 {
            current[0] += step;
            if current[0] > 1.0 + 0.001 {
                break;
            }
        }

        let sum: f64 = current.iter().take(idx + 1).sum();
        if sum > 1.0 + 0.001 {
            current[idx] = 0.0;
            if idx == 0 {
                break;
            }
            idx -= 1;
            continue;
        }

        if idx < n - 1 {
            idx += 1;
            continue;
        }

        let remaining = 1.0 - current.iter().take(n - 1).sum::<f64>();
        if remaining < 0.0 {
            idx = n.saturating_sub(2);
            continue;
        }
        current[n - 1] = remaining;

        // Check if return matches target
        let port_ret: f64 = symbols
            .iter()
            .enumerate()
            .map(|(i, s)| current[i] * expected_returns.get(s.as_str()).copied().unwrap_or(0.0))
            .sum();

        if (port_ret - target_return).abs() > step * 0.5 {
            idx = n.saturating_sub(2);
            continue;
        }

        // Compute portfolio variance
        let mut port_var = 0.0;
        for i in 0..n {
            for j in 0..n {
                let sym_i = symbols[i];
                let sym_j = symbols[j];
                let corr = correlations
                    .get(&(sym_i.to_string(), sym_j.to_string()))
                    .or_else(|| correlations.get(&(sym_j.to_string(), sym_i.to_string())))
                    .copied()
                    .unwrap_or(if i == j { 1.0 } else { 0.0 });
                port_var += current[i]
                    * current[j]
                    * volatilities.get(sym_i.as_str()).copied().unwrap_or(0.0)
                    * volatilities.get(sym_j.as_str()).copied().unwrap_or(0.0)
                    * corr;
            }
        }

        if port_var < best_var && port_var >= 0.0 {
            best_var = port_var;
            best_weights = symbols
                .iter()
                .enumerate()
                .map(|(i, s)| ((*s).clone(), current[i]))
                .collect();
        }

        current[n - 1] = 0.0;
        idx = n.saturating_sub(2);
    }

    let vol = best_var.sqrt();
    let max_ret = expected_returns.values().cloned().fold(f64::MIN, f64::max);
    let sharpe = if vol > 0.0 && max_ret > 0.0 {
        (max_ret - target_return) / vol
    } else {
        0.0
    };
    (vol, sharpe, best_weights)
}

// ── Value at Risk (VaR) ────────────────────────────────────────────────────

/// Value at Risk calculation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioVar {
    /// 95% VaR (1-day, 1-tailed)
    pub var_95: f64,
    /// 99% VaR (1-day)
    pub var_99: f64,
    /// 95% Conditional VaR (Expected Shortfall)
    pub cvar_95: f64,
    /// Maximum drawdown (peak-to-trough)
    pub max_drawdown: f64,
    /// Current drawdown from peak
    pub current_drawdown: f64,
    /// Number of observations used
    pub observations: usize,
}

/// Calculate Value at Risk from a series of daily returns.
///
/// Uses historical simulation (percentile-based).
///
/// # Arguments
/// * `daily_returns` — Array of daily portfolio return percentages (e.g., 0.01 = 1%)
/// * `current_equity` — Current portfolio equity (for dollar VaR)
pub fn calculate_var(daily_returns: &[f64], current_equity: f64) -> PortfolioVar {
    if daily_returns.is_empty() {
        return PortfolioVar {
            var_95: 0.0,
            var_99: 0.0,
            cvar_95: 0.0,
            max_drawdown: 0.0,
            current_drawdown: 0.0,
            observations: 0,
        };
    }

    let mut sorted = daily_returns.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let n = sorted.len() as f64;

    // 95% VaR: the 5th percentile return
    let idx_95 = (n * 0.05).floor() as usize;
    let idx_99 = (n * 0.01).floor() as usize;

    let var_95_pct = sorted[idx_95.min(sorted.len() - 1)];
    let var_99_pct = sorted[idx_99.min(sorted.len() - 1)];

    // Conditional VaR (Expected Shortfall): average of returns beyond VaR
    let cvar_95_pct = if idx_95 < sorted.len() {
        sorted[..=idx_95].iter().sum::<f64>() / (idx_95 + 1).max(1) as f64
    } else {
        var_95_pct
    };

    // Max drawdown
    let mut peak = f64::MIN;
    let mut max_dd = 0.0;
    let mut running_balance = current_equity;

    for &ret in daily_returns {
        running_balance *= 1.0 + ret;
        if running_balance > peak {
            peak = running_balance;
        }
        let dd = (peak - running_balance) / peak;
        if dd > max_dd {
            max_dd = dd;
        }
    }

    // Current drawdown
    let current_dd = if peak > 0.0 {
        (peak - running_balance) / peak
    } else {
        0.0
    };

    PortfolioVar {
        var_95: (var_95_pct * current_equity).abs(),
        var_99: (var_99_pct * current_equity).abs(),
        cvar_95: (cvar_95_pct * current_equity).abs(),
        max_drawdown: max_dd,
        current_drawdown: current_dd,
        observations: daily_returns.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kelly_fraction_positive_edge() {
        // 60% win rate, $100 avg win, $80 avg loss = edge
        // b = 100/80 = 1.25
        // f* = (0.6 * 1.25 - 0.4) / 1.25 = (0.75 - 0.4) / 1.25 = 0.28
        // Capped at 0.25 for safety
        let result = kelly_criterion_fraction(0.6, 100.0, 80.0, 10000.0, 100.0, false);
        assert!(
            result.fraction > 0.0,
            "Kelly fraction should be positive for positive edge, got {}",
            result.fraction
        );
        assert!(
            result.fraction <= 0.25,
            "Kelly fraction should be capped at 0.25, got {}",
            result.fraction
        );
        assert_eq!(result.half_kelly, result.fraction * 0.5);
        assert_eq!(result.quarter_kelly, result.fraction * 0.25);
    }

    #[test]
    fn test_kelly_fraction_negative_edge() {
        // 40% win rate, $100 avg win, $100 avg loss = negative edge
        // f* should be 0
        let result = kelly_criterion_fraction(0.4, 100.0, 100.0, 10000.0, 100.0, false);
        assert_eq!(result.fraction, 0.0);
    }

    #[test]
    fn test_kelly_conservative() {
        let aggressive = kelly_criterion_fraction(0.65, 150.0, 100.0, 50000.0, 200.0, false);
        let conservative = kelly_criterion_fraction(0.65, 150.0, 100.0, 50000.0, 200.0, true);
        assert_eq!(conservative.max_risk_pct, aggressive.max_risk_pct * 0.5);
    }

    #[test]
    fn test_kelly_position_sizing() {
        let result = kelly_criterion_fraction(0.55, 200.0, 100.0, 50000.0, 150.0, false);
        assert!(result.position_size_dollars > 0.0);
        assert!(result.position_size_units > 0.0);
    }

    #[test]
    fn test_optimal_kelly_portfolio_empty() {
        let result = optimal_kelly_portfolio(&[], 10000.0, false);
        assert!(result.is_empty());
    }

    #[test]
    fn test_optimal_kelly_portfolio_single() {
        let result = optimal_kelly_portfolio(&[("AAPL", 0.55, 200.0, 100.0)], 10000.0, false);
        assert!(!result.is_empty());
        assert_eq!(result[0].0, "AAPL");
    }

    #[test]
    fn test_mean_variance_single_asset() {
        let mut ers = HashMap::new();
        ers.insert("AAPL".into(), 0.12);
        let mut vols = HashMap::new();
        vols.insert("AAPL".into(), 0.20);
        let corrs = HashMap::new();

        let result = mean_variance_optimize(&ers, &vols, &corrs, 0.05);
        assert!((result.optimal_weights.get("AAPL").copied().unwrap_or(0.0) - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_mean_variance_two_assets() {
        let mut ers = HashMap::new();
        ers.insert("AAPL".into(), 0.15);
        ers.insert("MSFT".into(), 0.10);
        let mut vols = HashMap::new();
        vols.insert("AAPL".into(), 0.25);
        vols.insert("MSFT".into(), 0.18);
        let mut corrs = HashMap::new();
        corrs.insert(("AAPL".into(), "AAPL".into()), 1.0);
        corrs.insert(("AAPL".into(), "MSFT".into()), 0.5);
        corrs.insert(("MSFT".into(), "AAPL".into()), 0.5);
        corrs.insert(("MSFT".into(), "MSFT".into()), 1.0);

        let result = mean_variance_optimize(&ers, &vols, &corrs, 0.05);
        assert!(!result.optimal_weights.is_empty());
        assert!(result.sharpe_ratio > 0.0);
        assert!(result.diversification_ratio >= 1.0);
    }

    #[test]
    fn test_efficient_frontier() {
        let mut ers = HashMap::new();
        ers.insert("AAPL".into(), 0.15);
        ers.insert("MSFT".into(), 0.10);
        let mut vols = HashMap::new();
        vols.insert("AAPL".into(), 0.25);
        vols.insert("MSFT".into(), 0.18);
        let mut corrs = HashMap::new();
        corrs.insert(("AAPL".into(), "AAPL".into()), 1.0);
        corrs.insert(("AAPL".into(), "MSFT".into()), 0.5);
        corrs.insert(("MSFT".into(), "AAPL".into()), 0.5);
        corrs.insert(("MSFT".into(), "MSFT".into()), 1.0);

        let frontier = efficient_frontier_points(&ers, &vols, &corrs, 5);
        assert!(!frontier.is_empty());
        for point in &frontier {
            assert!(point.volatility >= 0.0);
        }
    }

    #[test]
    fn test_var_calculation() {
        let returns = vec![
            -0.02, 0.01, -0.015, 0.03, -0.01, 0.005, -0.025, 0.02, -0.005, 0.015, -0.03, 0.01,
            -0.02, 0.025, -0.01, 0.005, -0.04, 0.01, -0.015, 0.02,
        ];
        let var = calculate_var(&returns, 100000.0);
        assert!(
            var.var_95 > 0.0,
            "VaR 95 should be positive, got {}",
            var.var_95
        );
        assert!(var.var_99 >= var.var_95, "VaR 99 should be >= VaR 95");
        assert!(var.observations > 0);
        assert!(var.max_drawdown >= 0.0);
    }

    #[test]
    fn test_var_empty() {
        let var = calculate_var(&[], 100000.0);
        assert_eq!(var.observations, 0);
        assert_eq!(var.var_95, 0.0);
    }

    #[test]
    fn test_kelly_quarter() {
        let result = kelly_criterion_fraction(0.6, 100.0, 80.0, 10000.0, 100.0, false);
        assert!(result.fraction > 0.0);
        assert!(result.half_kelly < result.fraction);
        assert!(result.quarter_kelly < result.half_kelly);
    }
}
