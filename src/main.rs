mod clock;
mod render;
mod tasks;
mod usage;

use clock::InfobarClockAction;
use openaction::*;
use usage::ClaudeUsageAction;

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
