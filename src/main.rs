mod claude;
mod clock;
mod render;
mod tasks;
mod usage;
mod usage_key;

use clock::InfobarClockAction;
use openaction::*;
use usage::ClaudeUsageAction;
use usage_key::{ClaudeDailyUsageAction, ClaudeWeeklyUsageAction};

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
	register_action(ClaudeDailyUsageAction).await;
	register_action(ClaudeWeeklyUsageAction).await;
	run(std::env::args().collect()).await
}
