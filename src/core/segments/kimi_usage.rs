use super::Segment;
use crate::api::kimi_client::KimiApiClient;
use crate::api::kimi_types::KimiUsageStats;
use crate::config::{Config, InputData};
use crate::core::segments::{SegmentData, SegmentStyle};
use crate::terminal::{CharMode, TerminalDetector};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Format ISO 8601 reset time string as HH:MM in local time
fn format_iso_reset_time(iso_str: &str) -> Option<String> {
    use chrono::{DateTime, Local, Timelike};
    let dt: DateTime<chrono::FixedOffset> = chrono::DateTime::parse_from_rfc3339(iso_str).ok()?;
    let local: DateTime<Local> = dt.with_timezone(&Local);
    Some(format!("{}:{:02}", local.hour(), local.minute()))
}

/// Kimi usage segment with caching
pub struct KimiUsageSegment {
    cache: Arc<Mutex<Option<CacheEntry>>>,
    char_mode: CharMode,
}

struct CacheEntry {
    stats: KimiUsageStats,
    timestamp: Instant,
}

impl KimiUsageSegment {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(None)),
            char_mode: TerminalDetector::detect(),
        }
    }

    fn get_usage_stats(&self, config: &Config) -> Option<KimiUsageStats> {
        if config.cache.enabled {
            if let Some(entry) = self.cache.lock().unwrap().as_ref() {
                if entry.timestamp.elapsed() < Duration::from_secs(config.cache.ttl_seconds) {
                    return Some(entry.stats.clone());
                }
            }
        }

        let result: Option<KimiUsageStats> = match KimiApiClient::from_env() {
            Ok(client) => match client.fetch_usage_stats().ok() {
                Some(stats) => {
                    if config.cache.enabled {
                        *self.cache.lock().unwrap() = Some(CacheEntry {
                            stats: stats.clone(),
                            timestamp: Instant::now(),
                        });
                    }
                    Some(stats)
                }
                None => self.cache.lock().unwrap().as_ref().map(|e| e.stats.clone()),
            },
            Err(_) => None,
        };
        result
    }

    fn format_stats(stats: &KimiUsageStats, char_mode: CharMode) -> String {
        let (token_icon, clock_icon, calendar_icon) = match char_mode {
            CharMode::Emoji => ("🔋", "⏰", "📅"),
            CharMode::Ascii => ("$", "T", "%"),
        };

        let mut parts = Vec::new();

        // 5h percentage with reset time
        let reset_time = stats
            .five_hour_reset
            .as_deref()
            .and_then(format_iso_reset_time)
            .unwrap_or_else(|| "--:--".to_string());
        parts.push(format!(
            "{} {}% · {} {}",
            token_icon, stats.five_hour_pct, clock_icon, reset_time
        ));

        // Weekly percentage (Kimi always has weekly)
        parts.push(format!("{} {}%", calendar_icon, stats.weekly_pct));

        format!("Kimi {}", parts.join(" · "))
    }

    fn placeholder_text(&self) -> String {
        let (token_icon, clock_icon, calendar_icon) = match self.char_mode {
            CharMode::Emoji => ("🔋", "⏰", "📅"),
            CharMode::Ascii => ("$", "T", "%"),
        };
        format!(
            "Kimi {} % · {} --:-- · {} %",
            token_icon, clock_icon, calendar_icon
        )
    }
}

impl Default for KimiUsageSegment {
    fn default() -> Self {
        Self::new()
    }
}

impl Segment for KimiUsageSegment {
    fn id(&self) -> &str {
        "kimi_usage"
    }

    fn collect(&self, input: &InputData, config: &Config) -> Option<SegmentData> {
        // Only show for Kimi models
        if let Some(model) = &input.model {
            let model_id = model.id.to_lowercase();
            if !model_id.contains("kimi") {
                return None;
            }
        }

        let stats = self.get_usage_stats(config);

        let (text, style) = match &stats {
            Some(s) => (
                Self::format_stats(s, self.char_mode),
                SegmentStyle {
                    color_256: Some(79),
                    bold: true,
                    color: None,
                },
            ),
            None => (
                self.placeholder_text(),
                SegmentStyle {
                    color_256: Some(79),
                    bold: true,
                    color: None,
                },
            ),
        };

        if text.is_empty() {
            None
        } else {
            Some(SegmentData { text, style })
        }
    }
}
