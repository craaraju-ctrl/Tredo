# tredo-broker-alpaca

Alpaca Markets API v2 broker adapter for tredo — US equities and crypto trading via the [Alpaca Markets API](https://docs.alpaca.markets/).

## What it provides

- **Full `BrokerAdapter` implementation** for the Alpaca Markets API v2
- **Paper trading** support via `https://paper-api.alpaca.markets/`
- **Live trading** support via `https://api.alpaca.markets/`
- **Account info** — cash, equity, buying power, portfolio value, PDT status
- **Order management** — place market/limit/stop orders, cancel orders, get order status
- **Positions** — list open positions, get position by symbol, close positions
- **Trade history** — recent fills and closed trades
- **Portfolio summary** — unified `PortfolioSummary` for the tredo system

## Authentication

Alpaca uses API key pairs passed as HTTP headers:
- `APCA-API-KEY-ID` — Your API Key ID
- `APCA-API-SECRET-KEY` — Your Secret Key

Get your keys at:
- Live: https://app.alpaca.markets/
- Paper: https://paper-api.alpaca.markets/

Store them in `config/tredo.env`:
```bash
ALPACA_API_KEY=your-key-id
ALPACA_API_SECRET=your-secret-key
ALPACA_PAPER=true   # set to false for live
```

## Usage

```rust
use tredo_broker_alpaca::AlpacaBroker;
use tredo_core::paper_engine::{BrokerAdapter, OrderRequest, OrderType, TradingMode};
use tredo_core::TradeDirection;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let broker = AlpacaBroker::new(
        "your-api-key",
        "your-secret-key",
        true, // paper mode
    );
    broker.connect().await?;
    
    let summary = broker.get_summary().await?;
    println!("Cash: ${:.2}", summary.cash);
    
    let order = OrderRequest {
        symbol: "AAPL".to_string(),
        direction: TradeDirection::Long,
        quantity: 10.0,
        order_type: OrderType::Market,
        ..Default::default()
    };
    let status = broker.place_order(order).await?;
    println!("Order placed: {:?}", status);
    
    Ok(())
}
```

## CLI Configuration

Configure via the unified `tredo` CLI:
```bash
tredo configure alpaca
```

This will interactively prompt for your API key, secret, and paper/live preference.

## Safety

- This adapter is **gated by `PAPER_MODE=true`** by default.
- Live mode requires `PAPER_MODE=false` AND `--confirm-live` flag.
- Never commit real API keys to version control.

Depends on `tredo-core`.
