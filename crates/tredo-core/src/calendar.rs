use chrono::{Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// Impact level of an economic event
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventImpact {
    Low,
    Medium,
    High,
}

/// A scheduled economic event that the trading system should be aware of
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub title: String,
    pub date: String,         // "YYYY-MM-DD"
    pub time: Option<String>, // "HH:MM" IST / UTC
    pub impact: EventImpact,
    pub currency: String, // "USD", "INR", etc.
    pub description: String,
}

impl CalendarEvent {
    pub fn is_today(&self) -> bool {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        self.date == today
    }

    pub fn is_upcoming(&self, days: i64) -> bool {
        if let Ok(event_date) = NaiveDate::parse_from_str(&self.date, "%Y-%m-%d") {
            let today = Utc::now().date_naive();
            let diff = event_date.signed_duration_since(today).num_days();
            diff >= 0 && diff <= days
        } else {
            false
        }
    }
}

/// Generate a comprehensive list of recurring high-impact economic events.
/// These are approximate dates — in production, fetch from a calendar API.
pub fn generate_economic_calendar() -> Vec<CalendarEvent> {
    let year = Utc::now().year();

    vec![
        // ── US Federal Reserve (FOMC) ──
        CalendarEvent {
            title: "FOMC Interest Rate Decision".into(),
            date: format!("{}-01-29", year),
            time: Some("14:00 EST".into()),
            impact: EventImpact::High,
            currency: "USD".into(),
            description:
                "Federal Reserve interest rate decision — major volatility across all markets"
                    .into(),
        },
        CalendarEvent {
            title: "FOMC Minutes Release".into(),
            date: format!("{}-02-19", year),
            time: Some("14:00 EST".into()),
            impact: EventImpact::High,
            currency: "USD".into(),
            description: "FOMC meeting minutes — hints at future policy direction".into(),
        },
        CalendarEvent {
            title: "FOMC Interest Rate Decision".into(),
            date: format!("{}-03-19", year),
            time: Some("14:00 EST".into()),
            impact: EventImpact::High,
            currency: "USD".into(),
            description: "Federal Reserve interest rate decision".into(),
        },
        CalendarEvent {
            title: "FOMC Interest Rate Decision".into(),
            date: format!("{}-05-07", year),
            time: Some("14:00 EST".into()),
            impact: EventImpact::High,
            currency: "USD".into(),
            description: "Federal Reserve interest rate decision".into(),
        },
        CalendarEvent {
            title: "FOMC Interest Rate Decision".into(),
            date: format!("{}-06-18", year),
            time: Some("14:00 EST".into()),
            impact: EventImpact::High,
            currency: "USD".into(),
            description: "Federal Reserve interest rate decision".into(),
        },
        CalendarEvent {
            title: "FOMC Interest Rate Decision".into(),
            date: format!("{}-07-30", year),
            time: Some("14:00 EST".into()),
            impact: EventImpact::High,
            currency: "USD".into(),
            description: "Federal Reserve interest rate decision".into(),
        },
        CalendarEvent {
            title: "FOMC Interest Rate Decision".into(),
            date: format!("{}-09-17", year),
            time: Some("14:00 EST".into()),
            impact: EventImpact::High,
            currency: "USD".into(),
            description: "Federal Reserve interest rate decision".into(),
        },
        CalendarEvent {
            title: "FOMC Interest Rate Decision".into(),
            date: format!("{}-10-29", year),
            time: Some("14:00 EST".into()),
            impact: EventImpact::High,
            currency: "USD".into(),
            description: "Federal Reserve interest rate decision".into(),
        },
        CalendarEvent {
            title: "FOMC Interest Rate Decision".into(),
            date: format!("{}-12-10", year),
            time: Some("14:00 EST".into()),
            impact: EventImpact::High,
            currency: "USD".into(),
            description: "Federal Reserve interest rate decision".into(),
        },
        // ── US Economic Data ──
        CalendarEvent {
            title: "US Non-Farm Payrolls (NFP)".into(),
            date: format!("{}-02-07", year),
            time: Some("08:30 EST".into()),
            impact: EventImpact::High,
            currency: "USD".into(),
            description: "Monthly jobs report — major USD and equity market mover".into(),
        },
        CalendarEvent {
            title: "US CPI (Inflation)".into(),
            date: format!("{}-02-12", year),
            time: Some("08:30 EST".into()),
            impact: EventImpact::High,
            currency: "USD".into(),
            description: "Consumer Price Index — key inflation gauge".into(),
        },
        CalendarEvent {
            title: "US Non-Farm Payrolls (NFP)".into(),
            date: format!("{}-03-07", year),
            time: Some("08:30 EST".into()),
            impact: EventImpact::High,
            currency: "USD".into(),
            description: "Monthly jobs report".into(),
        },
        CalendarEvent {
            title: "US CPI (Inflation)".into(),
            date: format!("{}-03-12", year),
            time: Some("08:30 EST".into()),
            impact: EventImpact::High,
            currency: "USD".into(),
            description: "Consumer Price Index".into(),
        },
        CalendarEvent {
            title: "US Non-Farm Payrolls (NFP)".into(),
            date: format!("{}-04-04", year),
            time: Some("08:30 EST".into()),
            impact: EventImpact::High,
            currency: "USD".into(),
            description: "Monthly jobs report".into(),
        },
        CalendarEvent {
            title: "US CPI (Inflation)".into(),
            date: format!("{}-04-09", year),
            time: Some("08:30 EST".into()),
            impact: EventImpact::High,
            currency: "USD".into(),
            description: "Consumer Price Index".into(),
        },
        CalendarEvent {
            title: "US Non-Farm Payrolls (NFP)".into(),
            date: format!("{}-05-02", year),
            time: Some("08:30 EST".into()),
            impact: EventImpact::High,
            currency: "USD".into(),
            description: "Monthly jobs report".into(),
        },
        CalendarEvent {
            title: "US CPI (Inflation)".into(),
            date: format!("{}-05-14", year),
            time: Some("08:30 EST".into()),
            impact: EventImpact::High,
            currency: "USD".into(),
            description: "Consumer Price Index".into(),
        },
        CalendarEvent {
            title: "US Non-Farm Payrolls (NFP)".into(),
            date: format!("{}-06-06", year),
            time: Some("08:30 EST".into()),
            impact: EventImpact::High,
            currency: "USD".into(),
            description: "Monthly jobs report".into(),
        },
        CalendarEvent {
            title: "US CPI (Inflation)".into(),
            date: format!("{}-06-11", year),
            time: Some("08:30 EST".into()),
            impact: EventImpact::High,
            currency: "USD".into(),
            description: "Consumer Price Index".into(),
        },
        // ── Indian Market Events ──
        CalendarEvent {
            title: "RBI Monetary Policy".into(),
            date: format!("{}-02-06", year),
            time: Some("10:00 IST".into()),
            impact: EventImpact::High,
            currency: "INR".into(),
            description: "RBI interest rate decision — major NSE/BSE volatility".into(),
        },
        CalendarEvent {
            title: "RBI Monetary Policy".into(),
            date: format!("{}-04-08", year),
            time: Some("10:00 IST".into()),
            impact: EventImpact::High,
            currency: "INR".into(),
            description: "RBI interest rate decision".into(),
        },
        CalendarEvent {
            title: "RBI Monetary Policy".into(),
            date: format!("{}-06-05", year),
            time: Some("10:00 IST".into()),
            impact: EventImpact::High,
            currency: "INR".into(),
            description: "RBI interest rate decision".into(),
        },
        CalendarEvent {
            title: "RBI Monetary Policy".into(),
            date: format!("{}-08-07", year),
            time: Some("10:00 IST".into()),
            impact: EventImpact::High,
            currency: "INR".into(),
            description: "RBI interest rate decision".into(),
        },
        CalendarEvent {
            title: "RBI Monetary Policy".into(),
            date: format!("{}-10-08", year),
            time: Some("10:00 IST".into()),
            impact: EventImpact::High,
            currency: "INR".into(),
            description: "RBI interest rate decision".into(),
        },
        CalendarEvent {
            title: "RBI Monetary Policy".into(),
            date: format!("{}-12-03", year),
            time: Some("10:00 IST".into()),
            impact: EventImpact::High,
            currency: "INR".into(),
            description: "RBI interest rate decision".into(),
        },
        CalendarEvent {
            title: "India Union Budget".into(),
            date: format!("{}-02-01", year),
            time: Some("11:00 IST".into()),
            impact: EventImpact::High,
            currency: "INR".into(),
            description: "Annual Union Budget — major impact on Indian markets".into(),
        },
        // ── Crypto Events ──
        CalendarEvent {
            title: "Bitcoin Halving".into(),
            date: format!("{}-04-20", year),
            time: None,
            impact: EventImpact::High,
            currency: "BTC".into(),
            description: "Bitcoin block reward halving — historically drives bull runs".into(),
        },
    ]
}
