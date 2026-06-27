use async_trait::async_trait;
use chrono::Utc;
use std::error::Error;

use crate::state::SharedState;
use tredo_core::{calendar::EventImpact, Agent, AgentInput, AgentOutput, DisciplineCheck};

/// SUB AGENT 6 — RedFolderCheckerAgent (Deterministic)
/// Checks for high-impact economic events that warrant trading restrictions
pub struct RedFolderCheckerAgent {
    pub state: SharedState,
    /// Fallback high-impact dates when live calendar is empty (YYYY-MM-DD)
    pub red_folder_dates: Vec<String>,
}

impl RedFolderCheckerAgent {
    pub fn new(state: SharedState) -> Self {
        let red_folder_dates = vec![
            "2026-02-06".to_string(),
            "2026-04-08".to_string(),
            "2026-06-05".to_string(),
            "2026-08-07".to_string(),
            "2026-10-08".to_string(),
            "2026-12-03".to_string(),
            "2026-02-01".to_string(),
            "2026-04-15".to_string(),
            "2026-07-15".to_string(),
            "2026-10-15".to_string(),
            "2026-01-15".to_string(),
        ];

        Self {
            state,
            red_folder_dates,
        }
    }

    fn ist_today() -> String {
        let ist_now =
            Utc::now().with_timezone(&chrono::FixedOffset::east_opt(5 * 3600 + 1800).unwrap());
        ist_now.format("%Y-%m-%d").to_string()
    }

    async fn calendar_red_folder_today(&self) -> Option<String> {
        let today = Self::ist_today();
        let calendar = self.state.calendar_events.read().await;
        calendar
            .iter()
            .find(|e| e.date == today && e.impact == EventImpact::High)
            .map(|e| e.title.clone())
    }

    fn fallback_red_folder_today(&self) -> bool {
        let today = Self::ist_today();
        self.red_folder_dates.contains(&today)
    }

    async fn is_red_folder_today(&self) -> (bool, String) {
        if let Some(title) = self.calendar_red_folder_today().await {
            return (true, title);
        }
        if self.fallback_red_folder_today() {
            return (true, "Known high-impact date (fallback list)".into());
        }
        (false, String::new())
    }

    async fn upcoming_red_folder(&self, days: i64) -> Vec<String> {
        let ist_now =
            Utc::now().with_timezone(&chrono::FixedOffset::east_opt(5 * 3600 + 1800).unwrap());
        let today_naive = ist_now.date_naive();

        let calendar = self.state.calendar_events.read().await;
        let mut upcoming: Vec<String> = calendar
            .iter()
            .filter_map(|event| {
                if event.impact != EventImpact::High {
                    return None;
                }
                let event_date = chrono::NaiveDate::parse_from_str(&event.date, "%Y-%m-%d").ok()?;
                let diff = (event_date - today_naive).num_days();
                if diff > 0 && diff <= days {
                    Some(format!("{} ({})", event.title, event.date))
                } else {
                    None
                }
            })
            .collect();

        for date in &self.red_folder_dates {
            if let Ok(event_date) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") {
                let diff = (event_date - today_naive).num_days();
                if diff > 0 && diff <= days {
                    let label = format!("High-impact event ({})", date);
                    if !upcoming.iter().any(|u| u.contains(date)) {
                        upcoming.push(label);
                    }
                }
            }
        }
        upcoming
    }
}

#[async_trait]
impl Agent for RedFolderCheckerAgent {
    fn name(&self) -> &str {
        "RedFolderCheckerAgent"
    }
    fn tier(&self) -> tredo_core::AgentTier {
        tredo_core::AgentTier::Sub
    }

    async fn run(
        &self,
        _input: Option<AgentInput>,
    ) -> Result<AgentOutput, Box<dyn Error + Send + Sync>> {
        let (is_red, reason) = self.is_red_folder_today().await;
        let upcoming = self.upcoming_red_folder(3).await;

        if is_red {
            println!(
                "[RedFolderChecker] 🚨 RED FOLDER DAY - Trading restricted: {}",
                reason
            );
        } else if !upcoming.is_empty() {
            println!(
                "[RedFolderChecker] ⚠️ Upcoming high-impact event in next 3 days: {:?}",
                upcoming
            );
        } else {
            println!("[RedFolderChecker] ✅ No red folder events today");
        }

        let check = DisciplineCheck {
            passed: !is_red,
            reasons: if is_red {
                vec![format!("Red folder event today: {}", reason)]
            } else {
                vec![]
            },
            confluence_score: None,
        };

        Ok(AgentOutput::RiskResult(check))
    }
}
