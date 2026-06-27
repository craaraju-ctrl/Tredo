use chrono::{DateTime, Utc};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

/// A single news headline with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsItem {
    pub title: String,
    pub source: String,
    pub published_at: Option<DateTime<Utc>>,
    pub url: String,
}

/// A summarized news context ready for LLM injection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsContext {
    pub symbol: String,
    pub headlines: Vec<NewsItem>,
    pub summary: String, // Ollama-generated summary/sentiment
    pub fetched_at: DateTime<Utc>,
}

impl NewsContext {
    /// Format into a short string for LLM prompt injection.
    pub fn to_prompt_string(&self) -> String {
        if self.headlines.is_empty() {
            return "No recent news for this symbol.".to_string();
        }
        let headlines_str: String = self
            .headlines
            .iter()
            .take(5)
            .map(|h| format!("- [{}] {}", h.source, h.title))
            .collect::<Vec<_>>()
            .join("\n");

        format!("── NEWS ──\n{}\nSummary: {}\n", headlines_str, self.summary)
    }
}

/// Fetches news headlines using free public RSS feeds + multiple free key APIs researched 2026.
/// Priority (to respect rate limits + get sentiment where available):
/// Finnhub (news + sentiment), Marketaux (sentiment scores), NewsAPI, Alpha Vantage (NEWS_SENTIMENT),
/// CoinGecko (crypto market data as "headlines" proxy for meter synergy), then Google RSS fallback.
/// Keys from config (POLYGON/FRED added for meter elsewhere; CoinGecko often keyless).
pub struct NewsFetcher {
    client: reqwest::Client,
    config: crate::config::Config, // for API keys (free tiers from research: Alpha Vantage, Finnhub, Marketaux, NewsAPI etc.)
}

impl NewsFetcher {
    pub fn new(client: reqwest::Client, config: crate::config::Config) -> Self {
        Self { client, config }
    }

    /// Fetch news headlines for a given symbol.
    /// Tries key APIs first (more structured, often with sentiment), falls back to RSS.
    /// Returns up to 10 unique headlines. Enriches summary context for analyser.
    pub async fn fetch_headlines(
        &self,
        symbol: &str,
    ) -> Result<Vec<NewsItem>, Box<dyn std::error::Error + Send + Sync>> {
        let mut items: Vec<NewsItem> = Vec::new();

        // 1. Finnhub (free generous; company-news or general /news; sentiment via separate or included)
        if !self.config.finnhub_key.is_empty() {
            if let Ok(mut fin) = self.try_finnhub(symbol).await {
                items.append(&mut fin);
            }
        }

        // 2. Marketaux (free ~100/day; excellent sentiment scores in response)
        if !self.config.marketaux_key.is_empty() && items.len() < 6 {
            if let Ok(mut ma) = self.try_marketaux(symbol).await {
                items.append(&mut ma);
            }
        }

        // 3. NewsAPI (free dev 100/day)
        if !self.config.newsapi_key.is_empty() && items.len() < 7 {
            if let Ok(mut na) = self.try_newsapi(symbol).await {
                items.append(&mut na);
            }
        }

        // 4. Alpha Vantage NEWS_SENTIMENT (free but tight 25/day; great for meter synergy)
        if !self.config.alpha_vantage_key.is_empty() && items.len() < 8 {
            if let Ok(mut av) = self.try_alphavantage_news(symbol).await {
                items.append(&mut av);
            }
        }

        // 5. CoinGecko (keyless or demo; crypto market "news" via description + vol for symbols like BTC)
        if items.len() < 6
            && (symbol == "BTC"
                || symbol == "ETH"
                || symbol == "SOL"
                || symbol.to_lowercase().contains("coin"))
        {
            if let Ok(mut cg) = self.try_coingecko(symbol).await {
                items.append(&mut cg);
            }
        }

        // 6. RSS fallback (always works, Google News)
        if items.len() < 5 {
            if let Ok(mut rss) = self.fetch_rss_headlines(symbol).await {
                items.append(&mut rss);
            }
        }

        // Dedup by title (simple)
        items.sort_by_key(|b| std::cmp::Reverse(b.published_at));
        let mut seen = std::collections::HashSet::new();
        items.retain(|i| seen.insert(i.title.clone()));
        Ok(items.into_iter().take(10).collect())
    }

    async fn try_finnhub(
        &self,
        symbol: &str,
    ) -> Result<Vec<NewsItem>, Box<dyn std::error::Error + Send + Sync>> {
        // Finnhub /company-news or /news ; use last 2 days. Token in query.
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let from = (chrono::Utc::now() - chrono::Duration::days(2))
            .format("%Y-%m-%d")
            .to_string();
        let sym = symbol; // tickers or crypto like BINANCE:BTCUSDT but simple
        let url = format!(
            "https://finnhub.io/api/v1/company-news?symbol={}&from={}&to={}&token={}",
            sym, from, today, self.config.finnhub_key
        );
        let resp = self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(6))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Ok(vec![]);
        }
        let v: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!([]));
        let mut out = vec![];
        if let Some(arr) = v.as_array() {
            for it in arr.iter().take(6) {
                let title = it
                    .get("headline")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                if title.is_empty() {
                    continue;
                }
                let src = it
                    .get("source")
                    .and_then(|x| x.as_str())
                    .unwrap_or("Finnhub")
                    .to_string();
                let url = it
                    .get("url")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                out.push(NewsItem {
                    title,
                    source: src,
                    published_at: None,
                    url,
                });
            }
        }
        Ok(out)
    }

    async fn try_marketaux(
        &self,
        symbol: &str,
    ) -> Result<Vec<NewsItem>, Box<dyn std::error::Error + Send + Sync>> {
        let tick = match symbol {
            "BTC" => "BTC",
            "ETH" => "ETH",
            "NIFTY" => "NIFTY50",
            _ => symbol,
        };
        let url = format!(
            "https://api.marketaux.com/v1/news/all?api_token={}&symbols={}&filter_entities=true&limit=8&language=en",
            self.config.marketaux_key, tick
        );
        let resp = self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(7))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Ok(vec![]);
        }
        let v: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!({}));
        let mut out = vec![];
        if let Some(data) = v.get("data").and_then(|d| d.as_array()) {
            for it in data.iter().take(6) {
                let title = it
                    .get("title")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                if title.is_empty() {
                    continue;
                }
                let src = it
                    .get("source")
                    .and_then(|x| x.as_str())
                    .unwrap_or("Marketaux")
                    .to_string();
                let u = it
                    .get("url")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                // If sentiment present in entity or overall, we can note in future NewsItem extension; for now title carries signal
                out.push(NewsItem {
                    title,
                    source: src,
                    published_at: None,
                    url: u,
                });
            }
        }
        Ok(out)
    }

    async fn try_newsapi(
        &self,
        symbol: &str,
    ) -> Result<Vec<NewsItem>, Box<dyn std::error::Error + Send + Sync>> {
        let q = format!("{}+stock+OR+{}+finance", symbol, symbol);
        let url = format!(
            "https://newsapi.org/v2/everything?q={}&sortBy=publishedAt&pageSize=6&apiKey={}",
            q, self.config.newsapi_key
        );
        let resp = self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(6))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Ok(vec![]);
        }
        let v: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!({}));
        let mut out = vec![];
        if let Some(arts) = v.get("articles").and_then(|a| a.as_array()) {
            for a in arts.iter().take(5) {
                let title = a
                    .get("title")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                if title.is_empty() || title == "[Removed]" {
                    continue;
                }
                let src = a
                    .get("source")
                    .and_then(|s| s.get("name"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("NewsAPI")
                    .to_string();
                let u = a
                    .get("url")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                out.push(NewsItem {
                    title,
                    source: src,
                    published_at: None,
                    url: u,
                });
            }
        }
        Ok(out)
    }

    async fn try_alphavantage_news(
        &self,
        symbol: &str,
    ) -> Result<Vec<NewsItem>, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "https://www.alphavantage.co/query?function=NEWS_SENTIMENT&tickers={}&apikey={}&limit=6",
            symbol, self.config.alpha_vantage_key
        );
        let resp = self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(6))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Ok(vec![]);
        }
        let v: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!({}));
        let mut out = vec![];
        if let Some(feed) = v.get("feed").and_then(|f| f.as_array()) {
            for it in feed.iter().take(5) {
                let title = it
                    .get("title")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                if title.is_empty() {
                    continue;
                }
                let src = it
                    .get("source")
                    .and_then(|x| x.as_str())
                    .unwrap_or("AlphaVantage")
                    .to_string();
                let u = it
                    .get("url")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                out.push(NewsItem {
                    title,
                    source: src,
                    published_at: None,
                    url: u,
                });
            }
        }
        Ok(out)
    }

    async fn try_coingecko(
        &self,
        symbol: &str,
    ) -> Result<Vec<NewsItem>, Box<dyn std::error::Error + Send + Sync>> {
        // Keyless public CoinGecko for crypto "market pulse" as proxy headlines (volume, description changes)
        let id = match symbol {
            "BTC" => "bitcoin",
            "ETH" => "ethereum",
            "SOL" => "solana",
            _ => "bitcoin",
        };
        let url = format!("https://api.coingecko.com/api/v3/coins/{}?localization=false&tickers=false&market_data=true&community_data=false", id);
        let resp = self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Ok(vec![]);
        }
        let v: serde_json::Value = resp.json().await.unwrap_or(serde_json::json!({}));
        let mut out = vec![];
        if let Some(name) = v.get("name").and_then(|x| x.as_str()) {
            let default_md = serde_json::json!({});
            let md = v.get("market_data").unwrap_or(&default_md);
            let vol = md
                .get("total_volume")
                .and_then(|x| x.get("usd"))
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0);
            let price = md
                .get("current_price")
                .and_then(|x| x.get("usd"))
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0);
            let ch = md
                .get("price_change_percentage_24h")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0);
            let title = format!(
                "{} market: ${:.0} vol ${:.0} 24h {:.1}% (CoinGecko)",
                name, price, vol, ch
            );
            out.push(NewsItem {
                title,
                source: "CoinGecko".to_string(),
                published_at: None,
                url: format!("https://www.coingecko.com/en/coins/{}", id),
            });
        }
        Ok(out)
    }

    /// Original RSS only path (used as fallback).
    async fn fetch_rss_headlines(
        &self,
        symbol: &str,
    ) -> Result<Vec<NewsItem>, Box<dyn std::error::Error + Send + Sync>> {
        let search_query = match symbol {
            "BTC" => "Bitcoin+OR+%24BTC",
            "ETH" => "Ethereum+OR+%24ETH",
            "SOL" => "Solana+OR+%24SOL",
            "NIFTY" => "NIFTY+50+OR+NSE+India+stock+market",
            "RELIANCE" => "Reliance+Industries+OR+RELIANCE.NS",
            other => other,
        };

        let url = format!(
            "https://news.google.com/rss/search?q={}+stock+OR+finance&hl=en-US&gl=US&ceid=US:en",
            search_query
        );

        let resp = self
            .client
            .get(&url)
            .header(
                "User-Agent",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
            )
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await?;

        let body = resp.text().await?;
        let items = parse_rss_items(&body)?;

        Ok(items.into_iter().take(8).collect())
    }
}

/// Parse RSS XML items using quick-xml.
/// Extracts title, source (from <source> tag), pubDate, and link.
fn parse_rss_items(xml: &str) -> Result<Vec<NewsItem>, Box<dyn std::error::Error + Send + Sync>> {
    let mut reader = Reader::from_str(xml);
    reader.trim_text(true);

    let mut items = Vec::new();
    let mut in_item = false;
    let mut in_title = false;
    let mut in_link = false;
    let mut in_pubdate = false;
    let mut in_source = false;

    let mut current_title = String::new();
    let mut current_link = String::new();
    let mut current_date = String::new();
    let mut current_source = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => match e.name().as_ref() {
                b"item" => in_item = true,
                b"title" if in_item => in_title = true,
                b"link" if in_item => in_link = true,
                b"pubDate" if in_item => in_pubdate = true,
                b"source" if in_item => in_source = true,
                _ => {}
            },
            Ok(Event::Text(ref e)) => {
                let text = e.unescape().unwrap_or_default().to_string();
                if in_title {
                    current_title.push_str(&text);
                }
                if in_link {
                    current_link.push_str(&text);
                }
                if in_pubdate {
                    current_date.push_str(&text);
                }
                if in_source {
                    current_source.push_str(&text);
                }
            }
            Ok(Event::CData(ref e)) => {
                // CDATA content is raw — no entity decoding needed
                let text = String::from_utf8_lossy(e.as_ref()).to_string();
                if in_title {
                    current_title.push_str(&text);
                }
                if in_link {
                    current_link.push_str(&text);
                }
                if in_pubdate {
                    current_date.push_str(&text);
                }
                if in_source {
                    current_source.push_str(&text);
                }
            }
            Ok(Event::End(ref e)) => match e.name().as_ref() {
                b"item" => {
                    if !current_title.is_empty() {
                        let parsed_date = parse_rss_date(&current_date);
                        items.push(NewsItem {
                            title: current_title.clone(),
                            source: if current_source.is_empty() {
                                guess_source_from_title(&current_title)
                            } else {
                                current_source.clone()
                            },
                            published_at: parsed_date,
                            url: current_link.clone(),
                        });
                    }
                    in_item = false;
                    current_title.clear();
                    current_link.clear();
                    current_date.clear();
                    current_source.clear();
                }
                b"title" => in_title = false,
                b"link" => in_link = false,
                b"pubDate" => in_pubdate = false,
                b"source" => in_source = false,
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("RSS parse error: {e}").into()),
            _ => {}
        }
    }

    Ok(items)
}

/// Parse RSS date format like "Tue, 10 Jun 2025 14:30:00 GMT"
fn parse_rss_date(date_str: &str) -> Option<DateTime<Utc>> {
    let date_str = date_str.trim();
    // Try RFC 2822 format
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(date_str) {
        return Some(dt.with_timezone(&Utc));
    }
    // Try common RSS variations
    for fmt in &[
        "%a, %d %b %Y %H:%M:%S %z",
        "%Y-%m-%dT%H:%M:%S%z",
        "%Y-%m-%dT%H:%M:%SZ",
    ] {
        if let Ok(dt) = chrono::DateTime::parse_from_str(date_str, fmt) {
            return Some(dt.with_timezone(&Utc));
        }
    }
    None
}

/// Guess the news source from the title text if <source> tag was missing.
fn guess_source_from_title(title: &str) -> String {
    let known_sources = [
        "Reuters",
        "Bloomberg",
        "CNBC",
        "Financial Times",
        "Wall Street Journal",
        "Bloomberg Quint",
        "Economic Times",
        "Moneycontrol",
        "Business Standard",
        "Mint",
        "Livemint",
        "Yahoo Finance",
        "Investopedia",
        "CoinDesk",
        "CoinTelegraph",
        "BBC",
        "CNN",
        "The Hindu",
        "Times of India",
        "NDTV Profit",
    ];
    for source in &known_sources {
        if title.contains(source) {
            return source.to_string();
        }
    }
    "News".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rss_items_with_sample() {
        let sample_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
<channel>
<title>Google News</title>
<item>
<title><![CDATA[Bitcoin price surges to $100K as institutional adoption grows - Reuters]]></title>
<link>https://example.com/bitcoin-100k</link>
<pubDate>Tue, 10 Jun 2025 14:30:00 GMT</pubDate>
<source>Reuters</source>
</item>
<item>
<title><![CDATA[NIFTY hits all-time high above 25,000 - Economic Times]]></title>
<link>https://example.com/nifty-25000</link>
<pubDate>Mon, 09 Jun 2025 09:15:00 GMT</pubDate>
<source>Economic Times</source>
</item>
</channel>
</rss>"#;

        let items = parse_rss_items(sample_xml).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].source, "Reuters");
        assert!(items[0].title.contains("Bitcoin"));
        assert!(items[0].published_at.is_some());
        assert_eq!(items[1].source, "Economic Times");
        assert!(items[1].title.contains("NIFTY"));
    }

    #[test]
    fn test_parse_rss_without_source_tag() {
        let sample_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
<channel>
<item>
<title>Markets rally on Fed decision - Bloomberg News</title>
<link>https://example.com/rally</link>
<pubDate>Fri, 06 Jun 2025 20:00:00 GMT</pubDate>
</item>
</channel>
</rss>"#;

        let items = parse_rss_items(sample_xml).unwrap();
        assert_eq!(items.len(), 1);
        // Should guess Bloomberg from the title
        assert_eq!(items[0].source, "Bloomberg");
    }

    #[test]
    fn test_news_context_formatting() {
        let ctx = NewsContext {
            symbol: "BTC".to_string(),
            headlines: vec![NewsItem {
                title: "Bitcoin surges past $100K".to_string(),
                source: "Reuters".to_string(),
                published_at: None,
                url: "http://example.com".to_string(),
            }],
            summary: "Positive sentiment overall. Institutional adoption driving prices."
                .to_string(),
            fetched_at: Utc::now(),
        };

        let prompt = ctx.to_prompt_string();
        assert!(prompt.contains("Reuters"));
        assert!(prompt.contains("Bitcoin"));
        assert!(prompt.contains("Positive sentiment"));
    }

    #[test]
    fn test_empty_news_context() {
        let ctx = NewsContext {
            symbol: "NIFTY".to_string(),
            headlines: vec![],
            summary: String::new(),
            fetched_at: Utc::now(),
        };
        assert_eq!(ctx.to_prompt_string(), "No recent news for this symbol.");
    }

    #[test]
    fn test_guess_source() {
        assert_eq!(guess_source_from_title("S&P 500 rises - CNBC"), "CNBC");
        assert_eq!(
            guess_source_from_title("Oil prices drop - Reuters"),
            "Reuters"
        );
        assert_eq!(
            guess_source_from_title("Unknown publication article"),
            "News"
        );
    }
}
