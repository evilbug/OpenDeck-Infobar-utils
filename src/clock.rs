//! Infobar clock: time, date and weekday.

use crate::render::{Canvas, WHITE};
use crate::tasks;
use chrono::Local;
use openaction::*;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct InfobarClockSettings {}

fn render() -> Result<String, String> {
	let now = Local::now();
	let day: String = now.format("%A").to_string().to_uppercase().chars().take(6).collect();

	let mut canvas = Canvas::new();
	canvas
		.text(10, 8, 42.0, WHITE, &now.format("%H:%M:%S").to_string())
		.text(160, 5, 24.0, WHITE, &now.format("%m/%d").to_string())
		.text(160, 30, 24.0, WHITE, &day);
	canvas.to_data_uri()
}

pub struct InfobarClockAction;

#[async_trait]
impl Action for InfobarClockAction {
	const UUID: ActionUuid = "com.evilbug.infobarclock.clock";
	type Settings = InfobarClockSettings;

	async fn will_appear(&self, instance: &Instance, _settings: &Self::Settings) -> OpenActionResult<()> {
		tasks::start(instance.instance_id.clone(), Duration::from_secs(1), || async { render() });
		Ok(())
	}

	async fn will_disappear(&self, instance: &Instance, _settings: &Self::Settings) -> OpenActionResult<()> {
		tasks::stop(&instance.instance_id);
		Ok(())
	}
}
