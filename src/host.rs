//! What the library asks the application that hosts it.
//!
//! Install one [`Host`] at start-up with [`install`]; without one the library behaves as on a
//! system that reports nothing: motion and transparency are not reduced, and a context menu
//! opens with Shift+F10.

use std::sync::OnceLock;

use egui::{InputState, Key, Modifiers};

/// The application's side of the library.
pub trait Host: Send + Sync {
    /// The operating system's reduce-motion setting; `None` when it cannot be read. Called on a
    /// background thread, as reading it may run a helper program.
    fn prefers_reduced_motion(&self) -> Option<bool> {
        None
    }

    /// The operating system's reduce-transparency setting, as [`Host::prefers_reduced_motion`].
    fn prefers_reduced_transparency(&self) -> Option<bool> {
        None
    }

    /// Consumes the key press that opens a context menu from the keyboard, if `input` holds one.
    fn consume_context_menu_key(&self, input: &mut InputState) -> bool {
        input.consume_key(Modifiers::SHIFT, Key::F10)
    }
}

static HOST: OnceLock<&'static dyn Host> = OnceLock::new();

/// Installs the host. The first call wins; later ones are ignored.
pub fn install(host: &'static dyn Host) {
    let _ = HOST.set(host);
}

/// The installed host, if any.
pub(crate) fn get() -> Option<&'static dyn Host> {
    HOST.get().copied()
}

/// Consumes the context-menu key press through the installed host, else Shift+F10.
pub(crate) fn consume_context_menu_key(input: &mut InputState) -> bool {
    match get() {
        Some(host) => host.consume_context_menu_key(input),
        None => input.consume_key(Modifiers::SHIFT, Key::F10),
    }
}
