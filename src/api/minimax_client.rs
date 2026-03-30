use super::minimax_types::*;
use anyhow::Result;
use std::time::Duration;
use ureq::{Agent, Request};

/// MiniMax API client
pub struct MiniMaxApiClient {
    agent: Agent,
    base_url: String,
    token: String,
}

impl MiniMaxApiClient {
    /// Create client from environment variables.
    /// Requires `ANTHROPIC_BASE_URL` containing `minimaxi.com` or `minimax.io`.
    pub fn from_env() -> Result<Self> {
        let token = std::env::var("ANTHROPIC_AUTH_TOKEN")
            .map_err(|_| anyhow::anyhow!("Missing ANTHROPIC_AUTH_TOKEN"))?;

        let base_url = std::env::var("ANTHROPIC_BASE_URL")
            .map_err(|_| anyhow::anyhow!("Missing ANTHROPIC_BASE_URL"))?;

        // Verify it's a MiniMax URL
        if !base_url.contains("minimaxi.com") && !base_url.contains("minimax.io") {
            return Err(anyhow::anyhow!("Not a MiniMax base URL"));
        }

        // Extract domain for monitor API (same domain)
        let monitor_base = extract_domain(&base_url);

        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(5))
            .build();

        Ok(Self {
            agent,
            base_url: monitor_base,
            token,
        })
    }

    /// Fetch coding model usage stats
    pub fn fetch_usage_stats(&self) -> Result<MiniMaxUsageStats> {
        let mut last_error = None;

        for attempt in 0..=2 {
            match self.try_fetch() {
                Ok(stats) => return Ok(stats),
                Err(e) => {
                    last_error = Some(e);
                    if attempt < 2 {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            }
        }

        Err(last_error.unwrap())
    }

    fn try_fetch(&self) -> Result<MiniMaxUsageStats> {
        let url = format!(
            "{}/v1/api/openplatform/coding_plan/remains",
            self.base_url
        );

        let response = self
            .authenticated_request(&url)
            .call()
            .map_err(|e| anyhow::anyhow!("HTTP error: {}", e))?;

        if response.status() != 200 {
            return Err(anyhow::anyhow!("HTTP status {}", response.status()));
        }

        let body: MiniMaxRemainsResponse = response
            .into_json()
            .map_err(|e| anyhow::anyhow!("Parse error: {}", e))?;

        // Filter for coding model: model_name starts with "MiniMax-M"
        let coding_model = body
            .model_remains
            .iter()
            .find(|m| m.model_name.starts_with("MiniMax-M"));

        let model = match coding_model {
            Some(m) => m,
            None => return Err(anyhow::anyhow!("No coding model found in response")),
        };

        let interval_pct = if model.current_interval_total_count > 0 {
            ((model.current_interval_usage_count as f64
                / model.current_interval_total_count as f64)
                * 100.0)
                .round() as u8
        } else {
            0
        };

        // Weekly: only show if weekly_total > 0 (old plans have weekly_total_count=0)
        let (weekly_used, weekly_total, weekly_pct, weekly_reset) =
            if model.current_weekly_total_count > 0 {
                let pct = ((model.current_weekly_usage_count as f64
                    / model.current_weekly_total_count as f64)
                    * 100.0)
                    .round() as u8;
                (
                    Some(model.current_weekly_usage_count),
                    Some(model.current_weekly_total_count),
                    Some(pct),
                    model.weekly_end_time,
                )
            } else {
                (None, None, None, None)
            };

        Ok(MiniMaxUsageStats {
            interval_used: model.current_interval_usage_count,
            interval_total: model.current_interval_total_count,
            interval_pct,
            reset_time: model.end_time,
            weekly_used,
            weekly_total,
            weekly_pct,
            weekly_reset_time: weekly_reset,
        })
    }

    fn authenticated_request(&self, url: &str) -> Request {
        self.agent
            .get(url)
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Content-Type", "application/json")
    }
}

/// Extract scheme + domain from a URL like "https://api.minimaxi.com/anthropic"
fn extract_domain(url: &str) -> String {
    // Find the scheme
    let scheme_end = url.find("://").unwrap_or(0);
    let scheme = if scheme_end > 0 { &url[..scheme_end + 3] } else { "" };

    // Find the end of domain (next /)
    let rest = &url[scheme_end + 3..];
    let domain_end = rest.find('/').unwrap_or(rest.len());
    let domain = &rest[..domain_end];

    format!("{}{}", scheme, domain)
}
