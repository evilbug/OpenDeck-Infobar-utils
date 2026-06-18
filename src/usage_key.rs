//! Claude subscription usage as a single circular gauge on a normal key.
//!
//! Two flavors — [`ClaudeDailyUsageAction`] (the rolling 5-hour window) and
//! [`ClaudeWeeklyUsageAction`] (the rolling 7-day window) — share everything
//! but which window they read and the unit of the reset countdown. Each key
//! shows a donut whose fill is the utilization % and the time left until that
//! window resets in the center. The key caption is left to the host's native
//! title overlay. An optional alert color takes over past a load threshold.
//!
//! The actual API/credentials handling lives in [`crate::claude`].

use crate::claude::{self, FetchError, UsageReport, Window};
use crate::render::{Canvas, DIM, WHITE, parse_hex_color};
use crate::tasks;
use image::Rgba;
use openaction::*;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct ClaudeKeyUsageSettings {
	/// How often to refresh, in seconds. Clamped to a 180s minimum.
	refresh_secs: Option<u64>,
	/// Override for the credentials file (defaults to `~/.claude/.credentials.json`).
	credentials_path: Option<String>,
	/// `#RRGGBB` fill color for the arc (defaults to green).
	color: Option<String>,
	/// `#RRGGBB` alert color used once `alert_threshold` is reached (defaults to red).
	alert_color: Option<String>,
	/// Utilization % at which the alert color kicks in (defaults to 75; set 100 to
	/// effectively disable it).
	alert_threshold: Option<f32>,
}

/// Default arc color when none is configured (matches the green in `usage_color`).
const DEFAULT_COLOR: Rgba<u8> = Rgba([55, 200, 110, 255]);
/// Default alert color (matches the red in `usage_color`).
const DEFAULT_ALERT_COLOR: Rgba<u8> = Rgba([235, 60, 45, 255]);
const DEFAULT_ALERT_THRESHOLD: f32 = 75.0;

/// Resolved drawing style for one frame.
#[derive(Clone, Copy)]
struct Style {
	/// Base fill color.
	color: Rgba<u8>,
	/// Alert fill that overrides `color` at/above `threshold`, if set.
	alert: Option<Rgba<u8>>,
	threshold: f32,
}

impl Style {
	/// The arc color for a given utilization: the alert color past the
	/// threshold, otherwise the base color.
	fn fill(&self, percent: f32) -> Rgba<u8> {
		match self.alert {
			Some(alert) if percent >= self.threshold => alert,
			_ => self.color,
		}
	}
}

impl ClaudeKeyUsageSettings {
	fn interval(&self) -> Duration {
		Duration::from_secs(self.refresh_secs.unwrap_or(claude::DEFAULT_REFRESH_SECS).max(claude::MIN_REFRESH_SECS))
	}

	fn credentials_path(&self) -> String {
		self.credentials_path.clone().unwrap_or_else(claude::default_credentials_path)
	}

	fn style(&self) -> Style {
		Style {
			color: self.color.as_deref().and_then(parse_hex_color).unwrap_or(DEFAULT_COLOR),
			alert: Some(self.alert_color.as_deref().and_then(parse_hex_color).unwrap_or(DEFAULT_ALERT_COLOR)),
			threshold: self.alert_threshold.unwrap_or(DEFAULT_ALERT_THRESHOLD),
		}
	}
}

// ---------------------------------------------------------------------------
// Which window a key tracks
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum WindowKind {
	/// The rolling 5-hour window; reset shown in hours.
	Daily,
	/// The rolling 7-day window; reset shown in days.
	Weekly,
}

impl WindowKind {
	fn select<'a>(self, report: &'a UsageReport) -> Option<&'a Window> {
		match self {
			WindowKind::Daily => report.five_hour.as_ref(),
			WindowKind::Weekly => report.seven_day.as_ref(),
		}
	}

	/// Time-until-reset for this window: hours for daily, days for weekly.
	fn reset_label(self, window: &Window) -> String {
		match self {
			WindowKind::Daily => claude::reset_in(window.resets_at, 3_600, 'h'),
			WindowKind::Weekly => claude::reset_in(window.resets_at, 86_400, 'd'),
		}
	}
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Square key resolution. The Stream Deck scales this down to the physical key.
const SIZE: u32 = 144;

/// Draw the gauge key: a donut filled to `percent` with `center` text inside
/// it. The caption is handled by the host's native key title, not drawn here.
fn render_gauge(percent: f32, has_data: bool, center: &str, fill: Rgba<u8>) -> Result<String, String> {
	let mut canvas = Canvas::with_size(SIZE, SIZE);

	let cx = SIZE as f32 / 2.0;
	let cy = SIZE as f32 / 2.0;
	let r_outer = 62.0;
	let r_inner = 49.0;

	canvas.gauge(cx, cy, r_outer, r_inner, percent / 100.0, fill);

	// Center: time-to-reset, or "--" when the window/data is unavailable.
	let label = if has_data && !center.is_empty() { center } else { "--" };
	let size = if label.chars().count() >= 4 { 36.0 } else { 46.0 };
	let top = (cy - size * 0.5).round() as i32;
	canvas.text_centered(cx as i32, top, size, WHITE, label);

	canvas.to_data_uri()
}

/// A centered two-line message for the "no auth" / "expired" states.
fn render_message(title: &str, detail: &str) -> Result<String, String> {
	let mut canvas = Canvas::with_size(SIZE, SIZE);
	canvas.text_centered((SIZE / 2) as i32, 48, 28.0, WHITE, title);
	canvas.text_centered((SIZE / 2) as i32, 84, 18.0, DIM, detail);
	canvas.to_data_uri()
}

/// Produce the next frame for one window kind. `Err` keeps the previous frame.
async fn render_frame(kind: WindowKind, path: &str, style: &Style) -> Result<String, String> {
	match claude::fetch_usage(path).await {
		Ok(report) => {
			let window = kind.select(&report);
			let percent = window.map(|w| w.utilization).unwrap_or(0.0);
			let center = window.map(|w| kind.reset_label(w)).unwrap_or_default();
			render_gauge(percent, window.is_some(), &center, style.fill(percent))
		}
		Err(FetchError::NoAuth(hint)) => render_message("Claude", &hint),
		Err(FetchError::Unauthorized) => render_message("Claude", "auth expired"),
		Err(FetchError::Transient(e)) => Err(e),
	}
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

fn start(kind: WindowKind, instance: &Instance, settings: &ClaudeKeyUsageSettings) {
	let path = settings.credentials_path();
	let style = settings.style();
	tasks::start(instance.instance_id.clone(), settings.interval(), move || {
		let path = path.clone();
		async move { render_frame(kind, &path, &style).await }
	});
}

async fn manual_refresh(kind: WindowKind, instance: &Instance, settings: &ClaudeKeyUsageSettings) {
	let path = settings.credentials_path();
	let style = settings.style();
	if let Some(inst) = openaction::get_instance(instance.instance_id.clone()).await {
		match render_frame(kind, &path, &style).await {
			Ok(image) => {
				let _ = inst.set_image(Some(image), None).await;
			}
			Err(e) => {
				log::warn!("manual refresh failed: {e}");
				let _ = inst.show_alert().await;
			}
		}
	}
}

/// Generates an `Action` impl that delegates to the shared helpers for a window.
macro_rules! usage_key_action {
	($ty:ident, $uuid:literal, $kind:expr) => {
		pub struct $ty;

		#[async_trait]
		impl Action for $ty {
			const UUID: ActionUuid = $uuid;
			type Settings = ClaudeKeyUsageSettings;

			async fn will_appear(&self, instance: &Instance, settings: &Self::Settings) -> OpenActionResult<()> {
				start($kind, instance, settings);
				Ok(())
			}

			async fn did_receive_settings(&self, instance: &Instance, settings: &Self::Settings) -> OpenActionResult<()> {
				start($kind, instance, settings);
				Ok(())
			}

			async fn will_disappear(&self, instance: &Instance, _settings: &Self::Settings) -> OpenActionResult<()> {
				tasks::stop(&instance.instance_id);
				Ok(())
			}

			async fn key_down(&self, instance: &Instance, settings: &Self::Settings) -> OpenActionResult<()> {
				manual_refresh($kind, instance, settings).await;
				Ok(())
			}
		}
	};
}

usage_key_action!(ClaudeDailyUsageAction, "com.evilbug.infobarclock.claude.daily", WindowKind::Daily);
usage_key_action!(ClaudeWeeklyUsageAction, "com.evilbug.infobarclock.claude.weekly", WindowKind::Weekly);

#[cfg(test)]
mod tests {
	use super::*;

	const SAMPLE: &str = r#"{
		"five_hour": { "utilization": 4.0, "resets_at": "2026-06-09T00:20:01.022423+00:00" },
		"seven_day": { "utilization": 55.0, "resets_at": "2026-06-09T12:00:00.022448+00:00" }
	}"#;

	#[test]
	fn daily_selects_five_hour_window() {
		let report: UsageReport = serde_json::from_str(SAMPLE).unwrap();
		assert_eq!(WindowKind::Daily.select(&report).unwrap().utilization, 4.0);
		assert_eq!(WindowKind::Weekly.select(&report).unwrap().utilization, 55.0);
	}

	#[test]
	fn renders_a_valid_png_data_uri() {
		let uri = render_gauge(55.0, true, "2d", Rgba([10, 132, 255, 255])).expect("renders");
		assert!(uri.starts_with("data:image/png;base64,"));
		assert!(uri.len() > 100);
	}

	#[test]
	fn renders_without_data() {
		let uri = render_gauge(0.0, false, "", DEFAULT_COLOR).expect("renders");
		assert!(uri.starts_with("data:image/png;base64,"));
	}

	#[test]
	fn alert_color_overrides_past_threshold() {
		let base = Rgba([10, 132, 255, 255]);
		let alert = Rgba([235, 60, 45, 255]);
		let style = Style { color: base, alert: Some(alert), threshold: 75.0 };
		assert_eq!(style.fill(50.0), base, "below threshold keeps the base color");
		assert_eq!(style.fill(75.0), alert, "at the threshold switches to alert");
		assert_eq!(style.fill(90.0), alert, "above threshold stays on alert");

		// No alert color configured → always the base color.
		let plain = Style { color: base, alert: None, threshold: 75.0 };
		assert_eq!(plain.fill(99.0), base);
	}

	#[test]
	fn style_uses_defaults_without_settings() {
		let style = ClaudeKeyUsageSettings::default().style();
		assert_eq!(style.color, DEFAULT_COLOR);
		assert_eq!(style.alert, Some(DEFAULT_ALERT_COLOR));
		assert_eq!(style.threshold, DEFAULT_ALERT_THRESHOLD);
		// Defaults: green below 75%, red at/above.
		assert_eq!(style.fill(74.0), DEFAULT_COLOR);
		assert_eq!(style.fill(75.0), DEFAULT_ALERT_COLOR);
	}

	#[test]
	fn message_frame_is_valid() {
		let uri = render_message("Claude", "sign in: claude").expect("renders");
		assert!(uri.starts_with("data:image/png;base64,"));
	}

	fn dump(uri: &str, path: &str) {
		use base64::{Engine as _, engine::general_purpose};
		let b64 = uri.strip_prefix("data:image/png;base64,").unwrap();
		let bytes = general_purpose::STANDARD.decode(b64).unwrap();
		std::fs::write(path, bytes).unwrap();
	}

	#[test]
	#[ignore = "writes preview PNGs to /tmp for visual inspection"]
	fn write_preview() {
		let green = DEFAULT_COLOR;
		let red = Rgba([235, 60, 45, 255]);
		let style = Style { color: green, alert: Some(red), threshold: 75.0 };
		// Low load → base green.
		dump(&render_gauge(4.0, true, "5h", style.fill(4.0)).unwrap(), "/tmp/claude_key_daily.png");
		// Mid load (still below threshold) → base green.
		dump(&render_gauge(55.0, true, "2d", style.fill(55.0)).unwrap(), "/tmp/claude_key_weekly.png");
		// High load (past threshold) → alert red.
		dump(&render_gauge(96.0, true, "1h", style.fill(96.0)).unwrap(), "/tmp/claude_key_full.png");
	}
}
