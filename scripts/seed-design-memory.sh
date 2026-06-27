#!/usr/bin/env bash
# seed-design-memory.sh
# Seeds the Tredo Exchange Design System into agentmemory.
# Run this after starting agentmemory on port 3111.
#
# Usage: bash scripts/seed-design-memory.sh

set -euo pipefail

BASE_URL="${MEMORY_API_URL:-http://localhost:3111}"
SKILL_FILE="docs/skills/tredo-exchange-design.md"

cd "$(dirname "$0")/.."

if [ ! -f "$SKILL_FILE" ]; then
  echo "❌ Skill file not found: $SKILL_FILE"
  echo "   Run from the Tredo project root."
  exit 1
fi

echo "🧠 Seeding Tredo Exchange Design System into agentmemory..."
echo "   API: $BASE_URL"
echo ""

# ── Helper: POST a record ──────────────────────────────────────
post_record() {
  local id="$1"
  local content="$2"
  local content_type="$3"
  local tier="${4:-semantic}"
  local importance="${5:-0.6}"

  curl -s -o /dev/null -w "%{http_code}" \
    -X POST "$BASE_URL/records" \
    -H "Content-Type: application/json" \
    -d "{
      \"id\": \"$id\",
      \"content\": $(echo "$content" | jq -Rs .),
      \"content_type\": \"$content_type\",
      \"metadata\": {\"source\": \"tredo-design-system\", \"tier\": \"$tier\"},
      \"tier\": \"$tier\",
      \"importance\": $importance
    }"
}

# ── Seed Design System Tokens ───────────────────────────────────

echo "  1/6  Brand & colors ..."
CODE=$(post_record \
  "design-color-brand" \
  "Tredo Exchange Design System — Brand & Colors:
- Tredo Yellow (primary): #FCD535 — single brand color for CTAs, headlines, wordmark
- Tredo Yellow Active (pressed): #f0b90b
- Tredo Yellow Disabled: #3a3a1f
- Canvas Dark (page floor): #0b0e11 — near-black with warm tint
- Surface Card: #1e2329 — cards, nav dropdowns, markets table
- Surface Elevated: #2b3139 — nested cards, chart panels
- Canvas Light (transactional): #ffffff — buy/deposit pages
- Hairline on Light: #eaecef
- Hairline on Dark: #2b3139
- Ink (strong text on light): #181a20
- Body on Dark: #eaecef
- Muted (footer links, captions): #707a8a
- On Primary (text on yellow): #181a20" \
  "design-color-brand" "semantic" 0.8
)
echo "     → HTTP $CODE"

echo "  2/6  Trading semantics ..."
CODE=$(post_record \
  "design-trading-semantics" \
  "Tredo Exchange Design System — Trading Semantics:
- Trading Up (price increase): #0ecb81 — green, text color only, never button background
- Trading Down (price decrease): #f6465d — red, text color only, never button background
- Info / Focus Ring: #3b82f6 — input focus indicator
- Trading green/red are SEMANTIC price tokens — do NOT repurpose for success/error states" \
  "design-trading-semantics" "semantic" 0.7
)
echo "     → HTTP $CODE"

echo "  3/6  Typography ..."
CODE=$(post_record \
  "design-typography" \
  "Tredo Exchange Design System — Typography:
Font stack: Inter (sans-serif) for body/copy, JetBrains Mono (monospace) for numbers/prices.
Hierarchy:
- Hero Display: 64px / 700 / 1.1 — homepage h1
- Display LG: 48px / 700 / 1.1 — brand headlines
- Display MD: 40px / 600 / 1.15 — section heads
- Display SM: 32px / 600 / 1.2 — CTA band headlines
- Title LG: 24px / 600 / 1.3 — sub-section titles
- Title MD: 20px / 600 / 1.35 — feature card titles
- Title SM: 16px / 600 / 1.4 — trust badges
- Number Display: 40px / 700 / 1.1 — big stat numbers (monospace)
- Number MD: 16px / 500 / 1.4 — table prices (monospace)
- Number SM: 14px / 500 / 1.4 — inline prices (monospace)
- Body MD: 14px / 400 / 1.5 — default text
- Body SM: 13px / 400 / 1.5 — footer text
- Button: 14px / 600 / 1 — CTA labels
- Nav Link: 14px / 500 / 1.4 — top nav items
Numbers ALWAYS use monospace. Body ALWAYS uses sans-serif." \
  "design-typography" "semantic" 0.75
)
echo "     → HTTP $CODE"

echo "  4/6  Layout & spacing ..."
CODE=$(post_record \
  "design-layout" \
  "Tredo Exchange Design System — Layout:
- Max content width: 1280px (marketing) / 1440px (product/trading)
- Spacing base: 4px multiples
- Section padding: 80px
- Card internal padding: 24px (content cards) / 32px (promo cards)
- Grid: 12-column editorial / 8+4 product split
- Markets table: 5 columns (Pair / Last Price / 24h Change / 24h Volume / Action)
- Footer: 6-column link list" \
  "design-layout" "semantic" 0.6
)
echo "     → HTTP $CODE"

echo "  5/6  Components ..."
CODE=$(post_record \
  "design-components" \
  "Tredo Exchange Design System — Components:
- Primary Button: Yellow (#FCD535) bg, Black (#181a20) text, 12px×24px padding, 40px height, 6px radius
- Primary Pill Button: Larger pill, 14px×32px, 9999px radius, for top-of-page actions
- Secondary Button (dark): Surface card bg, white text, 6px radius
- Trading Up Button: Solid green (#0ecb81) for Buy/Long, 4px radius, 8px×20px padding
- Trading Down Button: Solid red (#f6465d) for Sell/Short
- Markets Table Card: Surface card bg, 12px radius, tab row + coin table
- Stat Callout: Transparent bg, yellow text, number display size in monospace
- Order Book: 3-column depth rows (Price / Size / Total), red asks / green bids, depth bars at 18% opacity" \
  "design-components" "semantic" 0.7
)
echo "     → HTTP $CODE"

echo "  6/6  Trading desk ..."
CODE=$(post_record \
  "design-trading-desk" \
  "Tredo Exchange Design System — Trading Desk:
- 3-column layout: Order Book (left) | Chart (center) | Trade Form + AI Pre-Trade (right)
- Symbol bar: selector, price, 24h change, timeframe buttons (1m/5m/15m/1H/4H/1D), AI edge pill
- Side Switch: Buy/Long (green) / Sell/Short (red) toggle
- Order Form: Price, Amount, % balance quick buttons (25/50/75/100), Total, Available balance
- AI Pre-Trade: Edge score, confluence, debate, kronos, guardian, memory, risk metrics
- Bottom tabs: Positions / Open Orders / Trade History / Recent Trades" \
  "design-trading-desk" "semantic" 0.65
)
echo "     → HTTP $CODE"

echo ""
echo "✅ Design system seeded into agentmemory successfully!"
echo "   → Search: tantra search 'tredo design'"
echo "   → Recall: AgentMemoryClient.recall('tredo design system colors')"
