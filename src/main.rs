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

#[tokio::main]
async fn main() -> OpenActionResult<()> {
	{
		use simplelog::*;
		if let Err(error) = TermLogger::init(LevelFilter::Debug, Config::default(), TerminalMode::Stdout, ColorChoice::Never) {
			eprintln!("Logger initialization failed: {error}");
		}
	}

	register_action(InfobarClockAction).await;
	run(std::env::args().collect()).await
}
