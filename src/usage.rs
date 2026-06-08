//! Claude **subscription** usage monitor.
//!
//! This shows the same data as Claude Code's `/usage` command: how much of your
//! Pro/Max/Team plan you have consumed in the rolling 5-hour and 7-day windows.
//!
//! It reads the OAuth access token that Claude Code stores in
//! `~/.claude/.credentials.json` and queries the undocumented endpoint
//! `GET https://api.anthropic.com/api/oauth/usage` — the one that backs
//! `/usage`. No API key is required, and (unlike the old implementation) it
//! never spends any tokens: the endpoint only reports plan utilization.
//!
//! Notes on the endpoint, learned the hard way:
//!   * A `User-Agent: claude-code/<version>` header is mandatory. Without it the
//!     request lands in an aggressively rate-limited bucket and returns 429.
//!   * It is safe to poll at ~180s intervals; we default to 300s because the
//!     windows move slowly anyway.
//!   * We re-read the credentials file every tick, so when Claude Code rotates
//!     the access token we pick up the fresh one automatically instead of
//!     trying (and racing) to refresh it ourselves.

use crate::render::{Canvas, DIM, WHITE, WIDTH, text_width, usage_color};
use crate::tasks;
use chrono::{DateTime, Utc};
use openaction::*;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::Duration;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const USER_AGENT: &str = "claude-code/2.0.0";
const DEFAULT_REFRESH_SECS: u64 = 300;
const MIN_REFRESH_SECS: u64 = 180;

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct ClaudeUsageSettings {
	/// How often to refresh, in seconds. Clamped to a 180s minimum.
	refresh_secs: Option<u64>,
	/// Override for the credentials file (defaults to `~/.claude/.credentials.json`).
	credentials_path: Option<String>,
}

impl ClaudeUsageSettings {
	fn interval(&self) -> Duration {
		Duration::from_secs(self.refresh_secs.unwrap_or(DEFAULT_REFRESH_SECS).max(MIN_REFRESH_SECS))
	}

	fn credentials_path(&self) -> String {
		self.credentials_path.clone().unwrap_or_else(default_credentials_path)
	}
}

fn default_credentials_path() -> String {
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
struct Window {
	utilization: f32,
	resets_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
struct UsageReport {
	five_hour: Option<Window>,
	seven_day: Option<Window>,
}

/// Why a fetch did not yield usage data.
enum FetchError {
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

async fn fetch_usage(path: &str) -> Result<UsageReport, FetchError> {
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
// Rendering
// ---------------------------------------------------------------------------

/// Seconds remaining until `resets_at` (never negative).
fn secs_until(resets_at: Option<DateTime<Utc>>) -> Option<i64> {
	resets_at.map(|r| (r - Utc::now()).num_seconds().max(0))
}

/// Time-until-reset rounded UP to the given unit, e.g. `5h` or `2d`.
fn reset_in(resets_at: Option<DateTime<Utc>>, unit_secs: i64, suffix: char) -> String {
	match secs_until(resets_at) {
		Some(s) => {
			let n = (s + unit_secs - 1) / unit_secs; // ceil division
			format!("{n}{suffix}")
		}
		None => String::new(),
	}
}

const CY: f32 = 29.0;
const R_OUTER: f32 = 27.0;
const R_INNER: f32 = 22.0;
const LETTER_SIZE: f32 = 20.0;
const NUM_SIZE: f32 = 26.0;
/// `%` font: big for 1–2 digit values, smaller for `100%` (one extra char).
const PCT_SIZE_BIG: f32 = 22.0;
const PCT_SIZE_SMALL: f32 = 18.0;

/// Draw one circular usage gauge centered on `cx`, with only the usage `%`
/// inside it (the window's letter is drawn to the left by `render_report`).
fn draw_gauge(canvas: &mut Canvas, cx: f32, window: Option<&Window>) {
	let percent = window.map(|w| w.utilization).unwrap_or(0.0);
	let color = usage_color(percent);
	canvas.gauge(cx, CY, R_OUTER, R_INNER, percent / 100.0, color);

	let pct = if window.is_some() {
		format!("{}%", percent.round() as i64)
	} else {
		"--".to_string()
	};
	// Shrink only when the string gets longer (i.e. "100%"); keep it big for "XX%".
	let size = if pct.chars().count() >= 4 { PCT_SIZE_SMALL } else { PCT_SIZE_BIG };
	let top = (CY - size * 0.5).round() as i32;
	canvas.text_centered(cx as i32, top, size, color, &pct);
}

fn render_report(report: &UsageReport) -> Result<String, String> {
	let mut canvas = Canvas::new();

	// Each group is `LETTER (gauge%) reset`: daily = 5-hour window (reset in
	// hours), weekly = 7-day window (reset in days).
	let daily = report.five_hour.as_ref().map(|w| reset_in(w.resets_at, 3_600, 'h')).unwrap_or_default();
	let weekly = report.seven_day.as_ref().map(|w| reset_in(w.resets_at, 86_400, 'd')).unwrap_or_default();

	let gw = (R_OUTER * 2.0) as i32; // gauge box width
	let gap = 5;
	let group_gap = 14;

	// Measure everything so the whole row can be horizontally centered.
	let total = text_width("D", LETTER_SIZE)
		+ gap + gw + gap
		+ text_width(&daily, NUM_SIZE)
		+ group_gap
		+ text_width("W", LETTER_SIZE)
		+ gap + gw + gap
		+ text_width(&weekly, NUM_SIZE);
	let mut x = (WIDTH as i32 - total) / 2;

	canvas.text(x, 19, LETTER_SIZE, DIM, "D");
	x += text_width("D", LETTER_SIZE) + gap;
	draw_gauge(&mut canvas, (x + gw / 2) as f32, report.five_hour.as_ref());
	x += gw + gap;
	canvas.text(x, 16, NUM_SIZE, WHITE, &daily);
	x += text_width(&daily, NUM_SIZE) + group_gap;

	canvas.text(x, 19, LETTER_SIZE, DIM, "W");
	x += text_width("W", LETTER_SIZE) + gap;
	draw_gauge(&mut canvas, (x + gw / 2) as f32, report.seven_day.as_ref());
	x += gw + gap;
	canvas.text(x, 16, NUM_SIZE, WHITE, &weekly);

	canvas.to_data_uri()
}

/// A centered two-line message used for the "no auth" / "expired" states.
fn render_message(title: &str, detail: &str) -> Result<String, String> {
	let mut canvas = Canvas::new();
	canvas.text(6, 5, 24.0, WHITE, title);
	canvas.text(6, 35, 16.0, DIM, detail);
	canvas.to_data_uri()
}

/// Produce the next frame, mapping fetch outcomes to either an image (Ok) or a
/// transient error (Err, which keeps the previous frame on screen).
async fn render_frame(path: &str) -> Result<String, String> {
	match fetch_usage(path).await {
		Ok(report) => render_report(&report),
		Err(FetchError::NoAuth(hint)) => render_message("Claude", &hint),
		Err(FetchError::Unauthorized) => render_message("Claude", "auth expired"),
		Err(FetchError::Transient(e)) => Err(e),
	}
}

// ---------------------------------------------------------------------------
// Action
// ---------------------------------------------------------------------------

pub struct ClaudeUsageAction;

impl ClaudeUsageAction {
	fn run(instance: &Instance, settings: &ClaudeUsageSettings) {
		let path = settings.credentials_path();
		tasks::start(instance.instance_id.clone(), settings.interval(), move || {
			let path = path.clone();
			async move { render_frame(&path).await }
		});
	}
}

#[async_trait]
impl Action for ClaudeUsageAction {
	const UUID: ActionUuid = "com.evilbug.infobarclock.claude";
	type Settings = ClaudeUsageSettings;

	async fn will_appear(&self, instance: &Instance, settings: &Self::Settings) -> OpenActionResult<()> {
		Self::run(instance, settings);
		Ok(())
	}

	async fn did_receive_settings(&self, instance: &Instance, settings: &Self::Settings) -> OpenActionResult<()> {
		// Restart the loop so a new interval or credentials path takes effect.
		Self::run(instance, settings);
		Ok(())
	}

	async fn will_disappear(&self, instance: &Instance, _settings: &Self::Settings) -> OpenActionResult<()> {
		tasks::stop(&instance.instance_id);
		Ok(())
	}

	async fn key_down(&self, instance: &Instance, settings: &Self::Settings) -> OpenActionResult<()> {
		// Pressing the key forces an immediate refresh.
		let path = settings.credentials_path();
		if let Some(inst) = openaction::get_instance(instance.instance_id.clone()).await {
			match render_frame(&path).await {
				Ok(image) => {
					let _ = inst.set_image(Some(image), None).await;
				}
				Err(e) => {
					log::warn!("manual refresh failed: {e}");
					let _ = inst.show_alert().await;
				}
			}
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	// Captured verbatim from a real `/api/oauth/usage` response.
	const SAMPLE: &str = r#"{
		"five_hour": { "utilization": 4.0, "resets_at": "2026-06-09T00:20:01.022423+00:00" },
		"seven_day": { "utilization": 55.0, "resets_at": "2026-06-09T12:00:00.022448+00:00" },
		"seven_day_oauth_apps": null,
		"seven_day_opus": null,
		"extra_usage": { "is_enabled": false, "monthly_limit": null, "used_credits": null, "utilization": null }
	}"#;

	#[test]
	fn parses_real_response() {
		let report: UsageReport = serde_json::from_str(SAMPLE).expect("deserializes");
		assert_eq!(report.five_hour.unwrap().utilization, 4.0);
		let seven = report.seven_day.unwrap();
		assert_eq!(seven.utilization, 55.0);
		assert!(seven.resets_at.is_some());
	}

	#[test]
	fn renders_a_valid_png_data_uri() {
		let report: UsageReport = serde_json::from_str(SAMPLE).unwrap();
		let uri = render_report(&report).expect("renders");
		assert!(uri.starts_with("data:image/png;base64,"));
		assert!(uri.len() > 100);
	}

	#[test]
	fn parses_credentials() {
		let raw = r#"{"claudeAiOauth":{"accessToken":"tok","refreshToken":"r","expiresAt":1780968008718}}"#;
		let creds: CredentialsFile = serde_json::from_str(raw).unwrap();
		assert_eq!(creds.oauth.access_token, "tok");
	}

	fn dump_preview(json: &str, path: &str) {
		use base64::{Engine as _, engine::general_purpose};
		let report: UsageReport = serde_json::from_str(json).unwrap();
		let uri = render_report(&report).unwrap();
		let b64 = uri.strip_prefix("data:image/png;base64,").unwrap();
		let bytes = general_purpose::STANDARD.decode(b64).unwrap();
		std::fs::write(path, bytes).unwrap();
	}

	#[test]
	#[ignore = "writes preview PNGs to /tmp for visual inspection"]
	fn write_preview() {
		dump_preview(SAMPLE, "/tmp/claude_usage_preview.png");
		let full = r#"{ "five_hour": { "utilization": 100.0, "resets_at": "2026-06-09T23:00:00+00:00" },
			"seven_day": { "utilization": 100.0, "resets_at": "2026-06-15T00:00:00+00:00" } }"#;
		dump_preview(full, "/tmp/claude_usage_preview_full.png");
	}

	#[test]
	fn percents_fit_inside_gauge() {
		// Both the big 2-digit size and the smaller "100%" size must fit the
		// inner circle (diameter = 2 * R_INNER).
		let inner_diameter = (R_INNER * 2.0) as i32;
		let big = text_width("99%", PCT_SIZE_BIG);
		let small = text_width("100%", PCT_SIZE_SMALL);
		assert!(big <= inner_diameter, "99% is {big}px wide but inner circle is {inner_diameter}px");
		assert!(small <= inner_diameter, "100% is {small}px wide but inner circle is {inner_diameter}px");
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
