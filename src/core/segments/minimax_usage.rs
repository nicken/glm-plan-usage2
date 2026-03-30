use super::Segment;
use crate::api::minimax_client::MiniMaxApiClient;
use crate::api::minimax_types::MiniMaxUsageStats;
use crate::config::{Config, InputData};
use crate::core::segments::{SegmentData, SegmentStyle};
use crate::terminal::{CharMode, TerminalDetector};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Format reset time as absolute time (HH:MM)
fn format_reset_time(reset_at: i64) -> Option<String> {
    use chrono::{DateTime, Local, TimeZone, Timelike};
    let dt: DateTime<Local> = Local.timestamp_opt(reset_at, 0).single()?;
    Some(format!("{}:{:02}", dt.hour(), dt.minute()))
}

/// MiniMax usage segment with caching
pub struct MiniMaxUsageSegment {
    cache: Arc<Mutex<Option<CacheEntry>>>,
    char_mode: CharMode,
}

struct CacheEntry {
    stats: MiniMaxUsageStats,
    timestamp: Instant,
}

impl MiniMaxUsageSegment {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(None)),
            char_mode: TerminalDetector::detect(),
        }
    }

    fn get_usage_stats(&self, config: &Config) -> Option<MiniMaxUsageStats> {
        if config.cache.enabled {
            if let Some(entry) = self.cache.lock().unwrap().as_ref() {
                if entry.timestamp.elapsed() < Duration::from_secs(config.cache.ttl_seconds) {
                    return Some(entry.stats.clone());
                }
            }
        }

        let result: Option<MiniMaxUsageStats> = match MiniMaxApiClient::from_env() {
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

    fn format_stats(stats: &MiniMaxUsageStats, char_mode: CharMode) -> String {
        let (token_icon, clock_icon, chart_icon, calendar_icon) = match char_mode {
            CharMode::Emoji => ("🪙", "⏰", "📊", "📅"),
            CharMode::Ascii => ("$", "T", "#", "%"),
        };

        let mut parts = Vec::new();

        // 5h interval percentage with reset time
        let reset_time = stats
            .reset_time
            .and_then(format_reset_time)
            .unwrap_or_else(|| "--:--".to_string());
        parts.push(format!(
            "{} {}% ({} {})",
            token_icon, stats.interval_pct, clock_icon, reset_time
        ));

        // Call count (used/total)
        parts.push(format!(
            "{} {}/{}",
            chart_icon, stats.interval_used, stats.interval_total
        ));

        // Weekly percentage (only if weekly limit exists)
        if let Some(pct) = stats.weekly_pct {
            parts.push(format!("{} {}%", calendar_icon, pct));
        }

        format!("MiniMax {}", parts.join(" · "))
    }

    fn placeholder_text(&self) -> String {
        let (token_icon, clock_icon, chart_icon, calendar_icon) = match self.char_mode {
            CharMode::Emoji => ("🪙", "⏰", "📊", "📅"),
            CharMode::Ascii => ("$", "T", "#", "%"),
        };
        format!(
            "MiniMax {} % ({} --:--) · {} / · {} %",
            token_icon, clock_icon, chart_icon, calendar_icon
        )
    }
}

impl Default for MiniMaxUsageSegment {
    fn default() -> Self {
        Self::new()
    }
}

impl Segment for MiniMaxUsageSegment {
    fn id(&self) -> &str {
        "minimax_usage"
    }

    fn collect(&self, input: &InputData, config: &Config) -> Option<SegmentData> {
        // Only show for MiniMax models
        if let Some(model) = &input.model {
            let model_id = model.id.to_lowercase();
            if !model_id.contains("minimax") {
                return None;
            }
        }

        let stats = self.get_usage_stats(config);

        let (text, style) = match &stats {
            Some(s) => (
                Self::format_stats(s, self.char_mode),
                SegmentStyle {
                    color_256: Some(208),
                    bold: true,
                    color: None,
                },
            ),
            None => (
                self.placeholder_text(),
                SegmentStyle {
                    color_256: Some(208),
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
