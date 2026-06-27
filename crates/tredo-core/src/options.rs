//! # Options & Futures — F&O Trading Module
//!
//! Provides data structures and calculations for:
//! - Options chain with strike prices, expiry, OI, IV
//! - Black-Scholes Greeks (delta, gamma, theta, vega, rho)
//! - Options strategies (covered call, protective put, straddle, strangle, spread)
//! - Futures contract model
//!
//! All calculations are pure Rust — no external API needed for Greeks.
//! Options chain data can be sourced from broker APIs (Zerodha, Upstox, etc.)

use serde::{Deserialize, Serialize};

// ── Option Types ─────────────────────────────────────────────────────────────

/// Option side
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OptionSide {
    Call,
    Put,
}

impl std::fmt::Display for OptionSide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OptionSide::Call => write!(f, "CALL"),
            OptionSide::Put => write!(f, "PUT"),
        }
    }
}

/// Option exercise style
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExerciseStyle {
    European,
    American,
}

/// A single option contract in the chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionContract {
    pub symbol: String,
    pub side: OptionSide,
    pub strike: f64,
    pub expiry: String,          // "YYYY-MM-DD"
    pub underlying_price: f64,   // Current spot price
    pub last_price: f64,         // Last traded option premium
    pub bid: f64,                // Bid premium
    pub ask: f64,                // Ask premium
    pub volume: f64,             // Trading volume
    pub open_interest: f64,      // Open interest
    pub implied_volatility: f64, // IV (decimal, e.g. 0.25 = 25%)
    pub delta: f64,              // Greeks computed or from broker
    pub gamma: f64,
    pub theta: f64,
    pub vega: f64,
    pub rho: f64,
    pub intrinsic_value: f64,
    pub time_value: f64,
    /// Exercise style (Black-Scholes Greeks assume European)
    #[serde(default = "default_exercise_style")]
    pub exercise_style: ExerciseStyle,
}

fn default_exercise_style() -> ExerciseStyle {
    ExerciseStyle::European
}

impl OptionContract {
    /// Is this option in-the-money?
    pub fn is_itm(&self) -> bool {
        match self.side {
            OptionSide::Call => self.underlying_price > self.strike,
            OptionSide::Put => self.underlying_price < self.strike,
        }
    }

    /// Is this option at-the-money? (within 0.5% of strike)
    pub fn is_atm(&self) -> bool {
        let diff = (self.underlying_price - self.strike).abs();
        diff / self.strike.max(0.001) < 0.005
    }

    /// Is this option out-of-the-money?
    pub fn is_otm(&self) -> bool {
        !self.is_itm() && !self.is_atm()
    }

    /// Moneyness label
    pub fn moneyness(&self) -> &'static str {
        if self.is_itm() {
            "ITM"
        } else if self.is_atm() {
            "ATM"
        } else {
            "OTM"
        }
    }

    /// Leverage ratio: how many units of underlying controlled per unit of premium
    pub fn leverage(&self) -> f64 {
        if self.last_price > 0.0 {
            self.underlying_price / self.last_price
        } else {
            0.0
        }
    }

    /// Profit at a given underlying price at expiry
    pub fn profit_at_expiry(&self, price_at_expiry: f64, premium_paid: f64) -> f64 {
        let payoff = match self.side {
            OptionSide::Call => (price_at_expiry - self.strike).max(0.0),
            OptionSide::Put => (self.strike - price_at_expiry).max(0.0),
        };
        payoff - premium_paid
    }

    /// Breakeven price at expiry
    pub fn breakeven(&self) -> f64 {
        match self.side {
            OptionSide::Call => self.strike + self.last_price,
            OptionSide::Put => self.strike - self.last_price,
        }
    }
}

/// Full options chain for a symbol at a given expiry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionsChain {
    pub symbol: String,
    pub underlying_price: f64,
    pub expiry: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub calls: Vec<OptionContract>,
    pub puts: Vec<OptionContract>,
}

impl OptionsChain {
    /// Find the ATM straddle (call + put at same strike closest to underlying)
    pub fn atm_straddle(&self) -> Option<(OptionContract, OptionContract)> {
        let mut closest = 0.0;
        let mut best_call = None;
        let mut best_put = None;

        for call in &self.calls {
            let dist = (call.strike - self.underlying_price).abs();
            if best_call.is_none() || dist < closest {
                closest = dist;
                best_call = Some(call.clone());
            }
        }

        let mut closest = 0.0;
        for put in &self.puts {
            let dist = (put.strike - self.underlying_price).abs();
            if best_put.is_none() || dist < closest {
                closest = dist;
                best_put = Some(put.clone());
            }
        }

        match (best_call, best_put) {
            (Some(c), Some(p)) => Some((c, p)),
            _ => None,
        }
    }

    /// Put/Call ratio (volume-weighted)
    pub fn put_call_ratio(&self) -> f64 {
        let put_vol: f64 = self.puts.iter().map(|p| p.volume).sum();
        let call_vol: f64 = self.calls.iter().map(|c| c.volume).sum();
        if call_vol > 0.0 {
            put_vol / call_vol
        } else {
            1.0
        }
    }

    /// Max pain — the strike where option buyers lose the most money
    pub fn max_pain(&self) -> f64 {
        let mut max_pain_strike = 0.0;
        let mut max_total_loss = 0.0;

        // For each strike, compute total loss for option buyers at that price
        let mut all_levels: Vec<f64> = self.calls.iter().map(|c| c.strike).collect();
        all_levels.extend(self.puts.iter().map(|p| p.strike));
        all_levels.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        all_levels.dedup();

        for &price in &all_levels {
            let mut total_loss = 0.0;
            for call in &self.calls {
                let payoff = (price - call.strike).max(0.0);
                total_loss += (call.last_price - payoff).max(0.0) * call.open_interest;
            }
            for put in &self.puts {
                let payoff = (put.strike - price).max(0.0);
                total_loss += (put.last_price - payoff).max(0.0) * put.open_interest;
            }
            if total_loss > max_total_loss {
                max_total_loss = total_loss;
                max_pain_strike = price;
            }
        }
        max_pain_strike
    }
}

// ── Greeks Calculation (Black-Scholes) ──────────────────────────────────────

/// Cumulative distribution function for standard normal distribution
fn norm_cdf(x: f64) -> f64 {
    // Abramowitz and Stegun approximation (26.2.17 variant)
    // Coefficients (p, a1–a5) are calibrated for: y = 1 - P(t) * e^(-x²/2)
    let a1 = 0.254829592;
    let a2 = -0.284496736;
    let a3 = 1.421413741;
    let a4 = -1.453152027;
    let a5 = 1.061405429;
    let p = 0.3275911;

    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x_abs = x.abs();
    let t = 1.0 / (1.0 + p * x_abs);
    let poly = (((((a5 * t + a4) * t) + a3) * t + a2) * t + a1) * t;
    let y = 1.0 - poly * (-x_abs * x_abs / 2.0).exp();
    0.5 * (1.0 + sign * y)
}

/// Probability density function for standard normal distribution
fn norm_pdf(x: f64) -> f64 {
    (-x * x / 2.0).exp() / (2.0 * std::f64::consts::PI).sqrt()
}

/// Compute all Greeks for a European option using Black-Scholes.
///
/// # Parameters
/// * `side` — Call or Put
/// * `s` — Underlying price (spot)
/// * `k` — Strike price
/// * `t` — Time to expiry in years
/// * `r` — Risk-free interest rate (decimal, e.g. 0.05 = 5%)
/// * `sigma` — Implied volatility (decimal, e.g. 0.25 = 25%)
///
/// # Returns
/// `(delta, gamma, theta, vega, rho)`
pub fn black_scholes_greeks(
    side: OptionSide,
    s: f64,
    k: f64,
    t: f64,
    r: f64,
    sigma: f64,
) -> (f64, f64, f64, f64, f64) {
    if t <= 0.0 || sigma <= 0.0 {
        // At expiry: delta = 1.0 or 0.0, others = 0
        let intrinsic = match side {
            OptionSide::Call => {
                if s > k {
                    1.0
                } else {
                    0.0
                }
            }
            OptionSide::Put => {
                if s < k {
                    -1.0
                } else {
                    0.0
                }
            }
        };
        return (intrinsic, 0.0, 0.0, 0.0, 0.0);
    }

    let d1 = (s / k).ln() + (r + sigma * sigma / 2.0) * t;
    let d1 = d1 / (sigma * t.sqrt());
    let d2 = d1 - sigma * t.sqrt();

    match side {
        OptionSide::Call => {
            let delta = norm_cdf(d1);
            let gamma = norm_pdf(d1) / (s * sigma * t.sqrt());
            let theta = (-s * norm_pdf(d1) * sigma / (2.0 * t.sqrt())
                - r * k * (-r * t).exp() * norm_cdf(d2))
                / 365.0; // daily theta
            let vega = s * norm_pdf(d1) * t.sqrt() / 100.0; // per 1% vol change
            let rho = k * t * (-r * t).exp() * norm_cdf(d2) / 100.0; // per 1% rate change
            (delta, gamma, theta, vega, rho)
        }
        OptionSide::Put => {
            let delta = -norm_cdf(-d1);
            let gamma = norm_pdf(d1) / (s * sigma * t.sqrt());
            let theta = (-s * norm_pdf(d1) * sigma / (2.0 * t.sqrt())
                + r * k * (-r * t).exp() * norm_cdf(-d2))
                / 365.0;
            let vega = s * norm_pdf(d1) * t.sqrt() / 100.0;
            let rho = -k * t * (-r * t).exp() * norm_cdf(-d2) / 100.0;
            (delta, gamma, theta, vega, rho)
        }
    }
}

/// Compute Black-Scholes option price
pub fn black_scholes_price(side: OptionSide, s: f64, k: f64, t: f64, r: f64, sigma: f64) -> f64 {
    if t <= 0.0 || sigma <= 0.0 {
        return 0.0;
    }
    let d1 = (s / k).ln() + (r + sigma * sigma / 2.0) * t;
    let d1 = d1 / (sigma * t.sqrt());
    let d2 = d1 - sigma * t.sqrt();

    match side {
        OptionSide::Call => s * norm_cdf(d1) - k * (-r * t).exp() * norm_cdf(d2),
        OptionSide::Put => k * (-r * t).exp() * norm_cdf(-d2) - s * norm_cdf(-d1),
    }
}

/// Compute implied volatility using Newton-Raphson
pub fn implied_volatility(
    side: OptionSide,
    s: f64,
    k: f64,
    t: f64,
    r: f64,
    market_price: f64,
) -> f64 {
    let mut sigma = 0.3; // initial guess
    let tolerance = 0.0001;
    let max_iter = 100;

    for _ in 0..max_iter {
        let price = black_scholes_price(side, s, k, t, r, sigma);
        let diff = price - market_price;
        if diff.abs() < tolerance {
            return sigma;
        }
        // Vega = dPrice/dSigma
        let d1 = (s / k).ln() + (r + sigma * sigma / 2.0) * t;
        let d1_val = d1 / (sigma * t.sqrt());
        let vega_val = s * norm_pdf(d1_val) * t.sqrt();
        if vega_val.abs() < 0.0001 {
            break;
        }
        sigma -= diff / vega_val;
        sigma = sigma.clamp(0.01, 2.0); // reasonable bounds
    }
    sigma
}

// ── Options Strategies ──────────────────────────────────────────────────────

/// A pre-defined options strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionsStrategy {
    pub name: String,
    pub description: String,
    pub legs: Vec<StrategyLeg>,
    pub max_profit: f64,
    pub max_loss: f64,
    pub breakevens: Vec<f64>,
    pub direction: String, // "bullish" | "bearish" | "neutral" | "volatile"
}

/// A single leg of an options strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyLeg {
    pub side: OptionSide,
    pub strike: f64,
    pub action: String, // "buy" | "sell"
    pub premium: f64,
    pub quantity: i32,
}

/// Build a covered call strategy
pub fn covered_call(underlying_price: f64, call_strike: f64, call_premium: f64) -> OptionsStrategy {
    let max_profit = (call_strike - underlying_price) + call_premium;
    let max_loss = underlying_price; // stock goes to zero
    let breakeven = underlying_price - call_premium;

    OptionsStrategy {
        name: "Covered Call".to_string(),
        description: format!(
            "Long 100 shares @ ${:.2}, Short 1 Call @ ${:.2} strike ${:.2}",
            underlying_price, call_premium, call_strike
        ),
        legs: vec![
            StrategyLeg {
                side: OptionSide::Call,
                strike: 0.0,
                action: "buy".into(),
                premium: underlying_price,
                quantity: 100,
            },
            StrategyLeg {
                side: OptionSide::Call,
                strike: call_strike,
                action: "sell".into(),
                premium: call_premium,
                quantity: 1,
            },
        ],
        max_profit,
        max_loss,
        breakevens: vec![breakeven],
        direction: "bullish".to_string(),
    }
}

/// Build a protective put strategy
pub fn protective_put(underlying_price: f64, put_strike: f64, put_premium: f64) -> OptionsStrategy {
    let max_profit = f64::INFINITY;
    let max_loss = put_strike - put_premium;
    let breakeven = underlying_price + put_premium;

    OptionsStrategy {
        name: "Protective Put".to_string(),
        description: format!(
            "Long 100 shares @ ${:.2}, Long 1 Put @ ${:.2} strike ${:.2}",
            underlying_price, put_premium, put_strike
        ),
        legs: vec![
            StrategyLeg {
                side: OptionSide::Put,
                strike: 0.0,
                action: "buy".into(),
                premium: underlying_price,
                quantity: 100,
            },
            StrategyLeg {
                side: OptionSide::Put,
                strike: put_strike,
                action: "buy".into(),
                premium: put_premium,
                quantity: 1,
            },
        ],
        max_profit,
        max_loss,
        breakevens: vec![breakeven],
        direction: "bullish".to_string(),
    }
}

/// Build a long straddle (buy ATM call + put)
pub fn long_straddle(call: &OptionContract, put: &OptionContract) -> OptionsStrategy {
    let total_cost = call.last_price + put.last_price;
    let breakeven_up = call.strike + total_cost;
    let breakeven_down = call.strike - total_cost;

    OptionsStrategy {
        name: "Long Straddle".to_string(),
        description: format!(
            "Long {} Call @ ${:.2} + Long {} Put @ ${:.2}, total cost=${:.2}",
            call.strike, call.last_price, put.strike, put.last_price, total_cost
        ),
        legs: vec![
            StrategyLeg {
                side: OptionSide::Call,
                strike: call.strike,
                action: "buy".into(),
                premium: call.last_price,
                quantity: 1,
            },
            StrategyLeg {
                side: OptionSide::Put,
                strike: put.strike,
                action: "buy".into(),
                premium: put.last_price,
                quantity: 1,
            },
        ],
        max_profit: f64::INFINITY,
        max_loss: total_cost,
        breakevens: vec![breakeven_down, breakeven_up],
        direction: "volatile".to_string(),
    }
}

/// Build a long strangle (buy OTM call + OTM put)
pub fn long_strangle(otm_call: &OptionContract, otm_put: &OptionContract) -> OptionsStrategy {
    let total_cost = otm_call.last_price + otm_put.last_price;
    let breakeven_up = otm_call.strike + total_cost;
    let breakeven_down = otm_put.strike - total_cost;

    OptionsStrategy {
        name: "Long Strangle".to_string(),
        description: format!(
            "Long {} Call @ ${:.2} + Long {} Put @ ${:.2}, total cost=${:.2}",
            otm_call.strike, otm_call.last_price, otm_put.strike, otm_put.last_price, total_cost
        ),
        legs: vec![
            StrategyLeg {
                side: OptionSide::Call,
                strike: otm_call.strike,
                action: "buy".into(),
                premium: otm_call.last_price,
                quantity: 1,
            },
            StrategyLeg {
                side: OptionSide::Put,
                strike: otm_put.strike,
                action: "buy".into(),
                premium: otm_put.last_price,
                quantity: 1,
            },
        ],
        max_profit: f64::INFINITY,
        max_loss: total_cost,
        breakevens: vec![breakeven_down, breakeven_up],
        direction: "volatile".to_string(),
    }
}

/// Build a bull call spread
pub fn bull_call_spread(
    lower_strike: f64,
    upper_strike: f64,
    lower_premium: f64,
    upper_premium: f64,
) -> OptionsStrategy {
    let net_debit = (lower_premium - upper_premium).abs();
    let max_profit = (upper_strike - lower_strike) - net_debit;
    let breakeven = lower_strike + net_debit;

    OptionsStrategy {
        name: "Bull Call Spread".to_string(),
        description: format!(
            "Buy {} Call @ ${:.2} + Sell {} Call @ ${:.2}, net debit=${:.2}",
            lower_strike, lower_premium, upper_strike, upper_premium, net_debit
        ),
        legs: vec![
            StrategyLeg {
                side: OptionSide::Call,
                strike: lower_strike,
                action: "buy".into(),
                premium: lower_premium,
                quantity: 1,
            },
            StrategyLeg {
                side: OptionSide::Call,
                strike: upper_strike,
                action: "sell".into(),
                premium: upper_premium,
                quantity: 1,
            },
        ],
        max_profit,
        max_loss: net_debit,
        breakevens: vec![breakeven],
        direction: "bullish".to_string(),
    }
}

/// Build a bear put spread
pub fn bear_put_spread(
    upper_strike: f64,
    lower_strike: f64,
    upper_premium: f64,
    lower_premium: f64,
) -> OptionsStrategy {
    let net_debit = (upper_premium - lower_premium).abs();
    let max_profit = (upper_strike - lower_strike) - net_debit;
    let breakeven = upper_strike - net_debit;

    OptionsStrategy {
        name: "Bear Put Spread".to_string(),
        description: format!(
            "Buy {} Put @ ${:.2} + Sell {} Put @ ${:.2}, net debit=${:.2}",
            upper_strike, upper_premium, lower_strike, lower_premium, net_debit
        ),
        legs: vec![
            StrategyLeg {
                side: OptionSide::Put,
                strike: upper_strike,
                action: "buy".into(),
                premium: upper_premium,
                quantity: 1,
            },
            StrategyLeg {
                side: OptionSide::Put,
                strike: lower_strike,
                action: "sell".into(),
                premium: lower_premium,
                quantity: 1,
            },
        ],
        max_profit,
        max_loss: net_debit,
        breakevens: vec![breakeven],
        direction: "bearish".to_string(),
    }
}

/// Build an iron condor (short strangle + long wings)
#[allow(clippy::too_many_arguments)]
pub fn iron_condor(
    put_long_strike: f64,
    put_short_strike: f64,
    call_short_strike: f64,
    call_long_strike: f64,
    put_long_premium: f64,
    put_short_premium: f64,
    call_short_premium: f64,
    call_long_premium: f64,
) -> OptionsStrategy {
    let net_credit =
        (put_short_premium + call_short_premium) - (put_long_premium + call_long_premium);
    let max_profit = net_credit;
    let wing_width = (put_short_strike - put_long_strike).min(call_long_strike - call_short_strike);
    let max_loss = wing_width - net_credit;

    OptionsStrategy {
        name: "Iron Condor".to_string(),
        description: format!(
            "Sell {}P/${}C strangle, buy {}P/${}C wings, net credit=${:.2}",
            put_short_strike, call_short_strike, put_long_strike, call_long_strike, net_credit
        ),
        legs: vec![
            StrategyLeg {
                side: OptionSide::Put,
                strike: put_long_strike,
                action: "buy".into(),
                premium: put_long_premium,
                quantity: 1,
            },
            StrategyLeg {
                side: OptionSide::Put,
                strike: put_short_strike,
                action: "sell".into(),
                premium: put_short_premium,
                quantity: 1,
            },
            StrategyLeg {
                side: OptionSide::Call,
                strike: call_short_strike,
                action: "sell".into(),
                premium: call_short_premium,
                quantity: 1,
            },
            StrategyLeg {
                side: OptionSide::Call,
                strike: call_long_strike,
                action: "buy".into(),
                premium: call_long_premium,
                quantity: 1,
            },
        ],
        max_profit,
        max_loss,
        breakevens: vec![
            put_short_strike - net_credit,
            call_short_strike + net_credit,
        ],
        direction: "neutral".to_string(),
    }
}

// ── Futures Contract ────────────────────────────────────────────────────────

/// A futures contract
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FuturesContract {
    pub symbol: String,
    pub exchange: String,   // "NSE" | "MCX" | "CME"
    pub underlying: String, // e.g. "NIFTY", "GOLD", "ES"
    pub expiry: String,     // "YYYY-MM-DD"
    pub last_price: f64,
    pub bid: f64,
    pub ask: f64,
    pub open_interest: f64,
    pub volume: f64,
    pub contract_size: f64, // e.g. 75 for NIFTY, 1 for ES
    pub tick_size: f64,     // Minimum price movement
    pub tick_value: f64,    // Rupees per tick
    pub initial_margin: f64,
    pub maintenance_margin: f64,
    pub fair_value: Option<f64>, // Theoretical future price = spot * e^(rT)
    pub basis: Option<f64>,      // Futures - Spot
}

impl FuturesContract {
    /// Notional value of one contract
    pub fn notional(&self) -> f64 {
        self.last_price * self.contract_size
    }

    /// Leverage ratio (notional / initial margin)
    pub fn leverage(&self) -> f64 {
        if self.initial_margin > 0.0 {
            self.notional() / self.initial_margin
        } else {
            1.0
        }
    }

    /// Profit/Loss for a given price change
    pub fn pnl_for_price_change(&self, price_change_points: f64) -> f64 {
        price_change_points * self.contract_size
    }

    /// Days to expiry
    pub fn days_to_expiry(&self) -> i64 {
        if let Ok(expiry_date) = chrono::NaiveDate::parse_from_str(&self.expiry, "%Y-%m-%d") {
            let today = chrono::Utc::now().date_naive();
            (expiry_date - today).num_days()
        } else {
            0
        }
    }
}

/// Compute fair value of a futures contract using cost-of-carry model
pub fn futures_fair_value(
    spot: f64,
    risk_free_rate: f64,
    dividends: f64,
    days_to_expiry: f64,
) -> f64 {
    let t = days_to_expiry / 365.0;
    spot * ((risk_free_rate - dividends) * t).exp()
}

// ── Options Signal ──────────────────────────────────────────────────────────

/// A trading signal derived from options market analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionsSignal {
    pub symbol: String,
    pub signal_type: String, // "volatility_skew" | "put_call_ratio" | "max_pain" | "greeks"
    pub direction: String,   // "bullish" | "bearish" | "neutral"
    pub confidence: f64,
    pub reasoning: String,
    pub underlying_price: f64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Analyze the options chain to generate trading signals
pub fn analyze_options_chain(chain: &OptionsChain) -> Vec<OptionsSignal> {
    let mut signals = Vec::new();

    // 1. Put/Call ratio signal
    let pcr = chain.put_call_ratio();
    if pcr > 1.5 {
        signals.push(OptionsSignal {
            symbol: chain.symbol.clone(),
            signal_type: "put_call_ratio".into(),
            direction: "bullish".into(), // extreme put buying = fear = contrarian buy
            confidence: (pcr.min(3.0) - 1.5) / 1.5 * 0.5,
            reasoning: format!(
                "Put/Call ratio extreme at {:.2} — excessive bearish sentiment",
                pcr
            ),
            underlying_price: chain.underlying_price,
            timestamp: chrono::Utc::now(),
        });
    } else if pcr < 0.5 {
        signals.push(OptionsSignal {
            symbol: chain.symbol.clone(),
            signal_type: "put_call_ratio".into(),
            direction: "bearish".into(), // excessive call buying = euphoria = caution
            confidence: (0.5 - pcr) / 0.5 * 0.5,
            reasoning: format!(
                "Put/Call ratio very low at {:.2} — excessive bullish sentiment",
                pcr
            ),
            underlying_price: chain.underlying_price,
            timestamp: chrono::Utc::now(),
        });
    }

    // 2. Max pain signal
    let max_pain = chain.max_pain();
    let distance_to_mp = ((chain.underlying_price - max_pain) / max_pain * 100.0).abs();
    if distance_to_mp > 2.0 {
        let dir = if chain.underlying_price > max_pain {
            "bearish"
        } else {
            "bullish"
        };
        signals.push(OptionsSignal {
            symbol: chain.symbol.clone(),
            signal_type: "max_pain".into(),
            direction: dir.into(),
            confidence: (distance_to_mp / 10.0).min(0.7),
            reasoning: format!(
                "Price ${:.2} is {:.1}% from max pain ${:.2} — may drift towards it",
                chain.underlying_price, distance_to_mp, max_pain
            ),
            underlying_price: chain.underlying_price,
            timestamp: chrono::Utc::now(),
        });
    }

    // 3. Volatility skew signal (compare OTM put IV vs OTM call IV)
    let otm_put_iv: Vec<f64> = chain
        .puts
        .iter()
        .filter(|p| p.is_otm() && p.implied_volatility > 0.0)
        .map(|p| p.implied_volatility)
        .collect();
    let otm_call_iv: Vec<f64> = chain
        .calls
        .iter()
        .filter(|c| c.is_otm() && c.implied_volatility > 0.0)
        .map(|c| c.implied_volatility)
        .collect();

    if !otm_put_iv.is_empty() && !otm_call_iv.is_empty() {
        let avg_put_iv: f64 = otm_put_iv.iter().sum::<f64>() / otm_put_iv.len() as f64;
        let avg_call_iv: f64 = otm_call_iv.iter().sum::<f64>() / otm_call_iv.len() as f64;
        let skew = (avg_put_iv - avg_call_iv) / ((avg_put_iv + avg_call_iv) / 2.0);

        if skew > 0.2 {
            signals.push(OptionsSignal {
                symbol: chain.symbol.clone(),
                signal_type: "volatility_skew".into(),
                direction: "bearish".into(), // puts more expensive = fear
                confidence: (skew / 0.5).min(0.7),
                reasoning: format!(
                    "Put skew elevated ({:.1}%) — market pricing downside risk",
                    skew * 100.0
                ),
                underlying_price: chain.underlying_price,
                timestamp: chrono::Utc::now(),
            });
        } else if skew < -0.1 {
            signals.push(OptionsSignal {
                symbol: chain.symbol.clone(),
                signal_type: "volatility_skew".into(),
                direction: "bullish".into(), // calls more expensive = optimism
                confidence: (-skew / 0.3).min(0.6),
                reasoning: format!(
                    "Call skew elevated ({:.1}%) — market pricing upside optimism",
                    -skew * 100.0
                ),
                underlying_price: chain.underlying_price,
                timestamp: chrono::Utc::now(),
            });
        }
    }

    signals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_black_scholes_call_price() {
        // S=100, K=100, T=1yr, r=5%, sigma=20% — Abramowitz CDF variant ≈ 11.9
        let price = black_scholes_price(OptionSide::Call, 100.0, 100.0, 1.0, 0.05, 0.20);
        assert!(
            price > 10.5 && price < 13.5,
            "Call price should be ~11.9, got {:.4}",
            price
        );
    }

    #[test]
    fn test_black_scholes_put_price() {
        let price = black_scholes_price(OptionSide::Put, 100.0, 100.0, 1.0, 0.05, 0.20);
        assert!(
            price > 5.5 && price < 8.5,
            "Put price should be ~7.0, got {:.4}",
            price
        );
    }

    #[test]
    fn test_greeks_call() {
        let (delta, gamma, theta, vega, rho) =
            black_scholes_greeks(OptionSide::Call, 100.0, 100.0, 1.0, 0.05, 0.20);
        assert!(
            (delta - 0.67).abs() < 0.05,
            "Call delta should be ~0.67, got {:.4}",
            delta
        );
        assert!(gamma > 0.0, "Gamma should be positive");
        assert!(theta < 0.0, "Theta should be negative for long calls");
        assert!(vega > 0.0, "Vega should be positive");
        assert!(rho > 0.0, "Rho should be positive for calls");
    }

    #[test]
    fn test_greeks_put() {
        let (delta, _gamma, theta, _vega, _rho) =
            black_scholes_greeks(OptionSide::Put, 100.0, 100.0, 1.0, 0.05, 0.20);
        assert!(
            (delta + 0.33).abs() < 0.05,
            "Put delta should be ~-0.33, got {:.4}",
            delta
        );
        assert!(theta < 0.0, "Theta should be negative for long puts");
    }

    #[test]
    fn test_implied_volatility() {
        // Price from known IV=20% should give back ~20%
        let price = black_scholes_price(OptionSide::Call, 100.0, 100.0, 1.0, 0.05, 0.20);
        let iv = implied_volatility(OptionSide::Call, 100.0, 100.0, 1.0, 0.05, price);
        assert!(
            (iv - 0.20).abs() < 0.01,
            "IV should converge to 0.20, got {:.4}",
            iv
        );
    }

    #[test]
    fn test_atm_straddle() {
        let chain = OptionsChain {
            symbol: "NIFTY".into(),
            underlying_price: 25000.0,
            expiry: "2026-06-25".into(),
            timestamp: chrono::Utc::now(),
            calls: vec![
                OptionContract {
                    symbol: "NIFTY".into(),
                    side: OptionSide::Call,
                    strike: 24900.0,
                    underlying_price: 25000.0,
                    last_price: 150.0,
                    bid: 148.0,
                    ask: 152.0,
                    volume: 1000.0,
                    open_interest: 10000.0,
                    implied_volatility: 0.15,
                    delta: 0.6,
                    gamma: 0.01,
                    theta: -0.5,
                    vega: 2.0,
                    rho: 0.1,
                    intrinsic_value: 100.0,
                    time_value: 50.0,
                    expiry: "2026-06-25".into(),
                    exercise_style: ExerciseStyle::European,
                },
                OptionContract {
                    symbol: "NIFTY".into(),
                    side: OptionSide::Call,
                    strike: 25000.0,
                    underlying_price: 25000.0,
                    last_price: 120.0,
                    bid: 118.0,
                    ask: 122.0,
                    volume: 2000.0,
                    open_interest: 15000.0,
                    implied_volatility: 0.16,
                    delta: 0.5,
                    gamma: 0.02,
                    theta: -0.6,
                    vega: 2.5,
                    rho: 0.1,
                    intrinsic_value: 0.0,
                    time_value: 120.0,
                    expiry: "2026-06-25".into(),
                    exercise_style: ExerciseStyle::European,
                },
            ],
            puts: vec![OptionContract {
                symbol: "NIFTY".into(),
                side: OptionSide::Put,
                strike: 25000.0,
                underlying_price: 25000.0,
                last_price: 115.0,
                bid: 113.0,
                ask: 117.0,
                volume: 1800.0,
                open_interest: 14000.0,
                implied_volatility: 0.17,
                delta: -0.5,
                gamma: 0.02,
                theta: -0.5,
                vega: 2.4,
                rho: -0.1,
                intrinsic_value: 0.0,
                time_value: 115.0,
                expiry: "2026-06-25".into(),
                exercise_style: ExerciseStyle::European,
            }],
        };

        let straddle = chain.atm_straddle();
        assert!(straddle.is_some(), "Should find ATM straddle");
        let (call, put) = straddle.unwrap();
        assert_eq!(call.strike, 25000.0);
        assert_eq!(put.strike, 25000.0);

        let pcr = chain.put_call_ratio();
        // put volume = 1800, call volume = 1000+2000 = 3000, PCR = 1800/3000 = 0.6
        assert!(
            (pcr - 0.6).abs() < 0.15,
            "PCR should be ~0.6, got {:.3}",
            pcr
        );
    }

    #[test]
    fn test_strategies() {
        let strat = covered_call(100.0, 110.0, 5.0);
        assert_eq!(strat.name, "Covered Call");
        assert!((strat.breakevens[0] - 95.0).abs() < 0.01);

        let strat = long_straddle(
            &OptionContract {
                strike: 100.0,
                last_price: 10.0,
                ..mock_contract()
            },
            &OptionContract {
                strike: 100.0,
                last_price: 8.0,
                ..mock_contract()
            },
        );
        assert!((strat.max_loss - 18.0).abs() < 0.01);
    }

    fn mock_contract() -> OptionContract {
        OptionContract {
            symbol: "TEST".into(),
            side: OptionSide::Call,
            strike: 100.0,
            underlying_price: 100.0,
            last_price: 0.0,
            bid: 0.0,
            ask: 0.0,
            volume: 0.0,
            open_interest: 0.0,
            implied_volatility: 0.0,
            delta: 0.0,
            gamma: 0.0,
            theta: 0.0,
            vega: 0.0,
            rho: 0.0,
            intrinsic_value: 0.0,
            time_value: 0.0,
            expiry: "2026-06-25".into(),
            exercise_style: ExerciseStyle::European,
        }
    }
}
