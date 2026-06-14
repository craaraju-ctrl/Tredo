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

/// Fetches news headlines using free public RSS feeds.
/// Primary source: Google News RSS (no API key required).
pub struct NewsFetcher {
    client: reqwest::Client,
}

impl NewsFetcher {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    /// Fetch news headlines for a given symbol via Google News RSS.
    /// Returns up to 8 headlines.
    pub async fn fetch_headlines(
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
