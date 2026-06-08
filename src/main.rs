use ab_glyph::{FontRef, PxScale};
use base64::{Engine as _, engine::general_purpose};
use chrono::Local;
use image::{Rgba, RgbaImage};
use imageproc::drawing::draw_text_mut;
use openaction::*;
use serde::{Deserialize, Serialize};
use std::io::Cursor;

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct InfobarClockSettings {}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct ClaudeUsageSettings {
	api_key: String,
}

pub struct InfobarClockAction;

fn generate_clock_image() -> Result<String, String> {
	let mut image = RgbaImage::new(248, 58);

	for pixel in image.pixels_mut() {
		*pixel = Rgba([0, 0, 0, 255]);
	}

	let font_data = include_bytes!("../assets/fonts/Roboto-Regular.ttf");
	let font = FontRef::try_from_slice(font_data).map_err(|e| e.to_string())?;

	let now = Local::now();
	let time_str = now.format("%H:%M:%S").to_string();
	let day_str = now.format("%A").to_string().to_uppercase();
	let day_display: String = day_str.chars().take(6).collect();
	let date_str = now.format("%m/%d").to_string();

	let text_color = Rgba([255, 255, 255, 255]);

	let time_scale = PxScale { x: 42.0, y: 42.0 };
	draw_text_mut(&mut image, text_color, 10, 8, time_scale, &font, &time_str);

	let date_scale = PxScale { x: 24.0, y: 24.0 };
	draw_text_mut(&mut image, text_color, 160, 5, date_scale, &font, &date_str);

	let day_scale = PxScale { x: 24.0, y: 24.0 };
	draw_text_mut(&mut image, text_color, 160, 30, day_scale, &font, &day_display);

	let mut buffer = Cursor::new(Vec::new());
	image.write_to(&mut buffer, image::ImageFormat::Png).map_err(|e| e.to_string())?;

	let b64 = general_purpose::STANDARD.encode(buffer.into_inner());
	Ok(format!("data:image/png;base64,{b64}"))
}

#[async_trait]
impl Action for InfobarClockAction {
	const UUID: ActionUuid = "com.evilbug.infobarclock.clock";
	type Settings = InfobarClockSettings;

	async fn will_appear(&self, instance: &Instance, _settings: &Self::Settings) -> OpenActionResult<()> {
		let instance_id = instance.instance_id.clone();

		tokio::spawn(async move {
			loop {
				match generate_clock_image() {
					Ok(b64_image) => {
						if let Some(inst) = openaction::get_instance(instance_id.clone()).await {
							if let Err(e) = inst.set_image(Some(b64_image), None).await {
								log::error!("Failed to set clock image: {e}");
							}
						} else {
							break;
						}
					}
					Err(e) => log::error!("Failed to generate clock image: {e}"),
				}

				tokio::time::sleep(std::time::Duration::from_secs(1)).await;
			}
		});

		Ok(())
	}

	async fn key_down(&self, _instance: &Instance, _settings: &Self::Settings) -> OpenActionResult<()> {
		Ok(())
	}
}

pub struct ClaudeUsageAction;

async fn fetch_claude_usage(api_key: &str) -> Result<(u64, u64), String> {
	let client = reqwest::Client::new();
	let _response = client
		.get("https://api.anthropic.com/v1/messages")
		.header("x-api-key", api_key)
		.header("anthropic-version", "2023-06-01")
		.header("content-type", "application/json")
		.json(&serde_json::json!({
			"model": "claude-3-haiku-20240307",
			"max_tokens": 1,
			"messages": [{"role": "user", "content": "test"}]
		}))
		.send()
		.await
		.map_err(|e| format!("Request failed: {e}"))?;

	// This is a placeholder - actual usage tracking requires session-based monitoring
	// For now, we'll display a mock usage or error
	Ok((0, 0))
}

fn generate_claude_image(input_tokens: u64, output_tokens: u64) -> Result<String, String> {
	let mut image = RgbaImage::new(248, 58);

	for pixel in image.pixels_mut() {
		*pixel = Rgba([0, 0, 0, 255]);
	}

	let font_data = include_bytes!("../assets/fonts/Roboto-Regular.ttf");
	let font = FontRef::try_from_slice(font_data).map_err(|e| e.to_string())?;

	let text_color = Rgba([255, 255, 255, 255]);

	let label_scale = PxScale { x: 18.0, y: 18.0 };
	draw_text_mut(&mut image, text_color, 5, 5, label_scale, &font, "CLAUDE:");

	let input_str = format!("IN: {}", input_tokens);
	let output_str = format!("OUT: {}", output_tokens);

	let value_scale = PxScale { x: 20.0, y: 20.0 };
	draw_text_mut(&mut image, text_color, 5, 30, value_scale, &font, &input_str);
	draw_text_mut(&mut image, text_color, 130, 30, value_scale, &font, &output_str);

	let mut buffer = Cursor::new(Vec::new());
	image.write_to(&mut buffer, image::ImageFormat::Png).map_err(|e| e.to_string())?;

	let b64 = general_purpose::STANDARD.encode(buffer.into_inner());
	Ok(format!("data:image/png;base64,{b64}"))
}

#[async_trait]
impl Action for ClaudeUsageAction {
	const UUID: ActionUuid = "com.evilbug.infobarclock.claude";
	type Settings = ClaudeUsageSettings;

	async fn will_appear(&self, instance: &Instance, settings: &Self::Settings) -> OpenActionResult<()> {
		let instance_id = instance.instance_id.clone();
		let api_key = settings.api_key.clone();

		if api_key.is_empty() {
			if let Some(inst) = openaction::get_instance(instance_id.clone()).await {
				let _ = inst.set_image(Some(generate_claude_image(0, 0).unwrap_or_else(|_| "data:image/png;base64,".to_string())), None).await;
			}
			return Ok(());
		}

		tokio::spawn(async move {
			loop {
				let result = fetch_claude_usage(&api_key).await;
				match result {
					Ok((input, output)) => {
						if let Ok(b64_image) = generate_claude_image(input, output) {
							if let Some(inst) = openaction::get_instance(instance_id.clone()).await {
								if let Err(e) = inst.set_image(Some(b64_image), None).await {
									log::error!("Failed to set Claude usage image: {e}");
								}
							} else {
								break;
							}
						}
					}
					Err(e) => log::error!("Failed to fetch Claude usage: {e}"),
				}

				tokio::time::sleep(std::time::Duration::from_secs(60)).await;
			}
		});

		Ok(())
	}

	async fn key_down(&self, _instance: &Instance, _settings: &Self::Settings) -> OpenActionResult<()> {
		Ok(())
	}
}

#[tokio::main]
async fn main() -> OpenActionResult<()> {
	{
		use simplelog::*;
		if let Err(error) = TermLogger::init(LevelFilter::Debug, Config::default(), TerminalMode::Stdout, ColorChoice::Never) {
			eprintln!("Logger initialization failed: {error}");
		}
	}

	register_action(InfobarClockAction).await;
	register_action(ClaudeUsageAction).await;
	run(std::env::args().collect()).await
}
