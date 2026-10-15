//! The keys a component shows for the action it triggers.

/// A shortcut hint such as `"Ctrl+Shift+F · f"`, as menus, tooltips and field hints show it. The
/// application builds one from its own key table (`impl From<AppCommand> for Shortcut`), so the
/// library needs no list of actions.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Shortcut(String);

impl Shortcut {
    /// A hint: chords joined by " · " (the chord that works everywhere, then the bare key).
    pub fn new(hint: impl Into<String>) -> Self {
        Self(hint.into())
    }

    /// The hint text; empty when nothing is bound.
    pub fn hint(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Shortcut {
    fn from(hint: &str) -> Self {
        Self::new(hint)
    }
}

impl From<String> for Shortcut {
    fn from(hint: String) -> Self {
        Self(hint)
    }
}
