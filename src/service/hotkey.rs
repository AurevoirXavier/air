// std
use std::{
	sync::{
		atomic::{AtomicBool, Ordering},
		Arc,
	},
	time::Duration,
};
// crates.io
use arboard::Clipboard;
use eframe::egui::{Context, ViewportCommand};
use futures::StreamExt;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tokio::{runtime::Runtime, task::AbortHandle, time};
// self
use crate::{
	component::{function::Function, setting::Hotkeys, Components},
	os::*,
	prelude::*,
	service::keyboard::Keyboard,
	state::State,
};

#[derive(Debug)]
pub struct Hotkey {
	abort_handle: AbortHandle,
	is_running: Arc<AtomicBool>,
}
impl Hotkey {
	// TODO: optimize parameters.
	pub fn init(
		ctx: &Context,
		keyboard: Keyboard,
		rt: &Runtime,
		components: &Components,
		state: &State,
	) -> Result<Self> {
		let ctx = ctx.to_owned();
		// TODO: use `state.setting.hotkeys`.
		let manager = Manager::init(&components.setting.hotkeys)?;
		let openai = components.openai.clone();
		let chat_input = state.chat.input.clone();
		let chat_output = state.chat.output.clone();
		let chat_setting = state.setting.chat.clone();
		let is_running = Arc::new(AtomicBool::new(false));
		let is_running_ = is_running.clone();
		let receiver = GlobalHotKeyEvent::receiver();
		let mut clipboard = Clipboard::new()?;
		// TODO: handle the error.
		let abort_handle = rt
			.spawn(async move {
				// The manager need to be kept alive during the whole program life.
				let manager = manager;

				loop {
					is_running_.store(false, Ordering::SeqCst);

					// Block the thread until a hotkey event is received.
					let e = receiver.recv().unwrap();

					// We don't care about the release event.
					if let HotKeyState::Pressed = e.state {
						// TODO: reset the hotkey state so that we don't need to wait for the user
						// to release the keys.

						is_running_.store(true, Ordering::SeqCst);

						let func = manager.match_func(e.id);
						let to_unhide = !func.is_directly();

						if to_unhide {
							Os::unhide();
						}

						// Sleep for a while to reset the keyboard state after user
						// triggers the hotkey.
						time::sleep(Duration::from_millis(1000)).await;

						keyboard.copy();

						// Give some time to the system to refresh the clipboard.
						time::sleep(Duration::from_millis(500)).await;

						let content = match clipboard.get_text() {
							Ok(c) if !c.is_empty() => c,
							_ => continue,
						};

						if to_unhide {
							// Generally, this needs some time to wait the window available
							// first, but the previous sleep in get selected text is enough.
							ctx.send_viewport_cmd(ViewportCommand::Focus);
						}

						chat_input.write().clone_from(&content);
						chat_output.write().clear();

						let chat_setting = chat_setting.read().to_owned();
						let mut stream = openai
							.lock()
							.await
							.chat(&func.prompt(&chat_setting), &content)
							.await
							.unwrap();

						while let Some(r) = stream.next().await {
							for s in r.unwrap().choices.into_iter().filter_map(|c| c.delta.content)
							{
								chat_output.write().push_str(&s);

								// TODO: move to outside of the loop.
								if !to_unhide {
									keyboard.text(s);
								}
							}
						}
					}
				}
			})
			.abort_handle();

		Ok(Self { abort_handle, is_running })
	}

	pub fn abort(&self) {
		self.abort_handle.abort();
	}

	pub fn is_running(&self) -> bool {
		self.is_running.load(Ordering::SeqCst)
	}
}

struct Manager {
	// The manager need to be kept alive during the whole program life.
	_inner: GlobalHotKeyManager,
	ids: [u32; 4],
}
impl Manager {
	fn init(hotkeys: &Hotkeys) -> Result<Self> {
		let _inner = GlobalHotKeyManager::new()?;
		let hotkeys = [
			hotkeys.rewrite,
			hotkeys.rewrite_directly,
			hotkeys.translate,
			hotkeys.translate_directly,
		];

		_inner.register_all(&hotkeys)?;

		let ids = hotkeys.iter().map(|h| h.id).collect::<Vec<_>>().try_into().unwrap();

		Ok(Self { _inner, ids })
	}

	fn match_func(&self, id: u32) -> Function {
		match id {
			i if i == self.ids[0] => Function::Rewrite,
			i if i == self.ids[1] => Function::RewriteDirectly,
			i if i == self.ids[2] => Function::Translate,
			i if i == self.ids[3] => Function::TranslateDirectly,
			_ => unreachable!(),
		}
	}
}
