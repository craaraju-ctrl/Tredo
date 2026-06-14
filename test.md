# tredo Frontend Test Runbook (Chrome Browser) — Full Features + Agent-Driven Trades & Debug

**Focus of this version:**  
Test **everything in the Chrome browser** using the rich vanilla JS frontend served by the orchestrator.  

**Core testing philosophy:**  
1. Start the orchestrator (the "tredo agent" backend).  
2. Open Chrome to the served UI.  
3. **Let the tredo agent do the work** — click RUN SYSTEM and watch the autonomous loops (fast/medium/slow) drive real paper trades using the full hierarchy, debate, skills, trained memory, and disciplined rules.  
4. Use the browser as your primary debugging and observability surface to inspect, trigger, and verify every feature — especially the new **Strong Skills + Rules + Trained Memory** layer.

The browser UI (served at the orchestrator's root) is a full-featured trading desk + agent observability console with:
- Dashboard with live edge scores, debate summaries, risk/guardian views, live COT snippets
- Trading desk (Binance-style) with **AI Edge Pre-Trade Analysis** (confluence + debate + kronos + guardian + **memory** + risk)
- Rich "Agent" page showing the entire Tredo workflow in boxes (Identifier → Verifier → Executer + full Multi-Agent Debate with Historian memory) + live COT tree
- Analysis, Crypto, Stocks pages with live data + actions
- Direct controls to trigger the agent, place AI-guided orders, etc.

All actions go through the real backend (paper only). No mocks.

---

## 1. Prerequisites & Services (same as before)

Make sure these are running before launching the agent:

```bash
# Ollama
ollama serve &
ollama pull ministral:3b   # or your configured model

# Kronos (port 8000)
cd kronos_service
python3 -m pip install -r requirements.txt
uvicorn main:app --host 0.0.0.0 --port 8000
# Verify
curl -s http://localhost:8000/health
```

(Optional) agentmemory on 3111 for long-term trained lessons.

---

## 2. One-Time Setup (recommended)

```bash
cd Desktop/TREDO
./tredo setup
source config/tredo.env
```

**Important for browser testing:**
- Choose **WS + Web API port 8082** during wizard (or manually set `WEB_API_ADDR=0.0.0.0:8082` and `WS_PORT=8082` in `config/tredo.env`).
- `PAPER_MODE=true` must be present.

---

## 3. Launch the tredo Agent + Browser Frontend (the key step)

This single command starts **everything** the agent needs:

- The full autonomous backend (temporal loops, debate, skills execution, trained memory recall, disciplined rules with memory adjustments, paper execution)
- The web server that serves the complete frontend SPA in Chrome
- Real-time WS updates for live COT, prices, positions, etc.

```bash
# Recommended: force the port the frontend expects
PORT=8082 ./tredo orchestrator
```

Or manually:
```bash
source config/tredo.env
PORT=8082 cargo run -p tredo-orchestrator
```

**What happens on launch:**
- System initializes watchlist and data feeds.
- **Autonomous mode activates automatically** (fast loop for prices/SLTP, medium loop for full pipeline + debate + skills + trained memory, slow for reflection/meta).
- HTTP server starts on the chosen port (8082 recommended) and serves the frontend at the root.
- You will see logs like:
  - Skills being executed
  - Hierarchical trained memory recall
  - Debate participants using memory + skills
  - `[Rules + TrainedMemory] Tightened...` when applicable
  - Paper trades when the agent decides

**Leave this terminal running.** The tredo agent is now actively trading (paper) and thinking.

---

## 4. Open in Chrome Browser — Your Main Testing & Debug Surface

1. Open **Google Chrome**.
2. Go to: `http://localhost:8082/`  
   (or the port shown in the orchestrator logs if different)
3. The full tredo SPA should load (dark professional trading interface with TradingView chart, multi-page navigation).

**Top controls you will use constantly:**
- **RUN SYSTEM** button (top right) — toggles the autonomous loops on/off. It should already be active after orchestrator start.
- Paper badge (always visible).
- Top nav buttons: **Dashboard | Trading | Agent | Analysis | Crypto | Stocks | Settings**

---

## 5. Let the tredo Agent Do the Trades (Primary Test Mode)

Do **not** manually trade at first.

1. In the browser, ensure **RUN SYSTEM** shows as running (green/active).
2. The agent (medium loop every ~5 minutes, or faster if you trigger) will:
   - Run Identifier group (MarketIntelligence + skills: sentiment, volatility, regime, patterns, confluence scorer)
   - Run debate (Proposer/Critic/Risk/Historian using trained memory + skills)
   - Apply Disciplined Core + `apply_trained_memory_to_rules`
   - Only take paper trades when everything passes
3. Watch **live** in the browser as the agent works and occasionally executes trades.

**Accelerate the agent (very useful for testing):**
- Go to **Trading** page.
- Click the **"Trigger Full Agent Cycle"** button (bottom of the order ticket area).
- This forces a complete pipeline run for the active symbol (debate + skills + memory + rules + possible paper trade).
- Repeat for different symbols.

**Observe the agent "doing trades":**
- Dashboard → Recent Trades table updates.
- Positions appear in Trading page bottom tabs.
- Equity / P&L cards change in real time.
- "AI Edge" numbers and debate summaries update.

---

## 6. Debug All Features in Chrome — Page by Page

### Dashboard Page (Overview + Live Intelligence)
- **Portfolio cards**: Equity, cash, positions, win rate (live from backend).
- **Risk & Discipline box**: Drawdown, consecutive losses, portfolio heat, Guardian rules (1%/3% etc.).
- **Live Edge Score + Debate Summary**: Shows confluence / debate / kronos / discipline + latest debate action. This is your first view into skills + memory influence.
- **Recent Trades + Lessons + Live COT**: Real recent paper trades + the very latest AI decision reasoning.
- **System Health**: Kronos, Ollama, Orchestrator, WS, Vector Memory episode count (grows as trained memory accumulates).
- **Actions**: Use "Refresh Edge Data" and switch to Agent page for full trace.

**Debug tip for new layer**: After a cycle, the Live COT card and edge breakdown should reflect memory usage and rule adjustments.

### Trading Page (Binance-style Desk + Powerful AI Pre-Trade)
This is excellent for controlled debugging while the agent runs.

- Symbol selector + timeframes + live price.
- Order book simulation (left).
- Chart area (center) with Kronos / Pivots / Regime overlays.
- **Right panel — AI Edge Pre-Trade Analysis** (this is gold for our new architecture):
  - Confluence, Debate, Kronos, **Memory** (the trained recall score), Guardian, Risk.
  - Recommendation text that incorporates the above.
  - Buttons: **Apply AI Reco**, "Open Debate", "Guardian Check".
- Big **BUY / LONG — EXECUTE WITH AI** (and sell) buttons — these go through the full backend discipline + paper engine.
- Bottom tabs: Positions (with live P&L and close buttons), Open Orders, Trade History, Recent Trades.
- "Trigger Full Agent Cycle" button — use this heavily to let the agent decide.

**Let the agent trade + debug**:
- Leave it running.
- Manually trigger cycles.
- When the agent posts a high-edge recommendation, click **Apply AI Reco** then execute — or just watch autonomous fills.
- After trades, check Positions tab for live updates.

### Agent Page (The Best Page for Debugging the Full Tredo Brain + New Layer)
This page is literally built to show the architecture we implemented.

- **Tredo Main Orchestrator & Temporal Loops** — Fast/Medium/Slow status.
- **Identifier Group** — Lists WatchlistScanner, MarketIntelligence (explicitly mentions "skills (sentiment, volatility, regime...)"), Pivot, ConfluenceScorer, Patterns, Session/RedFolder.
- **Verifier Group** — RiskPsychology, RiskCalculator, Reflector, **Disciplined Core Checks**.
- **Executer Group + Debate** — StrategyDecision (notes "Debate mode"), Portfolio/Execution, and a dedicated **Multi-Agent Debate** box:
  - Proposer (skills bias)
  - Critic (correlation / memory of past)
  - Risk (vol/expansion + memory)
  - **Historian** (Memory match + regret scoring) ← direct evidence of trained memory
- **Guardian Group** — Drawdown, Overtrading, OutcomeLogger (episodes + regret).
- **Reflection, Memory & Meta-Control** — Reflector, MetaControl (rule changes from high-regret), Episode Store + Vector Memory, Live Workflow COT.
- **Complete Live Chain-of-Thought (Full Workflow Trace)** — Expandable tree of every step. This is where you will see the raw `StrongRules+Skills+TrainedMemory` entries, recall blocks, debate turns with memory snippets, rule adjustments, skill executions, etc.

**How to debug here**:
- Click around pages to trigger data.
- Use the refresh buttons.
- Trigger cycles from Trading page → come back here and watch the COT tree and debate boxes populate with real agent reasoning that includes trained memory and skills.
- The "Live Workflow COT" mini box and the big timeline show the self-understanding in action.

### Analysis Page
- Multi-timeframe + regime
- Candlestick patterns + MTF confirmation
- Kronos forecast deep dive
- News sentiment + calendar
- Episode Memory + Debate Breakdown (links memory and past regret directly)

### Crypto & Stocks Pages
- Live multi-exchange prices (Binance, Coinbase, Kraken, CoinGecko for crypto; NSE/BSE/NASDAQ/NYSE for stocks).
- Filters, search, detail panels with exchange comparison and spark charts.
- **"Add to Watchlist"** buttons — adds symbols that the autonomous agent will then start analyzing and potentially trading.
- Test by adding a symbol, then triggering agent cycles and watching it appear in Dashboard/Trading/Agent views.

### Settings / Other
- Model switching, rule tweaks (if exposed), backtester launch, etc.
- Use the backtest button (often available) to run simulations that also push COT.

---

## 7. Specific Verification Steps for Strong Skills + Rules + Trained Memory in the Browser

After the agent has run at least a few cycles (use "Trigger Full Agent Cycle" repeatedly on BTC, NIFTY, etc.):

1. Go to **Agent** page → look in the Debate box for Historian mentioning memory/regret and in the big COT timeline for nodes containing "TrainedMemory", "recall", "regret", skill names.
2. Go to **Trading** page → open the AI Pre-Trade Analysis section. You should see a **Memory** metric (e.g. 0.82) and recommendations that reference debate + memory.
3. Dashboard "Live Edge Score + Debate Summary" should show debate scores influenced by the Historian memory component.
4. After a period of poor outcomes (or manually close losing paper trades), watch for evidence of rule tightening in future recommendations or COT (tighter risk, higher confluence requirement).
5. Vector Memory count in Dashboard health should slowly increase.
6. In COT / Agent timeline, look for explicit tags like `StrongRules+Skills+TrainedMemory`, "Used skill X", "Trained recall used".

All of this should appear **while the agent is autonomously deciding and executing paper trades**.

---

## 8. Additional Powerful Debug Actions in the Browser

- **Manual AI-guided trade**: In Trading desk, adjust the AI Pre-Trade numbers if needed, click "Apply AI Reco", then execute. The backend will still run full `validate_trade_setup` (with any memory-adjusted rules).
- **Add symbols to watchlist** (Crypto or Stocks pages) → the agent will start including them in future medium loops.
- **Toggle RUN SYSTEM** off/on to pause/resume the autonomous agent.
- Refresh COT / Edge data frequently.
- Use the chart overlays (Kronos, Pivots, Regime) while the agent is thinking.
- Watch the bottom Positions tab update live when the agent takes or exits paper trades.

---

## 9. Full End-to-End Flow to Validate (Browser + Agent)

1. Orchestrator running on 8082.
2. Chrome open to `http://localhost:8082/`.
3. RUN SYSTEM active.
4. Trigger 3–5 agent cycles on different symbols (or just wait 5–15 min).
5. Agent should produce debate in the UI, possibly take 1+ paper trades.
6. Inspect:
   - Trading desk AI Pre-Trade (memory visible)
   - Agent page full workflow + COT tree (skills + trained memory + rule application)
   - Positions updating
   - Health / vector memory count growing
7. Optionally close a position manually and trigger more cycles → watch reflection/meta eventually appear in COT.
8. Try adding a new symbol via Crypto page → agent starts trading it.

---

## 10. Common Issues & Fixes (Browser View)

- UI not loading / empty data → orchestrator not running or wrong port. Use exactly `PORT=8082 ./tredo orchestrator`.
- No COT / no debate visible → agent hasn't run a medium cycle yet. Use the "Trigger Full Agent Cycle" button.
- Services down (Kronos/Ollama) → health shows red, agent falls back gracefully (Neutral, simpler decisions). Still usable.
- No trades happening → normal. The agent is disciplined. Look at COT for "HOLD" reasons (low confluence, risk, debate block, memory caution).
- WS not live → health indicator will show; the UI still polls.

---

## 11. Quick One-Command Launch + Browser Test Sequence

```bash
source config/tredo.env
PORT=8082 ./tredo orchestrator > /tmp/agent.log 2>&1 &
sleep 12
open http://localhost:8082/   # macOS; on Linux use xdg-open or just type the URL
```

Then in Chrome:
- Confirm RUN SYSTEM is active.
- Go to Trading → click "Trigger Full Agent Cycle" a few times.
- Immediately switch to Agent page and refresh COT.
- Watch for the skills + trained memory + debate + rules evidence while the agent potentially opens paper positions.

---

This rewritten runbook makes the **Chrome browser frontend the central testing and debugging environment**, while explicitly instructing you to **let the tredo autonomous agent do the heavy lifting** (trades, debate, memory recall, rule adaptation) and use the beautiful UI to observe and interact with every layer — especially the new Strong Skills + Rules + Trained Memory system.

All data and actions are real (paper). Enjoy watching the agent think and trade. 

Report any UI bugs, missing data in the COT/agent views, or cases where the new memory/skills/rules layer is not visible in the browser.