//! Per-instance refresh loop management.
//!
//! The original plugin spawned an un-cancellable `tokio` task in `will_appear`
//! and relied on `get_instance` returning `None` to eventually stop it. That
//! leaked tasks whenever an action was re-added and never reacted to settings
//! changes. Here each instance owns exactly one loop whose handle we keep, so
//! `will_disappear` and `did_receive_settings` can cancel or restart it.

use openaction::{InstanceId, get_instance};
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tokio::task::JoinHandle;

fn registry() -> &'static Mutex<HashMap<InstanceId, JoinHandle<()>>> {
	static REGISTRY: OnceLock<Mutex<HashMap<InstanceId, JoinHandle<()>>>> = OnceLock::new();
	REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Start (or restart) the refresh loop for `instance_id`.
///
/// `render` is called immediately and then every `interval`; whatever data URI
/// it returns is pushed to the instance via `set_image`. Returning `Err`
/// renders nothing for that tick (the previous image stays) and logs the error.
/// Any loop already running for this instance is aborted first, so calling this
/// from `did_receive_settings` cleanly applies new settings.
pub fn start<F, Fut>(instance_id: InstanceId, interval: Duration, render: F)
where
	F: Fn() -> Fut + Send + 'static,
	Fut: Future<Output = Result<String, String>> + Send,
{
	stop(&instance_id);

	let id = instance_id.clone();
	let handle = tokio::spawn(async move {
		loop {
			match render().await {
				Ok(image) => match get_instance(id.clone()).await {
					Some(instance) => {
						if let Err(e) = instance.set_image(Some(image), None).await {
							log::error!("[{id}] set_image failed: {e}");
						}
					}
					// Instance is gone; nothing left to update.
					None => break,
				},
				Err(e) => log::warn!("[{id}] render failed: {e}"),
			}
			tokio::time::sleep(interval).await;
		}
	});

	registry().lock().unwrap().insert(instance_id, handle);
}

/// Abort and forget the loop for `instance_id`, if any.
pub fn stop(instance_id: &InstanceId) {
	if let Some(handle) = registry().lock().unwrap().remove(instance_id) {
		handle.abort();
	}
}
