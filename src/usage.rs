//! Claude **subscription** usage monitor for the infobar.
//!
//! Draws both the 5-hour ("daily") and 7-day ("weekly") windows side by side on
//! the infobar strip. The per-key gauges live in [`crate::usage_key`]; the
//! shared API client lives in [`crate::claude`].

use crate::claude::{self, FetchError, UsageReport, Window};
use crate::render::{Canvas, DIM, WHITE, WIDTH, text_width, usage_color};
use crate::tasks;
use openaction::*;
use serde::{Deserialize, Serialize};
use std::time::Duration;

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
		Duration::from_secs(self.refresh_secs.unwrap_or(claude::DEFAULT_REFRESH_SECS).max(claude::MIN_REFRESH_SECS))
	}

	fn credentials_path(&self) -> String {
		self.credentials_path.clone().unwrap_or_else(claude::default_credentials_path)
	}
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

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
	let daily = report.five_hour.as_ref().map(|w| claude::reset_in(w.resets_at, 3_600, 'h')).unwrap_or_default();
	let weekly = report.seven_day.as_ref().map(|w| claude::reset_in(w.resets_at, 86_400, 'd')).unwrap_or_default();

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
	match claude::fetch_usage(path).await {
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
	fn percents_fit_inside_gauge() {
		// Both the big 2-digit size and the smaller "100%" size must fit the
		// inner circle (diameter = 2 * R_INNER).
		let inner_diameter = (R_INNER * 2.0) as i32;
		let big = text_width("99%", PCT_SIZE_BIG);
		let small = text_width("100%", PCT_SIZE_SMALL);
		assert!(big <= inner_diameter, "99% is {big}px wide but inner circle is {inner_diameter}px");
		assert!(small <= inner_diameter, "100% is {small}px wide but inner circle is {inner_diameter}px");
	}
}
