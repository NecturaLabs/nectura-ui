//! What screen readers and the keyboard need from every component beyond its own role and name:
//! [`group`] (a container the widgets in a `Ui` are announced as parts of), [`disclosure_name`]
//! (an open or closed state Linux screen readers would not otherwise hear), [`FocusReturn`] (an
//! overlay gives focus back to the control that had it) and [`trap_focus`] (Tab stays inside an
//! overlay while it is open).

use egui::accesskit::Role;
use egui::{Context, Id, LayerId, Response, Ui};

/// Marks `ui`'s own accessibility node as a `role` named `label`, so the widgets inside it are
/// announced as its parts (the commits of a list, the controls of a dialog). Every egui `Ui` has a
/// node of its own, a generic container screen readers pass over; this gives it a meaning. Call
/// it on a `Ui` that holds only what belongs to the group (a `scope` around the rows).
pub fn group(ui: &Ui, role: Role, label: &str) {
    ui.ctx().accesskit_node_builder(ui.unique_id(), |n| {
        n.set_role(role);
        n.set_label(label);
    });
}

/// Marks `ui` (the card of an overlay) as a modal dialog named `label`: its controls are its
/// parts, and screen readers keep to it while it is open (an `AlertDialog` when it asks before
/// destroying something).
pub fn dialog(ui: &Ui, label: &str, alert: bool) {
    group(
        ui,
        if alert {
            Role::AlertDialog
        } else {
            Role::Dialog
        },
        label,
    );
    ui.ctx()
        .accesskit_node_builder(ui.unique_id(), egui::accesskit::Node::set_modal);
}

/// Whether the platform's accessibility adapter passes a node's expanded state on. AccessKit's
/// Linux adapter (`accesskit_atspi_common` 0.18) has no expanded state, so Orca cannot say
/// whether a disclosure is open; there the state goes into the name instead.
const ADAPTER_SPEAKS_EXPANDED: bool = !cfg!(all(unix, not(target_os = "macos")));

/// The name a disclosure (a tree row with children, a section header, a collapsing block) is
/// announced by: `label`, and on Linux its state after it ("Remotes, expanded"), because the
/// adapter there drops the expanded state the node also carries.
pub fn disclosure_name(label: &str, open: bool) -> String {
    if ADAPTER_SPEAKS_EXPANDED {
        label.to_owned()
    } else if open {
        format!("{label}, expanded")
    } else {
        format!("{label}, collapsed")
    }
}

/// Whether the platform's accessibility adapter says a disabled control is unavailable. AccessKit's
/// Linux adapter (`accesskit_atspi_common` 0.18) reports it only for roles with a read-only state
/// ([`reads_read_only`]) and calls every other control, a button among them, enabled.
const ADAPTER_SPEAKS_DISABLED: bool = !cfg!(all(unix, not(target_os = "macos")));

/// Roles whose disabled state the Linux adapter speaks as read-only (`accesskit_consumer`'s
/// `is_read_only_supported`).
fn reads_read_only(role: Role) -> bool {
    matches!(
        role,
        Role::TextInput
            | Role::MultilineTextInput
            | Role::SearchInput
            | Role::DateInput
            | Role::DateTimeInput
            | Role::WeekInput
            | Role::MonthInput
            | Role::TimeInput
            | Role::EmailInput
            | Role::NumberInput
            | Role::PasswordInput
            | Role::PhoneNumberInput
            | Role::UrlInput
            | Role::EditableComboBox
            | Role::SpinButton
            | Role::CheckBox
            | Role::ColorWell
            | Role::ComboBox
            | Role::Grid
            | Role::ListBox
            | Role::MenuItemCheckBox
            | Role::MenuItemRadio
            | Role::MenuListPopup
            | Role::RadioButton
            | Role::RadioGroup
            | Role::Slider
            | Role::Switch
            | Role::TreeGrid
    )
}

/// The name an unavailable control in `role` called `label` is announced by, when it differs:
/// on Linux, for a role the adapter would call enabled, ", unavailable" after the label (once).
fn unavailable_name(label: &str, role: Role) -> Option<String> {
    (!ADAPTER_SPEAKS_DISABLED && !reads_read_only(role) && !label.ends_with(", unavailable"))
        .then(|| format!("{label}, unavailable"))
}

/// Tells screen readers that the disabled control `response` is unavailable and why: `reason`, if
/// any, as its description, and on Linux the state in its name ([`unavailable_name`]). Call after
/// the control's `widget_info`.
pub fn unavailable(response: &Response, reason: Option<&str>) {
    response.ctx.accesskit_node_builder(response.id, |n| {
        if let Some(name) = n.label().and_then(|l| unavailable_name(l, n.role())) {
            n.set_label(name);
        }
        if let Some(reason) = reason {
            n.set_description(reason);
        }
    });
}

/// Where keyboard focus goes back to when an overlay (the palette, the cheatsheet) closes: the
/// control that had it when the overlay opened. Keep one in the overlay's state.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FocusReturn {
    to: Option<Id>,
    armed: bool,
}

impl FocusReturn {
    /// Remembers what has focus now; call it on the overlay's first frame, before it takes focus.
    pub fn remember(&mut self, ctx: &Context) {
        self.to = ctx.memory(egui::Memory::focused);
        self.armed = true;
    }

    /// Gives focus back to what had it when [`Self::remember`] ran, once, however the overlay on
    /// `layer` closed (Esc, a click outside, a chosen entry). Call it every frame the overlay is
    /// closed. Focus that has moved on elsewhere (into the dialog a chosen command opened) stays.
    pub fn restore(&mut self, ctx: &Context, layer: LayerId) {
        if !std::mem::take(&mut self.armed) {
            return;
        }
        let moved_on = ctx
            .memory(egui::Memory::focused)
            .and_then(|f| ctx.read_response(f))
            .is_some_and(|r| r.layer_id != layer);
        if let Some(id) = self.to.take()
            && !moved_on
        {
            ctx.memory_mut(|m| m.request_focus(id));
        }
    }
}

/// [`FocusReturn`] for an overlay whose state the caller keeps (`open`), not the overlay itself:
/// call it once a frame, before the overlay is drawn, with its `layer`. Focus is remembered the
/// frame `open` turns true and given back the frame after it turns false (egui clears focus at
/// the end of a frame that an Escape closed the overlay in, so earlier would be lost).
pub fn return_focus(
    ctx: &Context,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    open: bool,
    layer: LayerId,
) {
    let id = Id::new(("kit-return-focus", Id::new(id_salt)));
    let state: Option<(FocusReturn, Option<u64>)> = ctx.data(|d| d.get_temp(id));
    let pass = ctx.cumulative_pass_nr();
    match (open, state) {
        (true, None) => {
            let mut back = FocusReturn::default();
            back.remember(ctx);
            ctx.data_mut(|d| d.insert_temp(id, (back, None::<u64>)));
        }
        (true, Some((back, Some(_)))) => {
            // Opened again before the focus went back.
            ctx.data_mut(|d| d.insert_temp(id, (back, None::<u64>)));
        }
        (false, Some((back, None))) => {
            ctx.data_mut(|d| d.insert_temp(id, (back, Some(pass))));
            ctx.request_repaint();
        }
        (false, Some((mut back, Some(closed)))) if pass > closed => {
            ctx.data_mut(|d| d.remove_temp::<(FocusReturn, Option<u64>)>(id));
            back.restore(ctx, layer);
            ctx.request_repaint();
        }
        _ => {}
    }
}

/// Keeps keyboard focus and the pointer inside `layer` and the layers above it for this frame:
/// Tab cycles through the overlay's own controls, and nothing behind it takes a key or a click.
/// Call it every frame the overlay is open, with the overlay's lowest layer (its scrim).
pub fn trap_focus(ctx: &Context, layer: LayerId) {
    ctx.memory_mut(|m| m.set_modal_layer(layer));
}

/// Gives the widget with keyboard focus a node in this pass's accessibility tree when it has
/// none; call it once the whole UI is drawn. egui announces the focused id even when nothing was
/// drawn under it this pass (focus handed back to the command that opened a dialog now closing,
/// for one), and AccessKit panics on a focus outside the tree. The empty node lasts one pass:
/// egui drops focus from a widget that is no longer drawn.
pub fn keep_focus_in_tree(ctx: &Context) {
    if let Some(id) = ctx.memory(egui::Memory::focused) {
        ctx.accesskit_node_builder(id, |_| ());
    }
}

#[cfg(test)]
mod tests {
    use egui::{Key, Order};

    use super::super::{Button, testkit};
    use super::*;

    #[test]
    fn a_group_announces_the_widgets_inside_it_as_its_parts() {
        let ctx = testkit::ctx();
        ctx.enable_accesskit();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.scope(|ui| {
                group(ui, Role::List, "Branches");
                Button::new("main").show(ui);
            });
        });
        out.textures_delta.clear();
        let update = out.platform_output.accesskit_update.expect("accesskit");
        let label = |id| {
            update
                .nodes
                .iter()
                .find(|(n, _)| *n == id)
                .and_then(|(_, n)| n.label().map(str::to_owned))
        };
        let (_, list) = update
            .nodes
            .iter()
            .find(|(_, n)| n.role() == Role::List)
            .expect("the list");
        assert_eq!(list.label(), Some("Branches"));
        let children: Vec<_> = list.children().iter().filter_map(|c| label(*c)).collect();
        assert_eq!(children, ["main"], "the button is the list's part");
    }

    /// Focus handed to something not drawn this pass still lands inside the tree that goes to
    /// the screen reader, so AccessKit never sees a focus it cannot find.
    #[test]
    fn focus_on_something_not_drawn_stays_in_the_tree() {
        let ctx = testkit::ctx();
        ctx.enable_accesskit();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            Button::new("Cancel").show(ui);
            ui.ctx()
                .memory_mut(|m| m.request_focus(Id::new("a closed palette's entry")));
            keep_focus_in_tree(ui.ctx());
        });
        out.textures_delta.clear();
        let update = out.platform_output.accesskit_update.expect("accesskit");
        assert!(
            update.nodes.iter().any(|(id, _)| *id == update.focus),
            "the focus has a node"
        );
        assert!(
            update
                .nodes
                .iter()
                .any(|(_, n)| n.children().contains(&update.focus)),
            "and a parent"
        );
    }

    #[test]
    fn a_disclosure_says_its_state_where_the_adapter_cannot() {
        let open = disclosure_name("Remotes", true);
        let shut = disclosure_name("Remotes", false);
        if cfg!(all(unix, not(target_os = "macos"))) {
            assert_eq!(open, "Remotes, expanded");
            assert_eq!(shut, "Remotes, collapsed");
        } else {
            assert_eq!((open.as_str(), shut.as_str()), ("Remotes", "Remotes"));
        }
    }

    #[test]
    fn focus_returns_once_to_what_had_it() {
        let ctx = testkit::ctx();
        let id = Id::new("trigger");
        ctx.memory_mut(|m| m.request_focus(id));
        let mut back = FocusReturn::default();
        back.remember(&ctx);
        ctx.memory_mut(|m| m.request_focus(Id::new("overlay field")));
        back.restore(&ctx, LayerId::background());
        assert!(ctx.memory(|m| m.has_focus(id)));
        ctx.memory_mut(|m| m.request_focus(Id::new("elsewhere")));
        back.restore(&ctx, LayerId::background());
        assert!(!ctx.memory(|m| m.has_focus(id)), "restored only once");
    }

    #[test]
    fn an_overlay_the_caller_tracks_gives_focus_back_when_it_closes() {
        let ctx = testkit::ctx();
        let layer = LayerId::new(Order::Foreground, Id::new("settings"));
        let trigger = Id::new("trigger");
        ctx.memory_mut(|m| m.request_focus(trigger));
        return_focus(&ctx, "settings", false, layer);
        return_focus(&ctx, "settings", true, layer);
        ctx.memory_mut(|m| m.request_focus(Id::new("search")));
        return_focus(&ctx, "settings", true, layer);
        assert!(
            !ctx.memory(|m| m.has_focus(trigger)),
            "not while it is open"
        );
        return_focus(&ctx, "settings", false, layer);
        assert!(!ctx.memory(|m| m.has_focus(trigger)), "the frame after");
        testkit::frame(&ctx, Vec::new(), |_| {});
        return_focus(&ctx, "settings", false, layer);
        assert!(ctx.memory(|m| m.has_focus(trigger)));
    }

    /// Tab moves only between the overlay's controls while it traps focus.
    #[test]
    fn a_trapped_overlay_keeps_tab_inside() {
        let ctx = testkit::ctx();
        let scrim = LayerId::new(Order::Foreground, Id::new("test-scrim"));
        let mut show = |ui: &mut Ui| {
            Button::new("Behind").show(ui);
            trap_focus(ui.ctx(), scrim);
            egui::Area::new(Id::new("test-overlay"))
                .order(Order::Tooltip)
                .show(ui.ctx(), |ui| {
                    Button::new("First").show(ui);
                    Button::new("Second").show(ui);
                });
        };
        // The trap holds from the frame after the overlay first asks for it.
        testkit::frame(&ctx, Vec::new(), &mut show);
        let mut seen = Vec::new();
        for _ in 0..6 {
            testkit::frame(&ctx, testkit::key(Key::Tab), &mut show);
            testkit::frame(&ctx, Vec::new(), &mut show);
            let focused = ctx.memory(egui::Memory::focused);
            let layer = focused
                .and_then(|f| ctx.read_response(f))
                .map(|r| r.layer_id);
            seen.push(layer);
        }
        assert!(
            seen.iter()
                .all(|l| l.is_some_and(|l| l.order == Order::Tooltip)),
            "Tab left the overlay: {seen:?}"
        );
    }

    /// A11Y-19 kit-wide: an unavailable control says so and why. The Linux adapter reports a
    /// disabled button as enabled, so there its name carries the state; a checkbox's read-only
    /// state does reach Orca, so its name stays as it is.
    #[test]
    fn an_unavailable_control_says_so_and_why() {
        let ctx = testkit::ctx();
        let nodes = testkit::spoken(&ctx, |ui| {
            Button::new("Push")
                .enabled(false)
                .disabled_reason("Nothing to push")
                .show(ui);
            Button::new("Fetch").show(ui);
            super::super::toggle::Checkbox::new(&mut false, "Sign commits")
                .enabled(false)
                .disabled_reason("No signing key")
                .show(ui);
        });
        let named = |prefix: &str| {
            nodes
                .iter()
                .find(|n| n.label.as_deref().is_some_and(|l| l.starts_with(prefix)))
                .unwrap_or_else(|| panic!("{prefix}: {nodes:?}"))
                .clone()
        };
        let push = named("Push");
        let want = if ADAPTER_SPEAKS_DISABLED {
            "Push"
        } else {
            "Push, unavailable"
        };
        assert_eq!(push.label.as_deref(), Some(want));
        assert_eq!(push.description.as_deref(), Some("Nothing to push"));
        assert!(push.disabled);
        let fetch = named("Fetch");
        assert_eq!(fetch.label.as_deref(), Some("Fetch"));
        assert_eq!(fetch.description, None);
        let sign = named("Sign commits");
        assert_eq!(sign.label.as_deref(), Some("Sign commits"));
        assert_eq!(sign.description.as_deref(), Some("No signing key"));
        assert_eq!(
            unavailable_name("Push, unavailable", Role::Button),
            None,
            "once"
        );
    }
}
