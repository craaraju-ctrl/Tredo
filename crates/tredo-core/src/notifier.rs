//! Multi-channel notifier for Telegram, WhatsApp, Slack, and Email.
//! Configured via environment variables at startup.
//! Used for real-time alerts on signals, trades, drawdowns, regime changes, etc.
//!
//! Channels:
//! - Telegram: TELEGRAM_BOT_TOKEN + TELEGRAM_CHAT_ID
//! - WhatsApp: WHATSAPP_SID + WHATSAPP_TOKEN + WHATSAPP_FROM + WHATSAPP_RECIPIENT
//! - Slack:    SLACK_WEBHOOK_URL
//! - Email:    SMTP_HOST + SMTP_PORT + SMTP_USERNAME + SMTP_PASSWORD + SMTP_FROM + SMTP_TO

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
    whatsapp_recipient: String,
    slack_webhook_url: String,
    smtp_host: String,
    #[allow(dead_code)]
    smtp_port: u16,
    smtp_username: String,
    smtp_password: String,
    smtp_from: String,
    smtp_to: String,
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
            whatsapp_recipient: std::env::var("WHATSAPP_RECIPIENT").unwrap_or_default(),
            slack_webhook_url: std::env::var("SLACK_WEBHOOK_URL").unwrap_or_default(),
            smtp_host: std::env::var("SMTP_HOST").unwrap_or_default(),
            smtp_port: std::env::var("SMTP_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(587),
            smtp_username: std::env::var("SMTP_USERNAME").unwrap_or_default(),
            smtp_password: std::env::var("SMTP_PASSWORD").unwrap_or_default(),
            smtp_from: std::env::var("SMTP_FROM").unwrap_or_default(),
            smtp_to: std::env::var("SMTP_TO").unwrap_or_default(),
            client: Client::new(),
        }
    }

    pub async fn send(&self, title: &str, message: &str) {
        // Telegram
        if !self.telegram_bot_token.is_empty() && !self.telegram_chat_id.is_empty() {
            if let Err(e) = self.send_telegram(title, message).await {
                eprintln!("[Notifier] Telegram error: {e}");
            }
        }
        // WhatsApp
        if !self.whatsapp_sid.is_empty() && !self.whatsapp_token.is_empty() {
            if let Err(e) = self.send_whatsapp(title, message).await {
                eprintln!("[Notifier] WhatsApp error: {e}");
            }
        }
        // Slack
        if !self.slack_webhook_url.is_empty() {
            if let Err(e) = self.send_slack(title, message).await {
                eprintln!("[Notifier] Slack error: {e}");
            }
        }
        // Email
        if !self.smtp_host.is_empty() && !self.smtp_to.is_empty() {
            if let Err(e) = self.send_email(title, message).await {
                eprintln!("[Notifier] Email error: {e}");
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
        let url = format!(
            "https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json",
            self.whatsapp_sid
        );
        let recipient = if self.whatsapp_recipient.is_empty() {
            std::env::var("WHATSAPP_RECIPIENT").unwrap_or_default()
        } else {
            self.whatsapp_recipient.clone()
        };
        if recipient.is_empty() {
            return Err("WhatsApp recipient not configured (set WHATSAPP_RECIPIENT)".into());
        }
        let text = format!("{}: {}", title, message);
        let params = [
            ("From", format!("whatsapp:{}", self.whatsapp_from)),
            ("To", format!("whatsapp:{}", recipient)),
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

    async fn send_slack(
        &self,
        title: &str,
        message: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let full_text = format!("*[tredo] {}*\n{}", title, message);
        let body = json!({
            "text": full_text,
            "username": "tredo",
            "icon_emoji": ":chart_with_upwards_trend:",
            "mrkdwn": true
        });
        self.client
            .post(&self.slack_webhook_url)
            .json(&body)
            .send()
            .await?;
        Ok(())
    }

    async fn send_email(
        &self,
        title: &str,
        message: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Uses a simple HTTP-based email API (e.g., Mailgun, SendGrid, or custom SMTP relay).
        // For direct SMTP, we'd need the `lettre` crate — this uses an HTTP API for simplicity.
        let subject = format!("[tredo] {}", title);
        let body_text = format!("{}\n\n---\nSent by tredo monitoring system", message);

        let payload = json!({
            "from": self.smtp_from,
            "to": self.smtp_to,
            "subject": subject,
            "text": body_text
        });

        let _auth = format!("{}:{}", self.smtp_username, self.smtp_password);
        let api_url = format!("https://{}/send", self.smtp_host);

        self.client
            .post(&api_url)
            .header("Content-Type", "application/json")
            .basic_auth(&self.smtp_username, Some(&self.smtp_password))
            .json(&payload)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;

        Ok(())
    }

    /// Send an alert with priority emoji prefix.
    pub async fn alert(&self, level: &str, title: &str, message: &str) {
        let emoji = match level {
            "critical" => "🚨",
            "error" => "🔴",
            "warning" => "⚠️",
            "info" => "ℹ️",
            _ => "📢",
        };
        self.send(&format!("{} {}", emoji, title), message).await;
    }
}

// Convenience for orchestrator / agents
pub async fn alert(title: &str, message: &str) {
    let n = Notifier::from_env();
    n.send(title, message).await;
}
