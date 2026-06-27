# Binance Crypto Exchange Design System

## Overview
Binance reads like a financial trading platform that wants to feel both authoritative and energetic. The base atmosphere is **deep near-black canvas** (#0b0e11) holding white type and a single, ubiquitous accent: **Binance Yellow** (#FCD535). That yellow does almost all of the brand's heavy lifting — it carries every primary CTA, every value-claim headline ("FUNDS ARE SAFU"), every "Sign Up" pill, every featured tier indicator, and the wordmark itself. There is no secondary brand color.

## Colors

### Brand & Accent
- **Binance Yellow** (`--clr-primary` — #FCD535): The single brand color. Used for primary CTA backgrounds, the wordmark, brand-claim headlines, trust badges, large stat numbers, and inline links.
- **Binance Yellow Active** (`--clr-primary-active` — #f0b90b): Press/hover darker variant.
- **Binance Yellow Disabled** (`--clr-primary-disabled` — #3a3a1f): Desaturated dark-yellow used on disabled CTAs over dark canvas.

### Surface (Dark Mode — Marketing Default)
- **Canvas Dark** (`--clr-canvas-dark` — #0b0e11): The primary page floor. Near-black with a slight warm tint — never pure black.
- **Surface Card Dark** (`--clr-surface-card` — #1e2329): Cards, navigation dropdowns, secondary buttons over dark canvas, markets table.
- **Surface Elevated Dark** (`--clr-surface-elevated` — #2b3139): One step lighter, used for nested cards, hovered nav items, and chart background panels.

### Surface (Light Mode — Transactional)
- **Canvas Light** (`--clr-canvas-light` — #ffffff): The page floor on transactional pages (buy crypto, deposit forms, account dialogs).
- **Surface Soft Light** (`--clr-surface-soft` — #fafafa): Footer surface and disabled states.

### Hairlines & Borders
- **Hairline on Light** (`--clr-hairline-light` — #eaecef): The 1px border tone on light surfaces.
- **Hairline on Dark** (`--clr-hairline-dark` — #2b3139): The 1px border tone on dark surfaces.
- **Border Strong** (`--clr-border-strong` — #cdd1d6): A heavier border tone used on disabled secondary buttons.

### Text
- **Ink** (`--clr-ink` — #181a20): The strongest text on light surfaces.
- **Body on Dark** (`--clr-body` — #eaecef): Default running-text on dark canvas — deliberately not pure white.
- **Muted** (`--clr-muted` — #707a8a): Footer links, breadcrumbs, captions, table column headers.
- **On Primary** (`--clr-on-primary` — #181a20): Black text on yellow primary CTAs.
- **On Dark** (`--clr-on-dark` — #ffffff): Pure white for high-contrast headlines on dark canvas.

### Trading Semantics
- **Trading Up** (`--clr-trading-up` — #0ecb81): Price-up green, used as text color in tables, charts, and inline ticker arrows. NEVER as a button background for non-trading actions.
- **Trading Down** (`--clr-trading-down` — #f6465d): Price-down red. Same usage rules as trading-up.

## Typography

### Font Family
- **BinanceNova** → editorial type (headlines, paragraphs, button labels, nav) - substitute: Inter
- **BinancePlex** → tabular numerical type (prices, volumes, percentages, stat counters) - substitute: JetBrains Mono or IBM Plex Mono

### Hierarchy
| Token | Size | Weight | Line Height | Use |
|-------|------|--------|-------------|-----|
| Hero Display | 64px | 700 | 1.1 | Homepage h1 |
| Display LG | 48px | 700 | 1.1 | Brand-claim headlines |
| Display MD | 40px | 600 | 1.15 | Section heads |
| Display SM | 32px | 600 | 1.2 | CTA band headlines |
| Title LG | 24px | 600 | 1.3 | Sub-section titles |
| Title MD | 20px | 600 | 1.35 | Feature card titles |
| Title SM | 16px | 600 | 1.4 | Trust badges, FAQ rows |
| Number Display | 40px | 700 | 1.1 | Big stat numbers (BinancePlex) |
| Number MD | 16px | 500 | 1.4 | Markets table prices (BinancePlex) |
| Number SM | 14px | 500 | 1.4 | Inline prices, % changes (BinancePlex) |
| Body MD | 14px | 400 | 1.5 | Default running-text |
| Body SM | 13px | 400 | 1.5 | Footer body, cookie text |
| Button | 14px | 600 | 1 | Standard CTA button labels |
| Nav Link | 14px | 500 | 1.4 | Top nav menu items |

## Layout
- **Max content width**: ~1280px centered on marketing pages; ~1440px on product surfaces
- **Spacing base**: 4px multiples. Section padding: 80px. Card internal padding: 24px.
- **Grid**: 12-column for editorial; 8/4 split for product (main panel + side rail)
- **Markets table**: 5-column header (Pair / Last Price / 24h Change / 24h Volume / Action)

## Components

### Top Navigation
- `top-nav-dark`: 64px tall, `--clr-canvas-dark` background. Yellow wordmark, horizontal menu, right-side Login/SignUp.
- `top-nav-light`: `--clr-canvas-light` background with `--clr-ink` menu items for transactional pages.

### Buttons
- `button-primary`: Yellow (#FCD535) bg, black (#181a20) text, 12px×24px padding, 40px height, 6px radius. The system's iconic combination.
- `button-primary-pill`: Larger pill variant (14px×32px, 9999px radius). Use sparingly for top-of-page actions.
- `button-secondary-on-dark`: Surface card bg, white text, 6px radius.
- `button-trading-up`: Solid green (#0ecb81) for Buy/Long actions. 4px radius, 8px×20px padding.
- `button-trading-down`: Solid red (#f6465d) for Sell/Short actions. Same shape.

### Cards & Containers
- **Markets Table Card**: `--clr-surface-card` bg, 12px radius, 24px padding. Tab row + 5-column coin table.
- **Stat Callout Card**: Transparent bg, yellow (#FCD535) text, Number Display size in BinancePlex.
- **QR Promo Card**: Surface card bg, 12px radius, 32px padding.
- **Feature Photo Card**: Surface card bg, 12px radius, edge-to-edge photos.

### Order Book
- **Depth bars**: Red (asks) / Green (bids) with opacity 0.18 as background bars.
- **Spread display**: Mid price + spread % between best bid/ask.
- **Rows**: 3 columns (Price / Size / Total), sorted by price.

### Trading Desk
- **Symbol bar**: Symbol selector, price, 24h change %, timeframe buttons (1m/5m/15m/1H/4H/1D), AI edge pill.
- **Side switch**: Buy/Long (green) / Sell/Short (red) toggle.
- **Order form**: Price, Amount, % balance buttons (25/50/75/100), Total, Available balance.
- **AI Pre-Trade**: Edge score, confluence, debate, kronos, guardian, memory, risk metrics with recommendation.

## Principles
1. Reserve yellow for primary actions, brand headlines, and the wordmark only — scarcity makes it powerful.
2. Numbers always use monospace (BinancePlex/JetBrains Mono). Copy always uses sans-serif (BinanceNova/Inter).
3. Choose canvas mode by surface intent: dark for marketing/product showcase; light for transactional dialogs.
4. Trading green/red are semantic price tokens — never repurpose for "success"/"error" generic states.
5. Flat surfaces with color-block separation — no heavy drop shadows or glassmorphism.
6. Display sizes use weight 700 — heavier than most marketing systems for glanceable numbers.
