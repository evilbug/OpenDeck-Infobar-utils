//! Shared client for Claude's **subscription** usage endpoint.
//!
//! This backs both the infobar monitor ([`crate::usage`]) and the per-key
//! gauges ([`crate::usage_key`]). It shows the same data as Claude Code's
//! `/usage` command: how much of your Pro/Max/Team plan you have consumed in
//! the rolling 5-hour and 7-day windows.
//!
//! It reads the OAuth access token that Claude Code stores in
//! `~/.claude/.credentials.json` and queries the undocumented endpoint
//! `GET https://api.anthropic.com/api/oauth/usage` — the one that backs
//! `/usage`. No API key is required, and it never spends any tokens: the
//! endpoint only reports plan utilization.
//!
//! Notes on the endpoint, learned the hard way:
//!   * A `User-Agent: claude-code/<version>` header is mandatory. Without it the
//!     request lands in an aggressively rate-limited bucket and returns 429.
//!   * It is safe to poll at ~180s intervals; we default to 300s because the
//!     windows move slowly anyway.
//!   * We re-read the credentials file every tick, so when Claude Code rotates
//!     the access token we pick up the fresh one automatically instead of
//!     trying (and racing) to refresh it ourselves.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::sync::OnceLock;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const USER_AGENT: &str = "claude-code/2.0.0";
pub const DEFAULT_REFRESH_SECS: u64 = 300;
pub const MIN_REFRESH_SECS: u64 = 180;

pub fn default_credentials_path() -> String {
	let home = std::env::var("HOME").unwrap_or_default();
	format!("{home}/.claude/.credentials.json")
}

// ---------------------------------------------------------------------------
// Credentials & API types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CredentialsFile {
	#[serde(rename = "claudeAiOauth")]
	oauth: OauthCredentials,
}

#[derive(Deserialize)]
struct OauthCredentials {
	#[serde(rename = "accessToken")]
	access_token: String,
}

/// One rolling usage window (e.g. the 5-hour or 7-day limit).
#[derive(Deserialize)]
pub struct Window {
	pub utilization: f32,
	pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
pub struct UsageReport {
	pub five_hour: Option<Window>,
	pub seven_day: Option<Window>,
}

/// Why a fetch did not yield usage data.
pub enum FetchError {
	/// No usable credentials — render a "sign in" hint rather than going blank.
	NoAuth(String),
	/// Token rejected — Claude Code likely needs to refresh it.
	Unauthorized,
	/// Network/rate-limit/transient error — keep the previous frame.
	Transient(String),
}

fn http() -> &'static reqwest::Client {
	static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
	CLIENT.get_or_init(reqwest::Client::new)
}

fn read_access_token(path: &str) -> Result<String, FetchError> {
	let raw = std::fs::read_to_string(path).map_err(|_| FetchError::NoAuth("sign in: claude".to_string()))?;
	let creds: CredentialsFile = serde_json::from_str(&raw).map_err(|e| FetchError::NoAuth(format!("bad creds: {e}")))?;
	if creds.oauth.access_token.is_empty() {
		return Err(FetchError::NoAuth("sign in: claude".to_string()));
	}
	Ok(creds.oauth.access_token)
}

pub async fn fetch_usage(path: &str) -> Result<UsageReport, FetchError> {
	let token = read_access_token(path)?;

	let response = http()
		.get(USAGE_URL)
		.bearer_auth(&token)
		.header("User-Agent", USER_AGENT)
		.header("anthropic-beta", "oauth-2025-04-20")
		.send()
		.await
		.map_err(|e| FetchError::Transient(format!("request failed: {e}")))?;

	match response.status() {
		s if s.is_success() => response
			.json::<UsageReport>()
			.await
			.map_err(|e| FetchError::Transient(format!("decode failed: {e}"))),
		reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => Err(FetchError::Unauthorized),
		reqwest::StatusCode::TOO_MANY_REQUESTS => Err(FetchError::Transient("rate limited".to_string())),
		other => Err(FetchError::Transient(format!("http {other}"))),
	}
}

// ---------------------------------------------------------------------------
// Reset-time helpers
// ---------------------------------------------------------------------------

/// Seconds remaining until `resets_at` (never negative).
fn secs_until(resets_at: Option<DateTime<Utc>>) -> Option<i64> {
	resets_at.map(|r| (r - Utc::now()).num_seconds().max(0))
}

/// Time-until-reset rounded UP to the given unit, e.g. `5h` or `2d`.
pub fn reset_in(resets_at: Option<DateTime<Utc>>, unit_secs: i64, suffix: char) -> String {
	match secs_until(resets_at) {
		Some(s) => {
			let n = (s + unit_secs - 1) / unit_secs; // ceil division
			format!("{n}{suffix}")
		}
		None => String::new(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_credentials() {
		let raw = r#"{"claudeAiOauth":{"accessToken":"tok","refreshToken":"r","expiresAt":1780968008718}}"#;
		let creds: CredentialsFile = serde_json::from_str(raw).unwrap();
		assert_eq!(creds.oauth.access_token, "tok");
	}

	#[test]
	fn reset_rounds_up() {
		assert_eq!(reset_in(None, 3_600, 'h'), "");
		// 4h40m of hours, rounded up → 5h.
		let h = Utc::now() + chrono::Duration::minutes(280);
		assert_eq!(reset_in(Some(h), 3_600, 'h'), "5h");
		// 16h20m of days, rounded up → 1d.
		let d = Utc::now() + chrono::Duration::minutes(980);
		assert_eq!(reset_in(Some(d), 86_400, 'd'), "1d");
		// 3d4h of days, rounded up → 4d.
		let d2 = Utc::now() + chrono::Duration::hours(76);
		assert_eq!(reset_in(Some(d2), 86_400, 'd'), "4d");
	}
}
