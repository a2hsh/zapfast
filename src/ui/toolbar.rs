//! Toolbars: a group of buttons that is one Tab stop. Tab enters on the
//! button focused there last, or on the first, and leaves the toolbar at
//! once; Left and Right move between its buttons, or Up and Down in a
//! vertical toolbar such as the rail, and Home and End go to the first and
//! the last. A screen reader names the toolbar on the way in.
//!
//! Unlike a list (`list_nav`), a toolbar moves real egui focus: its buttons
//! are drawn whenever the toolbar is, so focus never names a node missing
//! from the accessibility tree.

use egui::accesskit::{self, Role};
use egui::{Context, FocusDirection, Id, IdMap, Rect, Response, Ui, UiBuilder};

use super::focus::{Stop, TabStop};
use super::list_nav::{self, Step};

/// Which way a toolbar's buttons run, and so which arrows walk them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    /// A row: Left and Right.
    Horizontal,
    /// A column, such as the rail: Up and Down.
    Vertical,
}

/// One toolbar, as last drawn.
#[derive(Clone)]
struct Bar {
    orientation: Orientation,
    /// The node of the Ui holding the toolbar: a widget laid out among the
    /// buttons without being one hangs from it (see [`beside`]).
    parent: Id,
    /// The frame the toolbar was last drawn in.
    frame: u64,
    /// This pass's buttons, in the order they were drawn.
    drawn: Vec<(Id, Rect)>,
    /// The buttons of the last pass that drew the toolbar, in reading order.
    order: Vec<Id>,
    /// The button Tab enters on: the one focused last.
    current: Option<Id>,
}

#[derive(Clone, Default)]
struct Toolbars {
    bars: IdMap<Bar>,
    /// The button this frame's key moved to. A re-layout pass replays the
    /// frame's input, and must not move a second time.
    moved: Option<(u64, Id)>,
}

fn state_id() -> Id {
    Id::new("toolbars")
}

/// Lays out `add_buttons` in a Ui whose accessibility node is a Toolbar
/// named `label`, with the id `id`. Buttons join it with
/// [`ToolbarButton::in_toolbar`]; one of them, the one focused last or else
/// the first, is the toolbar's Tab stop `stop`.
pub fn toolbar<R>(
    ui: &mut Ui,
    id: Id,
    label: &str,
    stop: Stop,
    add_buttons: impl FnOnce(&mut Ui) -> R,
) -> R {
    oriented(ui, id, label, stop, Orientation::Horizontal, add_buttons)
}

/// [`toolbar`] for buttons drawn one under another, such as the rail's: Up
/// and Down walk them, and a screen reader is told the toolbar is vertical.
pub fn vertical_toolbar<R>(
    ui: &mut Ui,
    id: Id,
    label: &str,
    stop: Stop,
    add_buttons: impl FnOnce(&mut Ui) -> R,
) -> R {
    oriented(ui, id, label, stop, Orientation::Vertical, add_buttons)
}

fn oriented<R>(
    ui: &mut Ui,
    id: Id,
    label: &str,
    stop: Stop,
    orientation: Orientation,
    add_buttons: impl FnOnce(&mut Ui) -> R,
) -> R {
    let ctx = ui.ctx().clone();
    let parent = ui.unique_id();
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|data| {
        let bars = &mut data.get_temp_mut_or_default::<Toolbars>(state_id()).bars;
        let bar = bars.entry(id).or_insert_with(|| Bar {
            orientation,
            parent,
            frame,
            drawn: Vec::new(),
            order: Vec::new(),
            current: None,
        });
        bar.orientation = orientation;
        bar.parent = parent;
        bar.frame = frame;
        bar.drawn.clear();
    });
    // The toolbar's own id, rather than a salt, names its node, so tests
    // find it. A scope leaves the layout exactly as it was without it.
    let inner = ui
        .scope_builder(UiBuilder::new().id(id), |ui| {
            let label = label.to_owned();
            ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
                node.set_role(Role::Toolbar);
                node.set_label(label);
                // A toolbar is horizontal unless it says otherwise.
                if orientation == Orientation::Vertical {
                    node.set_orientation(accesskit::Orientation::Vertical);
                }
            });
            add_buttons(ui)
        })
        .inner;
    let entry = ctx.data_mut(|data| {
        let bar = data
            .get_temp_mut_or_default::<Toolbars>(state_id())
            .bars
            .get_mut(&id)?;
        bar.order = reading_order(&bar.drawn);
        // A button that is gone, such as Back when the archive closes,
        // hands the stop to the first.
        bar.current
            .filter(|current| bar.order.contains(current))
            .or_else(|| bar.order.first().copied())
    });
    // Only this button is a stop: Tab reaches the toolbar once, and the
    // next Tab leaves it.
    if let Some(response) = entry.and_then(|entry| ctx.read_response(entry)) {
        response.tab_stop(stop);
    }
    inner
}

/// A button joining a toolbar drawn by [`toolbar`].
pub trait ToolbarButton: Sized {
    fn in_toolbar(self, toolbar: Id) -> Self;
}

impl ToolbarButton for Response {
    fn in_toolbar(self, toolbar: Id) -> Self {
        if self.enabled() {
            let focused = self.has_focus();
            self.ctx.data_mut(|data| {
                let bars = &mut data.get_temp_mut_or_default::<Toolbars>(state_id()).bars;
                if let Some(bar) = bars.get_mut(&toolbar) {
                    bar.drawn.push((self.id, self.rect));
                    if focused {
                        bar.current = Some(self.id);
                    }
                }
            });
        }
        self
    }
}

/// Lays out `add_contents` among a toolbar's buttons while keeping its
/// accessibility node out of the toolbar: for a widget that sits between
/// the buttons without being one, such as the message input between the
/// composer's. A screen reader would otherwise name the toolbar each time
/// focus reached the input.
pub fn beside<R>(ui: &mut Ui, toolbar: Id, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let parent = ui.ctx().data(|data| {
        data.get_temp::<Toolbars>(state_id())
            .and_then(|state| state.bars.get(&toolbar).map(|bar| bar.parent))
    });
    let builder = match parent {
        Some(parent) => UiBuilder::new().accessibility_parent(parent),
        None => UiBuilder::new(),
    };
    ui.scope_builder(builder, add_contents).inner
}

/// The keys of a toolbar one of whose buttons has focus: Left and Right move
/// to the button beside it (Up and Down in a vertical toolbar), Home and End
/// to the first and the last, and the other two arrows keep focus where it
/// is, where egui would move it to whatever is nearest on screen. Runs
/// before the widgets, from the toolbar as the last frame drew it, and
/// before the chat's own Home and End scrolling.
pub fn keys(ctx: &Context) {
    let Some(focused) = ctx.memory(|memory| memory.focused()) else {
        return;
    };
    let frame = ctx.cumulative_frame_nr();
    let Some((order, orientation, moved)) = ctx.data(|data| {
        let state = data.get_temp::<Toolbars>(state_id())?;
        let bar = state
            .bars
            .values()
            .find(|bar| bar.frame + 1 >= frame && bar.order.contains(&focused))?;
        Some((bar.order.clone(), bar.orientation, state.moved))
    }) else {
        return;
    };
    let (previous, next, across) = match orientation {
        Orientation::Horizontal => (
            egui::Key::ArrowLeft,
            egui::Key::ArrowRight,
            [egui::Key::ArrowUp, egui::Key::ArrowDown],
        ),
        Orientation::Vertical => (
            egui::Key::ArrowUp,
            egui::Key::ArrowDown,
            [egui::Key::ArrowLeft, egui::Key::ArrowRight],
        ),
    };
    let step = ctx.input_mut(|input| {
        [
            (previous, Some(Step::Previous)),
            (next, Some(Step::Next)),
            (egui::Key::Home, Some(Step::First)),
            (egui::Key::End, Some(Step::Last)),
            (across[0], None),
            (across[1], None),
        ]
        .into_iter()
        .find(|(key, _)| super::keys::take_plain(input, *key))
        .map(|(_, step)| step)
    });
    let Some(step) = step else {
        return;
    };
    let target = match moved {
        // The same key in a re-layout pass: focus already moved for it.
        Some((at, reached)) if at == frame && reached == focused => focused,
        _ => step
            .and_then(|step| {
                let index = order.iter().position(|id| *id == focused)?;
                list_nav::step(Some(index), order.len(), step).map(|index| order[index])
            })
            .unwrap_or(focused),
    };
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Toolbars>(state_id()).moved = Some((frame, target));
        // Moving in a toolbar is keyboard navigation, as Tab is: the focus
        // ring shows, and a chip scrolled out of view scrolls in.
        data.insert_temp(crate::theme::keyboard_focus_id(), true);
    });
    ctx.memory_mut(|memory| {
        // egui already read the arrow as a move of its own, to the nearest
        // widget on screen.
        memory.move_focus(FocusDirection::None);
        memory.request_focus(target);
    });
}

/// The buttons in the order the arrows walk them: left to right, row by row.
/// A vertical toolbar's buttons each make a row, so they read top to bottom.
/// Buttons laid out right to left, such as the headers' trailing icons, are
/// put back in reading order. ZapFast's chrome is never mirrored (no locale
/// it ships reads right to left), so left to right is reading order; a
/// mirrored layout would sort by descending x.
fn reading_order(drawn: &[(Id, Rect)]) -> Vec<Id> {
    let mut rows: Vec<Vec<(Id, Rect)>> = Vec::new();
    for &(id, rect) in drawn {
        // Buttons drawn one after another that share a band of height are
        // one row; the label chips under the filter chips start another.
        match rows.last_mut() {
            Some(row)
                if row.iter().any(|(_, other)| {
                    rect.top() < other.bottom() && other.top() < rect.bottom()
                }) =>
            {
                row.push((id, rect));
            }
            _ => rows.push(vec![(id, rect)]),
        }
    }
    rows.into_iter()
        .flat_map(|mut row| {
            row.sort_by(|(_, a), (_, b)| a.left().total_cmp(&b.left()));
            row
        })
        .map(|(id, _)| id)
        .collect()
}

/// The buttons of `toolbar` as the last pass drew them, in reading order.
#[cfg(any(test, feature = "demo"))]
pub(crate) fn buttons(ctx: &Context, toolbar: Id) -> Vec<Id> {
    ctx.data(|data| {
        data.get_temp::<Toolbars>(state_id())
            .and_then(|state| state.bars.get(&toolbar).map(|bar| bar.order.clone()))
            .unwrap_or_default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    fn button(name: &str, x: f32, y: f32) -> (Id, Rect) {
        (
            Id::new(name),
            Rect::from_min_size(pos2(x, y), vec2(30.0, 30.0)),
        )
    }

    #[test]
    fn buttons_read_left_to_right_and_row_by_row() {
        // The header draws its trailing icons right to left.
        let header = [
            button("profile", 0.0, 2.0),
            button("settings", 200.0, 4.0),
            button("new chat", 170.0, 4.0),
            button("sidebar", 140.0, 4.0),
        ];
        assert_eq!(
            reading_order(&header),
            ["profile", "sidebar", "new chat", "settings"].map(Id::new)
        );
        // The filter chips, then the label chips on the row below.
        let filters = [
            button("all", 0.0, 0.0),
            button("unread", 40.0, 0.0),
            button("work", 0.0, 40.0),
            button("manage", 40.0, 40.0),
        ];
        assert_eq!(
            reading_order(&filters),
            ["all", "unread", "work", "manage"].map(Id::new)
        );
        assert!(reading_order(&[]).is_empty());
    }

    fn key(key: egui::Key) -> egui::RawInput {
        egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        }
    }

    /// A vertical toolbar walks its buttons with Up and Down, top to bottom,
    /// and Left and Right keep focus where it is; a horizontal one is the
    /// other way round. Only a vertical toolbar names its orientation.
    #[test]
    fn up_and_down_walk_a_vertical_toolbar_left_and_right_a_row() {
        for vertical in [true, false] {
            let ctx = Context::default();
            ctx.enable_accesskit();
            let bar = Id::new("bar");
            let draw = |ui: &mut Ui| {
                crate::ui::focus::begin(ui.ctx(), true);
                keys(ui.ctx());
                let add =
                    |ui: &mut Ui| ["a", "b", "c"].map(|name| ui.button(name).in_toolbar(bar).id);
                let ids = if vertical {
                    vertical_toolbar(ui, bar, "Bar", Stop::Rail, |ui| ui.vertical(add).inner)
                } else {
                    toolbar(ui, bar, "Bar", Stop::Rail, |ui| ui.horizontal(add).inner)
                };
                crate::ui::focus::finish(ui.ctx(), true);
                ids
            };
            let mut ids = [Id::NULL; 3];
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| ids = draw(ui));
            output.textures_delta.clear();
            let orientation = output
                .platform_output
                .accesskit_update
                .and_then(|tree| {
                    tree.nodes
                        .into_iter()
                        .find(|(_, node)| node.role() == Role::Toolbar)
                })
                .map(|(_, node)| node.orientation());
            assert_eq!(
                orientation,
                Some(vertical.then_some(accesskit::Orientation::Vertical)),
                "vertical: {vertical}"
            );
            ctx.memory_mut(|memory| memory.request_focus(ids[0]));
            let (next, previous, across) = if vertical {
                (
                    egui::Key::ArrowDown,
                    egui::Key::ArrowUp,
                    egui::Key::ArrowRight,
                )
            } else {
                (
                    egui::Key::ArrowRight,
                    egui::Key::ArrowLeft,
                    egui::Key::ArrowDown,
                )
            };
            for (key, index) in [
                (next, 1),
                (next, 2),
                (next, 2),
                (across, 2),
                (previous, 1),
                (egui::Key::Home, 0),
                (egui::Key::End, 2),
            ] {
                let mut output = ctx.run_ui(super::tests::key(key), |ui| {
                    draw(ui);
                });
                output.textures_delta.clear();
                assert_eq!(
                    ctx.memory(|memory| memory.focused()),
                    Some(ids[index]),
                    "vertical: {vertical}, {key:?}"
                );
            }
        }
    }

    /// A re-layout pass replays the frame's key: focus moves once. Only the
    /// focused button is a Tab stop.
    #[test]
    fn a_relayout_moves_once_and_only_the_current_button_is_a_stop() {
        let ctx = Context::default();
        let bar = Id::new("bar");
        let draw = |ui: &mut Ui| {
            crate::ui::focus::begin(ui.ctx(), true);
            keys(ui.ctx());
            let ids = toolbar(ui, bar, "Bar", Stop::MessageActions, |ui| {
                ui.horizontal(|ui| ["a", "b", "c"].map(|name| ui.button(name).in_toolbar(bar).id))
                    .inner
            });
            crate::ui::focus::finish(ui.ctx(), true);
            ids
        };
        let mut ids = [Id::NULL; 3];
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| ids = draw(ui));
        output.textures_delta.clear();
        let stops = |ctx: &Context| {
            crate::ui::focus::stops(ctx)
                .into_iter()
                .filter(|(_, id)| ids.contains(id))
                .map(|(stop, id)| (stop, ids.iter().position(|known| *known == id)))
                .collect::<Vec<_>>()
        };
        assert_eq!(stops(&ctx), [(Stop::MessageActions, Some(0))]);
        ctx.memory_mut(|memory| memory.request_focus(ids[0]));
        let mut passes = 0;
        let mut output = ctx.run_ui(key(egui::Key::ArrowRight), |ui| {
            draw(ui);
            passes += 1;
            if passes == 1 {
                ui.ctx().request_discard("test re-layout");
            }
        });
        output.textures_delta.clear();
        assert_eq!(passes, 2);
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(ids[1]));
        assert_eq!(stops(&ctx), [(Stop::MessageActions, Some(1))]);
        assert_eq!(
            ctx.data(|data| data.get_temp::<bool>(crate::theme::keyboard_focus_id())),
            Some(true),
            "the arrows count as keyboard navigation"
        );
    }
}
