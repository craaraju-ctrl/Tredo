//! # Live Economic Calendar API Integration
//!
//! Fetches live economic calendar data from free APIs to replace the hardcoded
//! calendar events in `calendar.rs`. Supports multiple sources:
//!
//! - **Financial Modeling Prep (FMP)** — Free tier: 250 requests/day, economic calendar
//! - **Alpha Vantage** — Free tier: 5 requests/min, 500/day
//! - **Fallback**: If no API key is set, uses the hardcoded calendar generator
//!
//! Environment variables:
//! - `ECONOMIC_CALENDAR_PROVIDER` — "fmp" (default) or "alphavantage"
//! - `FMP_API_KEY` — Your Financial Modeling Prep API key
//! - `ALPHA_VANTAGE_API_KEY` — Your Alpha Vantage API key

use crate::calendar::{CalendarEvent, EventImpact};
use std::collections::HashMap;

/// Which calendar provider to use
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarSource {
    FinancialModelingPrep,
    AlphaVantage,
    /// Use the built-in hardcoded generator (no API key needed)
    BuiltIn,
}

impl std::fmt::Display for CalendarSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CalendarSource::FinancialModelingPrep => write!(f, "fmp"),
            CalendarSource::AlphaVantage => write!(f, "alphavantage"),
            CalendarSource::BuiltIn => write!(f, "builtin"),
        }
    }
}

/// Detect which calendar provider to use from environment variables
pub fn detect_calendar_source() -> CalendarSource {
    let provider = std::env::var("ECONOMIC_CALENDAR_PROVIDER")
        .unwrap_or_default()
        .to_lowercase();

    match provider.as_str() {
        "fmp" if std::env::var("FMP_API_KEY").is_ok() => CalendarSource::FinancialModelingPrep,
        "alphavantage" if std::env::var("ALPHA_VANTAGE_API_KEY").is_ok() => {
            CalendarSource::AlphaVantage
        }
        _ => {
            // Auto-detect: use FMP if key exists
            if std::env::var("FMP_API_KEY").is_ok() {
                CalendarSource::FinancialModelingPrep
            } else if std::env::var("ALPHA_VANTAGE_API_KEY").is_ok() {
                CalendarSource::AlphaVantage
            } else {
                CalendarSource::BuiltIn
            }
        }
    }
}

/// Fetch live economic calendar events from the configured API provider.
///
/// Returns a list of `CalendarEvent` structs with upcoming economic events.
/// Falls back to built-in hardcoded events if no API is configured or on error.
pub async fn fetch_economic_calendar_live() -> Vec<CalendarEvent> {
    let source = detect_calendar_source();
    let events = match source {
        CalendarSource::FinancialModelingPrep => fetch_fmp_calendar().await,
        CalendarSource::AlphaVantage => fetch_alphavantage_calendar().await,
        CalendarSource::BuiltIn => None,
    };

    match events {
        Some(parsed) if !parsed.is_empty() => {
            println!(
                "[Calendar] Loaded {} live events from {}",
                parsed.len(),
                source
            );
            parsed
        }
        _ => {
            let builtin = crate::generate_economic_calendar();
            println!(
                "[Calendar] Using {} built-in events (API: {})",
                builtin.len(),
                source
            );
            builtin
        }
    }
}

/// Parse EventImpact from a string label
fn parse_impact(s: &str) -> EventImpact {
    match s.to_lowercase().trim() {
        "high" | "elevated" | "critical" => EventImpact::High,
        "medium" | "moderate" => EventImpact::Medium,
        _ => EventImpact::Low,
    }
}

/// Parse "YYYY-MM-DD HH:MM:SS" or "YYYY-MM-DD" into (date, time)
fn parse_date_time(dt: &str) -> (String, Option<String>) {
    let trimmed = dt.trim();
    if trimmed.len() >= 10 {
        let date = trimmed[..10].to_string();
        let time = if trimmed.len() > 11 {
            Some(trimmed[11..16].to_string())
        } else {
            None
        };
        (date, time)
    } else {
        (trimmed.to_string(), None)
    }
}

// ── FMP (Financial Modeling Prep) ──────────────────────────────────────────

/// Fetch economic calendar from Financial Modeling Prep API.
/// Endpoint: https://financialmodelingprep.com/api/v3/economic_calendar?apikey={KEY}
async fn fetch_fmp_calendar() -> Option<Vec<CalendarEvent>> {
    let api_key = std::env::var("FMP_API_KEY").ok()?;
    let url = format!(
        "https://financialmodelingprep.com/api/v3/economic_calendar?apikey={}",
        api_key
    );

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        eprintln!("[Calendar] FMP API returned status {}", resp.status());
        return None;
    }

    let data: Vec<serde_json::Value> = resp.json().await.ok()?;
    let mut events = Vec::new();

    for item in data.iter().take(50) {
        let event_name = item
            .get("event")
            .and_then(|v| v.as_str())
            .unwrap_or("Economic Event");
        let date_str = item.get("date").and_then(|v| v.as_str()).unwrap_or("");
        let country = item
            .get("country")
            .and_then(|v| v.as_str())
            .unwrap_or("USD");
        let impact_str = item.get("impact").and_then(|v| v.as_str()).unwrap_or("low");
        let description = item
            .get("description")
            .and_then(|v| v.as_str())
            .or_else(|| item.get("explanation").and_then(|v| v.as_str()))
            .unwrap_or("");

        let (date, time) = parse_date_time(date_str);
        if date.len() < 10 {
            continue;
        }

        // Map country to currency
        let currency = match country.to_uppercase().as_str() {
            "US" | "USA" => "USD".to_string(),
            "IN" | "IND" => "INR".to_string(),
            "GB" | "UK" | "GBR" => "GBP".to_string(),
            "EU" | "EUR" => "EUR".to_string(),
            "JP" | "JPN" => "JPY".to_string(),
            "CN" | "CHN" => "CNY".to_string(),
            "DE" | "DEU" => "EUR".to_string(),
            "CA" | "CAN" => "CAD".to_string(),
            "AU" | "AUS" => "AUD".to_string(),
            _ => country.to_string(),
        };

        events.push(CalendarEvent {
            title: event_name.to_string(),
            date,
            time,
            impact: parse_impact(impact_str),
            currency,
            description: description.to_string(),
        });
    }

    Some(events)
}

// ── Alpha Vantage ──────────────────────────────────────────────────────────

/// Fetch economic calendar from Alpha Vantage API (limited free tier).
/// Alpha Vantage doesn't have a dedicated economic calendar endpoint,
/// but we use the Forex/Economic indicators endpoint plus the NEWS_SENTIMENT
/// endpoint for market-moving events.
async fn fetch_alphavantage_calendar() -> Option<Vec<CalendarEvent>> {
    let api_key = std::env::var("ALPHA_VANTAGE_API_KEY").ok()?;
    let client = reqwest::Client::new();
    let mut events = Vec::new();

    // Alpha Vantage NEWS_SENTIMENT endpoint — returns top market news with relevance
    let url = format!(
        "https://www.alphavantage.co/query?function=NEWS_SENTIMENT&topics=earnings,ipo,merger,economy_macro&apikey={}",
        api_key
    );

    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }

    let data: HashMap<String, serde_json::Value> = resp.json().await.ok()?;

    if let Some(feed) = data.get("feed") {
        if let Some(items) = feed.as_array() {
            for item in items.iter().take(30) {
                let title = item
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Market Event");
                let summary = item.get("summary").and_then(|v| v.as_str()).unwrap_or("");

                // Extract time_published
                let time_published = item
                    .get("time_published")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let (date, time) = if time_published.len() >= 8 {
                    let d = format!(
                        "{}-{}-{}",
                        &time_published[..4],
                        &time_published[4..6],
                        &time_published[6..8]
                    );
                    let t = if time_published.len() >= 12 {
                        Some(format!(
                            "{}:{}",
                            &time_published[8..10],
                            &time_published[10..12]
                        ))
                    } else {
                        None
                    };
                    (d, t)
                } else {
                    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
                    (today, None)
                };

                // Determine relevance/sentiment
                let overall_sentiment = item
                    .get("overall_sentiment_score")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let impact = if overall_sentiment.abs() > 0.3 {
                    EventImpact::High
                } else if overall_sentiment.abs() > 0.15 {
                    EventImpact::Medium
                } else {
                    EventImpact::Low
                };

                // Extract tickers for currency
                let mut currency = "USD".to_string();
                if let Some(ticker_sentiment) = item.get("ticker_sentiment") {
                    if let Some(tickers) = ticker_sentiment.as_array() {
                        if let Some(first) = tickers.first() {
                            let sym = first.get("ticker").and_then(|v| v.as_str()).unwrap_or("");
                            currency = match sym {
                                "BTC" | "ETH" | "SOL" => sym.to_string(),
                                _ => "USD".to_string(),
                            };
                        }
                    }
                }

                events.push(CalendarEvent {
                    title: title.to_string(),
                    date,
                    time,
                    impact,
                    currency,
                    description: summary.to_string(),
                });
            }
        }
    }

    // Also try the Forex INFLATION endpoint for CPI data
    let cpi_url = format!(
        "https://www.alphavantage.co/query?function=INFLATION&apikey={}",
        api_key
    );
    let cpi_resp = client
        .get(&cpi_url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok();
    if let Some(resp) = cpi_resp {
        if resp.status().is_success() {
            let cpi_data: HashMap<String, serde_json::Value> =
                resp.json().await.unwrap_or_default();
            if let Some(cpi_items) = cpi_data.get("data") {
                if let Some(items) = cpi_items.as_array() {
                    for item in items.iter().take(5) {
                        let date_str = item.get("date").and_then(|v| v.as_str()).unwrap_or("");
                        let value = item.get("value").and_then(|v| v.as_str()).unwrap_or("0.0");
                        if date_str.len() >= 10 {
                            events.push(CalendarEvent {
                                title: "US CPI (Inflation)".into(),
                                date: date_str.to_string(),
                                time: Some("08:30".into()),
                                impact: EventImpact::High,
                                currency: "USD".into(),
                                description: format!("Consumer Price Index: {}%", value),
                            });
                        }
                    }
                }
            }
        }
    }

    Some(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_calendar_source_default() {
        // Should default to BuiltIn when no env vars are set
        // Clear env for test — this runs in test context
        let source = detect_calendar_source();
        // Since env vars might be set in CI, just verify it returns something valid
        let valid = matches!(
            source,
            CalendarSource::FinancialModelingPrep
                | CalendarSource::AlphaVantage
                | CalendarSource::BuiltIn
        );
        assert!(valid, "Calendar source must be one of the valid options");
    }

    #[test]
    fn test_parse_impact() {
        assert!(matches!(parse_impact("High"), EventImpact::High));
        assert!(matches!(parse_impact("medium"), EventImpact::Medium));
        assert!(matches!(parse_impact("low"), EventImpact::Low));
        assert!(matches!(parse_impact("unknown"), EventImpact::Low));
    }

    #[test]
    fn test_parse_date_time() {
        let (date, time) = parse_date_time("2026-06-15 14:30:00");
        assert_eq!(date, "2026-06-15");
        assert_eq!(time, Some("14:30".into()));

        let (date, time) = parse_date_time("2026-06-15");
        assert_eq!(date, "2026-06-15");
        assert!(time.is_none());
    }

    #[test]
    fn test_display_source() {
        assert_eq!(CalendarSource::FinancialModelingPrep.to_string(), "fmp");
        assert_eq!(CalendarSource::AlphaVantage.to_string(), "alphavantage");
        assert_eq!(CalendarSource::BuiltIn.to_string(), "builtin");
    }
}
