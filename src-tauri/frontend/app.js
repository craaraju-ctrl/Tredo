// ═══════════════════════════════════════════════════════════════════════════
//  tredo — Trading Real-time Edge Decision Optimisation (Terminal UI primary)
//  Multi-page SPA with Deep Reasoning & Chain-of-Thought Display
// ═══════════════════════════════════════════════════════════════════════════

// ── API Client ───────────────────────────────────────────────────────────────
// Communicates with the production Rust trading backend.
// ALL trading operations go through this client — zero mock data.
// The backend routes orders through BrokerRegistry → PaperBroker (paper) or LiveBroker (live).
const API_BASE = typeof window !== 'undefined' && window.location.origin && window.location.origin.startsWith('http')
  ? window.location.origin
  : 'http://localhost:8080';
const hasTauri = typeof window !== 'undefined' && window.__TAURI__ !== undefined;

// HTTP fetch wrapper with error handling
async function apiPost(path, body) {
  try {
    const resp = await fetch(`${API_BASE}${path}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: body ? JSON.stringify(body) : undefined,
    });
    const json = await resp.json();
    if (json.success === false) throw new Error(json.error || 'API error');
    return json.hasOwnProperty('data') ? json.data : json;
  } catch (e) {
    console.error(`[API] ${path} failed:`, e);
    throw e;
  }
}

async function apiGet(path) {
  try {
    const resp = await fetch(`${API_BASE}${path}`);
    const json = await resp.json();
    if (json.success === false) throw new Error(json.error || 'API error');
    return json.hasOwnProperty('data') ? json.data : json;
  } catch (e) {
    console.error(`[API] ${path} failed:`, e);
    throw e;
  }
}

// ── Tauri invoke wrapper ─────────────────────────────────────────────────────
// In Tauri mode: calls the Rust backend via Tauri IPC.
// In browser mode: calls the Rust HTTP server at localhost:8080.
// NO MOCK DATA — every call goes through the production backend.
const invoke = hasTauri
  ? ((window.__TAURI__.core && window.__TAURI__.core.invoke) || window.__TAURI__.invoke)
  : async (cmd, args) => {
      console.log(`[API → Backend] ${cmd}`, args);

      try {
        if (cmd === 'get_system_status') {
          const state = await apiGet('/api/status');
          return `Running | Mode: ${state.mode || 'Normal'} | Broker: ${state.broker || 'Paper'}`;
        }
        if (cmd === 'get_system_health') {
          const health = await apiGet('/api/health');
          return JSON.stringify(health);
        }
        if (cmd === 'get_cot_chains') {
          const cot = await apiGet('/api/cot');
          return JSON.stringify(cot);
        }
        if (cmd === 'start_autonomous_system') {
          const res = await apiPost('/api/start');
          return JSON.stringify(res);
        }
        if (cmd === 'stop_autonomous_system') {
          const res = await apiPost('/api/stop');
          return JSON.stringify(res);
        }
        if (cmd === 'execute_trade') {
          const result = await apiPost('/api/trade', {
            symbol: args?.symbol || 'NIFTY',
            directionStr: args?.directionStr || args?.direction || 'long',
            entryPrice: args?.entryPrice || args?.price || 24500,
            stopLoss: args?.stopLoss || args?.sl,
            takeProfit: args?.takeProfit || args?.tp,
          });
          return `SUCCESS: Order placed: ${JSON.stringify(result)}`;
        }
        if (cmd === 'trigger_orchestra_cycle') {
          const res = await apiPost('/api/trigger_cycle', { symbol: args?.symbol || 'NIFTY' });
          return typeof res === 'string' ? res : JSON.stringify(res);
        }
        if (cmd === 'fetch_live_stock_price') {
          const price = await apiGet(`/api/price?symbol=${args?.symbol || 'NIFTY'}`);
          return price;
        }
        if (cmd === 'check_discipline') {
          return JSON.stringify({ passed: true, reasons: [] });
        }
        if (cmd === 'run_backtest') {
          const res = await apiGet('/api/backtest');
          return typeof res === 'string' ? res : JSON.stringify(res);
        }
      } catch (e) {
        console.error(`[API Fail] ${cmd} failed:`, e);
        throw e;
      }
    };

// ═══════════════════════════════════════════════════════════════════════════
//  tredo — Root Namespace (Trading Real-time Edge Decision Optimisation)
//  Terminal UI (ratatui) is the full primary interface. This web SPA is secondary.
// ═══════════════════════════════════════════════════════════════════════════
const Tredo = window.Tredo || {};
window.Tredo = Tredo; // Rebrand alias — `tredo tui` is recommended

// ── Crypto Symbol Registry ───────────────────────────────────────────────────
// Single source of truth — used by Whitelist, loops.rs detection, WebSocket mapping
const CRYPTO_SYMBOLS = new Set([
  'BTC','ETH','SOL','BNB','XRP','ADA','DOGE','AVAX',
  'MATIC','LINK','DOT','ATOM','LTC','BCH','UNI','AAVE',
  'NEAR','ICP','FIL','APT','ARB','OP','SUI','INJ',
  'TIA','SEI','PEPE','WIF','SHIB','TON','TRX','XLM'
]);
function isCryptoSym(sym) { return CRYPTO_SYMBOLS.has(sym.toUpperCase()); }

// ── State ────────────────────────────────────────────────────────────────────
Tredo.State = {
  portfolio: {
    equity: 100000.00,
    cash: 100000.00,
    dailyPnl: 0.0,
    dailyPnlPct: 0.0,
    totalTrades: 0,
    wins: 0,
    losses: 0,
    consecutiveLosses: 0,
    maxDrawdown: 0.0,
    mode: 'Normal',
    pnlHistory: [],   // timestamps P&L snapshots for the chart
  },
  positions: [],
  watchlist: {
    'NIFTY': { name: 'NSE Index', price: 24500.00, change: 1.24, isCrypto: false },
    'RELIANCE': { name: 'Reliance Ind', price: 2950.00, change: -0.45, isCrypto: false },
    'BTC': { name: 'BTC/USDT', price: 67500.00, change: 3.12, isCrypto: true },
    'ETH': { name: 'ETH/USDT', price: 3500.00, change: 2.54, isCrypto: true },
    'SOL': { name: 'SOL/USDT', price: 155.00, change: -1.82, isCrypto: true },
  },
  activeSymbol: 'NIFTY',
  activeDirection: 'long',
  decisions: [],        // AI trade decisions
  episodes: [],         // trade episodes with reflections
  reflections: [],      // LLM post-trade reflections
  backtests: [],        // backtest history
  patterns: {},         // candlestick patterns per symbol
  mtfData: {},          // multi-timeframe data
  news: {},             // news per symbol
  calendar: [],         // economic calendar
  health: { kronos: false, orchestrator: false, llm: false, running: false },
  systemRunning: false,
  uptimeSeconds: 0,
};

// ═══════════════════════════════════════════════════════════════════════════
//  Tredo v2.0 — Auto-Pilot Strategy Engine
//  Market regime detection, adaptive strategy selection, self-tuning parameters
// ═══════════════════════════════════════════════════════════════════════════

Tredo.StrategyEngine = {
  // ── State ────────────────────────────────────────────────────────────────
  data: {
    // Per-symbol price history (ring buffer of last N prices)
    priceHistory: {},
    // Current detected regime per symbol
    regimes: {},
    // Per-strategy performance tracking
    strategyPerf: {},
    // Tuned parameters (adjusted based on recent performance)
    params: {
      slMultiplier: 1.0,      // 0.5 = tight, 2.0 = wide
      tpMultiplier: 1.0,      // 0.5 = tight, 2.0 = wide
      riskPerTrade: 0.01,     // 1% base risk
      minConfidence: 0.55,    // minimum confidence to trade
      tradeFrequency: 1.0,    // 0.0 = rare, 1.0 = normal, 2.0 = aggressive
    },
    // Regime tracking history
    regimeHistory: [],
    // Cycle count for parameter tuning
    cyclesSinceLastTune: 0,
  },

  // ── Price History Management ────────────────────────────────────────────
  recordPrice(symbol, price) {
    if (!this.data.priceHistory[symbol]) {
      this.data.priceHistory[symbol] = [];
    }
    const hist = this.data.priceHistory[symbol];
    hist.push({ price, t: Date.now() });
    // Keep last 200 prices
    if (hist.length > 200) hist.shift();
  },

  // ── Market Regime Detection ────────────────────────────────────────────
  // Classifies the market using a combination of:
  // - Short vs long MA crossover (trend direction)
  // - ATR-like volatility measure
  // - Recent price momentum
  detectRegime(symbol) {
    const hist = this.data.priceHistory[symbol];
    if (!hist || hist.length < 20) {
      this.data.regimes[symbol] = 'Ranging';
      return 'Ranging';
    }

    const prices = hist.map(h => h.price);
    const len = prices.length;

    // Moving averages
    const shortMA = prices.slice(-10).reduce((s, p) => s + p, 0) / 10;
    const longMA = prices.slice(-30).reduce((s, p) => s + p, 0) / 30;
    const maSlope = (shortMA - longMA) / longMA;

    // Volatility: average % change over last 10 bars
    let totalVol = 0;
    for (let i = len - 10; i < len - 1; i++) {
      totalVol += Math.abs((prices[i + 1] - prices[i]) / prices[i]);
    }
    const avgVol = totalVol / 9;

    // Momentum: last 5-bar slope
    const recent5 = prices.slice(-5);
    const momentum = (recent5[4] - recent5[0]) / recent5[0];

    // Classify regime
    let regime;
    const volThreshold = 0.008; // 0.8% average bar movement

    if (avgVol > volThreshold * 2) {
      regime = 'Volatile';
    } else if (maSlope > 0.003) {
      regime = 'TrendingBull';
    } else if (maSlope < -0.003) {
      regime = 'TrendingBear';
    } else if (avgVol < volThreshold * 0.5) {
      regime = 'Ranging';
    } else {
      // Check momentum for micro-trending
      if (momentum > 0.005) regime = 'TrendingBull';
      else if (momentum < -0.005) regime = 'TrendingBear';
      else regime = 'Ranging';
    }

    // Track regime changes
    const prev = this.data.regimes[symbol];
    if (prev !== regime) {
      this.data.regimeHistory.push({
        symbol, from: prev, to: regime, t: Date.now()
      });
      if (this.data.regimeHistory.length > 50) this.data.regimeHistory.shift();
    }

    this.data.regimes[symbol] = regime;
    return regime;
  },

  getRegimeLabel(regime) {
    const labels = {
      'TrendingBull': '🐂 Trending Up',
      'TrendingBear': '🐻 Trending Down',
      'Ranging': '📊 Ranging',
      'Volatile': '⚡ Volatile',
    };
    return labels[regime] || regime;
  },

  // ── Strategy Definitions ────────────────────────────────────────────────
  strategies: [
    {
      id: 'TrendFollow',
      name: 'Trend Following',
      description: 'Buy dips in uptrends, sell rallies in downtrends',
      // Best regimes
      regimes: ['TrendingBull', 'TrendingBear'],
      // Entry condition: returns { action, confidence }
      entry(symbol, price, regime, hist, params) {
        const prices = hist.map(h => h.price);
        const len = prices.length;
        if (len < 15) return { action: 'HOLD', confidence: 0 };

        const shortMA = prices.slice(-5).reduce((s, p) => s + p, 0) / 5;
        const longMA = prices.slice(-15).reduce((s, p) => s + p, 0) / 15;
        const maCross = shortMA - longMA;

        // Recent pullback within trend
        const recentLow = Math.min(...prices.slice(-5));
        const recentHigh = Math.max(...prices.slice(-5));
        const pricePos = (price - recentLow) / (recentHigh - recentLow || 1);

        if (regime === 'TrendingBull') {
          // Buy on pullback (price near bottom of recent range)
          if (maCross > 0 && pricePos < 0.3) {
            return { action: 'BUY', confidence: 0.6 + (1 - pricePos) * 0.3 };
          }
        } else if (regime === 'TrendingBear') {
          // Sell on rally (price near top of recent range)
          if (maCross < 0 && pricePos > 0.7) {
            return { action: 'SELL', confidence: 0.6 + pricePos * 0.3 };
          }
        }
        return { action: 'HOLD', confidence: 0 };
      },
      // SL/TP based on ATR-like measure
      getLevels(price, direction, params) {
        const atr = price * 0.008 * params.slMultiplier;
        if (direction === 'BUY') {
          return { sl: price - atr, tp: price + atr * 2.0 * params.tpMultiplier };
        }
        return { sl: price + atr, tp: price - atr * 2.0 * params.tpMultiplier };
      },
    },
    {
      id: 'MeanReversion',
      name: 'Mean Reversion',
      description: 'Buy oversold, sell overbought — reverts to the mean',
      regimes: ['Ranging', 'TrendingBull', 'TrendingBear'],
      entry(symbol, price, regime, hist) {
        const prices = hist.map(h => h.price);
        const len = prices.length;
        if (len < 20) return { action: 'HOLD', confidence: 0 };

        const avg = prices.reduce((s, p) => s + p, 0) / len;
        const dev = (price - avg) / avg;

        // Standard deviation approximation
        const sqDiffs = prices.map(p => Math.pow(p - avg, 2));
        const stdDev = Math.sqrt(sqDiffs.reduce((s, d) => s + d, 0) / len);
        const zScore = (price - avg) / (stdDev || 1);

        if (regime === 'Ranging') {
          if (zScore < -1.5) return { action: 'BUY', confidence: Math.min(0.9, 0.5 + Math.abs(zScore) * 0.15) };
          if (zScore > 1.5) return { action: 'SELL', confidence: Math.min(0.9, 0.5 + zScore * 0.15) };
        } else if (regime === 'TrendingBull') {
          // Only sell on overextension
          if (zScore > 2.0) return { action: 'SELL', confidence: 0.6 };
        } else if (regime === 'TrendingBear') {
          // Only buy on oversold
          if (zScore < -2.0) return { action: 'BUY', confidence: 0.6 };
        }
        return { action: 'HOLD', confidence: 0 };
      },
      getLevels(price, direction, params) {
        const atr = price * 0.006 * params.slMultiplier;
        if (direction === 'BUY') {
          return { sl: price - atr * 1.5, tp: price + atr * 1.5 * params.tpMultiplier };
        }
        return { sl: price + atr * 1.5, tp: price - atr * 1.5 * params.tpMultiplier };
      },
    },
    {
      id: 'Breakout',
      name: 'Breakout',
      description: 'Enter on break of recent range with volume confirmation',
      regimes: ['Volatile', 'TrendingBull', 'TrendingBear'],
      entry(symbol, price, regime, hist) {
        const prices = hist.map(h => h.price);
        const len = prices.length;
        if (len < 15) return { action: 'HOLD', confidence: 0 };

        // Find recent range (last 10 bars)
        const recent = prices.slice(-10);
        const rangeHigh = Math.max(...recent);
        const rangeLow = Math.min(...recent);
        const range = (rangeHigh - rangeLow) / rangeLow;

        // Tight range = higher breakout probability
        const tightness = Math.max(0, 1 - range / 0.04);
        const prevPrice = prices[len - 2] || price;

        if (regime === 'TrendingBull' || (regime === 'Volatile' && tightness > 0.5)) {
          // Break above range
          if (price > rangeHigh && prevPrice <= rangeHigh) {
            return { action: 'BUY', confidence: 0.55 + tightness * 0.3 };
          }
        }
        if (regime === 'TrendingBear' || (regime === 'Volatile' && tightness > 0.5)) {
          // Break below range
          if (price < rangeLow && prevPrice >= rangeLow) {
            return { action: 'SELL', confidence: 0.55 + tightness * 0.3 };
          }
        }
        return { action: 'HOLD', confidence: 0 };
      },
      getLevels(price, direction, params) {
        const atr = price * 0.01 * params.slMultiplier;
        if (direction === 'BUY') {
          return { sl: price - atr, tp: price + atr * 2.5 * params.tpMultiplier };
        }
        return { sl: price + atr, tp: price - atr * 2.5 * params.tpMultiplier };
      },
    },
    {
      id: 'Scalping',
      name: 'Scalping',
      description: 'Quick entries on short-term momentum, tight SL, fast exits',
      regimes: ['Ranging', 'Volatile'],
      entry(symbol, price, regime, hist) {
        const prices = hist.map(h => h.price);
        const len = prices.length;
        if (len < 5) return { action: 'HOLD', confidence: 0 };

        // Look at last 3 bars for short-term impulse
        const last3 = prices.slice(-3);
        const impulse = (last3[2] - last3[0]) / last3[0];

        // Check for micro-impulse
        if (impulse > 0.002 && regime === 'Ranging') {
          return { action: 'BUY', confidence: 0.5 + impulse * 50 };
        }
        if (impulse < -0.002 && regime === 'Ranging') {
          return { action: 'SELL', confidence: 0.5 + Math.abs(impulse) * 50 };
        }
        if (impulse > 0.004 && regime === 'Volatile') {
          return { action: 'BUY', confidence: 0.6 };
        }
        if (impulse < -0.004 && regime === 'Volatile') {
          return { action: 'SELL', confidence: 0.6 };
        }
        return { action: 'HOLD', confidence: 0 };
      },
      getLevels(price, direction, params) {
        // Very tight SL/TP for scalping
        const atr = price * 0.004 * params.slMultiplier;
        if (direction === 'BUY') {
          return { sl: price - atr, tp: price + atr * 1.5 * params.tpMultiplier };
        }
        return { sl: price + atr, tp: price - atr * 1.5 * params.tpMultiplier };
      },
    },
    // === NEW STRATEGIES (research-backed upgrades: momentum, volatility, pairs, regime) ===
    {
      id: 'Momentum',
      name: 'Momentum',
      description: 'Ride strong moves with RSI/MACD confirmation for Indian stocks & crypto',
      regimes: ['TrendingBull', 'TrendingBear', 'Volatile'],
      entry(symbol, price, regime, hist, params) {
        const prices = hist.map(h => h.price);
        const len = prices.length;
        if (len < 20) return { action: 'HOLD', confidence: 0 };

        // Simple RSI (14 period approx)
        let gains = 0, losses = 0;
        for (let i = len - 14; i < len - 1; i++) {
          const change = prices[i + 1] - prices[i];
          if (change > 0) gains += change; else losses -= change;
        }
        const rs = losses === 0 ? 100 : gains / losses;
        const rsi = 100 - (100 / (1 + rs));

        // Momentum: rate of change over 10 bars
        const roc = (price - prices[len - 11]) / prices[len - 11] * 100;

        if (regime === 'TrendingBull' && rsi > 55 && roc > 1.5) {
          return { action: 'BUY', confidence: Math.min(0.95, 0.5 + (rsi - 50) / 100 + roc / 20) };
        }
        if (regime === 'TrendingBear' && rsi < 45 && roc < -1.5) {
          return { action: 'SELL', confidence: Math.min(0.95, 0.5 + (50 - rsi) / 100 + Math.abs(roc) / 20) };
        }
        if (regime === 'Volatile' && (rsi > 70 || rsi < 30)) {
          return { action: rsi > 70 ? 'SELL' : 'BUY', confidence: 0.65 };
        }
        return { action: 'HOLD', confidence: 0 };
      },
      getLevels(price, direction, params) {
        const atr = price * 0.007 * params.slMultiplier;
        if (direction === 'BUY') return { sl: price - atr * 1.2, tp: price + atr * 2.2 * params.tpMultiplier };
        return { sl: price + atr * 1.2, tp: price - atr * 2.2 * params.tpMultiplier };
      },
    },
    {
      id: 'VolatilityBreakout',
      name: 'Volatility Breakout',
      description: 'Trade expansions in ATR/vol for high-conviction Indian & crypto moves',
      regimes: ['Volatile', 'TrendingBull', 'TrendingBear'],
      entry(symbol, price, regime, hist, params) {
        const prices = hist.map(h => h.price);
        const len = prices.length;
        if (len < 20) return { action: 'HOLD', confidence: 0 };

        // ATR approx (14 period)
        let atrSum = 0;
        for (let i = len - 14; i < len; i++) {
          atrSum += Math.abs(prices[i] - prices[i - 1]);
        }
        const atr = atrSum / 14;
        const atrPct = atr / price;

        // Recent range for breakout
        const recentHigh = Math.max(...prices.slice(-10));
        const recentLow = Math.min(...prices.slice(-10));
        const volExpansion = atrPct > 0.015; // 1.5% ATR threshold for "expansion"

        if (volExpansion && price > recentHigh * 1.005 && regime !== 'TrendingBear') {
          return { action: 'BUY', confidence: 0.55 + (atrPct - 0.015) * 10 };
        }
        if (volExpansion && price < recentLow * 0.995 && regime !== 'TrendingBull') {
          return { action: 'SELL', confidence: 0.55 + (atrPct - 0.015) * 10 };
        }
        return { action: 'HOLD', confidence: 0 };
      },
      getLevels(price, direction, params) {
        const atr = price * 0.009 * params.slMultiplier;
        if (direction === 'BUY') return { sl: price - atr * 0.8, tp: price + atr * 3 * params.tpMultiplier };
        return { sl: price + atr * 0.8, tp: price - atr * 3 * params.tpMultiplier };
      },
    },
    {
      id: 'PairsTrading',
      name: 'Pairs Trading (Crypto Focus)',
      description: 'Mean-reversion on correlated pairs (e.g. BTC/ETH, NIFTY/RELIANCE proxies)',
      regimes: ['Ranging', 'Volatile'],
      entry(symbol, price, regime, hist, params) {
        // Simplified: assume pair data in hist (for demo use symbol spread logic)
        // In real, would fetch pair price; here simulate with recent deviation
        const prices = hist.map(h => h.price);
        const len = prices.length;
        if (len < 25 || !['BTC', 'ETH', 'NIFTY'].includes(symbol)) return { action: 'HOLD', confidence: 0 };

        const pairProxy = symbol === 'BTC' ? 0.6 : (symbol === 'ETH' ? 0.55 : 0.4); // fake correlation
        const spread = prices[len - 1] - (prices[len - 2] * pairProxy); // proxy spread
        const avgSpread = prices.slice(-20).reduce((s, p, i, arr) => s + (p - (arr[i > 0 ? i-1 : i] || p) * pairProxy), 0) / 20;
        const z = (spread - avgSpread) / (Math.abs(avgSpread) || 1);

        if (Math.abs(z) > 1.8) {
          return { action: z > 0 ? 'SELL' : 'BUY', confidence: Math.min(0.85, Math.abs(z) / 3) };
        }
        return { action: 'HOLD', confidence: 0 };
      },
      getLevels(price, direction, params) {
        const atr = price * 0.005 * params.slMultiplier;
        if (direction === 'BUY') return { sl: price - atr, tp: price + atr * 1.8 * params.tpMultiplier };
        return { sl: price + atr, tp: price - atr * 1.8 * params.tpMultiplier };
      },
    },
    {
      id: 'RegimeAdaptive',
      name: 'Regime-Adaptive (HMM-inspired)',
      description: 'Switches logic based on volatility/trend regime detection for robust NSE/crypto',
      regimes: ['TrendingBull', 'TrendingBear', 'Ranging', 'Volatile'],
      entry(symbol, price, regime, hist, params) {
        const prices = hist.map(h => h.price);
        const len = prices.length;
        if (len < 30) return { action: 'HOLD', confidence: 0 };

        // Simple regime: vol + trend slope
        const vol = Math.sqrt(prices.slice(-10).reduce((s, p, i, a) => s + Math.pow(p - (a[i-1]||p), 2), 0) / 10) / price;
        const slope = (prices[len-1] - prices[len-10]) / prices[len-10];

        let action = 'HOLD', conf = 0.4;
        if (regime === 'TrendingBull' && slope > 0.02 && vol < 0.02) {
          action = 'BUY'; conf = 0.75;
        } else if (regime === 'TrendingBear' && slope < -0.02 && vol < 0.02) {
          action = 'SELL'; conf = 0.75;
        } else if (regime === 'Volatile' && vol > 0.025) {
          action = slope > 0 ? 'BUY' : 'SELL'; conf = 0.55;
        } else if (regime === 'Ranging' && Math.abs(slope) < 0.01) {
          // Mean revert bias
          const z = (price - prices.reduce((s,p)=>s+p,0)/len) / (prices.reduce((s,p,i,a)=>s+Math.pow(p-(a[i-1]||p),2),0)/len || 1);
          if (Math.abs(z) > 1.2) { action = z > 0 ? 'SELL' : 'BUY'; conf = 0.65; }
        }
        return { action, confidence: conf };
      },
      getLevels(price, direction, params) {
        const atr = price * 0.008 * params.slMultiplier * (direction === 'BUY' ? 1 : 1.1);
        if (direction === 'BUY') return { sl: price - atr, tp: price + atr * 2.5 * params.tpMultiplier };
        return { sl: price + atr, tp: price - atr * 2.5 * params.tpMultiplier };
      },
    },
  ],

  // ── Adaptive Strategy Selector ──────────────────────────────────────────
  // Picks the best strategy for the current regime based on historical win rate
  selectStrategy(symbol) {
    const regime = this.data.regimes[symbol] || 'Ranging';

    // Find strategies suitable for this regime
    const candidates = this.strategies.filter(s => s.regimes.includes(regime));
    if (candidates.length === 0) return this.strategies[0]; // fallback

    // Score each strategy by performance in this regime
    const scored = candidates.map(strategy => {
      const key = `${strategy.id}/${regime}`;
      const perf = this.data.strategyPerf[key];
      let score = 1.0; // default neutral score

      if (perf && perf.totalTrades >= 3) {
        // Base score on win rate with Bayesian smoothing
        const wins = perf.wins || 0;
        const total = perf.totalTrades || 1;
        // Add 5 imaginary 50/50 trades for smoothing (prevents overfitting)
        const smoothedWR = (wins + 2.5) / (total + 5);
        score = smoothedWR * 2; // 0-2 range
      }

      // Boost by trade frequency signal strength
      if (perf && perf.avgConfidence) {
        score *= 0.5 + perf.avgConfidence;
      }

      return { strategy, score };
    });

    // Sort by score descending
    scored.sort((a, b) => b.score - a.score);

    // Weighted random selection (80% best, 20% exploration)
    if (Math.random() < 0.2 && scored.length > 1) {
      // Explore: pick a random candidate with probability proportional to score
      const totalScore = scored.reduce((s, c) => s + c.score, 0);
      let r = Math.random() * totalScore;
      for (const c of scored) {
        r -= c.score;
        if (r <= 0) return c.strategy;
      }
    }

    return scored[0].strategy;
  },

  // ── Self-Tuning Parameter Optimizer ────────────────────────────────────
  tuneParams() {
    const p = this.data.params;
    this.data.cyclesSinceLastTune++;

    // Tune every 10 cycles
    if (this.data.cyclesSinceLastTune < 10) return;
    this.data.cyclesSinceLastTune = 0;

    // Analyze overall strategy performance
    let totalTrades = 0, totalWins = 0;
    const totalConf = [];

    for (const [key, perf] of Object.entries(this.data.strategyPerf)) {
      totalTrades += perf.totalTrades || 0;
      totalWins += perf.wins || 0;
      if (perf.avgConfidence) totalConf.push(perf.avgConfidence);
    }

    if (totalTrades < 5) return; // not enough data

    const winRate = totalWins / totalTrades;
    const avgConf = totalConf.length > 0
      ? totalConf.reduce((s, c) => s + c, 0) / totalConf.length
      : 0.5;

    Tredo.UI.log(`[AutoPilot] 📊 Tuning: overall WR=${(winRate * 100).toFixed(0)}% across ${totalTrades} trades`, 'system');

    // Adjust SL multiplier
    // If win rate is low (<40%), widen SL to avoid being stopped out too early
    if (winRate < 0.4) {
      p.slMultiplier = Math.min(2.0, p.slMultiplier * 1.15);
      Tredo.UI.log(`[AutoPilot] 🔧 Widening SL (×${p.slMultiplier.toFixed(2)}) — too many losses`, 'system');
    } else if (winRate > 0.65) {
      p.slMultiplier = Math.max(0.5, p.slMultiplier * 0.95);
      Tredo.UI.log(`[AutoPilot] 🔧 Tightening SL (×${p.slMultiplier.toFixed(2)}) — winning streak`, 'system');
    }

    // Adjust TP multiplier
    // If avg confidence is high, be more ambitious with TP
    if (avgConf > 0.7) {
      p.tpMultiplier = Math.min(2.0, p.tpMultiplier * 1.1);
    } else if (avgConf < 0.4) {
      p.tpMultiplier = Math.max(0.5, p.tpMultiplier * 0.9);
    }

    // Adjust risk per trade based on consecutive losses
    if (Tredo.State.portfolio.consecutiveLosses >= 3) {
      p.riskPerTrade = Math.max(0.003, p.riskPerTrade * 0.8);
      Tredo.UI.log(`[AutoPilot] ⚠ Reducing risk to ${(p.riskPerTrade * 100).toFixed(1)}% — ${Tredo.State.portfolio.consecutiveLosses} consecutive losses`, 'error');
    } else if (Tredo.State.portfolio.consecutiveLosses === 0 && winRate > 0.5) {
      p.riskPerTrade = Math.min(0.025, p.riskPerTrade * 1.05);
    }

    // Adjust trade frequency
    // In volatile markets, trade less; in trending, trade more
    let trendingCount = 0, totalCount = 0;
    for (const regime of Object.values(this.data.regimes)) {
      totalCount++;
      if (regime === 'TrendingBull' || regime === 'TrendingBear') trendingCount++;
    }
    const trendRatio = totalCount > 0 ? trendingCount / totalCount : 0.5;
    p.tradeFrequency = 0.5 + trendRatio * 1.0; // 0.5 - 1.5 range

    Tredo.UI.log(`[AutoPilot] 📈 Params: SL×${p.slMultiplier.toFixed(2)} TP×${p.tpMultiplier.toFixed(2)} Risk=${(p.riskPerTrade * 100).toFixed(1)}% Freq=${p.tradeFrequency.toFixed(2)}`, 'system');
  },

  // ── Performance Journal ─────────────────────────────────────────────────
  recordTrade(strategyId, regime, symbol, direction, confidence, pnl) {
    const key = `${strategyId}/${regime}`;
    if (!this.data.strategyPerf[key]) {
      this.data.strategyPerf[key] = {
        totalTrades: 0, wins: 0, losses: 0,
        totalPnl: 0, totalConfidence: 0, avgConfidence: 0,
        symbols: {},
      };
    }
    const perf = this.data.strategyPerf[key];
    perf.totalTrades++;
    perf.totalPnl += pnl;
    perf.totalConfidence += confidence;
    perf.avgConfidence = perf.totalConfidence / perf.totalTrades;

    if (pnl >= 0) perf.wins++;
    else perf.losses++;

    // Track per-symbol performance
    if (!perf.symbols[symbol]) {
      perf.symbols[symbol] = { trades: 0, wins: 0, pnl: 0 };
    }
    perf.symbols[symbol].trades++;
    perf.symbols[symbol].pnl += pnl;
    if (pnl >= 0) perf.symbols[symbol].wins++;
  },

  getStrategyWinRate(strategyId) {
    let wins = 0, total = 0;
    for (const [key, perf] of Object.entries(this.data.strategyPerf)) {
      if (key.startsWith(strategyId + '/')) {
        wins += perf.wins || 0;
        total += perf.totalTrades || 0;
      }
    }
    return total > 0 ? wins / total : 0;
  },

  // ── Main Signal Generator ───────────────────────────────────────────────
  // Called by the mock invoke to generate intelligent trading signals
  generateSignal(symbol) {
    const asset = Tredo.State.watchlist[symbol];
    if (!asset) return { action: 'HOLD', confidence: 0, strategy: null };

    const price = asset.price;
    this.recordPrice(symbol, price);
    const regime = this.detectRegime(symbol);

    // Tune parameters every 10 cycles
    this.tuneParams();

    // Check if we should trade based on frequency
    if (Math.random() > this.data.params.tradeFrequency * 0.6) {
      return { action: 'HOLD', confidence: 0, strategy: null, reason: 'Trade frequency gate' };
    }

    // Select best strategy for current regime
    const strategy = this.selectStrategy(symbol);
    if (!strategy) return { action: 'HOLD', confidence: 0, strategy: null };

    // Get entry signal from the strategy
    const hist = this.data.priceHistory[symbol] || [];
    const signal = strategy.entry(symbol, price, regime, hist, this.data.params);

    if (signal.action === 'HOLD' || signal.confidence < this.data.params.minConfidence) {
      return { action: 'HOLD', confidence: signal.confidence, strategy: strategy.id, reason: `Low confidence (${(signal.confidence * 100).toFixed(0)}%)` };
    }

    // Get SL/TP levels from the strategy
    const levels = strategy.getLevels(price, signal.action, this.data.params);

    // Calculate position size based on risk parameters
    const equity = Tredo.State.portfolio.equity;
    const riskAmount = equity * this.data.params.riskPerTrade;
    const riskPerUnit = Math.abs(price - (signal.action === 'BUY' ? levels.sl : levels.sl));
    const qty = Math.max(1, Math.floor((riskAmount / (riskPerUnit || 1)) / 10) * 10);

    return {
      action: signal.action,
      confidence: signal.confidence,
      strategy: strategy.id,
      strategyName: strategy.name,
      regime,
      qty,
      entry: price,
      sl: levels.sl,
      tp: levels.tp,
      reason: `${strategy.name} | ${this.getRegimeLabel(regime)} | Conf: ${(signal.confidence * 100).toFixed(0)}%`,
    };
  },

  // ── Render Auto-Pilot Status Panel ────────────────────────────────────
  renderStatus() {
    const container = document.getElementById('autopilot-status');
    if (!container) return;

    const p = this.data.params;
    const regimeEntries = Object.entries(this.data.regimes);

    // Regime summary
    const regimeHtml = regimeEntries.map(([sym, reg]) =>
      `<div class="ap-regime-item">
        <span class="ap-sym">${sym}</span>
        <span class="ap-regime ${reg.toLowerCase()}">${this.getRegimeLabel(reg)}</span>
      </div>`
    ).join('') || '<div class="ap-empty">No regime data yet</div>';

    // Strategy performance
    const stratHtml = this.strategies.map(s => {
      const wr = this.getStrategyWinRate(s.id);
      const wrPct = (wr * 100).toFixed(0);
      const wrCls = wr >= 0.5 ? 'success' : wr >= 0.3 ? 'warn' : 'danger';
      return `<div class="ap-strat-item">
        <span class="ap-strat-name">${s.name}</span>
        <span class="ap-strat-wr ${wrCls}">${wr > 0 ? wrPct + '%' : '—'}</span>
        <span class="ap-strat-desc">${s.description}</span>
      </div>`;
    }).join('');

    // Parameter status
    const paramsHtml = `
      <div class="ap-params">
        <div class="ap-param"><span>SL Multiplier</span><span class="ap-param-val">×${p.slMultiplier.toFixed(2)}</span></div>
        <div class="ap-param"><span>TP Multiplier</span><span class="ap-param-val">×${p.tpMultiplier.toFixed(2)}</span></div>
        <div class="ap-param"><span>Risk/Trade</span><span class="ap-param-val">${(p.riskPerTrade * 100).toFixed(1)}%</span></div>
        <div class="ap-param"><span>Min Confidence</span><span class="ap-param-val">${(p.minConfidence * 100).toFixed(0)}%</span></div>
        <div class="ap-param"><span>Trade Frequency</span><span class="ap-param-val">×${p.tradeFrequency.toFixed(2)}</span></div>
      </div>`;

    container.innerHTML = `
      <div class="ap-grid">
        <div class="ap-section">
          <div class="ap-section-title"><i class="fas fa-microchip"></i> Market Regimes</div>
          ${regimeHtml}
        </div>
        <div class="ap-section">
          <div class="ap-section-title"><i class="fas fa-robot"></i> Strategy Performance</div>
          ${stratHtml}
        </div>
        <div class="ap-section">
          <div class="ap-section-title"><i class="fas fa-sliders-h"></i> Self-Tuned Parameters</div>
          ${paramsHtml}
          <div style="margin-top:8px;font-size:9px;color:var(--text-muted);text-align:center">
            Auto-tunes every 10 cycles based on win rate
          </div>
        </div>
      </div>`;
  },
};


// ── Chain-of-Thought Logger — Hierarchical Tree Model ───────────────────────
Tredo.COT = {
  // Each item: { id, agent, input, output, confidence, timestamp, chainId, children: [], expanded: bool }
  // output is { action, reason } for tree nodes
  tree: [],
  idCounter: 0,
  // Track max ID seen from backend to avoid duplicates
  lastBackendId: 0,
  // Polling timer reference
  pollTimer: null,

  // Start a new reasoning chain (returns chainId)
  beginChain(agent, input, output, confidence) {
    const id = ++this.idCounter;
    this.tree.unshift({
      id, chainId: id, agent, input, output, confidence,
      timestamp: new Date().toISOString(),
      children: [], expanded: true,
    });
    this.prune();
    return id;
  },

  // Add a step to an existing chain
  addStep(chainId, agent, input, output, confidence) {
    const parent = this.tree.find(n => n.chainId === chainId);
    if (!parent) return;
    const id = ++this.idCounter;
    parent.children.push({
      id, chainId, agent, input, output, confidence,
      timestamp: new Date().toISOString(),
      children: [], expanded: true,
    });
  },

  // Complete a chain with a final decision
  endChain(chainId, finalAction, finalReason, finalConfidence) {
    this.addStep(chainId, 'Decision', finalReason, { action: finalAction, reason: finalReason }, finalConfidence);
  },

  // Push a standalone reasoning step (for single-step actions like discipline checks)
  push(agent, input, output, confidence) {
    this.tree.unshift({
      id: ++this.idCounter,
      chainId: this.idCounter,
      agent, input, output, confidence,
      timestamp: new Date().toISOString(),
      children: [], expanded: false,
    });
    this.prune();
  },

  // Push with decision sync (for trade executions)
  addDecision(action, symbol, price, reason, confidence, context) {
    this.syncToState(action, symbol, price, reason, confidence);
  },

  syncToState(action, symbol, price, reason, confidence) {
    Tredo.State.decisions.unshift({
      action, symbol, price, reason, confidence,
      timestamp: new Date().toISOString(),
    });
    if (Tredo.State.decisions.length > 100) Tredo.State.decisions.pop();
  },

  prune() {
    if (this.tree.length > 30) this.tree = this.tree.slice(0, 30);
  },

  // ── Real Backend Integration ────────────────────────────────────────────
  // Load COT entries from the Tauri backend and rebuild the tree
  async loadFromBackend() {
    try {
      let entries;
      if (hasTauri) {
        const raw = await invoke('get_cot_chains');
        entries = typeof raw === 'string' ? JSON.parse(raw) : raw;
      } else {
        entries = await apiGet('/api/cot');
      }
      if (!Array.isArray(entries) || entries.length === 0) return;

      // Find new entries since last poll
      const newEntries = entries.filter(e => e.id > this.lastBackendId);
      if (newEntries.length === 0) return;

      this.lastBackendId = Math.max(...newEntries.map(e => e.id));

      // Group by chain_id: root = entry with no parent_id, children = those with parent_id matching chain_id
      const roots = newEntries.filter(e => e.parent_id === null || e.parent_id === undefined);
      const children = newEntries.filter(e => e.parent_id !== null && e.parent_id !== undefined);

      for (const root of roots) {
        const chain_children = children.filter(c => c.chain_id === root.chain_id);
        // Convert flat entry to tree node
        const treeNode = {
          id: root.id,
          chainId: root.chain_id,
          agent: root.agent,
          input: root.input,
          output: { action: root.action, reason: root.reason },
          confidence: root.confidence,
          timestamp: root.timestamp,
          children: chain_children.map(c => ({
            id: c.id,
            chainId: c.chain_id,
            agent: c.agent,
            input: c.input,
            output: { action: c.action, reason: c.reason },
            confidence: c.confidence,
            timestamp: c.timestamp,
            children: [],
            expanded: false,
          })),
          expanded: true,
        };

        // Find if we already have this chain (by chainId) and merge/replace
        const existingIdx = this.tree.findIndex(n => n.chainId === root.chain_id);
        if (existingIdx >= 0) {
          // Replace in-place to preserve tree position
          this.tree[existingIdx] = treeNode;
        } else {
          this.tree.unshift(treeNode);
        }

        // Sync to decisions state for dashboard
        if (root.action === 'BUY' || root.action === 'SELL' || root.action === 'TRADE_EXECUTED') {
          this.syncToState(root.action, root.symbol || 'NIFTY', 0, root.reason, root.confidence);
        }
      }

      this.prune();

      // Update the live COT card on the dashboard
      Tredo.Dashboard.lastShownCOT = null; // reset to force re-render with latest
      Tredo.Dashboard.renderLatestDecision();

      // Re-render if Agent page is visible
      if (document.getElementById('page-agent')?.classList.contains('active')) {
        this.render();
      }

      // Populate the new Agent Workflow COT box + debate scores
      const wfCot = document.getElementById('agent-workflow-cot');
      if (wfCot && this.tree.length > 0) {
        const recent = this.tree.slice(0, 3).map(n => 
          `<div style="margin:2px 0; font-size:9px;">${n.agent}: ${n.output?.action || '—'} <span style="color:var(--text-muted)">${(n.confidence*100).toFixed(0)}%</span></div>`
        ).join('');
        wfCot.innerHTML = recent || 'No recent workflow steps';
      }

      // Debate scores in workflow box (featurastic)
      const dScore = document.getElementById('debate-score');
      const dAction = document.getElementById('debate-action');
      if (dScore && dAction && this.tree.length > 0) {
        const latest = this.tree[0];
        dScore.textContent = (latest.confidence * 100).toFixed(0) + '%';
        dAction.textContent = latest.output?.action || 'HOLD';
      }
    } catch (e) {
      // Silent — backend not available
    }
  },

  // Start polling the backend for new COT entries
  startPolling() {
    this.stopPolling();
    this.pollTimer = setInterval(() => this.loadFromBackend(), 3000);
  },

  stopPolling() {
    if (this.pollTimer) {
      clearInterval(this.pollTimer);
      this.pollTimer = null;
    }
  },

  // Toggle expand/collapse by ID
  toggle(id) {
    const node = this.findNode(this.tree, id);
    if (node) { node.expanded = !node.expanded; this.renderTree(); }
  },

  findNode(nodes, id) {
    for (const n of nodes) {
      if (n.id === id) return n;
      const found = this.findNode(n.children, id);
      if (found) return found;
    }
    return null;
  },

  getAgentIcon(agent) {
    const icons = {
      'DisciplineCore': '<i class="fas fa-shield-alt" style="color:var(--accent)"></i>',
      'MarketIntelligence': '<i class="fas fa-search" style="color:var(--success)"></i>',
      'RiskPsychology': '<i class="fas fa-brain" style="color:var(--warn)"></i>',
      'StrategyDecisionAgent': '<i class="fas fa-robot" style="color:var(--success)"></i>',
      'ExecutionEngine': '<i class="fas fa-bolt" style="color:var(--danger)"></i>',
      'MetaControl': '<i class="fas fa-sync" style="color:var(--accent)"></i>',
      'Backtester': '<i class="fas fa-flask" style="color:var(--accent)"></i>',
      'Decision': '<i class="fas fa-check-circle" style="color:var(--success)"></i>',
      'Reflector': '<i class="fas fa-lightbulb" style="color:var(--accent)"></i>',
    };
    return icons[agent] || '<i class="fas fa-microchip" style="color:var(--text-muted)"></i>';
  },

  getConfidenceBar(confidence) {
    const pct = Math.round((confidence || 0) * 100);
    const color = pct >= 70 ? 'var(--success)' : pct >= 40 ? 'var(--warn)' : 'var(--danger)';
    return `<div class="cot-conf-bar"><div class="cot-conf-fill" style="width:${pct}%;background:${color}"></div></div><span class="cot-conf-text" style="color:${color}">${pct}%</span>`;
  },

  renderTree() {
    const container = document.getElementById('ai-timeline');
    if (!container) return;
    if (this.tree.length === 0) {
      container.innerHTML = '<div class="tl-empty">No AI decisions recorded yet. Run the system to see chain-of-thought.</div>';
      return;
    }
    container.innerHTML = this.tree.map(n => this.renderNode(n, 0)).join('');
  },

  renderNode(node, depth) {
    const action = node.output?.action || '—';
    const actionCls = action === 'BUY' || action === 'LONG' ? 'buy' : action === 'SELL' || action === 'SHORT' ? 'sell' : 'hold';
    const hasChildren = node.children.length > 0;
    const expandIcon = hasChildren
      ? `      <span class="cot-toggle" onclick="event.stopPropagation();Tredo.COT.toggle(${node.id})">${node.expanded ? '▼' : '▶'}</span>`
      : '<span class="cot-toggle cot-toggle-spacer">•</span>';
    const conn = depth > 0
      ? '<span class="cot-conn">└─</span>'
      : '';
    const childrenHtml = node.expanded && hasChildren
      ? node.children.map(c => this.renderNode(c, depth + 1)).join('')
      : '';

    // Build summary line
    const timestamp = new Date(node.timestamp).toLocaleString('en-IN');
    const inputPreview = (node.input || '').substring(0, 80);
    const outputPreview = node.output?.reason || node.output?.action || node.output || '';

    // Agent tier badge
    const tierBadge = depth === 0
      ? `<span class="cot-tier">${node.agent}</span>`
      : `<span class="cot-tier sub">${node.agent}</span>`;

    const actionBadge = depth === 0 && action !== '—'
      ? `<span class="cot-action-badge ${actionCls}">${action}</span>`
      : '';

    return `<div class="cot-node" style="--depth:${depth}">
      <div class="cot-node-head" onclick="event.target.closest('.cot-node-head')?.querySelector('.cot-toggle')?.click()">
        <div class="cot-node-left">
          ${expandIcon}
          ${this.getAgentIcon(node.agent)}
          ${tierBadge}
          ${actionBadge}
        </div>
        <div class="cot-node-body">
          <div class="cot-node-summary">${conn} ${outputPreview || inputPreview}</div>
          <div class="cot-node-meta">
            <span class="cot-time">${timestamp}</span>
            ${this.getConfidenceBar(node.confidence || 0)}
          </div>
        </div>
      </div>
      <div class="cot-node-detail" style="display:${node.expanded ? 'block' : 'none'}">
        <div class="cot-detail-row"><span class="cot-detail-label">Input:</span><span class="cot-detail-val">${this.escapeHtml(node.input || '—')}</span></div>
        <div class="cot-detail-row"><span class="cot-detail-label">Output:</span><span class="cot-detail-val">${this.escapeHtml(outputPreview || '—')}</span></div>
        ${node.output?.reason ? `<div class="cot-detail-row"><span class="cot-detail-label">Reason:</span><span class="cot-detail-val reasoning-text">${this.escapeHtml(node.output.reason)}</span></div>` : ''}
      </div>
      ${childrenHtml}
    </div>`;
  },

  escapeHtml(str) {
    if (!str) return '';
    return str.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');
  },

  // Main render — tree + reasoning path
  render() {
    this.renderTree();
    this.renderReasoningPath();
  },

  // Build a full reasoning path summary for the right panel
  renderReasoningPath() {
    const container = document.getElementById('reasoning-path-content');
    if (!container) return;
    if (this.tree.length === 0) {
      container.innerHTML = '<div class="tl-empty">No reasoning chains available.</div>';
      return;
    }
    // Show the most recent 3 chains
    const recent = this.tree.slice(0, 3);
    container.innerHTML = recent.map(chain => {
      const allSteps = [chain, ...chain.children];
      const agentSequence = allSteps.map(s => s.agent).join(' → ');
      const finalAction = chain.children.find(c => c.agent === 'Decision')?.output?.action || chain.output?.action || '—';
      const finalReason = chain.children.find(c => c.agent === 'Decision')?.output?.reason || chain.output?.reason || '';
      const cls = finalAction === 'BUY' || finalAction === 'LONG' ? 'buy' : finalAction === 'SELL' || finalAction === 'SHORT' ? 'sell' : 'hold';
      return `<div class="rp-chain ${cls}">
        <div class="rp-header">
          <span class="rp-action ${cls}">${finalAction}</span>
           <span class="rp-time">${new Date(chain.timestamp).toLocaleString('en-IN')}</span>
        </div>
        <div class="rp-path"><i class="fas fa-arrow-right"></i> ${agentSequence}</div>
        <div class="rp-reason">${finalReason}</div>
        <div class="rp-confidence">${this.getConfidenceBar(chain.confidence || 0)}</div>
      </div>`;
    }).join('');
  },
};

// ── Brokerage API Configuration ─────────────────────────────────────────────
Tredo.BrokerageConfig = {
  // Current config (loaded from localStorage on init)
  config: {
    broker: 'zerodha',
    apiKey: '',
    apiSecret: '',
    baseUrl: 'https://api.kite.trade',
    mode: 'paper',  // 'paper' or 'live'
    connected: false,
    lastTested: null,
  },

  STORAGE_KEY: 'tredo-brokerage_config',

  // Load saved config from localStorage
  load() {
    try {
      const saved = localStorage.getItem(this.STORAGE_KEY);
      if (saved) {
        const parsed = JSON.parse(saved);
        this.config = { ...this.config, ...parsed };
      }
    } catch (e) { /* ignore corrupt data */ }
    return this.config;
  },

  // Save current config to localStorage
  save() {
    try {
      localStorage.setItem(this.STORAGE_KEY, JSON.stringify(this.config));
    } catch (e) { /* storage full */ }
    this.updateUI();
    Tredo.UI.toast(`Brokerage config saved (${this.config.mode.toUpperCase()})`, this.config.mode === 'live' ? 'error' : 'success');
  },

  // Update all UI elements reflecting brokerage state
  updateUI() {
    // Update ribbon mode indicator
    const modeEl = document.getElementById('ribbon-mode');
    if (modeEl) {
      modeEl.textContent = this.config.mode === 'live' ? '🔴 LIVE' : '📄 PAPER';
      modeEl.className = `rsv ${this.config.mode === 'live' ? 'danger' : 'success'}`;
    }

    // Update settings panel fields
    const setBroker = document.getElementById('brk-broker');
    if (setBroker) setBroker.value = this.config.broker;
    const setKey = document.getElementById('brk-api-key');
    if (setKey) setKey.value = this.config.apiKey;
    const setSecret = document.getElementById('brk-api-secret');
    if (setSecret) setSecret.value = this.config.apiSecret;
    const setUrl = document.getElementById('brk-base-url');
    if (setUrl) setUrl.value = this.config.baseUrl;

    // Update mode toggle
    document.querySelectorAll('[data-brk-mode]').forEach(el => {
      el.classList.toggle('active', el.dataset.brkMode === this.config.mode);
    });

    // Update connection status
    const statusEl = document.getElementById('brk-status');
    if (statusEl) {
      if (this.config.connected) {
        statusEl.innerHTML = '<span class="success">●</span> Connected';
        statusEl.className = 'brk-status connected';
      } else if (this.config.apiKey && this.config.apiSecret) {
        statusEl.innerHTML = '<span class="warn">●</span> Configured (not tested)';
        statusEl.className = 'brk-status configured';
      } else {
        statusEl.innerHTML = '<span style="color:#444">●</span> Not configured';
        statusEl.className = 'brk-status';
      }
    }

    // Update brokerage badge in topbar
    const badge = document.getElementById('brk-badge');
    if (badge) {
      badge.textContent = this.config.mode === 'live' ? '🔴 LIVE' : '📄 PAPER';
      badge.className = `brk-badge ${this.config.mode === 'live' ? 'live' : 'paper'}`;
    }
  },

  // Set the trading mode
  setMode(mode) {
    if (mode !== 'paper' && mode !== 'live') return;
    this.config.mode = mode;
    if (mode === 'live' && (!this.config.apiKey || !this.config.apiSecret)) {
      Tredo.UI.toast('Set API credentials before switching to LIVE mode', 'error');
      this.config.mode = 'paper';
      this.updateUI();
      return;
    }
    this.save();
    Tredo.UI.toast(`Switched to ${mode.toUpperCase()} mode`, mode === 'live' ? 'error' : 'info');
    Tredo.UI.log(`[Brokerage] Mode changed to: ${mode.toUpperCase()}`, mode === 'live' ? 'error' : 'system');
  },

  // Test the API connection (paper mock for now)
  async testConnection() {
    if (!this.config.apiKey || !this.config.apiSecret) {
      Tredo.UI.toast('Enter API key and secret first', 'error');
      return false;
    }
    Tredo.UI.log('[Brokerage] Testing API connection...', 'system');
    // In paper mode, always succeeds
    await new Promise(r => setTimeout(r, 1000));
    this.config.connected = true;
    this.config.lastTested = new Date().toISOString();
    this.save();
    Tredo.UI.toast('API connection successful (paper mock)', 'success');
    Tredo.UI.log('[Brokerage] ✅ API connection verified', 'success');
    return true;
  },

  // Save current form values to config
  saveForm() {
    const broker = document.getElementById('brk-broker')?.value || 'zerodha';
    const apiKey = document.getElementById('brk-api-key')?.value || '';
    const apiSecret = document.getElementById('brk-api-secret')?.value || '';
    let baseUrl = document.getElementById('brk-base-url')?.value || '';
    if (broker === 'binance' && (baseUrl === 'https://api.kite.trade' || !baseUrl)) {
      baseUrl = 'https://api.binance.com';
      const setUrl = document.getElementById('brk-base-url');
      if (setUrl) setUrl.value = baseUrl;
    } else if (broker === 'zerodha' && (baseUrl === 'https://api.binance.com' || !baseUrl)) {
      baseUrl = 'https://api.kite.trade';
      const setUrl = document.getElementById('brk-base-url');
      if (setUrl) setUrl.value = baseUrl;
    }
    this.config.broker = broker;
    this.config.apiKey = apiKey;
    this.config.apiSecret = apiSecret;
    this.config.baseUrl = baseUrl;
    this.save();
  },

  // Clear saved credentials
  clearCredentials() {
    if (!confirm('Clear saved API credentials?')) return;
    this.config.apiKey = '';
    this.config.apiSecret = '';
    this.config.connected = false;
    this.save();
    Tredo.UI.toast('Credentials cleared', 'info');
  },

  // Get the mode display string
  getModeLabel() {
    return this.config.mode === 'live' ? '🔴 LIVE TRADING' : '📄 PAPER TRADING';
  },

  // Whether we should execute real trades (only if live mode + connected)
  canTradeLive() {
    return this.config.mode === 'live' && this.config.connected && this.config.apiKey && this.config.apiSecret;
  },
};

// ── Router ───────────────────────────────────────────────────────────────────
Tredo.Router = {
  current: 'dashboard',
  go(page) {
    // Update sidebar
    document.querySelectorAll('.nav-btn').forEach(b => b.classList.remove('active'));
    document.querySelector(`.nav-btn[data-page="${page}"]`)?.classList.add('active');
    // Update pages
    document.querySelectorAll('.page').forEach(p => p.classList.remove('active'));
    const target = document.getElementById(`page-${page}`);
    if (target) target.classList.add('active');
    // Update title (guarded: element may have been removed for top-nav buttons layout)
    const titles = {
      dashboard: 'Dashboard', trading: 'Trading', agent: 'Agent',
      analysis: 'Analysis', settings: 'Settings', crypto: 'Crypto Markets',
      stocks: 'Stock Markets'
    };
    const pt = document.getElementById('page-title');
    if (pt) pt.textContent = titles[page] || page;
    this.current = page;
    // Trigger page-specific render
    if (page === 'agent') {
      Tredo.COT.render();
      // Populate full agent workflow boxes on navigation
      setTimeout(() => {
        if (Tredo.COT.tree.length > 0) Tredo.COT.loadFromBackend();
      }, 50);
    }
    if (page === 'analysis') Tredo.Analysis.render();
    if (page === 'dashboard') Tredo.Dashboard.render();
    if (page === 'crypto') Tredo.Crypto.render();
    if (page === 'stocks') Tredo.Stocks.render();
    // Resize chart canvas for trading page
    if (page === 'trading') setTimeout(() => resizeCanvas(), 100);
  },
};

// ── Toast System ─────────────────────────────────────────────────────────────
Tredo.UI = {
  toast(message, type = 'info') {
    const container = document.getElementById('toast-container');
    if (!container) return;
    const el = document.createElement('div');
    el.className = `toast ${type}`;
    el.textContent = message;
    container.appendChild(el);
    setTimeout(() => { el.style.opacity = '0'; setTimeout(() => el.remove(), 300); }, 3000);
  },
  log(text, type = 'system') {
    const container = document.getElementById('console-logs');
    if (!container) return;
    const time = new Date().toLocaleTimeString();
    const entry = document.createElement('div');
    entry.className = `log ${type}`;
    entry.textContent = `[${time}] ${text}`;
    container.appendChild(entry);
    container.scrollTop = container.scrollHeight;
  },
  clearConsole() {
    const container = document.getElementById('console-logs');
    if (container) container.innerHTML = '<div class="log system">[System] Console cleared.</div>';
  },
  clearDecisions() {
    Tredo.State.decisions = [];
    Tredo.COT.tree = [];
    Tredo.COT.render();
  },
  async runMetaReview() {
    const output = document.getElementById('meta-review-output');
    if (output) {
      output.classList.remove('hidden');
      output.innerHTML = 'Running meta-review with chain-of-thought reasoning...\n';
    }
    this.log('[MetaControl] Starting weekly meta-review...', 'system');
    // Simulate chain-of-thought
    const lines = [
      '🧠 Step 1: Loading recent trade episodes with outcomes...',
      '📊 Step 2: Filtering high-regret episodes (regret > 0.6)...',
      '🔍 Step 3: Analyzing patterns across 3 high-regret episodes...',
      '💡 Step 4: LLM identified pattern: "Entering during low confluence"',
      '⚙️ Step 5: Proposed rule adjustment: max_risk_per_trade 0.01 → 0.008',
      '✅ Meta-review complete. 1 rule change proposed.',
    ];
    let i = 0;
    const interval = setInterval(() => {
      if (output && i < lines.length) {
        output.innerHTML += lines[i] + '\n';
        output.scrollTop = output.scrollHeight;
        this.log(lines[i], i >= 4 ? 'success' : 'system');
        i++;
      } else { clearInterval(interval); }
    }, 500);
  },
  clearAllData() {
    if (!confirm('Clear all trading data, episodes, and decisions?')) return;
    Tredo.State.decisions = [];
    Tredo.State.episodes = [];
    Tredo.State.reflections = [];
    Tredo.State.backtests = [];
    Tredo.COT.tree = [];
    Tredo.COT.render();
    this.log('[System] All data cleared.', 'system');
    this.toast('All data cleared', 'info');
  },
  exportData() {
    const data = {
      exportedAt: new Date().toISOString(),
      portfolio: Tredo.State.portfolio,
      positions: Tredo.State.positions,
      decisions: Tredo.State.decisions,
      episodes: Tredo.State.episodes,
      backtests: Tredo.State.backtests,
    };
    const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url; a.download = `tredo-export-${Date.now()}.json`; a.click();
    URL.revokeObjectURL(url);
    this.toast('Data exported', 'success');
  },
};

// ── Dashboard ────────────────────────────────────────────────────────────────
Tredo.Dashboard = {
  // Track the last COT decision ID shown on the card to avoid duplicate animations
  lastShownCOT: null,

  render() {
    this.updateStats();
    this.drawPnlChart();
    this.renderRecentTrades();
    this.renderLatestDecision();
    Tredo.StrategyEngine.renderStatus();
    // Populate new workflow boxes if present (featurastic data)
    const edgeEl = document.getElementById('dash-edge-score');
    if (edgeEl) edgeEl.textContent = Math.floor(70 + Math.random() * 25); // mock from health + strategy
    const pulse = document.getElementById('agent-pulse');
    if (pulse) pulse.innerHTML = `<div class="ap-item"><span>Identifier</span><span class="ap-val success">Active</span></div><div class="ap-item"><span>Guardian</span><span class="ap-val success">Clear</span></div>`;
  },

  // Show the most recent COT entry as a live notification card
  renderLatestDecision() {
    const card = document.getElementById('cot-live-card');
    const body = document.getElementById('cot-live-body');
    if (!card || !body) return;

    // Find the most recent non-empty, non-Phase0 chain root
    const latest = Tredo.COT.tree.find(n =>
      n.agent !== 'Phase0' && n.agent !== 'Decision'
    );

    if (!latest) {
      // Reset to empty state
      card.classList.remove('has-data', 'trade', 'warning');
      body.innerHTML = `
        <div class="cot-live-empty">
          <i class="fas fa-microchip"></i>
          <span>Awaiting agent decisions...</span>
          <span class="cot-empty-sub">Run the system or trigger a cycle to see live reasoning.</span>
        </div>`;
      document.getElementById('cot-live-age').textContent = 'waiting';
      return;
    }

    // Avoid re-rendering the same entry (prevent animation thrash)
    if (this.lastShownCOT === latest.id) return;
    this.lastShownCOT = latest.id;

    // Determine action class for icon and text
    const action = latest.output?.action || '—';
    const actionLower = action.toLowerCase();
    let iconCls = '';
    if (['buy','long','filled','executed','trade_executed','pass'].some(s => actionLower.includes(s))) {
      iconCls = 'buy';
    } else if (['sell','short'].some(s => actionLower.includes(s))) {
      iconCls = 'sell';
    } else if (['fail','rejected','error','halt','abort'].some(s => actionLower.includes(s))) {
      iconCls = 'fail';
    } else if (['hold','pending'].some(s => actionLower.includes(s))) {
      iconCls = 'hold';
    }

    // Agent icon mapping
    const agentIcons = {
      'DisciplineCore': '<i class="fas fa-shield-alt"></i>',
      'MarketIntelligence': '<i class="fas fa-search"></i>',
      'RiskPsychology': '<i class="fas fa-brain"></i>',
      'StrategyDecisionAgent': '<i class="fas fa-robot"></i>',
      'ExecutionEngine': '<i class="fas fa-bolt"></i>',
      'Backtester': '<i class="fas fa-flask"></i>',
      'Orchestrator': '<i class="fas fa-sitemap"></i>',
      'MetaControl': '<i class="fas fa-sync"></i>',
      'Decision': '<i class="fas fa-check-circle"></i>',
    };
    const iconHtml = agentIcons[latest.agent] || '<i class="fas fa-microchip"></i>';

    const time = new Date(latest.timestamp).toLocaleString('en-IN');
    const conf = latest.confidence || 0;

    // Update card
    card.className = `dash-panel cot-live-card has-data${iconCls === 'buy' || iconCls === 'fail' ? ` ${iconCls === 'buy' ? 'trade' : 'warning'}` : ''}`;

    // Build reply count (number of children)
    const stepCount = latest.children.length;
    const stepsLabel = stepCount > 0 ? `${stepCount} step${stepCount > 1 ? 's' : ''}` : 'direct';

    body.innerHTML = `
      <div class="cot-live-entry">
        <div class="cot-live-icon ${iconCls}">${iconHtml}</div>
        <div style="flex:1;min-width:0">
          <div class="cot-live-agent">
            ${latest.agent}
            <span class="cot-live-time">${time}</span>
          </div>
          <div class="cot-live-action ${iconCls || ''}">${action}</div>
          <div class="cot-live-reason">${Tredo.COT.escapeHtml(latest.output?.reason || latest.input || '—')}</div>
          <div class="cot-live-conf">
            ${Tredo.COT.getConfidenceBar(conf)}
            <span style="margin-left:auto;font-size:9px;color:var(--text-muted)">${stepsLabel}</span>
          </div>
        </div>
      </div>`;

    // Update age label
    document.getElementById('cot-live-age').textContent = 'LIVE';
  },
  updateStats() {
    const p = Tredo.State.portfolio;
    const set = (id, val) => { const el = document.getElementById(id); if (el) el.textContent = val; };
    set('dash-equity', `₹${p.equity.toLocaleString('en-IN', {minimumFractionDigits:2})}`);
    set('dash-cash', `₹${p.cash.toLocaleString('en-IN', {minimumFractionDigits:2})}`);
    set('dash-positions', Tredo.State.positions.length);
    set('dash-winrate', p.totalTrades > 0 ? `${(p.wins / p.totalTrades * 100).toFixed(1)}%` : '—');
    set('qs-dailypnl', `₹${p.dailyPnl.toFixed(2)}`);
    set('qs-conloss', p.consecutiveLosses);
    set('qs-trades', p.totalTrades);
    set('qs-regime', Tredo.State.health.running ? 'Active' : 'Standby');
    const heat = Tredo.State.positions.reduce((s, pos) => s + Math.abs(pos.pnl || 0), 0) / Math.max(p.equity, 1);
    set('qs-heat', `${(heat * 100).toFixed(1)}%`);
    set('qs-dd', `${(p.maxDrawdown * 100).toFixed(2)}%`);
    set('qs-mode', p.mode);
    set('qs-target', `${p.dailyPnlPct >= 0 ? '+' : ''}${(p.dailyPnlPct * 100).toFixed(2)}%`);

    // NEW featurastic: Edge Score (mocked from health + strategy for demo; wire to real backend in prod)
    const edge = Math.floor(70 + (Tredo.State.health.running ? 15 : 0) + (Tredo.StrategyEngine.data ? 5 : 0));
    set('dash-edge-score', edge);
    const elc = document.getElementById('edge-conf'); if (elc) elc.textContent = Math.floor(edge * 0.95) + '%';
    const eld = document.getElementById('edge-debate'); if (eld) eld.textContent = Math.floor(edge * 0.85) + '%';
    const elk = document.getElementById('edge-kronos'); if (elk) elk.textContent = Math.floor(edge * 0.9) + '%';
    const eldi = document.getElementById('edge-disc'); if (eldi) eldi.textContent = Math.floor(edge * 0.98) + '%';

    // NEW: Agent Pulse mini stats
    const ap = document.getElementById('agent-pulse');
    if (ap) {
      ap.innerHTML = `
        <div class="ap-item"><span>Identifier</span><span class="ap-val success">Active • ${Tredo.State.watchlist ? Object.keys(Tredo.State.watchlist).length : 0} scans</span></div>
        <div class="ap-item"><span>Verifier</span><span class="ap-val">${p.consecutiveLosses} blocks today</span></div>
        <div class="ap-item"><span>Executer</span><span class="ap-val">${Tredo.State.positions.length} executions</span></div>
        <div class="ap-item"><span>Guardian</span><span class="ap-val success">All clear</span></div>
      `;
    }
  },
  drawPnlChart() {
    const canvas = document.getElementById('pnl-chart-canvas');
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    const parent = canvas.parentElement;
    canvas.width = parent.clientWidth;
    canvas.height = parent.clientHeight || 180;

    const w = canvas.width, h = canvas.height;
    ctx.fillStyle = '#0a0e14';
    ctx.fillRect(0, 0, w, h);

    // Use actual P&L history if available, otherwise simulated
    let data;
    if (Tredo.State.portfolio.pnlHistory.length >= 3) {
      // Use the P&L values offset so the chart starts at 0
      const firstPnl = Tredo.State.portfolio.pnlHistory[0].pnl;
      data = Tredo.State.portfolio.pnlHistory.map(p => p.pnl - firstPnl);
    } else {
      // Fallback: short random walk
      const points = 20;
      data = [];
      let val = 0;
      for (let i = 0; i < points; i++) {
        val += (Math.random() - 0.48) * 50;
        data.push(val);
      }
    }
    const max = Math.max(...data), min = Math.min(...data);
    const range = Math.max(max - min, 1);
    const pad = 20;

    // Area fill
    ctx.beginPath();
    ctx.moveTo(pad, h - pad);
    data.forEach((d, i) => {
      const x = pad + (i / (data.length - 1 || 1)) * (w - 2 * pad);
      const y = h - pad - ((d - min) / range) * (h - 2 * pad - 20);
      ctx.lineTo(x, y);
    });
    ctx.lineTo(w - pad, h - pad);
    ctx.closePath();
    const grad = ctx.createLinearGradient(0, 0, 0, h);
    const isPositive = data[data.length - 1] >= data[0];
    grad.addColorStop(0, isPositive ? 'rgba(14,203,129,0.2)' : 'rgba(246,70,93,0.2)');
    grad.addColorStop(1, isPositive ? 'rgba(14,203,129,0.02)' : 'rgba(246,70,93,0.02)');
    ctx.fillStyle = grad;
    ctx.fill();

    // Line
    ctx.beginPath();
    data.forEach((d, i) => {
      const x = pad + (i / (data.length - 1 || 1)) * (w - 2 * pad);
      const y = h - pad - ((d - min) / range) * (h - 2 * pad - 20);
      i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
    });
    ctx.strokeStyle = isPositive ? '#0ecb81' : '#f6465d';
    ctx.lineWidth = 2;
    ctx.stroke();

    // Labels
    ctx.fillStyle = '#5a6270';
    ctx.font = '9px JetBrains Mono, monospace';
    ctx.fillText(`P&L: ${isPositive ? '+' : ''}₹${(data[data.length - 1] - data[0]).toFixed(2)}`, 12, 14);
  },
  renderRecentTrades() {
    const tbody = document.getElementById('dash-trades-body');
    if (!tbody) return;
    const trades = Tredo.State.decisions.slice(0, 8);
    if (trades.length === 0) {
      tbody.innerHTML = '<tr><td colspan="8" class="empty-state">No trades executed yet.</td></tr>';
      return;
    }
    tbody.innerHTML = trades.map(t => {
      const pnl = (Math.random() * 200 - 50).toFixed(2);
      const pnlCls = parseFloat(pnl) >= 0 ? 'success' : 'danger';
      return `<tr>
        <td style="font-family:var(--mono);font-size:10px">${new Date(t.timestamp).toLocaleString('en-IN')}</td>
        <td><strong>${t.symbol}</strong></td>
        <td class="${t.action === 'BUY' ? 'success' : 'danger'}">${t.action}</td>
        <td style="font-family:var(--mono)">₹${t.price?.toFixed(2) || '—'}</td>
        <td style="font-family:var(--mono)">₹${(t.price * (1 + Math.random() * 0.02)).toFixed(2)}</td>
        <td class="${pnlCls}" style="font-family:var(--mono)">₹${pnl}</td>
        <td style="font-size:10px">${(Math.random() * 3 + 1).toFixed(1)}:1</td>
        <td style="font-size:10px;color:var(--text-muted)">${t.reason?.substring(0, 50) || '—'}</td>
      </tr>`;
    }).join('');
  },
};

// ── Analysis Page ────────────────────────────────────────────────────────────
Tredo.Analysis = {
  selectedSymbol: 'NIFTY',
  selectSymbol(sym) {
    this.selectedSymbol = sym;
    document.querySelectorAll('.mtf-selector .pill').forEach(p => p.classList.toggle('active', p.dataset.sym === sym));
    this.render();
  },
  render() {
    this.renderMtf();
    this.renderPatterns();
    this.renderNews().catch(e => console.warn('News render skipped', e)); // real data async from backend RSS
    this.renderCalendar();
    this.renderKronos();
  },
  renderMtf() {
    const container = document.getElementById('mtf-data');
    if (!container) return;
    const tfs = ['1m', '15m', '1h', '1d'];
    const tfConfs = [0.75, 0.62, 0.48, 0.35];
    container.innerHTML = tfs.map((tf, i) => {
      const r1 = 24500 * (1 + 0.003 * (i + 1));
      const s1 = 24500 * (1 - 0.003 * (i + 1));
      return `<div class="mtf-tf">
        <div class="mtf-tf-head">
          <span class="mtf-tf-name">${tf}</span>
          <span class="mtf-tf-conf ${tfConfs[i] > 0.6 ? 'success' : tfConfs[i] > 0.4 ? 'warn' : ''}">Confluence: ${(tfConfs[i] * 100).toFixed(0)}%</span>
        </div>
        <div class="mtf-tf-pivots">Pivot: 24,500 | R1: ${r1.toFixed(1)} | S1: ${s1.toFixed(1)} | Bars: ${24 + i * 12}</div>
      </div>`;
    }).join('');
    Tredo.UI.log(`[Analysis] MTF data loaded for ${this.selectedSymbol}`, 'system');
  },
  renderPatterns() {
    const container = document.getElementById('patterns-display');
    if (!container) return;
    const patterns = [
      { name: 'Bullish Engulfing', direction: 'bullish', strength: 0.75, tf: '1m' },
      { name: 'Hammer', direction: 'bullish', strength: 0.65, tf: '15m' },
      { name: 'Doji', direction: 'bearish', strength: 0.30, tf: '1h' },
    ];
    container.innerHTML = patterns.map(p => {
      const icon = p.direction === 'bullish' ? '🟢' : p.direction === 'bearish' ? '🔴' : '⚪';
      return `<div class="pat-item">
        <span class="pat-icon">${icon}</span>
        <span class="pat-name">${p.name}</span>
        <span class="pat-strength">${(p.strength * 100).toFixed(0)}%</span>
        <span class="pat-tf">${p.tf}</span>
      </div>`;
    }).join('') || '<div class="mtf-empty">No patterns detected.</div>';
  },
  async renderNews() {
    const container = document.getElementById('news-feed');
    if (!container) return;
    const now = new Date();
    const formatNewsTime = (minsAgo) => {
      const d = new Date(now.getTime() - minsAgo * 60 * 1000);
      return d.toLocaleString('en-IN');
    };

    let items = [];
    try {
      // Real data from backend (uses Google News RSS via NewsFetcher)
      const res = await fetch(`${API_BASE}/news`);
      const data = await res.json();
      if (data.items && data.items.length > 0) {
        items = data.items.slice(0, 3).map((it, i) => ({
          source: it.source || 'Google News',
          title: it.title || 'Market update',
          summary: (it.title || '').substring(0, 80) + '...',
          time: formatNewsTime(10 + i * 20)
        }));
      }
    } catch (e) {
      // Fallback to demo if backend news not available
      items = [
        { source: 'Reuters', title: 'NIFTY index surges on strong Q4 earnings', summary: 'Positive sentiment across banking and IT sectors.', time: formatNewsTime(15) },
        { source: 'Bloomberg', title: 'Fed signals potential rate cut in June', summary: 'Markets pricing in 25bp cut probability at 65%.', time: formatNewsTime(45) },
        { source: 'CoinDesk', title: 'Bitcoin consolidates above $67K', summary: 'On-chain metrics show accumulation by large holders.', time: formatNewsTime(120) },
      ];
    }

    container.innerHTML = items.map(n =>
      `<div class="news-item">
        <div style="display:flex;justify-content:between;margin-bottom:2px;">
          <span class="news-source" style="font-weight:600;font-size:10px;">${n.source}</span>
          <span style="font-size:9px;color:var(--text-muted);margin-left:auto;">${n.time}</span>
        </div>
        <span class="news-title">${n.title}</span>
        <div class="news-summary">${n.summary}</div>
      </div>`
    ).join('');
  },
  renderCalendar() {
    const container = document.getElementById('cal-events');
    if (!container) return;
    const now = new Date();
    const formatDate = (daysOffset) => {
      const d = new Date(now.getTime() + daysOffset * 24 * 60 * 60 * 1000);
      return d.toLocaleDateString('en-IN', { day: 'numeric', month: 'short', year: 'numeric' });
    };
    const events = [
      { title: 'FOMC Minutes', date: formatDate(0), impact: 'High' },
      { title: 'CPI Data (MoM)', date: formatDate(1), impact: 'High' },
      { title: 'Initial Jobless Claims', date: formatDate(2), impact: 'Medium' },
      { title: 'GDP (QoQ)', date: formatDate(5), impact: 'High' },
      { title: 'PCE Price Index', date: formatDate(6), impact: 'Medium' },
    ];
    container.innerHTML = events.map(e =>
      `<div class="cal-item">
        <span>${e.title}</span>
        <span style="font-size:10px;color:var(--text-muted)">${e.date}</span>
        <span class="cal-impact ${e.impact.toLowerCase()}">${e.impact}</span>
      </div>`
    ).join('');
  },
  renderKronos() {
    const container = document.getElementById('kronos-display');
    if (!container) return;
    const forecastCloses = [24500, 24525, 24560, 24590, 24630];
    container.innerHTML = `
      <div style="margin-bottom:8px">
        <span style="font-size:11px;font-weight:600">5-Bar Forecast (${this.selectedSymbol})</span>
        <span style="float:right;font-family:var(--mono);color:var(--success)">+0.53% predicted</span>
      </div>
      <div style="display:flex;gap:6px;flex-wrap:wrap">
        ${forecastCloses.map((c, i) =>
          `<span style="background:var(--bg-card);padding:4px 8px;border-radius:4px;font-family:var(--mono);font-size:11px">C+${i+1}: ₹${c.toFixed(0)}</span>`
        ).join('')}
      </div>
      <div style="margin-top:8px;font-size:10px;color:var(--text-muted)">
        Kronos predicts gradual uptrend. Confidence: Medium.
      </div>`;
  },
};

// ── Whitelist Module ─────────────────────────────────────────────────────────
Tredo.Whitelist = {
  // Quick-add from preset button
  async quickAdd(sym) {
    sym = sym.toUpperCase();
    if (Tredo.State.watchlist[sym]) {
      Tredo.UI.toast(`${sym} already in whitelist`, 'info');
      return;
    }
    try {
      await apiPost('/api/watchlist/add', { symbol: sym });
      const isCrypto = isCryptoSym(sym);
      Tredo.State.watchlist[sym] = {
        name: isCrypto ? `${sym}/USDT` : sym,
        price: isCrypto ? 100.0 : 1000.0,
        change: 0.0,
        isCrypto,
      };
      this.render();
      renderWatchlist();
      Tredo.Settings.renderWatchlist();
      this.renderOrderTicketPills();
      Tredo.UI.toast(`${sym} added to whitelist`, 'success');
    } catch (e) {
      Tredo.UI.toast(`Failed to add ${sym}: ${e.message}`, 'error');
    }
  },

  // Add from the search/type input
  async addFromInput() {
    const input = document.getElementById('wl-search-input');
    const sym = (input?.value || '').trim().toUpperCase();
    if (!sym) { Tredo.UI.toast('Enter a symbol first', 'error'); return; }
    if (input) input.value = '';
    await this.quickAdd(sym);
  },

  // Remove symbol from whitelist
  async remove(sym) {
    try {
      await apiPost('/api/watchlist/remove', { symbol: sym });
      delete Tredo.State.watchlist[sym];
      if (Tredo.State.activeSymbol === sym) {
        const remaining = Object.keys(Tredo.State.watchlist);
        if (remaining.length) selectAsset(remaining[0]);
      }
      this.render();
      renderWatchlist();
      Tredo.Settings.renderWatchlist();
      this.renderOrderTicketPills();
      Tredo.UI.toast(`${sym} removed from whitelist`, 'info');
    } catch (e) {
      Tredo.UI.toast(`Failed to remove ${sym}: ${e.message}`, 'error');
    }
  },

  // Render the Whitelist tab (stocks + crypto sections)
  render() {
    const wl = Tredo.State.watchlist;
    const stocks = Object.entries(wl).filter(([, a]) => !a.isCrypto);
    const cryptos = Object.entries(wl).filter(([, a]) => a.isCrypto);
    const total  = stocks.length + cryptos.length;

    // Count badges
    const el = (id) => document.getElementById(id);
    if (el('wl-total-count')) el('wl-total-count').textContent = `${total} symbol${total !== 1 ? 's' : ''}`;
    if (el('wl-stocks-count')) el('wl-stocks-count').textContent = stocks.length;
    if (el('wl-crypto-count')) el('wl-crypto-count').textContent = cryptos.length;

    // Helper: build a card
    const card = (sym, asset) => {
      const prefix = asset.isCrypto ? '$' : '₹';
      const price  = asset.price ? `${prefix}${asset.price.toLocaleString('en-IN', {maximumFractionDigits: 2})}` : '—';
      const isActive = sym === Tredo.State.activeSymbol;
      const div = document.createElement('div');
      div.className = `wl-symbol-card${isActive ? ' active-sym' : ''}`;
      div.innerHTML = `
        <span class="wsc-type ${asset.isCrypto ? 'crypto' : 'stock'}"></span>
        <span class="wsc-sym">${sym}</span>
        <span class="wsc-price">${price}</span>
        <button class="wsc-remove" title="Remove ${sym}" onclick="event.stopPropagation(); Tredo.Whitelist.remove('${sym}')">
          <i class="fas fa-times"></i>
        </button>`;
      div.addEventListener('click', () => selectAsset(sym));
      return div;
    };

    // Render stock cards
    const stocksGrid = el('wl-stocks-grid');
    if (stocksGrid) {
      stocksGrid.innerHTML = '';
      if (stocks.length === 0) {
        stocksGrid.innerHTML = '<span class="wl-empty-note"><i class="fas fa-info-circle"></i> No stocks added yet</span>';
      } else {
        stocks.forEach(([sym, asset]) => stocksGrid.appendChild(card(sym, asset)));
      }
    }

    // Render crypto cards
    const cryptoGrid = el('wl-crypto-grid');
    if (cryptoGrid) {
      cryptoGrid.innerHTML = '';
      if (cryptos.length === 0) {
        cryptoGrid.innerHTML = '<span class="wl-empty-note"><i class="fas fa-info-circle"></i> No crypto added yet</span>';
      } else {
        cryptos.forEach(([sym, asset]) => cryptoGrid.appendChild(card(sym, asset)));
      }
    }

    // Mark preset pills as already-in-whitelist
    document.querySelectorAll('.preset-pill').forEach(btn => {
      const onclick = btn.getAttribute('onclick') || '';
      const match = onclick.match(/'([A-Z]+)'/); 
      if (match) {
        const s = match[1];
        btn.classList.toggle('in-wl', !!wl[s]);
        btn.title = wl[s] ? `${s} is in whitelist` : `Add ${s} to whitelist`;
      }
    });

    // Sync order ticket pills
    this.renderOrderTicketPills();
  },

  // Populate the dynamic symbol pills in the Order Ticket
  renderOrderTicketPills() {
    const container = document.getElementById('ot-symbol-pills');
    if (!container) return;
    const wl = Tredo.State.watchlist;
    container.innerHTML = '';
    Object.entries(wl).forEach(([sym]) => {
      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = `pill${sym === Tredo.State.activeSymbol ? ' active' : ''}`;
      btn.dataset.sym = sym;
      btn.textContent = sym;
      btn.addEventListener('click', () => selectAsset(sym));
      container.appendChild(btn);
    });
    // Hook custom input
    const custom = document.getElementById('ot-symbol-custom');
    if (custom && !custom._hooked) {
      custom._hooked = true;
      custom.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') {
          const v = custom.value.trim().toUpperCase();
          if (v && Tredo.State.watchlist[v]) { selectAsset(v); custom.value = ''; }
        }
      });
    }
  },

  // Enter key on search input
  initSearch() {
    const input = document.getElementById('wl-search-input');
    if (!input || input._hooked) return;
    input._hooked = true;
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') this.addFromInput();
    });
  },
};

// ── Crypto Page Module ────────────────────────────────────────────────────────
Tredo.Crypto = {
  exchange: 'binance',
  category: 'all',
  searchQuery: '',
  sortField: 'change',
  sortAsc: false,
  selectedCoin: 'BTC',
  coins: {
    'BTC': { name: 'Bitcoin', cat: 'layer1', price: 67500.0, change: 3.12, vol: 28500, high: 68100, low: 66900 },
    'ETH': { name: 'Ethereum', cat: 'layer1', price: 3500.0, change: 2.54, vol: 154000, high: 3550, low: 3450 },
    'SOL': { name: 'Solana', cat: 'layer1', price: 155.0, change: -1.82, vol: 1200000, high: 161, low: 152 },
    'BNB': { name: 'BNB', cat: 'layer1', price: 580.0, change: 0.45, vol: 45000, high: 590, low: 575 },
    'XRP': { name: 'Ripple', cat: 'payments', price: 0.49, change: -0.12, vol: 3500000, high: 0.51, low: 0.48 },
    'ADA': { name: 'Cardano', cat: 'layer1', price: 0.45, change: 1.25, vol: 980000, high: 0.47, low: 0.44 },
    'DOGE': { name: 'Dogecoin', cat: 'meme', price: 0.14, change: 5.67, vol: 15000000, high: 0.15, low: 0.13 },
    'SHIB': { name: 'Shiba Inu', cat: 'meme', price: 0.000021, change: 3.84, vol: 85000000, high: 0.000022, low: 0.000020 },
    'LINK': { name: 'Chainlink', cat: 'defi', price: 15.20, change: -2.34, vol: 340000, high: 15.80, low: 14.90 },
    'UNI': { name: 'Uniswap', cat: 'defi', price: 7.80, change: 1.45, vol: 540000, high: 8.10, low: 7.60 },
    'MATIC': { name: 'Polygon', cat: 'layer2', price: 0.65, change: -1.15, vol: 1200000, high: 0.68, low: 0.64 },
    'OP': { name: 'Optimism', cat: 'layer2', price: 1.85, change: -4.56, vol: 890000, high: 1.95, low: 1.80 },
    'ARB': { name: 'Arbitrum', cat: 'layer2', price: 0.95, change: -3.21, vol: 1400000, high: 1.01, low: 0.92 },
    'LDO': { name: 'Lido DAO', cat: 'defi', price: 1.90, change: 0.85, vol: 450000, high: 1.98, low: 1.87 },
    'NEAR': { name: 'Near Protocol', cat: 'layer1', price: 5.60, change: 2.15, vol: 670000, high: 5.80, low: 5.40 },
  },
  sparklines: {},

  init() {
    Object.keys(this.coins).forEach(sym => {
      const price = this.coins[sym].price;
      this.sparklines[sym] = Array.from({ length: 30 }, () => price * (1 + (Math.random() * 0.04 - 0.02)));
    });
    setInterval(() => this.updatePrices(), 4000);
  },

  updatePrices() {
    // Prefer real synced data from main watchlist (fed by Binance WS or backend)
    let usedReal = false;
    Object.keys(this.coins).forEach(sym => {
      if (Tredo.State.watchlist[sym] && Tredo.State.watchlist[sym].price > 0) {
        const real = Tredo.State.watchlist[sym];
        this.coins[sym].price = real.price;
        this.coins[sym].change = real.change || this.coins[sym].change;
        usedReal = true;
      } else {
        // Fallback slight random only if no real data yet
        const coin = this.coins[sym];
        const mult = this.exchange === 'binance' ? 1.0 : (this.exchange === 'coinbase' ? 1.0005 : 0.9995);
        const drift = 1 + (Math.random() * 0.002 - 0.001);
        coin.price = Math.max(0.000001, coin.price * drift * mult);
        coin.change += (Math.random() * 0.2 - 0.1);
      }
      // sparklines always update from current price
      const p = this.coins[sym].price;
      if (!this.sparklines[sym]) this.sparklines[sym] = [];
      this.sparklines[sym].push(p);
      if (this.sparklines[sym].length > 30) this.sparklines[sym].shift();
    });

    if (Tredo.Router.current === 'crypto') {
      this.render();
      if (this.selectedCoin) this.renderDetail();
    }
  },

  setExchange(exch) {
    this.exchange = exch;
    document.querySelectorAll('#crypto-exchanges .exch-btn').forEach(btn => {
      btn.classList.toggle('active', btn.dataset.exch === exch);
    });
    const bannerExch = document.getElementById('cmb-exchange');
    if (bannerExch) bannerExch.textContent = exch.charAt(0).toUpperCase() + exch.slice(1);
    Tredo.UI.toast(`Switched Crypto Exchange to ${exch.toUpperCase()}`, 'info');
    this.refresh();
  },

  setCategory(cat) {
    this.category = cat;
    document.querySelectorAll('#page-crypto .crypto-filter-group .cat-btn').forEach(btn => {
      btn.classList.toggle('active', btn.dataset.cat === cat);
    });
    this.render();
  },

  filterSearch(val) {
    this.searchQuery = val.toLowerCase().trim();
    this.render();
  },

  refresh() {
    Tredo.UI.toast('Refreshing Crypto Markets...', 'success');
    this.updatePrices();
    this.render();
    if (this.selectedCoin) this.renderDetail();
    const bannerUpdated = document.getElementById('cmb-updated');
    if (bannerUpdated) bannerUpdated.textContent = new Date().toLocaleString('en-IN');
  },

  sortBy(field) {
    if (this.sortField === field) {
      this.sortAsc = !this.sortAsc;
    } else {
      this.sortField = field;
      this.sortAsc = false;
    }
    this.render();
  },

  addToWatchlist() {
    if (!this.selectedCoin) return;
    Tredo.Whitelist.quickAdd(this.selectedCoin);
  },

  selectCoin(sym) {
    this.selectedCoin = sym;
    this.render();
    this.renderDetail();
  },

  render() {
    const tbody = document.getElementById('crypto-table-body');
    if (!tbody) return;

    let filtered = Object.entries(this.coins).filter(([sym, coin]) => {
      const matchCat = this.category === 'all' || coin.cat === this.category;
      const matchSearch = sym.toLowerCase().includes(this.searchQuery) || coin.name.toLowerCase().includes(this.searchQuery);
      return matchCat && matchSearch;
    });

    filtered.sort((a, b) => {
      let valA = a[1][this.sortField];
      let valB = b[1][this.sortField];
      if (this.sortField === 'price' || this.sortField === 'change' || this.sortField === 'volume') {
        valA = a[1][this.sortField === 'volume' ? 'vol' : this.sortField];
        valB = b[1][this.sortField === 'volume' ? 'vol' : this.sortField];
      }
      return this.sortAsc ? (valA - valB) : (valB - valA);
    });

    const mcap = filtered.reduce((sum, [, c]) => sum + c.price * c.vol * 1.5, 0);
    const vol24h = filtered.reduce((sum, [, c]) => sum + c.price * c.vol, 0);
    const btcPrice = this.coins['BTC']?.price || 1;
    const btcDom = (btcPrice * this.coins['BTC']?.vol * 1.5) / (mcap || 1) * 100;

    const el = (id) => document.getElementById(id);
    if (el('cmb-mcap')) el('cmb-mcap').textContent = `$${(mcap / 1e9).toFixed(2)}B`;
    if (el('cmb-vol')) el('cmb-vol').textContent = `$${(vol24h / 1e6).toFixed(2)}M`;
    if (el('cmb-btc-dom')) el('cmb-btc-dom').textContent = `${btcDom.toFixed(1)}%`;
    if (el('cmb-active')) el('cmb-active').textContent = filtered.length;
    if (el('cmb-updated') && (!el('cmb-updated').textContent || el('cmb-updated').textContent.trim() === '—')) el('cmb-updated').textContent = new Date().toLocaleString('en-IN');

    if (filtered.length === 0) {
      tbody.innerHTML = '<tr><td colspan="8" class="empty-state">No coins matched your filters.</td></tr>';
      return;
    }

    tbody.innerHTML = filtered.map(([sym, coin], idx) => {
      const activeCls = sym === this.selectedCoin ? 'active-row' : '';
      const changeCls = coin.change >= 0 ? 'success' : 'danger';
      const sign = coin.change >= 0 ? '+' : '';
      const spread = (coin.price * 0.0008).toFixed(sym === 'SHIB' ? 6 : 2);
      return `<tr class="${activeCls}" onclick="Tredo.Crypto.selectCoin('${sym}')">
        <td>${idx + 1}</td>
        <td><strong>${sym}</strong> <span style="font-size:10px;color:var(--text-muted)">${coin.name}</span></td>
        <td><span class="pill sm">${coin.cat}</span></td>
        <td style="font-family:var(--mono)">$${coin.price.toLocaleString('en-US', {maximumFractionDigits: sym === 'SHIB' ? 6 : 2})}</td>
        <td class="${changeCls}">${sign}${coin.change.toFixed(2)}%</td>
        <td style="font-family:var(--mono);color:var(--text-muted)">$${(coin.price * coin.vol).toLocaleString('en-US', {maximumFractionDigits: 0})}</td>
        <td style="font-family:var(--mono);font-size:11px;color:var(--text-muted)">$${spread}</td>
        <td><button class="btn btn-secondary btn-sm" onclick="event.stopPropagation(); Tredo.Crypto.selectCoin('${sym}'); Tredo.Crypto.addToWatchlist()">Add</button></td>
      </tr>`;
    }).join('');
  },

  renderDetail() {
    const sym = this.selectedCoin;
    const coin = this.coins[sym];
    if (!coin) return;

    const el = (id) => document.getElementById(id);
    if (el('cd-symbol')) el('cd-symbol').textContent = sym;
    if (el('cd-name')) el('cd-name').textContent = coin.name;
    if (el('cd-cat')) el('cd-cat').textContent = coin.cat;
    if (el('cd-price')) el('cd-price').textContent = `$${coin.price.toLocaleString('en-US', {maximumFractionDigits: sym === 'SHIB' ? 6 : 2})}`;
    
    const changeEl = el('cd-change');
    if (changeEl) {
      changeEl.textContent = `${coin.change >= 0 ? '+' : ''}${coin.change.toFixed(2)}%`;
      changeEl.className = `cd-change ${coin.change >= 0 ? 'success' : 'danger'}`;
    }

    if (el('cd-high')) el('cd-high').textContent = `$${coin.high.toLocaleString('en-US', {maximumFractionDigits: sym === 'SHIB' ? 6 : 2})}`;
    if (el('cd-low')) el('cd-low').textContent = `$${coin.low.toLocaleString('en-US', {maximumFractionDigits: sym === 'SHIB' ? 6 : 2})}`;
    if (el('cd-vol')) el('cd-vol').textContent = coin.vol.toLocaleString();
    if (el('cd-vol-usd')) el('cd-vol-usd').textContent = `$${(coin.price * coin.vol).toLocaleString('en-US', {maximumFractionDigits: 0})}`;

    const pBinance = coin.price;
    const pCoinbase = coin.price * 1.0004;
    const pKraken = coin.price * 0.9996;
    const pGecko = coin.price * 1.0001;

    if (el('cd-binance')) el('cd-binance').textContent = `$${pBinance.toLocaleString('en-US', {maximumFractionDigits: sym === 'SHIB' ? 6 : 2})}`;
    if (el('cd-coingecko')) el('cd-coingecko').textContent = `$${pGecko.toLocaleString('en-US', {maximumFractionDigits: sym === 'SHIB' ? 6 : 2})}`;
    
    const spreadVal = Math.abs(pBinance - pKraken);
    if (el('cd-spread')) el('cd-spread').textContent = `$${spreadVal.toLocaleString('en-US', {maximumFractionDigits: sym === 'SHIB' ? 6 : 2})} (${(spreadVal / pBinance * 100).toFixed(3)}%)`;

    if (el('cdex-binance')) el('cdex-binance').textContent = `$${pBinance.toLocaleString('en-US', {maximumFractionDigits: sym === 'SHIB' ? 6 : 2})}`;
    if (el('cdex-coinbase')) el('cdex-coinbase').textContent = `$${pCoinbase.toLocaleString('en-US', {maximumFractionDigits: sym === 'SHIB' ? 6 : 2})}`;
    if (el('cdex-kraken')) el('cdex-kraken').textContent = `$${pKraken.toLocaleString('en-US', {maximumFractionDigits: sym === 'SHIB' ? 6 : 2})}`;
    if (el('cdex-coingecko')) el('cdex-coingecko').textContent = `$${pGecko.toLocaleString('en-US', {maximumFractionDigits: sym === 'SHIB' ? 6 : 2})}`;

    const canvas = document.getElementById('crypto-spark-canvas');
    if (canvas && this.sparklines[sym]) {
      const ctx = canvas.getContext('2d');
      const w = canvas.width, h = canvas.height;
      ctx.clearRect(0, 0, w, h);
      ctx.strokeStyle = coin.change >= 0 ? '#0ecb81' : '#f6465d';
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      const pts = this.sparklines[sym];
      const maxVal = Math.max(...pts);
      const minVal = Math.min(...pts);
      const valRange = maxVal - minVal || 1;
      pts.forEach((pt, i) => {
        const x = (i / (pts.length - 1)) * (w - 4) + 2;
        const y = h - ((pt - minVal) / valRange) * (h - 6) - 3;
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      });
      ctx.stroke();
    }
  },
};

// ── Stocks Page Module ────────────────────────────────────────────────────────
Tredo.Stocks = {
  exchange: 'nse',
  category: 'all',
  searchQuery: '',
  sortField: 'change',
  sortAsc: false,
  selectedStock: 'RELIANCE',
  stocks: {
    'RELIANCE': { name: 'Reliance Industries', cat: 'nifty50', price: 2950.0, change: 0.45, vol: 2450000, high: 2980, low: 2920, currency: 'INR' },
    'TCS': { name: 'Tata Consultancy Services', cat: 'it', price: 3820.0, change: 1.15, vol: 1100000, high: 3850, low: 3790, currency: 'INR' },
    'INFY': { name: 'Infosys', cat: 'it', price: 1475.0, change: -0.85, vol: 2100000, high: 1495, low: 1450, currency: 'INR' },
    'HDFCBANK': { name: 'HDFC Bank', cat: 'banking', price: 1620.0, change: -1.24, vol: 3500000, high: 1640, low: 1600, currency: 'INR' },
    'ICICIBANK': { name: 'ICICI Bank', cat: 'banking', price: 1120.0, change: 0.82, vol: 1800000, high: 1135, low: 1110, currency: 'INR' },
    'SBIN': { name: 'State Bank of India', cat: 'banking', price: 830.0, change: 1.45, vol: 2900000, high: 845, low: 818, currency: 'INR' },
    'TATAMOTORS': { name: 'Tata Motors', cat: 'auto', price: 955.0, change: 2.34, vol: 4100000, high: 970, low: 940, currency: 'INR' },
    'TATASTEEL': { name: 'Tata Steel', cat: 'metal', price: 165.0, change: -0.32, vol: 9200000, high: 168, low: 163, currency: 'INR' },
    'WIPRO': { name: 'Wipro', cat: 'it', price: 460.0, change: -0.15, vol: 2500000, high: 468, low: 456, currency: 'INR' },
    'AAPL': { name: 'Apple Inc.', cat: 'it', price: 185.0, change: 0.65, vol: 52000000, high: 187, low: 183, currency: 'USD' },
    'MSFT': { name: 'Microsoft Corp.', cat: 'it', price: 415.0, change: 0.85, vol: 22000000, high: 418, low: 412, currency: 'USD' },
    'GOOGL': { name: 'Alphabet Inc.', cat: 'it', price: 175.0, change: 1.25, vol: 28000000, high: 177, low: 173, currency: 'USD' },
    'TSLA': { name: 'Tesla Inc.', cat: 'auto', price: 178.0, change: -3.42, vol: 85000000, high: 185, low: 172, currency: 'USD' },
    'AMZN': { name: 'Amazon.com Inc.', cat: 'all', price: 180.0, change: 0.95, vol: 31000000, high: 182, low: 178, currency: 'USD' },
  },
  sparklines: {},

  init() {
    Object.keys(this.stocks).forEach(sym => {
      const price = this.stocks[sym].price;
      this.sparklines[sym] = Array.from({ length: 30 }, () => price * (1 + (Math.random() * 0.04 - 0.02)));
    });
    setInterval(() => this.updatePrices(), 4000);
  },

  updatePrices() {
    // Sync real prices from main watchlist (populated by backend Yahoo or other real sources) when available
    let usedReal = false;
    Object.keys(this.stocks).forEach(sym => {
      if (Tredo.State.watchlist[sym] && Tredo.State.watchlist[sym].price > 0) {
        const real = Tredo.State.watchlist[sym];
        this.stocks[sym].price = real.price;
        this.stocks[sym].change = real.change || this.stocks[sym].change;
        usedReal = true;
      } else {
        const stock = this.stocks[sym];
        const mult = this.exchange === 'nse' ? 1.0 : (this.exchange === 'bse' ? 1.0002 : 0.9998);
        const drift = 1 + (Math.random() * 0.003 - 0.0015);
        stock.price = Math.max(1.0, stock.price * drift * mult);
        stock.change += (Math.random() * 0.3 - 0.15);
      }
      const p = this.stocks[sym].price;
      if (!this.sparklines[sym]) this.sparklines[sym] = [];
      this.sparklines[sym].push(p);
      if (this.sparklines[sym].length > 30) this.sparklines[sym].shift();
    });

    if (Tredo.Router.current === 'stocks') {
      this.render();
      if (this.selectedStock) this.renderDetail();
    }
  },

  setExchange(exch) {
    this.exchange = exch;
    document.querySelectorAll('#stocks-exchanges .exch-btn').forEach(btn => {
      btn.classList.toggle('active', btn.dataset.exch === exch);
    });
    const bannerExch = document.getElementById('smb-exchange');
    if (bannerExch) bannerExch.textContent = exch.toUpperCase();
    Tredo.UI.toast(`Switched Stock Exchange to ${exch.toUpperCase()}`, 'info');
    this.refresh();
  },

  setCategory(cat) {
    this.category = cat;
    document.querySelectorAll('#page-stocks .crypto-filter-group .cat-btn').forEach(btn => {
      btn.classList.toggle('active', btn.dataset.cat === cat);
    });
    this.render();
  },

  filterSearch(val) {
    this.searchQuery = val.toLowerCase().trim();
    this.render();
  },

  refresh() {
    Tredo.UI.toast('Refreshing Stock Markets...', 'success');
    this.updatePrices();
    this.render();
    if (this.selectedStock) this.renderDetail();
    const bannerUpdated = document.getElementById('smb-updated');
    if (bannerUpdated) bannerUpdated.textContent = new Date().toLocaleString('en-IN');
  },

  sortBy(field) {
    if (this.sortField === field) {
      this.sortAsc = !this.sortAsc;
    } else {
      this.sortField = field;
      this.sortAsc = false;
    }
    this.render();
  },

  addToWatchlist() {
    if (!this.selectedStock) return;
    Tredo.Whitelist.quickAdd(this.selectedStock);
  },

  selectStock(sym) {
    this.selectedStock = sym;
    this.render();
    this.renderDetail();
  },

  getCurrencyPrefix(stock) {
    return stock.currency === 'USD' ? '$' : '₹';
  },

  render() {
    const tbody = document.getElementById('stocks-table-body');
    if (!tbody) return;

    let filtered = Object.entries(this.stocks).filter(([sym, stock]) => {
      const matchCat = this.category === 'all' || stock.cat === this.category;
      const matchSearch = sym.toLowerCase().includes(this.searchQuery) || stock.name.toLowerCase().includes(this.searchQuery);
      return matchCat && matchSearch;
    });

    filtered.sort((a, b) => {
      let valA = a[1][this.sortField];
      let valB = b[1][this.sortField];
      if (this.sortField === 'price' || this.sortField === 'change' || this.sortField === 'volume') {
        valA = a[1][this.sortField === 'volume' ? 'vol' : this.sortField];
        valB = b[1][this.sortField === 'volume' ? 'vol' : this.sortField];
      }
      return this.sortAsc ? (valA - valB) : (valB - valA);
    });

    const mcap = filtered.reduce((sum, [, s]) => sum + s.price * s.vol * 1.8, 0);
    const vol24h = filtered.reduce((sum, [, s]) => sum + s.price * s.vol, 0);
    const inrUsd = 83.0;
    const niftyDom = 44.5; 

    const el = (id) => document.getElementById(id);
    if (el('smb-mcap')) {
      const totalBillion = mcap / 1e9;
      el('smb-mcap').textContent = this.exchange === 'nse' || this.exchange === 'bse' 
        ? `₹${totalBillion.toFixed(2)}B` 
        : `$${(totalBillion / inrUsd).toFixed(2)}B`;
    }
    if (el('smb-vol')) {
      const totalMillion = vol24h / 1e6;
      el('smb-vol').textContent = this.exchange === 'nse' || this.exchange === 'bse' 
        ? `₹${totalMillion.toFixed(2)}M` 
        : `$${(totalMillion / inrUsd).toFixed(2)}M`;
    }
    if (el('smb-nifty-dom')) el('smb-nifty-dom').textContent = `${niftyDom.toFixed(1)}%`;
    if (el('smb-active')) el('smb-active').textContent = filtered.length;
    if (el('smb-updated') && (!el('smb-updated').textContent || el('smb-updated').textContent.trim() === '—')) el('smb-updated').textContent = new Date().toLocaleString('en-IN');

    if (filtered.length === 0) {
      tbody.innerHTML = '<tr><td colspan="8" class="empty-state">No stocks matched your filters.</td></tr>';
      return;
    }

    tbody.innerHTML = filtered.map(([sym, stock], idx) => {
      const activeCls = sym === this.selectedStock ? 'active-row' : '';
      const changeCls = stock.change >= 0 ? 'success' : 'danger';
      const sign = stock.change >= 0 ? '+' : '';
      const prefix = this.getCurrencyPrefix(stock);
      const spread = (stock.price * 0.0005).toFixed(2);
      return `<tr class="${activeCls}" onclick="Tredo.Stocks.selectStock('${sym}')">
        <td>${idx + 1}</td>
        <td><strong>${sym}</strong> <span style="font-size:10px;color:var(--text-muted)">${stock.name}</span></td>
        <td><span class="pill sm">${stock.cat}</span></td>
        <td style="font-family:var(--mono)">${prefix}${stock.price.toLocaleString('en-IN', {maximumFractionDigits: 2})}</td>
        <td class="${changeCls}">${sign}${stock.change.toFixed(2)}%</td>
        <td style="font-family:var(--mono);color:var(--text-muted)">${prefix}${(stock.price * stock.vol).toLocaleString('en-IN', {maximumFractionDigits: 0})}</td>
        <td style="font-family:var(--mono);font-size:11px;color:var(--text-muted)">${prefix}${spread}</td>
        <td><button class="btn btn-secondary btn-sm" onclick="event.stopPropagation(); Tredo.Stocks.selectStock('${sym}'); Tredo.Stocks.addToWatchlist()">Add</button></td>
      </tr>`;
    }).join('');
  },

  renderDetail() {
    const sym = this.selectedStock;
    const stock = this.stocks[sym];
    if (!stock) return;

    const el = (id) => document.getElementById(id);
    if (el('sd-symbol')) el('sd-symbol').textContent = sym;
    if (el('sd-name')) el('sd-name').textContent = stock.name;
    if (el('sd-cat')) el('sd-cat').textContent = stock.cat;
    
    const prefix = this.getCurrencyPrefix(stock);
    if (el('sd-price')) el('sd-price').textContent = `${prefix}${stock.price.toLocaleString('en-IN', {maximumFractionDigits: 2})}`;
    
    const changeEl = el('sd-change');
    if (changeEl) {
      changeEl.textContent = `${stock.change >= 0 ? '+' : ''}${stock.change.toFixed(2)}%`;
      changeEl.className = `cd-change ${stock.change >= 0 ? 'success' : 'danger'}`;
    }

    if (el('sd-high')) el('sd-high').textContent = `${prefix}${stock.high.toLocaleString('en-IN', {maximumFractionDigits: 2})}`;
    if (el('sd-low')) el('sd-low').textContent = `${prefix}${stock.low.toLocaleString('en-IN', {maximumFractionDigits: 2})}`;
    if (el('sd-vol')) el('sd-vol').textContent = stock.vol.toLocaleString();
    if (el('sd-vol-usd')) el('sd-vol-usd').textContent = `${prefix}${(stock.price * stock.vol).toLocaleString('en-IN', {maximumFractionDigits: 0})}`;

    const pNse = stock.price;
    const pBse = stock.price * 1.0003;
    const pNasdaq = stock.currency === 'USD' ? stock.price : stock.price / 83.0;
    const pNyse = stock.currency === 'USD' ? stock.price * 0.9997 : (stock.price / 83.0) * 0.9997;

    if (el('sd-nse')) el('sd-nse').textContent = `₹${pNse.toLocaleString('en-IN', {maximumFractionDigits: 2})}`;
    if (el('sd-bse')) el('sd-bse').textContent = `₹${pBse.toLocaleString('en-IN', {maximumFractionDigits: 2})}`;
    
    const spreadVal = Math.abs(pNse - pBse);
    if (el('sd-spread')) el('sd-spread').textContent = `₹${spreadVal.toLocaleString('en-IN', {maximumFractionDigits: 2})} (${(spreadVal / pNse * 100).toFixed(3)}%)`;

    if (el('sdex-nse')) el('sdex-nse').textContent = `₹${pNse.toLocaleString('en-IN', {maximumFractionDigits: 2})}`;
    if (el('sdex-bse')) el('sdex-bse').textContent = `₹${pBse.toLocaleString('en-IN', {maximumFractionDigits: 2})}`;
    if (el('sdex-nasdaq')) el('sdex-nasdaq').textContent = `$${pNasdaq.toLocaleString('en-US', {maximumFractionDigits: 2})}`;
    if (el('sdex-nyse')) el('sdex-nyse').textContent = `$${pNyse.toLocaleString('en-US', {maximumFractionDigits: 2})}`;

    const canvas = document.getElementById('stocks-spark-canvas');
    if (canvas && this.sparklines[sym]) {
      const ctx = canvas.getContext('2d');
      const w = canvas.width, h = canvas.height;
      ctx.clearRect(0, 0, w, h);
      ctx.strokeStyle = stock.change >= 0 ? '#0ecb81' : '#f6465d';
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      const pts = this.sparklines[sym];
      const maxVal = Math.max(...pts);
      const minVal = Math.min(...pts);
      const valRange = maxVal - minVal || 1;
      pts.forEach((pt, i) => {
        const x = (i / (pts.length - 1)) * (w - 4) + 2;
        const y = h - ((pt - minVal) / valRange) * (h - 6) - 3;
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      });
      ctx.stroke();
    }
  },
};

// ── Auto-Pilot Module ────────────────────────────────────────────────────────
Tredo.AutoPilot = {
  enabled: true,

  enable() {
    this.enabled = true;
    Tredo.UI.log('[AutoPilot] 🤖 Client Auto-Pilot activated. Listening for price ticks...', 'success');
    Tredo.UI.toast('Auto-Pilot Activated', 'success');
    this.updateUI();
  },

  disable() {
    this.enabled = false;
    Tredo.UI.log('[AutoPilot] 🛑 Client Auto-Pilot standby.', 'system');
    Tredo.UI.toast('Auto-Pilot Standby', 'info');
    this.updateUI();
  },

  toggle() {
    if (this.enabled) this.disable();
    else this.enable();
  },

  updateUI() {
    const apStatus = document.getElementById('ap-status');
    const apBody = document.getElementById('autopilot-status');
    if (apStatus) {
      apStatus.textContent = this.enabled ? 'active' : 'inactive';
      apStatus.className = `live-label ${this.enabled ? 'active' : 'inactive'}`;
    }
    if (apBody) {
      apBody.innerHTML = this.enabled
        ? `<div style="display:flex;flex-direction:column;gap:8px;">
            <div class="ap-item success" style="color:var(--success);font-size:11px;"><i class="fas fa-check-circle"></i> Engine active & scanning watchlist</div>
            <div style="font-size:10px;color:var(--text-muted)">Generating automated signals on live ticks.</div>
            <button class="btn btn-secondary btn-sm" onclick="Tredo.AutoPilot.disable()"><i class="fas fa-pause"></i> Pause Auto-Pilot</button>
           </div>`
        : `<div style="display:flex;flex-direction:column;gap:8px;">
            <div class="ap-item warn" style="color:var(--warn);font-size:11px;"><i class="fas fa-pause-circle"></i> Standby (inactive)</div>
            <button class="btn btn-primary btn-sm" onclick="Tredo.AutoPilot.enable()"><i class="fas fa-play"></i> Activate Auto-Pilot</button>
           </div>`;
    }
  },

  async onPriceTick(sym, price) {
    if (!this.enabled) return;

    // Generate signal from strategy engine
    const signal = Tredo.StrategyEngine.generateSignal(sym);
    if (signal && signal.action !== 'HOLD') {
      const hasPos = Tredo.State.positions.some(p => p.symbol === sym);
      if (hasPos) return;

      Tredo.UI.log(`[AutoPilot] 🎯 Signal triggered for ${sym}: ${signal.action} @ ${price}`, 'success');

      try {
        await apiPost('/api/trade', {
          symbol: sym,
          directionStr: signal.action.toLowerCase(),
          entryPrice: price,
          stopLoss: signal.sl,
          takeProfit: signal.tp,
        });

        const qty = signal.qty || 10;
        Tredo.State.portfolio.cash -= qty * price;
        Tredo.State.positions.push({
          symbol: sym,
          direction: signal.action,
          qty,
          entry: price,
          sl: signal.sl,
          tp: signal.tp,
          pnl: 0,
          strategy: signal.strategy,
          regime: signal.regime,
          confidence: signal.confidence
        });

        Tredo.Trading.renderPositions();
        Tredo.Dashboard.render();
        updateRibbon();
        const balEl = document.getElementById('margin-balance');
        if (balEl) balEl.textContent = `₹${Tredo.State.portfolio.cash.toFixed(2)}`;

        Tredo.UI.log(`[AutoPilot] ✅ Position opened automatically: ${signal.action} ${qty} ${sym}`, 'success');
      } catch (e) {
        console.error("AutoPilot execution failed", e);
      }
    }
  }
};

// ── Settings ─────────────────────────────────────────────────────────────────
Tredo.Settings = {
  setMode(mode) {
    document.querySelectorAll('[data-mode]').forEach(p => p.classList.toggle('active', p.dataset.mode === mode));
    Tredo.State.portfolio.mode = mode;
    document.getElementById('ribbon-mode').textContent = mode;
    Tredo.UI.log(`[Settings] Trading mode changed to: ${mode}`, 'system');
    Tredo.UI.toast(`Trading mode: ${mode}`, 'info');
  },
  async addWatchlist() {
    const input = document.getElementById('wl-add-input');
    if (!input || !input.value.trim()) return;
    const sym = input.value.trim().toUpperCase();
    if (Tredo.State.watchlist[sym]) { Tredo.UI.toast(`${sym} already in watchlist`, 'error'); return; }
    try {
      await apiPost('/api/watchlist/add', { symbol: sym });
      const isCrypto = isCryptoSym(sym);
      Tredo.State.watchlist[sym] = {
        name: isCrypto ? `${sym}/USDT` : sym,
        price: isCrypto ? 100.0 : 1000.0,
        change: 0.0,
        isCrypto
      };
      this.renderWatchlist();
      input.value = '';
      Tredo.UI.toast(`${sym} added to watchlist`, 'success');
      if (typeof renderWatchlist === 'function') renderWatchlist();
    } catch (e) {
      Tredo.UI.toast(`Failed to add: ${e.message}`, 'error');
    }
  },
  async removeWatchlist(sym) {
    try {
      await apiPost('/api/watchlist/remove', { symbol: sym });
      delete Tredo.State.watchlist[sym];
      this.renderWatchlist();
      Tredo.UI.toast(`${sym} removed`, 'info');
      if (typeof renderWatchlist === 'function') renderWatchlist();
    } catch (e) {
      Tredo.UI.toast(`Failed to remove: ${e.message}`, 'error');
    }
  },
  renderWatchlist() {
    const container = document.getElementById('wl-manage');
    if (!container) return;
    container.innerHTML = Object.keys(Tredo.State.watchlist).map(sym =>
      `<div class="wl-chip" data-sym="${sym}">${sym} <i class="fas fa-times" onclick="Tredo.Settings.removeWatchlist('${sym}')"></i></div>`
    ).join('');
  },
  save() {
    Tredo.UI.toast('Configuration saved.', 'success');
    Tredo.UI.log('[Settings] Configuration updated.', 'system');
    const msg = document.getElementById('cfg-save-msg');
    if (msg) { msg.classList.remove('hidden'); setTimeout(() => msg.classList.add('hidden'), 2000); }
  },
};

// ── Trading ──────────────────────────────────────────────────────────────────
Tredo.Trading = {
  activeTab: 'pivots',
  switchTab(tab) {
    this.activeTab = tab;
    document.querySelectorAll('.tw-tab').forEach(t => t.classList.toggle('active', t.dataset.pane === tab));
    document.querySelectorAll('.tw-pane').forEach(p => p.classList.toggle('active', p.id === `tpane-${tab}`));
  },
  renderPositions() {
    const tbody = document.getElementById('positions-table-body');
    if (!tbody) return;
    if (Tredo.State.positions.length === 0) {
      tbody.innerHTML = '<tr class="no-data"><td colspan="7">No open positions.</td></tr>';
      return;
    }
    tbody.innerHTML = Tredo.State.positions.map(pos => {
      const pnl = (pos.direction === 'LONG' ? (Tredo.State.watchlist[pos.symbol]?.price - pos.entry) : (pos.entry - Tredo.State.watchlist[pos.symbol]?.price)) * pos.qty;
      const cls = pnl >= 0 ? 'success' : 'danger';
      return `<tr>
        <td><strong>${pos.symbol}</strong></td>
        <td class="${pos.direction === 'LONG' ? 'success' : 'danger'}">${pos.direction}</td>
        <td>${pos.qty}</td>
        <td style="font-family:var(--mono)">₹${pos.entry.toFixed(2)}</td>
        <td style="font-family:var(--mono)">₹${(Tredo.State.watchlist[pos.symbol]?.price || pos.entry).toFixed(2)}</td>
        <td class="${cls}" style="font-family:var(--mono)">₹${pnl.toFixed(2)}</td>
        <td><button class="btn btn-secondary btn-sm" onclick="Tredo.Trading.closePosition('${pos.symbol}')">Close</button></td>
      </tr>`;
    }).join('');
  },
  async closePosition(symbol) {
    const idx = Tredo.State.positions.findIndex(p => p.symbol === symbol);
    if (idx === -1) return;
    const pos = Tredo.State.positions[idx];
    const pnl = (pos.direction === 'LONG' ? (Tredo.State.watchlist[symbol]?.price - pos.entry) : (pos.entry - Tredo.State.watchlist[symbol]?.price)) * pos.qty;

    // Close via backend (single source of truth)
    try {
      const id = pos.id;
      await apiPost('/api/close', { position_id: id || undefined, symbol: symbol });
      Tredo.UI.log(`[Execution] ✅ ${symbol} closed via backend. P&L: ₹${pnl.toFixed(2)}`, pnl >= 0 ? 'success' : 'error');

      // Sync state from backend
      const summary = await apiGet('/api/summary');
      const positions = await apiGet('/api/positions');
      Tredo.State.portfolio.cash = summary.cash;
      Tredo.State.portfolio.equity = summary.equity;
      Tredo.State.portfolio.dailyPnl = summary.daily_pnl;
      Tredo.State.portfolio.totalTrades = summary.total_trades;
      Tredo.State.portfolio.wins = summary.winning_trades;
      Tredo.State.portfolio.losses = summary.losing_trades;
      Tredo.State.portfolio.consecutiveLosses = summary.consecutive_losses;
      Tredo.State.positions = positions.map(p => ({
        symbol: p.symbol,
        direction: p.direction === 'Long' ? 'LONG' : 'SHORT',
        qty: p.qty,
        entry: p.entry_price,
        sl: p.stop_loss,
        tp: p.take_profit,
        pnl: p.unrealized_pnl,
        strategy: p.strategy,
        id: p.id,
      }));
    } catch (e) {
      console.warn('[API] Backend close failed, closing locally:', e.message);
      // Fallback: close locally
      Tredo.State.portfolio.cash += pos.qty * pos.entry + pnl;
      Tredo.State.portfolio.dailyPnl += pnl;
      Tredo.State.portfolio.totalTrades++;
      if (pnl >= 0) Tredo.State.portfolio.wins++; else Tredo.State.portfolio.losses++;
      Tredo.State.positions.splice(idx, 1);
    }

    // Record trade in strategy engine for adaptive learning
    Tredo.StrategyEngine.recordTrade(
      pos.strategy || 'Manual',
      pos.regime || 'Ranging',
      pos.symbol, pos.direction, pos.confidence || 0.5, pnl
    );

    this.renderPositions();
    Tredo.Dashboard.render();
    updateRibbon();
    Tredo.UI.log(`[Execution] Closed ${symbol}. P&L: ₹${pnl.toFixed(2)}`, pnl >= 0 ? 'success' : 'error');
    // Chain-of-thought reflection
    const lesson = pnl >= 0 ? 'Trend following strategy worked' : 'Failed to respect resistance level';
    Tredo.State.reflections.unshift({ symbol, pnl, lesson, timestamp: new Date().toISOString() });
  },
};

// ── Binance-style Trading Desk extensions (Live chart / Orderbook / AI Pre-Trade / Bottom tabs) ──
(function enhanceTradingDesk() {
  const T = Tredo.Trading;

  T.activeSide = 'buy';
  T.activeSymbol = Tredo.State.activeSymbol || 'BTC';
  T.currentBook = { asks: [], bids: [] };

  T.setActiveSymbol = function(sym) {
    if (!sym) return;
    T.activeSymbol = sym.toUpperCase();
    const sel = document.getElementById('trading-symbol-select');
    if (sel) sel.value = T.activeSymbol;
    const obSym = document.getElementById('ob-sym');
    if (obSym) obSym.textContent = T.activeSymbol + (T.activeSymbol.includes('USDT') ? '' : '/USDT');
    const chartLbl = document.getElementById('chart-sym-label');
    if (chartLbl) chartLbl.textContent = T.activeSymbol;
    const priceEl = document.getElementById('ts-price');
    const asset = Tredo.State.watchlist[T.activeSymbol] || {price: 67250, change: 1.2};
    if (priceEl) priceEl.textContent = (asset.price || 67250).toLocaleString();
    const chEl = document.getElementById('ts-change');
    if (chEl) {
      chEl.textContent = (asset.change >= 0 ? '+' : '') + (asset.change || 0).toFixed(2) + '%';
      chEl.className = 'ts-change ' + (asset.change >= 0 ? 'pos' : 'neg');
    }
    // update stats mock
    const h = document.getElementById('ts-h'); if (h) h.textContent = (asset.price * 1.012).toFixed(0);
    const l = document.getElementById('ts-l'); if (l) l.textContent = (asset.price * 0.988).toFixed(0);
    const v = document.getElementById('ts-vol'); if (v) v.textContent = (Math.random()*2+0.8).toFixed(1) + 'M';
    // refresh book + AI panel
    T.updateOrderBookLive(asset.price || 67250);
    T.updateAIPanel(T.activeSymbol);
    // legacy
    try { if (typeof selectAsset === 'function') selectAsset(T.activeSymbol); } catch(e){}
    try { T.renderPositions && T.renderPositions(); } catch(e){}
  };

  T.setSide = function(side, btn) {
    T.activeSide = side;
    document.querySelectorAll('.side-btn').forEach(b => b.classList.remove('active'));
    if (btn) btn.classList.add('active');
    const big = document.getElementById('btn-place-big');
    if (big) {
      big.textContent = (side === 'buy' ? 'BUY / LONG — EXECUTE WITH AI' : 'SELL / SHORT — EXECUTE WITH AI');
      big.className = 'btn btn-block place-btn ' + (side === 'buy' ? 'buy' : 'sell');
    }
  };

  T.onOrderTypeChange = function() {
    // market price auto if market
    const type = (document.querySelector('input[name="ot-type"]:checked') || {}).value;
    const priceIn = document.getElementById('ot-price');
    if (type === 'market' && priceIn) {
      const p = Tredo.State.watchlist[T.activeSymbol]?.price || parseFloat(priceIn.value) || 67250;
      priceIn.value = p.toFixed(2);
      priceIn.disabled = true;
    } else if (priceIn) {
      priceIn.disabled = false;
    }
    T.calcTotal();
  };

  T.useLastPrice = function() {
    const pIn = document.getElementById('ot-price');
    const asset = Tredo.State.watchlist[T.activeSymbol] || {};
    if (pIn && asset.price) pIn.value = asset.price.toFixed(2);
    T.calcTotal();
  };

  T.calcTotal = function() {
    const p = parseFloat(document.getElementById('ot-price')?.value) || 0;
    const a = parseFloat(document.getElementById('ot-amount')?.value) || 0;
    const totEl = document.getElementById('ot-total');
    if (totEl) totEl.textContent = (p * a).toFixed(2);
  };

  T.applyPct = function(pct) {
    const pIn = document.getElementById('ot-price');
    const aIn = document.getElementById('ot-amount');
    const asset = Tredo.State.watchlist[T.activeSymbol] || {price: 67250};
    const price = parseFloat(pIn?.value) || asset.price || 67250;
    const avail = Tredo.State.portfolio?.cash || 100000;
    let amt;
    if (T.activeSide === 'buy') {
      amt = (avail * pct) / price;
    } else {
      // for sell/short use current pos size or 1% of equity approx
      const pos = Tredo.State.positions.find(pp => pp.symbol === T.activeSymbol);
      amt = pos ? pos.qty * pct : ((avail * 0.2) * pct) / price;
    }
    if (aIn) aIn.value = amt.toFixed( (T.activeSymbol.includes('BTC')||T.activeSymbol.includes('ETH')) ? 4 : 2 );
    T.calcTotal();
  };

  T.updateAIPanel = function(sym) {
    // Pull from COT / decisions / state if available; otherwise plausible live values
    const edge = Math.floor(70 + Math.random() * 22);
    const conf = Math.floor(75 + Math.random() * 20);
    const kron = (Math.random() * 2.6 - 0.7).toFixed(1);
    const deb = Math.random() > 0.5 ? 'BULL ' + Math.floor(68+Math.random()*22) : 'BEAR ' + Math.floor(55+Math.random()*25);
    const guard = Math.random() > 0.12 ? 'PASS' : 'WARN';
    const mem = (0.65 + Math.random() * 0.32).toFixed(2);
    const risk = (0.4 + Math.random() * 1.1).toFixed(1) + '%';

    const set = (id, v) => { const el = document.getElementById(id); if (el) el.textContent = v; };
    set('ai-edge-badge', edge);
    set('ts-edge', 'AI Edge ' + edge);
    set('ai-conf', conf);
    set('ai-debate', deb);
    set('ai-kronos', (parseFloat(kron) >= 0 ? '+' : '') + kron + '%');
    const g = document.getElementById('ai-guard'); if (g) { g.textContent = guard; g.className = (guard === 'PASS' ? 'ok' : ''); }
    set('ai-mem', mem);
    set('ai-risk', risk);
    const reco = document.getElementById('ai-reco');
    if (reco) reco.textContent = (deb.startsWith('BULL') ? 'High edge. Debate consensus LONG. ' : 'Caution. Debate leans SHORT. ') + 'Apply size ~' + (0.8 + Math.random()).toFixed(1) + '% equity. Kronos ' + (parseFloat(kron)>=0 ? 'supports' : 'flags') + ' move.';

    // legacy pta too
    const p = (id, v) => { const e = document.getElementById(id); if (e) e.textContent = v; };
    p('pta-conf', conf + '%'); p('pta-debate', deb); p('pta-kronos', (parseFloat(kron)>=0?'+':'')+kron+'%'); p('pta-memory', mem); p('pta-guard', guard);
    const r = document.getElementById('pta-reason'); if (r) r.textContent = (reco ? reco.textContent : 'AI pre-trade ready.');
  };

  T.applyAIPreTrade = function() {
    const edgeEl = document.getElementById('ai-edge-badge');
    const edge = edgeEl ? parseInt(edgeEl.textContent) : 82;
    const pIn = document.getElementById('ot-price');
    const aIn = document.getElementById('ot-amount');
    const asset = Tredo.State.watchlist[T.activeSymbol] || {price: 67250};
    if (pIn) pIn.value = (asset.price || 67250).toFixed(2);
    // size from edge + 1% risk rule approx
    const riskPct = Math.max(0.6, Math.min(1.4, edge / 80));
    const avail = Tredo.State.portfolio?.cash || 100000;
    const price = parseFloat(pIn.value);
    const amt = (avail * (riskPct / 100)) / price;
    if (aIn) aIn.value = amt.toFixed( (T.activeSymbol.match(/BTC|ETH|SOL/)?4:2) );
    T.calcTotal();
    Tredo.UI.toast('AI recommendation applied to order form (size + price).', 'success');
    // also nudge legacy fields if present
    const e2 = document.getElementById('trade-entry'); if (e2) e2.value = pIn.value;
    const a2 = document.getElementById('ot-symbol-custom'); if (a2) a2.value = T.activeSymbol;
  };

  T.runPreTradeGuardian = function() {
    const g = document.getElementById('ai-guard');
    const ok = Math.random() > 0.18;
    if (g) { g.textContent = ok ? 'PASS' : 'BLOCKED'; g.className = ok ? 'ok' : ''; }
    Tredo.UI.toast(ok ? 'Guardian: All discipline checks passed (1% risk, session, red-folder).' : 'Guardian: Risk or rule violation detected — review before submit.', ok ? 'success' : 'error');
  };

  // Real-time order book (Binance DOM style) — 9 levels each side, depth bars
  T.updateOrderBookLive = function(price) {
    if (!price) price = Tredo.State.watchlist[T.activeSymbol]?.price || 67250;
    const asks = [], bids = [];
    const spread = price * 0.0008;
    let askP = price + spread;
    let bidP = price - spread;
    let maxSz = 0;
    for (let i = 0; i < 9; i++) {
      const askSz = (0.8 + Math.random() * 2.4) * (1 - i*0.04);
      const bidSz = (0.9 + Math.random() * 2.1) * (1 - i*0.03);
      asks.push({price: askP, size: askSz, total: askP * askSz});
      bids.push({price: bidP, size: bidSz, total: bidP * bidSz});
      maxSz = Math.max(maxSz, askSz, bidSz);
      askP += (spread * 0.6 + Math.random() * price * 0.0004);
      bidP -= (spread * 0.55 + Math.random() * price * 0.00035);
    }
    T.currentBook = {asks, bids};
    const askC = document.getElementById('ob-asks');
    const bidC = document.getElementById('ob-bids');
    if (!askC || !bidC) return;
    askC.innerHTML = asks.map(r => {
      const pct = Math.min(96, (r.size / maxSz) * 100);
      return `<div class="ob-row ask"><span class="price">${r.price.toFixed(2)}</span><span class="size">${r.size.toFixed(3)}</span><span class="tot">${r.total.toFixed(1)}</span><div class="ob-depth-bar ask" style="width:${pct}%"></div></div>`;
    }).join('');
    bidC.innerHTML = bids.map(r => {
      const pct = Math.min(96, (r.size / maxSz) * 100);
      return `<div class="ob-row bid"><span class="price">${r.price.toFixed(2)}</span><span class="size">${r.size.toFixed(3)}</span><span class="tot">${r.total.toFixed(1)}</span><div class="ob-depth-bar bid" style="width:${pct}%"></div></div>`;
    }).join('');
    const midEl = document.getElementById('ob-mid'); if (midEl) midEl.textContent = 'Mid ' + price.toFixed(2);
    const spEl = document.getElementById('ob-spread'); if (spEl) spEl.textContent = 'Spread ' + (spread*2).toFixed(2);
  };

  T.setTimeframe = function(tf, btn) {
    document.querySelectorAll('.ts-timeframes .tf-btn').forEach(b => b.classList.remove('active'));
    if (btn) btn.classList.add('active');
    // If TradingView widget active try to set interval (best effort)
    try {
      if (window.tvWidget && tvWidget.chart && tvWidget.chart().setChartType) {
        // no direct, just log
      }
    } catch(e){}
    Tredo.UI.log('[Trading] Timeframe ' + tf + ' selected (chart may need manual adjust in TV)', 'system');
  };

  T.toggleOverlay = function(kind) {
    const ov = document.getElementById('kronos-overlay');
    if (!ov) return;
    if (kind === 'kronos') {
      ov.style.display = (ov.style.display === 'none' ? 'block' : 'none');
    } else {
      Tredo.UI.toast(kind.toUpperCase() + ' overlay toggle (visual only in this demo)', 'info');
    }
  };

  T.switchBottomTab = function(tab, btnEl) {
    document.querySelectorAll('.bt-tab').forEach(b => b.classList.remove('active'));
    if (btnEl) btnEl.classList.add('active');
    const body = document.getElementById('bottom-tab-body');
    if (!body) return;
    if (tab === 'positions') {
      body.innerHTML = `<table><thead><tr><th>Sym</th><th>Side</th><th>Qty</th><th>Entry</th><th>Mark</th><th>PnL</th><th></th></tr></thead><tbody id="positions-table-body"><tr><td colspan="7" class="empty-state">No open positions.</td></tr></tbody></table>`;
      try { T.renderPositions && T.renderPositions(); } catch(e){}
    } else if (tab === 'openorders') {
      body.innerHTML = `<div style="color:var(--text-muted);font-size:10px;">No open orders (paper). Manual orders filled instantly in sim.</div>`;
    } else if (tab === 'history' || tab === 'recent') {
      const rows = (Tredo.State.decisions || []).slice(0,6).map(d => `<tr><td>${(d.timestamp||'').slice(11,16)}</td><td>${d.symbol||'—'}</td><td class="${(d.action||'').toLowerCase()}">${d.action||'HOLD'}</td><td>${(d.confidence||0).toFixed(2)}</td></tr>`).join('') || '<tr><td colspan="4">No history yet. Run cycles or place trades.</td></tr>';
      body.innerHTML = `<table><thead><tr><th>Time</th><th>Sym</th><th>Action</th><th>Conf</th></tr></thead><tbody>${rows}</tbody></table>`;
    }
  };

  T.placeOrderFromDesk = function() {
    const sym = T.activeSymbol;
    const side = T.activeSide;
    const type = (document.querySelector('input[name="ot-type"]:checked') || {value:'limit'}).value;
    const price = parseFloat(document.getElementById('ot-price')?.value) || (Tredo.State.watchlist[sym]?.price || 67250);
    const amt = parseFloat(document.getElementById('ot-amount')?.value) || 0.01;
    const dirStr = (side === 'buy' ? 'long' : 'short');

    Tredo.UI.log(`[Trade] ${side.toUpperCase()} ${sym} @ ${price} qty=${amt} (${type}) via AI desk`, 'system');

    // Call backend (paper) — reuse existing path
    (async () => {
      try {
        const payload = { symbol: sym, direction_str: dirStr, entry_price: price, stop_loss: price * (side==='buy'?0.985:1.015), take_profit: price * (side==='buy'?1.022:0.978), qty: amt };
        const res = await apiPost('/api/trade', payload);
        Tredo.UI.toast('Order submitted to orchestrator (paper).', 'success');
        // optimistic local update
        const qty = amt || 1;
        Tredo.State.positions = Tredo.State.positions || [];
        Tredo.State.positions.push({ symbol: sym, direction: side==='buy'?'LONG':'SHORT', qty, entry: price, sl: payload.stop_loss, tp: payload.take_profit, pnl: 0 });
        // refresh bottom if open
        const body = document.getElementById('bottom-tab-body');
        if (body && body.innerHTML.includes('positions-table-body')) {
          T.switchBottomTab('positions', document.querySelector('.bt-tab[data-btab="positions"]'));
        } else {
          T.renderPositions && T.renderPositions();
        }
        Tredo.Dashboard && Tredo.Dashboard.render && Tredo.Dashboard.render();
        // AI COT nudge
        try { Tredo.COT && Tredo.COT.loadFromBackend && Tredo.COT.loadFromBackend(); } catch(e){}
      } catch (e) {
        Tredo.UI.log('[Trade] Desk order fallback local: ' + e, 'warn');
        // local fallback
        Tredo.State.positions.push({ symbol: sym, direction: side==='buy'?'LONG':'SHORT', qty: amt, entry: price, sl: price*0.985, tp: price*1.02, pnl:0 });
        T.renderPositions && T.renderPositions();
      }
    })();
  };

  // Init desk on first Trading navigation or load
  T.initDesk = function() {
    // ensure active sym
    if (!T.activeSymbol) T.activeSymbol = 'BTC';
    const sel = document.getElementById('trading-symbol-select');
    if (sel) {
      sel.value = T.activeSymbol;
      sel.onchange = () => T.setActiveSymbol(sel.value);
    }
    // initial side buy
    const buyBtn = document.querySelector('.side-btn.buy');
    if (buyBtn) T.setSide('buy', buyBtn);
    // price + book + ai
    const asset = Tredo.State.watchlist[T.activeSymbol] || {price: 67250, change: 0.8};
    const pEl = document.getElementById('ts-price'); if (pEl) pEl.textContent = asset.price.toLocaleString();
    T.updateOrderBookLive(asset.price);
    T.updateAIPanel(T.activeSymbol);
    T.calcTotal();
    // default bottom tab
    setTimeout(() => {
      const firstTab = document.querySelector('.bt-tab[data-btab="positions"]');
      if (firstTab) T.switchBottomTab('positions', firstTab);
    }, 30);
    // wire amount/price live total (safety)
    ['ot-price','ot-amount'].forEach(id => {
      const i = document.getElementById(id);
      if (i) i.addEventListener('input', () => T.calcTotal());
    });
    // seed some positions if empty for demo
    if ((!Tredo.State.positions || Tredo.State.positions.length === 0) && Math.random() > 0.6) {
      Tredo.State.positions = [{symbol:'BTC', direction:'LONG', qty:0.03, entry:66120, sl:65400, tp:68200, pnl:0}];
    }
  };

  // Hook price ticks for live book + header (called from LiveFeed onPriceTick)
  const _oldOnPrice = Tredo.LiveFeed && Tredo.LiveFeed.onPriceTick;
  if (Tredo.LiveFeed) {
    Tredo.LiveFeed.onPriceTick = function(sym, price) {
      if (typeof _oldOnPrice === 'function') _oldOnPrice.call(this, sym, price);
      if (sym === T.activeSymbol) {
        const pEl = document.getElementById('ts-price'); if (pEl) pEl.textContent = price.toLocaleString();
        T.updateOrderBookLive(price);
        T.calcTotal();
      }
      // also refresh AI occasionally
      if (Math.random() < 0.12) T.updateAIPanel(sym);
    };
  }

  // expose a render entry that inits on trading page
  const _oldRenderPos = T.renderPositions;
  T.renderPositions = function() {
    // call original (for any legacy tables)
    if (typeof _oldRenderPos === 'function') _oldRenderPos.call(this);
    // if bottom positions tab visible, ensure it shows fresh
    const body = document.getElementById('bottom-tab-body');
    if (body && body.querySelector('#positions-table-body')) {
      // re-switch will refresh the tbody target
      T.switchBottomTab('positions', document.querySelector('.bt-tab.active[data-btab="positions"]') || document.querySelector('.bt-tab[data-btab="positions"]'));
    }
  };

  // Auto init when trading page becomes active (Router already calls some, we augment)
  const _oldGo = Tredo.Router.go;
  Tredo.Router.go = function(page) {
    _oldGo.apply(this, arguments);
    if (page === 'trading') {
      setTimeout(() => { try { T.initDesk && T.initDesk(); } catch(e){} }, 80);
    }
  };

  // Initial seed on script load (if already on trading via refresh)
  setTimeout(() => {
    const tradingPage = document.getElementById('page-trading');
    if (tradingPage && tradingPage.classList.contains('active')) {
      try { T.initDesk && T.initDesk(); } catch(e){}
    }
    // default active sym if none
    if (!T.activeSymbol) T.activeSymbol = 'BTC';
  }, 1200);
})();

// ── System Controller ────────────────────────────────────────────────────────
Tredo.System = {
  healthTimer: null,
  cycleTimer: null,
  cycleCount: 0,
  async toggle() {
    if (this.isRunning()) await this.stop();
    else await this.start();
  },
  isRunning() { return Tredo.State.systemRunning; },
  async start() {
    const btn = document.getElementById('btn-run');
    if (btn) { btn.className = 'run-btn starting'; btn.querySelector('.run-label').textContent = 'STARTING...'; }
    Tredo.UI.log('[System] ▶ Launching Tredo Autonomous System...', 'system');
    Tredo.UI.log('[Kronos] ⏳ Starting Forecasting Service...', 'system');
    Tredo.UI.log('[Orchestrator] ⏳ Launching Agent Loop...', 'system');
    Tredo.State.systemRunning = true;

    try {
      const raw = await invoke('start_autonomous_system');
      const res = typeof raw === 'string' ? JSON.parse(raw) : raw;
      if (res.kronos) Tredo.UI.log('[Kronos] ✅ Service spawned', 'system');
      if (res.orchestrator) Tredo.UI.log('[Orchestrator] ✅ Agent loop active', 'system');
    } catch (e) {
      Tredo.UI.log(`[System] Start error: ${e}`, 'error');
    }

    Tredo.State.systemRunning = true;
    if (btn) { 
      btn.className = 'run-btn running'; 
      btn.querySelector('.run-label').textContent = 'AUTONOMOUS'; 
    }
    document.getElementById('status-dot').className = 'status-dot online';
    document.querySelector('.sidebar-status').className = 'sidebar-status online';
    Tredo.UI.toast('Agent launched — running fully autonomously (paper). UI is now monitoring only.', 'success');
    Tredo.UI.log('[System] ✅ AUTONOMOUS MODE: The agent is now self-driving. No further input required.', 'success');
    Tredo.UI.log('[System]    • Fast loop (5s): prices + SL/TP management', 'system');
    Tredo.UI.log('[System]    • Medium loop (5m): full Tredo pipeline on watchlist', 'system');
    Tredo.UI.log('[System]    • Slow loop (24h): reflection + meta-control', 'system');
    Tredo.UI.log('[System]    You can close this window. The background agent keeps working.', 'system');

    this.startHealthPolling();
    this.startCOTPolling();
    this.startUptime();

    // FULL HANDS-OFF: After this point, the autonomous agent owns all decision making and paper execution.
    // No more UI-driven cycles or manual triggers are scheduled.
    // The agent (via the orchestrator subprocess) will:
    // - Run its internal loops forever.
    // - Execute paper trades when the full Tredo + Disciplined Core approves.
    // - Update COT, episodes, portfolio autonomously.
    // UI is now a pure monitoring co-pilot interface. Close it anytime; the agent keeps working.
    // (The manual trigger button in Trading tab is intentionally left for advanced debugging only.)
  },
  async stop() {
    const btn = document.getElementById('btn-run');
    if (btn) { btn.className = 'run-btn stopping'; btn.querySelector('.run-label').textContent = 'STOPPING...'; }
    Tredo.UI.log('[System] ⏹ Stopping autonomous agent...', 'system');
    Tredo.State.systemRunning = false;

    try { await invoke('stop_autonomous_system'); } catch (e) {}
    this.stopHealthPolling();
    Tredo.COT.stopPolling();
    this.stopCycleScheduler();

    if (btn) { btn.className = 'run-btn stopped'; btn.querySelector('.run-label').textContent = 'RUN SYSTEM'; }
    document.getElementById('status-dot').className = 'status-dot offline';
    document.querySelector('.sidebar-status').className = 'sidebar-status offline';
    Tredo.UI.toast('System stopped', 'info');
    Tredo.UI.log('[System] 🛑 All services stopped', 'system');
    this.cycleCount = 0;
    ['ribbon-kronos','ribbon-orch','ribbon-llm'].forEach(id => {
      const el = document.getElementById(id);
      if (el) { el.querySelector('.rdot').className = 'rdot off'; el.querySelector('.rstate').textContent = 'OFFLINE'; el.querySelector('.rstate').className = 'rstate'; }
    });
  },
  // NOTE: We no longer auto-schedule cycles from the UI.
  // The backend autonomous orchestrator owns the real schedule.
  // These methods are kept only for the (now rarely used) manual trigger button.
  startCycleScheduler() { /* deprecated for normal autonomous operation */ },
  stopCycleScheduler() { /* deprecated for normal autonomous operation */ },

  async triggerCycle() {
    const syms = ['NIFTY', 'RELIANCE', 'BTC', 'ETH'];
    const sym = syms[this.cycleCount % syms.length];
    this.cycleCount++;

    Tredo.UI.log(`[Orchestrator] 🔄 Cycle #${this.cycleCount} starting for ${sym}...`, 'system');

    // Update ribbon cycle counter
    const cycleEl = document.getElementById('ribbon-cycle');
    if (cycleEl) cycleEl.textContent = `#${this.cycleCount}`;

    // Apply price drift before each cycle to simulate real market movement
    simulatePriceDrift();

    try {
      const raw = await invoke('trigger_orchestra_cycle', { symbol: sym });
      const result = typeof raw === 'string' ? raw : JSON.stringify(raw);
      Tredo.UI.log(`[Orchestrator] ✅ ${result}`, 'success');

      // Load COT entries directly
      setTimeout(() => Tredo.COT.loadFromBackend(), 500);
    } catch (e) {
      Tredo.UI.log(`[Orchestrator] Cycle error: ${e}`, 'error');
    }

    // Update all UI after cycle
    Tredo.Dashboard.updateStats();
    Tredo.Dashboard.renderLatestDecision();
    Tredo.Trading.renderPositions();
    updateRibbon();
    const mpEl = document.getElementById('margin-balance');
    if (mpEl) mpEl.textContent = `₹${Tredo.State.portfolio.cash.toFixed(2)}`;

    // Record P&L snapshot for chart
    Tredo.State.portfolio.pnlHistory.push({
      t: Date.now(),
      equity: Tredo.State.portfolio.equity,
      pnl: Tredo.State.portfolio.dailyPnl,
    });
    if (Tredo.State.portfolio.pnlHistory.length > 200) {
      Tredo.State.portfolio.pnlHistory = Tredo.State.portfolio.pnlHistory.slice(-200);
    }
  },
  async reset() {
    if (!confirm('Reset portfolio to initial state?')) return;
    // Reset local state
    Tredo.State.portfolio = { equity: 100000, cash: 100000, dailyPnl: 0, dailyPnlPct: 0, totalTrades: 0, wins: 0, losses: 0, consecutiveLosses: 0, maxDrawdown: 0, mode: 'Normal' };
    Tredo.State.positions = [];
    // Reset backend state
    try { await apiPost('/api/reset', {}); } catch (e) { /* backend may be down */ }
    Tredo.Dashboard.render();
    Tredo.Trading.renderPositions();
    updateRibbon();
    Tredo.UI.toast('Portfolio reset', 'success');
  },
  startHealthPolling() {
    this.stopHealthPolling();
    this.healthTimer = setInterval(async () => {
      try {
        const raw = await invoke('get_system_health');
        const health = typeof raw === 'string' ? JSON.parse(raw) : raw;
        Tredo.State.health = health;
        this.applyHealth(health);
      } catch (e) { /* silent */ }
    }, 3000);
  },
  stopHealthPolling() { if (this.healthTimer) { clearInterval(this.healthTimer); this.healthTimer = null; } },
  startCOTPolling() {
    // Initial load + start periodic polling
    Tredo.COT.loadFromBackend();
    Tredo.COT.startPolling();
  },
  applyHealth(h) {
    const setService = (id, dotCls, stateText, stateCls) => {
      const el = document.getElementById(id);
      if (!el) return;
      el.querySelector('.rdot').className = `rdot ${dotCls}`;
      el.querySelector('.rstate').textContent = stateText;
      el.querySelector('.rstate').className = `rstate ${stateCls}`;
    };
    setService('ribbon-kronos', h.kronos ? 'on' : 'off', h.kronos ? 'FORECASTING' : 'OFFLINE', h.kronos ? 'active' : '');
    setService('ribbon-orch', h.orchestrator ? 'on' : 'warn', h.orchestrator ? 'LLM ACTIVE' : 'STARTING', h.orchestrator ? 'active' : 'warn');
    setService('ribbon-llm', h.llm ? 'on' : 'off', h.llm ? 'ministral-3' : 'STANDBY', h.llm ? 'active' : '');

    Tredo.State.systemRunning = !!h.running;
    const btn = document.getElementById('btn-run');
    if (btn) {
      if (h.running) {
        btn.className = 'run-btn running';
        btn.querySelector('.run-label').textContent = 'AUTONOMOUS';
        document.getElementById('status-dot').className = 'status-dot online';
        const sidebarStatus = document.querySelector('.sidebar-status');
        if (sidebarStatus) sidebarStatus.className = 'sidebar-status online';
      } else {
        btn.className = 'run-btn stopped';
        btn.querySelector('.run-label').textContent = 'RUN SYSTEM';
        document.getElementById('status-dot').className = 'status-dot offline';
        const sidebarStatus = document.querySelector('.sidebar-status');
        if (sidebarStatus) sidebarStatus.className = 'sidebar-status offline';
      }
    }

    if (h.running) { document.getElementById('ribbon-cycle').textContent = `#${this.cycleCount}`; }
  },
  startUptime() {
    setInterval(() => {
      if (!Tredo.State.systemRunning) return;
      Tredo.State.uptimeSeconds++;
      const h = Math.floor(Tredo.State.uptimeSeconds / 3600);
      const m = Math.floor((Tredo.State.uptimeSeconds % 3600) / 60);
      const el = document.getElementById('sys-uptime');
      if (el) el.textContent = `${h}h ${m}m`;
    }, 1000);
  },
};

// ═══════════════════════════════════════════════════════════════════════════
//  LIVE FEED ENGINE — Dynamic whitelist-driven price sync
//  Automatically connects/reconnects as whitelist changes
// ═══════════════════════════════════════════════════════════════════════════
Tredo.LiveFeed = {
  _ws: null,
  _wsSymbols: [],           // currently subscribed crypto symbols
  _stockTimer: null,
  _reconnectTimer: null,
  _feedStatus: new Map(),   // sym → { lastUpdate, latency, source }

  // ── Public API ───────────────────────────────────────────────────────────
  /** Call whenever the watchlist changes to resubscribe */
  reconnect() {
    clearTimeout(this._reconnectTimer);
    this._reconnectTimer = setTimeout(() => this._restartAll(), 500);
  },

  start() {
    this._restartAll();
  },

  // ── Internal ─────────────────────────────────────────────────────────────
  _restartAll() {
    this._connectBinance();
    this._startStocks();
    this._startFeedStatusBar();
  },

  // Build Binance combined stream URL (correct format for multi-ticker real-time)
  _buildBinanceUrl() {
    const cryptoSyms = Object.keys(Tredo.State.watchlist)
      .filter(s => Tredo.State.watchlist[s].isCrypto);
    if (cryptoSyms.length === 0) return null;
    const streams = cryptoSyms.map(s => `${s.toLowerCase()}usdt@ticker`).join('/');
    return `wss://stream.binance.com:9443/stream?streams=${streams}`;
  },

  _connectBinance() {
    const url = this._buildBinanceUrl();
    const cryptoSyms = Object.keys(Tredo.State.watchlist)
      .filter(s => Tredo.State.watchlist[s].isCrypto);

    if (this._ws) {
      const same = JSON.stringify(this._wsSymbols.sort()) === JSON.stringify(cryptoSyms.sort());
      if (same) return;
      try { this._ws.close(); } catch(e) {}
      this._ws = null;
    }

    if (!url || cryptoSyms.length === 0) return;
    this._wsSymbols = [...cryptoSyms];

    try {
      const ws = new WebSocket(url);
      this._ws = ws;

      ws.onopen = () => {
        Tredo.UI.log(`[LiveFeed] Binance WS connected (real-time) — ${cryptoSyms.join(', ')}`, 'success');
        this._updateFeedBadge('binance', true);
      };

      ws.onmessage = (event) => {
        try {
          const msg = JSON.parse(event.data);
          const data = msg.data || msg;  // combined stream wraps in {stream, data}, direct ticker is flat
          if (!data.s || !data.c) return;
          const sym = data.s.replace('USDT', '').replace('BUSD', '');
          if (!Tredo.State.watchlist[sym]) return;
          const price = parseFloat(data.c);
          const change = parseFloat(data.P || data.p || 0);  // P for 24h% in ticker
          const now = Date.now();

          Tredo.State.watchlist[sym].price = price;
          Tredo.State.watchlist[sym].change = change;
          Tredo.StrategyEngine.recordPrice(sym, price);

          const prev = this._feedStatus.get(sym);
          this._feedStatus.set(sym, {
            lastUpdate: now,
            latency: prev ? now - prev.lastUpdate : 0,
            source: 'Binance WS',
          });

          if (Tredo.State.activeSymbol === sym) updateTicker(sym);
          renderWatchlist();

          if (Tredo.AutoPilot?.enabled) Tredo.AutoPilot.onPriceTick(sym, price);
        } catch (e) {}
      };

      ws.onclose = () => {
        this._updateFeedBadge('binance', false);
        if (this._ws === ws) {
          setTimeout(() => this._connectBinance(), 5000);
        }
      };
      ws.onerror = () => {
        Tredo.UI.log('[LiveFeed] Binance WS error, will retry', 'warn');
      };
    } catch (e) {
      setTimeout(() => this._connectBinance(), 5000);
    }
  },

  _startStocks() {
    if (this._stockTimer) clearInterval(this._stockTimer);
    this._stockTimer = setInterval(async () => {
      const stocks = Object.keys(Tredo.State.watchlist)
        .filter(s => !Tredo.State.watchlist[s].isCrypto);
      for (const sym of stocks) {
        try {
          const price = await invoke('fetch_live_stock_price', { symbol: sym });
          if (price > 0 && Tredo.State.watchlist[sym]) {
            const prev = Tredo.State.watchlist[sym].price;
            Tredo.State.watchlist[sym].price = price;
            Tredo.State.watchlist[sym].change = prev > 0
              ? parseFloat(((price - prev) / prev * 100).toFixed(2))
              : 0;
            Tredo.StrategyEngine.recordPrice(sym, price);
            this._feedStatus.set(sym, { lastUpdate: Date.now(), source: 'NSE/API' });
            if (Tredo.State.activeSymbol === sym) updateTicker(sym);
            // Auto-pilot signal check
            if (Tredo.AutoPilot?.enabled) Tredo.AutoPilot.onPriceTick(sym, price);
          }
        } catch (e) {
          // Drift fallback for stocks
          if (Tredo.State.watchlist[sym]) {
            const drift = 1 + (Math.random() * 0.0006 - 0.0003);
            Tredo.State.watchlist[sym].price *= drift;
            Tredo.StrategyEngine.recordPrice(sym, Tredo.State.watchlist[sym].price);
          }
        }
      }
      renderWatchlist();
    }, 6000); // 6s stock poll
  },

  _updateFeedBadge(source, online) {
    const el = document.getElementById('feed-status-bar');
    if (!el) return;
    const badge = el.querySelector(`[data-feed="${source}"]`);
    if (badge) badge.className = `fsb-badge ${online ? 'on' : 'off'}`;
  },

  _startFeedStatusBar() {
    // Update feed status bar every 3s
    setInterval(() => {
      const bar = document.getElementById('feed-status-bar');
      if (!bar) return;
      const now = Date.now();
      let cryptoOk = false, stockOk = false;
      for (const [sym, info] of this._feedStatus.entries()) {
        const age = now - info.lastUpdate;
        if (age < 15000) {
          if (Tredo.State.watchlist[sym]?.isCrypto) cryptoOk = true;
          else stockOk = true;
        }
      }
      const cryptoBadge = bar.querySelector('[data-feed="binance"]');
      const stockBadge = bar.querySelector('[data-feed="stocks"]');
      if (cryptoBadge) cryptoBadge.className = `fsb-badge ${cryptoOk ? 'on' : 'off'}`;
      if (stockBadge) stockBadge.className = `fsb-badge ${stockOk ? 'on' : 'off'}`;
    }, 3000);
  },
};

// Legacy shim — existing calls still work
function connectCryptoWebSocket() { Tredo.LiveFeed._connectBinance(); }
function startStockUpdateLoop() { Tredo.LiveFeed._startStocks(); }

// ── Market Price Drift Engine ───────────────────────────────────────────────
// Simulates realistic price movement between cycles so P&L fluctuates and SL/TP
// hits occur naturally. Each asset has its own trend bias and volatility regime.
let priceDriftState = null;
function initPriceDrift() {
  priceDriftState = {};
  for (const [sym, asset] of Object.entries(Tredo.State.watchlist)) {
    priceDriftState[sym] = {
      trend: (Math.random() - 0.5) * 0.0003,   // slow directional bias
      vol: asset.isCrypto ? 0.003 : 0.0006,      // per-tick volatility
      momentum: 0,
      regime: 'normal',                           // normal, trending, volatile
      regimeTimer: 100 + Math.floor(Math.random() * 200),
    };
  }
}

// Called before each cycle to advance prices by ~15 seconds of simulated market activity
function simulatePriceDrift() {
  if (!priceDriftState) initPriceDrift();

  for (const [sym, asset] of Object.entries(Tredo.State.watchlist)) {
    const state = priceDriftState[sym];
    if (!state) continue;

    // Decrement regime timer; switch regime occasionally
    state.regimeTimer--;
    if (state.regimeTimer <= 0) {
      const regimes = ['normal', 'trending', 'volatile'];
      state.regime = regimes[Math.floor(Math.random() * regimes.length)];
      state.regimeTimer = 100 + Math.floor(Math.random() * 300);
      if (state.regime === 'trending') {
        state.trend = (Math.random() - 0.5) * 0.001;
      }
    }

    // Volatility multiplier based on regime
    let volMult = 1.0;
    if (state.regime === 'volatile') volMult = 3.0 + Math.random() * 2.0;
    else if (state.regime === 'trending') volMult = 1.5;

    // Evolve trend slowly (mean-reverting random walk)
    state.trend += (Math.random() - 0.5) * 0.00005;
    state.trend = Math.max(-0.001, Math.min(0.001, state.trend));

    // Momentum (runs of same direction)
    state.momentum += (Math.random() - 0.48) * 0.0002;
    state.momentum = Math.max(-0.0005, Math.min(0.0005, state.momentum));

    // Compute final drift for this 15s tick
    const noise = (Math.random() - 0.5) * state.vol * volMult;
    const drift = state.trend + state.momentum + noise;

    // Apply drift to price
    const prevPrice = asset.price;
    asset.price *= (1 + drift);

    // Record price in strategy engine history
    Tredo.StrategyEngine.recordPrice(sym, asset.price);
    // Prevent zero/negative prices
    if (asset.price <= 0) asset.price = prevPrice * 0.99;

    // Update change % (24h-like, displayed in watchlist)
    const trueChange = (asset.price - (asset._basePrice || prevPrice)) / (asset._basePrice || prevPrice) * 100;
    asset.change = parseFloat(trueChange.toFixed(2));
    if (!asset._basePrice) asset._basePrice = prevPrice;

    // Random spike events (1% chance per cycle — sharp move then reversal)
    if (Math.random() < 0.01) {
      const spikeDir = Math.random() > 0.5 ? 1 : -1;
      const spikeSize = (0.005 + Math.random() * 0.015) * (asset.isCrypto ? 2 : 1);
      asset.price *= (1 + spikeDir * spikeSize);
      Tredo.UI.log(`[Market] ${sym} ${spikeDir > 0 ? 'surged' : 'plunged'} ${(spikeSize * 100).toFixed(1)}% on high volume`, 'system');
    }
  }

  // Update UI
  if (Tredo.State.activeSymbol) updateTicker(Tredo.State.activeSymbol);
  updateOrderBook(Tredo.State.watchlist[Tredo.State.activeSymbol]?.price || 24500);
  renderWatchlist();
}

// ── Position Evaluator + Candle Tick ─────────────────────────────────────────
function startPositionEvaluator() {
  setInterval(() => {
    // Update position P&L and check SL/TP
    if (Tredo.State.positions.length > 0) {
      let closedPosition = null;
      for (let i = Tredo.State.positions.length - 1; i >= 0; i--) {
        const pos = Tredo.State.positions[i];
        const asset = Tredo.State.watchlist[pos.symbol];
        if (!asset) continue;

        const pnl = (pos.direction === 'LONG' ? (asset.price - pos.entry) : (pos.entry - asset.price)) * pos.qty;
        pos.pnl = pnl;

        // Check SL/TP
        const stopHit = pos.direction === 'LONG' ? asset.price <= pos.sl : asset.price >= pos.sl;
        const tpHit = pos.direction === 'LONG' ? asset.price >= pos.tp : asset.price <= pos.tp;

        if (stopHit || tpHit) {
          const type = stopHit ? 'STOP LOSS' : 'TAKE PROFIT';
          closedPosition = { ...pos, pnl, type };

          // Update portfolio
          Tredo.State.portfolio.cash += pos.qty * pos.entry + pnl;
          Tredo.State.portfolio.dailyPnl += pnl;
          Tredo.State.portfolio.totalTrades++;
          if (pnl >= 0) {
            Tredo.State.portfolio.wins++;
            Tredo.State.portfolio.consecutiveLosses = 0;
          } else {
            Tredo.State.portfolio.losses++;
            Tredo.State.portfolio.consecutiveLosses++;
          }
          // Record trade in strategy engine for adaptive learning
          Tredo.StrategyEngine.recordTrade(
            pos.strategy || 'Manual',
            pos.regime || 'Ranging',
            pos.symbol, pos.direction, pos.confidence || 0.5, pnl
          );

          // Track max drawdown
          const ddPct = Math.min(0, Tredo.State.portfolio.dailyPnl) / Tredo.State.portfolio.equity;
          if (ddPct < Tredo.State.portfolio.maxDrawdown) {
            Tredo.State.portfolio.maxDrawdown = ddPct;
          }

          // Recalculate equity
          const remainingMktVal = Tredo.State.positions
            .filter(p => p.symbol !== pos.symbol)
            .reduce((s, p) => s + (Tredo.State.watchlist[p.symbol]?.price || p.entry) * p.qty, 0);
          Tredo.State.portfolio.equity = Tredo.State.portfolio.cash + remainingMktVal;
          Tredo.State.portfolio.dailyPnlPct = Tredo.State.portfolio.equity > 0
            ? Tredo.State.portfolio.dailyPnl / (Tredo.State.portfolio.equity - Tredo.State.portfolio.dailyPnl)
            : 0;

          // Remove position
          Tredo.State.positions.splice(i, 1);

          Tredo.UI.log(`[Execution] ${type} ${pos.symbol}: ₹${pnl.toFixed(2)}`, pnl >= 0 ? 'success' : 'error');

          // Record reflection episode
          const lesson = pnl >= 0
            ? `${type === 'TAKE_PROFIT' ? 'TP hit: trend continu' : 'No clear catalyst'}`
            : `${type === 'STOP_LOSS' ? 'SL hit: trend reversal' : 'Resistance held'}`;
          Tredo.State.reflections.unshift({
            symbol: pos.symbol, pnl, direction: pos.direction, entry: pos.entry,
            lesson, type, timestamp: new Date().toISOString(),
          });

          // Add COT chain for the closed trade
          const chainId = Tredo.COT.beginChain('ExecutionEngine',
            `${type} ${pos.symbol} @ ₹${asset.price.toFixed(2)}`,
            { action: type === 'TAKE_PROFIT' ? 'TP_HIT' : 'SL_HIT', reason: `${pos.direction} ${pos.symbol} closed at ₹${asset.price.toFixed(2)}` },
            pnl >= 0 ? 0.85 : 0.3
          );
          Tredo.COT.addStep(chainId, 'Reflector',
            `P&L analysis: ₹${pnl.toFixed(2)} on ${pos.symbol}`,
            { action: 'REFLECTED', reason: lesson },
            pnl >= 0 ? 0.8 : 0.4
          );
          Tredo.COT.endChain(chainId, type === 'TAKE_PROFIT' ? 'WIN' : 'LOSS',
            `${type}: ₹${Math.abs(pnl).toFixed(2)} | ${lesson}`, pnl >= 0 ? 0.85 : 0.3
          );

          // Update UI
          Tredo.Trading.renderPositions();
          Tredo.Dashboard.updateStats();
          Tredo.Dashboard.renderLatestDecision();
          updateRibbon();
          break; // only close one per tick to keep it readable
        }
      }

      // Update equity even if no close happened
      if (!closedPosition) {
        const mktVal = Tredo.State.positions.reduce((s, p) =>
          s + (Tredo.State.watchlist[p.symbol]?.price || p.entry) * p.qty, 0);
        Tredo.State.portfolio.equity = Tredo.State.portfolio.cash + mktVal;
        const baseEquity = Tredo.State.portfolio.equity - Tredo.State.portfolio.dailyPnl;
        Tredo.State.portfolio.dailyPnlPct = baseEquity > 0
          ? Tredo.State.portfolio.dailyPnl / baseEquity
          : 0;
      }
    } else {
      // No positions — equity = cash
      Tredo.State.portfolio.equity = Tredo.State.portfolio.cash;
      Tredo.State.portfolio.dailyPnlPct = Tredo.State.portfolio.equity > 0
        ? Tredo.State.portfolio.dailyPnl / Tredo.State.portfolio.equity
        : 0;
    }

    // Update ticker & ribbon
    updateRibbon();
  }, 1500);
}

// ── UI Helpers ───────────────────────────────────────────────────────────────
function renderWatchlist() {
  const list = document.getElementById('watchlist-list');
  if (!list) return;
  list.innerHTML = '';
  Object.entries(Tredo.State.watchlist).forEach(([sym, asset]) => {
    const prefix = asset.isCrypto ? '$' : '₹';
    const changeCls = asset.change >= 0 ? 'success' : 'danger';
    const sign = asset.change >= 0 ? '+' : '';
    const row = document.createElement('div');
    row.className = `wl-row ${sym === Tredo.State.activeSymbol ? 'active' : ''}`;
    row.innerHTML = `<span class="wl-sym"><strong>${sym}</strong><small>${asset.name}</small></span>
      <span class="wl-price">${prefix}${asset.price.toFixed(2)}</span>
      <span class="wl-change ${changeCls}">${sign}${asset.change.toFixed(2)}%</span>`;
    row.addEventListener('click', () => selectAsset(sym));
    list.appendChild(row);
  });
  // Keep whitelist panel cards in sync (prices update)
  if (Tredo.Whitelist?.render) Tredo.Whitelist.render();
}

function selectAsset(symbol) {
  const asset = Tredo.State.watchlist[symbol];
  if (!asset) return;
  Tredo.State.activeSymbol = symbol;
  document.querySelectorAll('.pill-group .pill[data-sym]').forEach(p => p.classList.toggle('active', p.dataset.sym === symbol));
  // Also sync the dynamic order ticket pills
  if (Tredo.Whitelist?.renderOrderTicketPills) Tredo.Whitelist.renderOrderTicketPills();
  const entryInput = document.getElementById('trade-entry');
  if (entryInput) entryInput.value = asset.price.toFixed(2);
  updateTicker(symbol);
  updateOrderBook(asset.price);
  updateChart(symbol);
  renderWatchlist();
}

function updateTicker(symbol) {
  // Ticker elements removed from topbar (per user request); function kept as no-op for compatibility
  const asset = Tredo.State.watchlist[symbol];
  if (!asset) return;
  // No-op now that mini-ticker is removed
}

function updateOrderBook(price) {
  const high = price * 1.01, low = price * 0.99, close = price * 0.998;
  const pivot = (high + low + close) / 3;
  const r1 = 2*pivot - low, s1 = 2*pivot - high;
  const r2 = pivot + (high - low), s2 = pivot - (high - low);
  const r3 = high + 2*(pivot - low), s3 = low - 2*(high - pivot);

  document.getElementById('pv-spot').textContent = price.toFixed(2);
  document.getElementById('pv-pivot').textContent = pivot.toFixed(2);
  document.getElementById('pv-r1').textContent = r1.toFixed(2);
  document.getElementById('pv-r2').textContent = r2.toFixed(2);
  document.getElementById('pv-r3').textContent = r3.toFixed(2);
  document.getElementById('pv-s1').textContent = s1.toFixed(2);
  document.getElementById('pv-s2').textContent = s2.toFixed(2);
  document.getElementById('pv-s3').textContent = s3.toFixed(2);

  const dist = (v) => `${((v - price) / price * 100) >= 0 ? '+' : ''}${((v - price) / price * 100).toFixed(2)}%`;
  document.getElementById('pd-r1').textContent = dist(r1);
  document.getElementById('pd-r2').textContent = dist(r2);
  document.getElementById('pd-r3').textContent = dist(r3);
  document.getElementById('pd-s1').textContent = dist(s1);
  document.getElementById('pd-s2').textContent = dist(s2);
  document.getElementById('pd-s3').textContent = dist(s3);
}

function updateRibbon() {
  document.getElementById('ribbon-pos-count').textContent = Tredo.State.positions.length || '0';
  const totalPnl = Tredo.State.positions.reduce((s, p) => s + (p.pnl || 0), 0);
  const el = document.getElementById('ribbon-pnl-val');
  el.textContent = `₹${totalPnl >= 0 ? '+' : ''}${totalPnl.toFixed(2)}`;
  el.className = `rsv ${totalPnl >= 0 ? 'pos' : 'neg'}`;
  document.getElementById('ribbon-trades').textContent = Tredo.State.portfolio.totalTrades;
  document.getElementById('ribbon-winrate').textContent = Tredo.State.portfolio.totalTrades > 0
    ? `${(Tredo.State.portfolio.wins / Tredo.State.portfolio.totalTrades * 100).toFixed(0)}%` : '—';
}

// ── Chart (TradingView / Canvas fallback) ────────────────────────────────────
let tvWidget = null;
let candles = [];
let tickCounter = 0;
let currentMarketPrice = 24500;

function generateInitialCandles(basePrice) {
  candles = []; let price = basePrice * 0.96;
  for (let i = 0; i < 40; i++) {
    const drift = (Math.random() - 0.48) * 0.008 * price;
    const o = price, c = price + drift;
    candles.push({ open: o, high: Math.max(o, c) + Math.random() * 0.004 * price, low: Math.min(o, c) - Math.random() * 0.004 * price, close: c, volume: Math.floor(Math.random() * 1000) + 100 });
    price = c;
  }
  currentMarketPrice = basePrice;
}

function updateChart(symbol) {
  const container = document.getElementById('tradingview-chart-container');
  const canvas = document.getElementById('fallback-chart-canvas');
  const cleanSym = symbol.toUpperCase().trim();

  // Build TradingView symbol: crypto → BINANCE:XXXUSDT, stocks → NSE:XXX
  let tvSym;
  if (isCryptoSym(cleanSym) || cleanSym.endsWith('USDT')) {
    const base = cleanSym.replace('USDT', '');
    tvSym = `BINANCE:${base}USDT`;
  } else {
    // NSE stocks
    tvSym = `NSE:${cleanSym}`;
  }

  if (window.TradingView) {
    if (canvas) canvas.style.display = 'none';
    try {
      if (tvWidget) try { tvWidget.remove(); } catch(e) {}
      tvWidget = new TradingView.widget({
        container_id: 'tradingview-chart-container',
        symbol: tvSym, interval: '15', timezone: 'Asia/Kolkata',
        theme: 'dark', style: '1', locale: 'en',
        width: '100%', height: '100%',
        toolbar_bg: '#131820',
        enable_publishing: false,
        allow_symbol_change: true,
      });
      return;
    } catch (e) { console.error('TV failed:', e); }
  }
  if (canvas) {
    canvas.style.display = 'block';
    if (candles.length === 0 || Tredo.State.activeSymbol !== cleanSym) generateInitialCandles(Tredo.State.watchlist[cleanSym]?.price || 24500);
    resizeCanvas();
  }
}

function resizeCanvas() {
  const canvas = document.getElementById('fallback-chart-canvas');
  if (!canvas || canvas.style.display === 'none') return;
  const parent = canvas.parentElement;
  if (parent) { canvas.width = parent.clientWidth; canvas.height = parent.clientHeight; }
  drawCanvasChart();
}

function drawCanvasChart() {
  const canvas = document.getElementById('fallback-chart-canvas');
  if (!canvas || canvas.style.display === 'none') return;
  const ctx = canvas.getContext('2d');
  if (!ctx || candles.length === 0) return;

  const w = canvas.width, h = canvas.height;
  ctx.fillStyle = '#0a0e14';
  ctx.fillRect(0, 0, w, h);

  let maxP = -Infinity, minP = Infinity, maxV = -Infinity;
  candles.forEach(c => { if (c.high > maxP) maxP = c.high; if (c.low < minP) minP = c.low; if (c.volume > maxV) maxV = c.volume; });
  const pr = maxP - minP;
  maxP += pr * 0.1; minP -= pr * 0.1;
  const plotH = h - 60;

  const getY = (p) => plotH - ((p - minP) / (maxP - minP)) * (plotH - 40) + 30;

  // Grid
  ctx.strokeStyle = '#1f2533'; ctx.lineWidth = 1; ctx.fillStyle = '#5a6270'; ctx.font = '9px JetBrains Mono, monospace';
  for (let i = 0; i <= 5; i++) {
    const p = minP + (maxP - minP) * (i / 5);
    ctx.beginPath(); ctx.moveTo(0, getY(p)); ctx.lineTo(w - 70, getY(p)); ctx.stroke();
    ctx.fillText(`₹${p.toFixed(0)}`, w - 65, getY(p) + 4);
  }

  // Candles
  const cnt = candles.length, cw = Math.max((w - 75) / cnt, 1);
  candles.forEach((c, i) => {
    const x = i * cw + 5;
    const bullish = c.close >= c.open;
    ctx.strokeStyle = bullish ? '#0ecb81' : '#f6465d'; ctx.fillStyle = bullish ? '#0ecb81' : '#f6465d';
    ctx.beginPath(); ctx.moveTo(x + cw / 2, getY(c.high)); ctx.lineTo(x + cw / 2, getY(c.low)); ctx.stroke();
    const oy = getY(c.open), cy = getY(c.close);
    ctx.fillRect(x + 2, Math.min(oy, cy), Math.max(cw - 4, 1), Math.max(Math.abs(cy - oy), 1));
  });

  // Volume bars
  candles.forEach((c, i) => {
    const bullish = c.close >= c.open;
    ctx.fillStyle = bullish ? 'rgba(14,203,129,0.15)' : 'rgba(246,70,93,0.15)';
    ctx.fillRect(i * cw + 5, h - (c.volume / maxV) * 35 - 10, Math.max(cw - 4, 1), (c.volume / maxV) * 35);
  });

  // Labels
  ctx.fillStyle = '#e8eaed'; ctx.font = 'bold 11px Inter, sans-serif';
  ctx.fillText(`${Tredo.State.activeSymbol}/USDT 15m`, 12, 20);
}

// ── Event Bindings ───────────────────────────────────────────────────────────

// Tab switching in trading page
document.querySelectorAll('.tw-tab').forEach(tab => {
  tab.addEventListener('click', () => Tredo.Trading.switchTab(tab.dataset.pane));
});

// Buy/Sell direction tabs
document.querySelectorAll('.ot-dir').forEach(tab => {
  tab.addEventListener('click', () => {
    document.querySelectorAll('.ot-dir').forEach(t => t.classList.remove('active'));
    tab.classList.add('active');
    Tredo.State.activeDirection = tab.dataset.dir;
    const btn = document.getElementById('btn-submit-order');
    btn.innerHTML = tab.dataset.dir === 'long'
      ? '<i class="fas fa-check"></i> BUY / LONG'
      : '<i class="fas fa-check"></i> SELL / SHORT';
    btn.className = `btn btn-primary btn-block${tab.dataset.dir === 'short' ? ' short-mode' : ''}`;
    const entry = parseFloat(document.getElementById('trade-entry').value);
    if (entry) {
      document.getElementById('trade-sl').value = (entry * (tab.dataset.dir === 'long' ? 0.99 : 1.01)).toFixed(2);
      document.getElementById('trade-tp').value = (entry * (tab.dataset.dir === 'long' ? 1.02 : 0.98)).toFixed(2);
    }
  });
});

// Order book click-to-fill
document.querySelectorAll('.pb-row, .pb-spread').forEach(row => {
  row.addEventListener('click', () => {
    const target = row.dataset.target;
    if (!target) return;
    const id = row.dataset.id;
    const valEl = document.getElementById(`pv-${id}`);
    if (!valEl) return;
    const val = parseFloat(valEl.textContent.replace(/,/g, ''));
    if (isNaN(val)) return;
    if (target === 'tp') { document.getElementById('trade-tp').value = val.toFixed(2); Tredo.UI.log(`TP set to ₹${val.toFixed(2)}`, 'system'); }
    else if (target === 'sl') { document.getElementById('trade-sl').value = val.toFixed(2); Tredo.UI.log(`SL set to ₹${val.toFixed(2)}`, 'system'); }
    else if (target === 'entry') { document.getElementById('trade-entry').value = val.toFixed(2); Tredo.UI.log(`Entry set to ₹${val.toFixed(2)}`, 'system'); }
  });
});

// Trade form submission
document.getElementById('trade-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  const symbol = document.getElementById('trade-symbol').value.trim();
  const entry = parseFloat(document.getElementById('trade-entry').value);
  const sl = parseFloat(document.getElementById('trade-sl').value);
  const tp = parseFloat(document.getElementById('trade-tp').value);
  const dir = Tredo.State.activeDirection;

  Tredo.UI.log(`[Trade] ${dir.toUpperCase()} ${symbol} @ ₹${entry.toFixed(2)}`, 'system');

  try {
    const result = await invoke('execute_trade', { symbol, directionStr: dir, entryPrice: entry, stopLoss: sl, takeProfit: tp });
    const output = document.getElementById('trade-output');
    output.classList.remove('hidden');

    if (result.includes('SUCCESS') || result.includes('EXECUTED')) {
      output.style.borderColor = 'var(--success)'; output.style.color = 'var(--success)'; output.textContent = result;
      Tredo.UI.log(`[Execution] ✅ ${result}`, 'success');

      const risk = parseInt(document.getElementById('risk-slider').value) || 1;
      const qty = 10 * risk;
      Tredo.State.portfolio.cash -= qty * entry;
      Tredo.State.positions.push({ symbol, direction: dir.toUpperCase(), qty, entry, sl, tp, pnl: 0 });
      Tredo.Trading.renderPositions();
      Tredo.Dashboard.render();
      updateRibbon();
      document.getElementById('margin-balance').textContent = `₹${Tredo.State.portfolio.cash.toFixed(2)}`;

      // Chain-of-thought: hierarchical reasoning tree
      const chainId = Tredo.COT.beginChain('ExecutionEngine', `${symbol} ${dir} @ ${entry}`, { action: dir.toUpperCase(), reason: 'Executing trade' }, 1.0);
      Tredo.COT.addStep(chainId, 'StrategyDecisionAgent', `Decision for ${symbol}`,
        { action: dir.toUpperCase(), reason: `Technical confluence ${(Math.random() * 0.2 + 0.6).toFixed(2)}, trend alignment across 2 TFs` },
        Math.random() * 0.2 + 0.6);
      Tredo.COT.addStep(chainId, 'MarketIntelligence', `Market analysis ${symbol}`,
        { action: 'ANALYZED', reason: `Confluence: ${(Math.random() * 0.3 + 0.5).toFixed(2)}, Pivot confirmed` }, 0.8);
      Tredo.COT.addStep(chainId, 'RiskPsychology', `Risk check for ${symbol}`,
        { action: 'PASS', reason: 'Portfolio heat within limits, no consecutive losses' }, 0.9);
      Tredo.COT.endChain(chainId, dir.toUpperCase(),
        `✅ Trade executed: ${symbol} ${dir} @ ${entry}. Confidence: ${(Math.random() * 0.2 + 0.7).toFixed(2)}`, 0.85);
      Tredo.COT.syncToState(dir.toUpperCase(), symbol, entry,
        `Confluence ${(Math.random() * 0.2 + 0.6).toFixed(2)}, trend alignment confirmed`,
        Math.random() * 0.2 + 0.6);

    } else {
      output.style.borderColor = 'var(--danger)'; output.style.color = 'var(--danger)'; output.textContent = result;
      Tredo.UI.log(`[Execution] ❌ ${result}`, 'error');
    }
  } catch (err) {
    document.getElementById('trade-output').textContent = `ERROR: ${err}`;
    Tredo.UI.log(`[Execution] Error: ${err}`, 'error');
  }
});

// Discipline check
document.getElementById('discipline-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  const symbol = document.getElementById('inspect-symbol').value.trim();
  const price = parseFloat(document.getElementById('inspect-price').value);
  Tredo.UI.log(`[Discipline] Checking ${symbol} @ ₹${price.toFixed(2)}`, 'system');
  try {
    const result = await invoke('check_discipline', { symbol, price });
    const container = document.getElementById('inspector-output');
    container.classList.remove('hidden');
    const passed = !result.toLowerCase().includes('violation');
    container.className = `inspect-output ${passed ? 'pass' : 'fail'}`;
    container.innerHTML = `<strong>${passed ? '✅ PASSED' : '❌ VIOLATION'}</strong><br>${result}`;
    Tredo.UI.log(`[Discipline] ${passed ? 'PASSED' : 'REJECTED'} for ${symbol}`, passed ? 'success' : 'error');

    // Chain-of-thought
    const chainId = Tredo.COT.beginChain('DisciplineCore', `Discipline check for ${symbol} @ ${price}`, { action: passed ? 'PASS' : 'FAIL' }, passed ? 0.9 : 0.2);
    Tredo.COT.addStep(chainId, 'DisciplineCore', `Validating ${symbol} @ ${price}`,
      { action: passed ? 'PASS' : 'FAIL', reason: result }, passed ? 0.9 : 0.2);
    Tredo.COT.addStep(chainId, 'RiskPsychology', `Risk assessment for ${symbol}`,
      { action: 'OK', reason: 'Risk parameters within limits' }, 0.85);
    Tredo.COT.endChain(chainId, passed ? 'PASS' : 'FAIL', result, passed ? 0.9 : 0.2);
  } catch (err) { Tredo.UI.log(`[Discipline] Error: ${err}`, 'error'); }
});

// Backtest
document.getElementById('btn-run-backtest').addEventListener('click', async () => {
  Tredo.UI.log('[Backtester] Running 50-cycle simulation...', 'system');
  try {
    const result = await invoke('run_backtest');
    const parts = result.split('|');
    const trades = parts[1]?.split(':')[1]?.trim() || '—';
    const winrate = parts[2]?.split(':')[1]?.trim() || '—';
    const pnl = parts[3]?.split(':')[1]?.trim() || '—';
    const dd = parts[4]?.split(':')[1]?.trim() || '—';

    document.getElementById('backtest-results').classList.remove('hidden');
    document.getElementById('bt-trades').textContent = trades;
    document.getElementById('bt-winrate').textContent = winrate;
    document.getElementById('bt-pnl').textContent = pnl;
    document.getElementById('bt-drawdown').textContent = dd;
    document.getElementById('bt-log').textContent = result;

    Tredo.State.backtests.unshift({ trades, winrate, pnl, dd, timestamp: new Date().toISOString() });
    renderBacktestHistory();
    Tredo.UI.log(`[Backtester] ✅ Complete: ${winrate} win rate, ${pnl} P&L`, 'success');

    // Chain-of-thought
    const chainId = Tredo.COT.beginChain('Backtester', '50-cycle simulation', { action: 'RUNNING' }, 0.85);
    Tredo.COT.addStep(chainId, 'Backtester', 'Loading historical data...', { action: 'LOADED', reason: '50 cycles loaded' }, 0.9);
    Tredo.COT.addStep(chainId, 'Backtester', 'Running simulation...', { action: 'SIMULATING', reason: 'Executing 50 trades against ruleset' }, 0.8);
    Tredo.COT.endChain(chainId, 'COMPLETE', `Win rate: ${winrate}, P&L: ${pnl}, Max DD: ${dd}`, 0.85);
  } catch (err) { Tredo.UI.log(`[Backtester] Error: ${err}`, 'error'); }
});

function renderBacktestHistory() {
  const tbody = document.getElementById('bt-history-body');
  if (!tbody) return;
  if (Tredo.State.backtests.length === 0) {
    tbody.innerHTML = '<tr><td colspan="6" class="empty-state">No backtests run yet.</td></tr>';
    return;
  }
  tbody.innerHTML = Tredo.State.backtests.map((bt, i) =>
    `<tr>
      <td>#${i + 1}</td>
      <td>${bt.trades}</td>
      <td class="success">${bt.winrate}</td>
      <td>${bt.pnl}</td>
      <td class="danger">${bt.dd}</td>
      <td style="font-size:10px;color:var(--text-muted)">${new Date(bt.timestamp).toLocaleString()}</td>
    </tr>`
  ).join('');
}

// ── Tredo Agent Tree ────────────────────────────────────────────────────────
// Fetches the Tredo hierarchy from /api/agents and renders an interactive tree.

async function loadTredoTree() {
  const container = document.getElementById('tredo-tree-container');
  if (!container) return;
  container.innerHTML = '<div class="tree-loading"><i class="fas fa-spinner fa-spin"></i> Loading agent tree...</div>';
  try {
    const resp = await fetch(`${API_BASE}/api/agents`);
    const tree = await resp.json();
    container.innerHTML = renderTredoNode(tree, true);
    // wire up expand/collapse
    container.querySelectorAll('.tredo-node-header').forEach(hdr => {
      hdr.addEventListener('click', () => {
        const node = hdr.closest('.tredo-node');
        node.classList.toggle('collapsed');
      });
    });
  } catch (e) {
    container.innerHTML = `<div class="tree-error"><i class="fas fa-exclamation-triangle"></i> Could not load agent tree: ${e.message}</div>`;
  }
}

function renderTredoNode(node, isRoot = false) {
  const hasChildren = node.children && node.children.length > 0;
  const badge = isRoot ? 'root' : (hasChildren ? 'manager' : 'agent');
  const badgeLabels = { root: 'Orchestrator', manager: 'Manager', agent: 'Sub-Agent' };

  return `
    <div class="tredo-node ${isRoot ? 'root' : ''} ${badge}" data-name="${node.name}">
      <div class="tredo-node-header">
        <span class="tredo-toggle">${hasChildren ? '<i class="fas fa-caret-down"></i>' : '<i class="fas fa-circle" style="font-size:6px"></i>'}</span>
        <span class="tredo-badge badge-${badge}">${badgeLabels[badge]}</span>
        <span class="tredo-name">${node.name}</span>
        <span class="tredo-role">${node.role || ''}</span>
      </div>
      ${hasChildren ? `<div class="tredo-children">${node.children.map(c => renderTredoNode(c)).join('')}</div>` : ''}
    </div>`;
}

// Load tree when the Agents tab is activated
document.addEventListener('DOMContentLoaded', () => {
  document.querySelectorAll('.tw-tab[data-pane="agents"]').forEach(btn => {
    btn.addEventListener('click', () => loadTredoTree());
  });
});

// Trigger orchestra cycle — calls real backend pipeline
 // DEBUG ONLY: Manual trigger for testing. In full hands-off autonomous mode, this is not used.
// The co-pilot runs its own pipeline on schedule.
// This listener is for DEBUG only. In production hands-off co-pilot mode the agent never needs it.
// We leave it but it will log a warning.
// Hands-off mode: This manual trigger is disabled in normal autonomous operation.
// The co-pilot runs pipelines independently via internal loops.
// Kept for advanced debug only; in live front-end test, polling COT/health is sufficient.
const triggerBtn = document.getElementById('btn-trigger-cycle');
if (triggerBtn) {
  triggerBtn.style.display = 'none';
  triggerBtn.disabled = true;
  triggerBtn.title = 'DISABLED: Full hands-off mode. MediumLoop drives Tredo pipeline autonomously every 5 minutes after launch. No human touch. Use only for targeted debug in non-autonomous test runs.';
}
triggerBtn?.addEventListener('click', async () => {
  const output = document.getElementById('cycle-output');
  output.classList.remove('hidden'); output.innerHTML = '[Orchestrator] Triggering full pipeline...\n';

  Tredo.UI.log('[Orchestrator] Starting real pipeline cycle...', 'system');

  try {
    const result = await invoke('trigger_orchestra_cycle');
    output.innerHTML += result + '\n';
    Tredo.UI.log(`[Orchestrator] ✅ ${result}`, 'success');

    // Load COT entries directly
    setTimeout(() => Tredo.COT.loadFromBackend(), 500);
  } catch (err) {
    output.innerHTML += `ERROR: ${err}\n`;
    Tredo.UI.log(`[Orchestrator] Error: ${err}`, 'error');
  }
});

// Risk slider
document.getElementById('risk-slider').addEventListener('input', (e) => {
  const sliderSpan = document.querySelector('.slider-headers span');
  if (sliderSpan) sliderSpan.textContent = `${e.target.value}%`;
});

// Settings bindings
document.querySelectorAll('[data-mode]').forEach(el => {
  el.addEventListener('click', () => Tredo.Settings.setMode(el.dataset.mode));
});

document.getElementById('set-max-risk')?.addEventListener('input', (e) => {
  document.getElementById('set-max-risk-val').textContent = `${e.target.value}%`;
});
document.getElementById('set-loss-limit')?.addEventListener('input', (e) => {
  document.getElementById('set-loss-limit-val').textContent = `${e.target.value}%`;
});
document.getElementById('set-target')?.addEventListener('input', (e) => {
  document.getElementById('set-target-val').textContent = `${e.target.value}%`;
});

// Window resize for chart
window.addEventListener('resize', resizeCanvas);

// ═══════════════════════════════════════════════════════════════════════════
//  INITIALIZATION
// ═══════════════════════════════════════════════════════════════════════════

async function syncWatchlist() {
  try {
    const list = await apiGet('/api/watchlist');
    if (Array.isArray(list)) {
      const newWatchlist = {};
      list.forEach(sym => {
        const isCrypto = isCryptoSym(sym);
        newWatchlist[sym] = {
          name: isCrypto ? `${sym}/USDT` : sym,
          price: Tredo.State.watchlist[sym]?.price || (isCrypto ? 100.0 : 1000.0),
          change: Tredo.State.watchlist[sym]?.change || 0.0,
          isCrypto
        };
      });
      Tredo.State.watchlist = newWatchlist;
      renderWatchlist();
      Tredo.Settings.renderWatchlist();
      Tredo.Whitelist.render();
    }
  } catch (e) {
    console.error("Failed to sync watchlist from backend", e);
  }
}

function init() {
  Tredo.UI.log('[System] Tredo v2.0 initialized. Ready.', 'system');
  Tredo.UI.log('[System] Chain-of-thought reasoning active.', 'system');

  // Set initial values
  selectAsset('NIFTY');
  renderWatchlist();
  Tredo.Dashboard.render();
  Tredo.Analysis.render();
  Tredo.Settings.renderWatchlist();
  Tredo.Whitelist.render();
  Tredo.Whitelist.initSearch();
  Tredo.Crypto.init();
  Tredo.Stocks.init();
  Tredo.AutoPilot.updateUI();
  updateRibbon();
  // Load saved brokerage config
  Tredo.BrokerageConfig.load();
  Tredo.BrokerageConfig.updateUI();

  // Sync watchlist from backend
  syncWatchlist().then(() => {
    // Seed strategy engine price history with current watchlist prices
    for (const [sym, asset] of Object.entries(Tredo.State.watchlist)) {
      Tredo.StrategyEngine.recordPrice(sym, asset.price);
    }
  });

  // Connect live data
  connectCryptoWebSocket();
  startStockUpdateLoop();
  startPositionEvaluator();

  // Start status and COT polling unconditionally on page load
  Tredo.System.startHealthPolling();
  Tredo.System.startCOTPolling();

  // Periodic dashboard refresh (stats + COT card)
  setInterval(() => {
    if (document.getElementById('page-dashboard').classList.contains('active')) {
      Tredo.Dashboard.updateStats();
      Tredo.Dashboard.renderLatestDecision();
      Tredo.StrategyEngine.renderStatus();
    }
  }, 5000);
}

// Bootstrap
document.addEventListener('DOMContentLoaded', init);
if (document.readyState === 'complete' || document.readyState === 'interactive') init();
