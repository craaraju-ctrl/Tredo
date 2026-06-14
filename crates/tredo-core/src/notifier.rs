//! Simple notifier for Telegram and WhatsApp (Business/Meta Cloud or Twilio).
//! Configured during `./tredo setup` (see tredo bash wizard).
//! Used for real-time alerts on signals, trades, drawdowns, regime changes, etc.
//!
//! Production: rate-limit, templates, error handling, retries.

use reqwest::Client;
use serde_json::json;
use std::error::Error;

#[derive(Clone)]
pub struct Notifier {
    telegram_bot_token: String,
    telegram_chat_id: String,
    whatsapp_sid: String,
    whatsapp_token: String,
    whatsapp_from: String,
    client: Client,
}

impl Notifier {
    pub fn from_env() -> Self {
        Self {
            telegram_bot_token: std::env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default(),
            telegram_chat_id: std::env::var("TELEGRAM_CHAT_ID").unwrap_or_default(),
            whatsapp_sid: std::env::var("WHATSAPP_SID").unwrap_or_default(),
            whatsapp_token: std::env::var("WHATSAPP_TOKEN").unwrap_or_default(),
            whatsapp_from: std::env::var("WHATSAPP_FROM").unwrap_or_default(),
            client: Client::new(),
        }
    }

    pub async fn send(&self, title: &str, message: &str) {
        if !self.telegram_bot_token.is_empty() && !self.telegram_chat_id.is_empty() {
            if let Err(e) = self.send_telegram(title, message).await {
                eprintln!("[Notifier] Telegram error: {e}");
            }
        }
        if !self.whatsapp_sid.is_empty() && !self.whatsapp_token.is_empty() {
            if let Err(e) = self.send_whatsapp(title, message).await {
                eprintln!("[Notifier] WhatsApp error: {e}");
            }
        }
    }

    async fn send_telegram(
        &self,
        title: &str,
        message: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let url = format!(
            "https://api.telegram.org/bot{}/sendMessage",
            self.telegram_bot_token
        );
        let text = format!("*{}*\n{}", title, message);
        let body = json!({
            "chat_id": self.telegram_chat_id,
            "text": text,
            "parse_mode": "Markdown"
        });

        self.client.post(&url).json(&body).send().await?;
        Ok(())
    }

    async fn send_whatsapp(
        &self,
        title: &str,
        message: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Example for Twilio WhatsApp or Meta Cloud API (adjust endpoint/auth as needed).
        // For Twilio:
        let url = format!(
            "https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json",
            self.whatsapp_sid
        );

        let text = format!("{}: {}", title, message);
        let params = [
            ("From", format!("whatsapp:{}", self.whatsapp_from)),
            ("To", "whatsapp:+YOUR_RECIPIENT".to_string()), // configure recipient
            ("Body", text),
        ];

        self.client
            .post(&url)
            .basic_auth(&self.whatsapp_sid, Some(&self.whatsapp_token))
            .form(&params)
            .send()
            .await?;

        Ok(())
    }
}

// Convenience for orchestrator / agents
pub async fn alert(title: &str, message: &str) {
    let n = Notifier::from_env();
    n.send(title, message).await;
}
