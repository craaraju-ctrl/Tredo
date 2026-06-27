# tredo-broker-zerodha

Zerodha Kite Connect v3 broker adapter for tredo — real-money trading via the [Kite Connect REST API](https://kite.trade/docs/connect/v3/).

## What it provides

- **Full `BrokerAdapter` implementation** for Zerodha Kite Connect v3
- **OAuth2 authentication flow** — request token → access token exchange
- **Signed requests** — HMAC-SHA256 signature on all API calls
- **Account info** — funds, margins, profile, account type
- **Order management** — place regular/AMO/CO/BO orders, modify, cancel, get order status
- **Positions** — day and net positions, position conversion
- **Holdings** — long-term holdings portfolio
- **Trade history** — recent fills and execution details
- **Portfolio summary** — unified `PortfolioSummary` for the tredo system

## Authentication Flow

1. User authorizes via: `https://kite.zerodha.com/connect/login?v=3&api_key={API_KEY}`
2. Zerodha redirects to callback URL with a `request_token`
3. Exchange `request_token` for an `access_token` (valid until 6 AM next day)
4. All subsequent requests use `Authorization: token {api_key}:{access_token}`

Store credentials in `config/tredo.env`:
```bash
ZERODHA_API_KEY=your-api-key
ZERODHA_API_SECRET=your-api-secret
ZERODHA_REQUEST_TOKEN=from-login-callback
# access_token is derived automatically
```

## Usage

```rust
use tredo_broker_zerodha::ZerodhaKiteBroker;
use tredo_core::paper_engine::{BrokerAdapter, OrderRequest, OrderType};
use tredo_core::TradeDirection;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let broker = ZerodhaKiteBroker::new(
        "your-api-key",
        "your-api-secret",
        "https://api.kite.trade",
        "request-token-from-callback",
    );
    broker.connect().await?;
    
    let summary = broker.get_summary().await?;
    println!("Cash: ₹{:.2}", summary.cash);
    
    let order = OrderRequest {
        symbol: "RELIANCE".to_string(),
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
tredo configure zerodha
```

This will interactively prompt for your API key, secret, and base URL.

## Safety

- This adapter is **gated by `PAPER_MODE=true`** by default.
- Live mode requires `PAPER_MODE=false` AND `--confirm-live` flag.
- Zerodha tokens expire at 6 AM daily — the system will prompt for re-auth.
- Never commit real API keys or secrets to version control.

Depends on `tredo-core`.
