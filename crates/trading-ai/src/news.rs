use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use anyhow::Result;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use trading_core::NewsItem;

const CALENDAR_URL: &str = "https://nfs.faireconomy.media/ff_calendar_thisweek.json";
const CACHE_TTL: Duration = Duration::from_secs(3600); // 1 hour

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub title: String,
    pub country: String,
    pub date: String,
    pub impact: String,
    #[serde(default)]
    pub forecast: String,
    #[serde(default)]
    pub previous: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TraderPsychology {
    Patient,
    Confident,
    Defensive,
    NewsShield,
}

impl TraderPsychology {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Patient => "🧘 PATIENT (Waiting for A+ setups)",
            Self::Confident => "⚡ CONFIDENT (High momentum edge)",
            Self::Defensive => "🛡️ DEFENSIVE (Capital preservation active)",
            Self::NewsShield => "🚨 NEWS SHIELD (Freeze: High-Impact Dump/Pump imminent)",
        }
    }
}

#[derive(Clone)]
pub struct NewsRadar {
    client: Client,
    cache: Arc<Mutex<Option<(Instant, Vec<CalendarEvent>)>>>,
}

impl NewsRadar {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .build()
            .unwrap_or_default();
        Self {
            client,
            cache: Arc::new(Mutex::new(None)),
        }
    }

    /// Refresh ForexFactory calendar events if cache has expired.
    pub async fn refresh(&self) -> Result<Vec<CalendarEvent>> {
        {
            let guard = self.cache.lock().unwrap();
            if let Some((t, ref events)) = *guard {
                if t.elapsed() < CACHE_TTL {
                    return Ok(events.clone());
                }
            }
        }

        let resp = self.client.get(CALENDAR_URL).send().await?;
        let events: Vec<CalendarEvent> = resp.json().await?;

        // Filter for currencies we trade: USD, EUR, GBP, JPY
        let relevant: Vec<CalendarEvent> = events
            .into_iter()
            .filter(|e| matches!(e.country.as_str(), "USD" | "EUR" | "GBP" | "JPY" | "All"))
            .collect();

        *self.cache.lock().unwrap() = Some((Instant::now(), relevant.clone()));
        log::info!("NewsRadar: updated {} macro economic events from ForexFactory", relevant.len());
        Ok(relevant)
    }

    /// Get minutes until the next HIGH-impact news for currencies relevant to a symbol.
    pub async fn minutes_until_high_impact(&self, symbol: &str) -> Option<(i64, CalendarEvent)> {
        let events = self.refresh().await.ok()?;
        let now = Utc::now();
        let target_currencies = currencies_for_symbol(symbol);

        let mut next_event: Option<(i64, CalendarEvent)> = None;

        for e in events {
            if e.impact != "High" {
                continue;
            }
            if !target_currencies.contains(&e.country.as_str()) && e.country != "All" {
                continue;
            }
            if let Ok(dt) = DateTime::parse_from_rfc3339(&e.date) {
                let dt_utc = dt.with_timezone(&Utc);
                let diff_mins = (dt_utc - now).num_minutes();
                // Event is upcoming (within the next 12 hours) or occurred in the last 15 mins
                if (-15..=720).contains(&diff_mins) {
                    if next_event.as_ref().map(|(m, _)| diff_mins < *m).unwrap_or(true) {
                        next_event = Some((diff_mins, e));
                    }
                }
            }
        }

        next_event
    }

    /// Check if news shield should freeze new entries for this symbol.
    /// Freezes from 15 minutes BEFORE until 5 minutes AFTER high impact news.
    pub async fn should_freeze_for_news(&self, symbol: &str) -> Option<String> {
        if let Some((mins, event)) = self.minutes_until_high_impact(symbol).await {
            if (-5..=15).contains(&mins) {
                let status_str = if mins >= 0 {
                    format!("in {} mins", mins)
                } else {
                    format!("{} mins ago", mins.abs())
                };
                return Some(format!(
                    "HIGH IMPACT NEWS {}: {} ({})",
                    status_str, event.title, event.country
                ));
            }
        }
        None
    }

    /// Evaluate current synthetic trader psychology / emotion state.
    pub async fn evaluate_psychology(&self, symbol: &str, consecutive_losses: i32) -> TraderPsychology {
        if self.should_freeze_for_news(symbol).await.is_some() {
            return TraderPsychology::NewsShield;
        }

        if consecutive_losses >= 2 {
            return TraderPsychology::Defensive;
        }

        if let Some((mins, _)) = self.minutes_until_high_impact(symbol).await {
            if mins <= 60 {
                // High impact news within 1 hour -> exercise caution
                return TraderPsychology::Defensive;
            }
        }

        TraderPsychology::Patient
    }

    /// Build UI-friendly NewsItem list for the dashboard.
    pub async fn get_dashboard_news(&self) -> Vec<NewsItem> {
        let events = self.refresh().await.unwrap_or_default();
        let now = Utc::now();
        let mut items = Vec::new();

        for e in events {
            if let Ok(dt) = DateTime::parse_from_rfc3339(&e.date) {
                let dt_utc = dt.with_timezone(&Utc);
                let diff_hours = (dt_utc - now).num_hours();
                if (-24..=48).contains(&diff_hours) {
                    let time_str = dt_utc.format("%a %H:%M UTC").to_string();
                    let details = if !e.forecast.is_empty() {
                        format!("{} (Exp: {} / Prev: {})", e.title, e.forecast, e.previous)
                    } else {
                        e.title.clone()
                    };
                    items.push(NewsItem {
                        title: format!("[{}] {}", e.country, details),
                        source: "ForexFactory".to_string(),
                        time: time_str,
                        impact: e.impact.clone(),
                    });
                }
            }
        }

        // Keep highest impact and closest events first
        items.truncate(20);
        items
    }
}

impl Default for NewsRadar {
    fn default() -> Self {
        Self::new()
    }
}

fn currencies_for_symbol(symbol: &str) -> Vec<&'static str> {
    match symbol {
        "EUR_USD" | "EURUSD" => vec!["EUR", "USD"],
        "GBP_USD" | "GBPUSD" => vec!["GBP", "USD"],
        "USD_JPY" | "USDJPY" => vec!["USD", "JPY"],
        "XAU_USD" | "XAUUSD" => vec!["USD"],
        _ => vec!["USD"],
    }
}
