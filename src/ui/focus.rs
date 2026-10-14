//! Deliberate Tab navigation for the main chat view. Message content and chat
//! rows remain pointer/accessibility targets, not stops in the primary cycle;
//! the message list and the chat list as a whole are one stop each, and so
//! is each toolbar (`toolbar.rs`). The message list's keys reach a message's
//! links and phone numbers.

use egui::{Context, FocusDirection, Id, Response};

/// The Tab order is this order: the window from the top left, so the rail
/// comes first, then the chat list's stops, and the composer's last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stop {
    /// The rail on the left, the window's first stop: one vertical toolbar
    /// of Chats, Calls and Starred, Show the chat list while the list is
    /// hidden, then Archived, Locked, Settings and our picture. The chats'
    /// avatars between them are not its buttons: while the list is hidden
    /// they are the chat list, the next stop.
    Rail,
    /// The chat list header's toolbar: Back, New chat (New call on the
    /// Calls page) and hide the list.
    ChatListActions,
    Search,
    /// The filter chips' toolbar: the built-in filters and the label chips,
    /// or the Calls page's All and Missed.
    Filters,
    /// The chat list, full or collapsed to the rail's avatars: one stop
    /// below the chips, whose arrows walk its chats. The open chat's stops
    /// follow it.
    ChatList,
    /// The chat header's toolbar: chat info, search and the chat's menu,
    /// and Back in a narrow window.
    ChatActions,
    /// The search pane's controls in reading order. The arrows walk its
    /// results from the field.
    ChatSearchClose,
    ChatSearchDate,
    ChatSearchField,
    ChatSearchDay,
    /// The info panel's close or back button, then its buttons and tabs
    /// in reading order. Its members, pictures, documents and links are
    /// pointer and accessibility targets, like chat rows.
    InfoClose,
    InfoControl(u8),
    /// The open chat's message list: one stop, the arrows walk its messages.
    Messages,
    /// The message field, after the messages, as it is drawn.
    Composer,
    /// The composer's toolbar: plus, emoji, and send or record.
    MessageActions,
}

#[derive(Clone, Copy, Debug)]
struct Move {
    from: Option<Stop>,
    steps: i32,
}

#[derive(Clone, Default)]
struct Order {
    frame: Option<u64>,
    controls: Vec<(Stop, Id)>,
    movement: Option<Move>,
}

fn order_id() -> Id {
    Id::new("main-chat-tab-order")
}

/// Attach a semantic position to an enabled main-view control. Using the real
/// response id keeps pointer clicks and keyboard activation on the same widget.
pub trait TabStop: Sized {
    fn tab_stop(self, stop: Stop) -> Self;
}

impl TabStop for Response {
    fn tab_stop(self, stop: Stop) -> Self {
        if self.enabled() {
            // `reveal_focus` has queued the scroll while this control's
            // scroll area is open. Re-layout so its ring is visible now.
            if self.gained_focus() && self.interact_rect != self.rect {
                self.ctx.request_discard("reveal primary Tab control");
            }
            self.ctx.data_mut(|data| {
                let order = data.get_temp_mut_or_default::<Order>(order_id());
                order.controls.retain(|(known, _)| *known != stop);
                order.controls.push((stop, self.id));
            });
        }
        self
    }
}

/// The widget last registered for `stop`, such as an icon button without
/// painted text, so a scripted demo can find it where it was drawn.
#[cfg(any(test, feature = "demo"))]
pub(crate) fn control(ctx: &Context, stop: Stop) -> Option<Id> {
    ctx.data(|data| data.get_temp::<Order>(order_id()))?
        .controls
        .into_iter()
        .find_map(|(known, id)| (known == stop).then_some(id))
}

/// Intercept Tab before any widgets are registered. Merely consuming the key
/// is insufficient: egui already chose a focus direction during begin_pass.
pub fn begin(ctx: &Context, active: bool) {
    let mut pressed = false;
    let mut steps = 0;
    if active {
        ctx.input_mut(|input| {
            input.events.retain(|event| {
                if let egui::Event::Key {
                    key: egui::Key::Tab,
                    pressed: true,
                    modifiers,
                    ..
                } = event
                    && (!modifiers.any() || modifiers.shift_only())
                {
                    pressed = true;
                    steps += if modifiers.shift { -1 } else { 1 };
                    false
                } else {
                    true
                }
            });
        });
    }
    let focused = ctx.memory(|memory| memory.focused());
    let frame = ctx.cumulative_frame_nr();
    let target = ctx.data_mut(|data| {
        let order = data.get_temp_mut_or_default::<Order>(order_id());
        // Re-layout passes replay the same key. Always navigate from the
        // original source, not from the target chosen in the previous pass.
        if order.frame != Some(frame) {
            order.frame = Some(frame);
            order.movement = (active && pressed && steps != 0).then(|| Move {
                from: order
                    .controls
                    .iter()
                    .find(|(_, id)| Some(*id) == focused)
                    .map(|(stop, _)| *stop),
                steps,
            });
        }
        let target = active
            .then(|| {
                order
                    .movement
                    .and_then(|movement| next(&order.controls, movement))
            })
            .flatten();
        order.controls.clear();
        target
    });
    if active && (pressed || target.is_some()) {
        ctx.memory_mut(|memory| {
            memory.move_focus(FocusDirection::None);
            if let Some(target) = target {
                memory.request_focus(target);
            }
        });
    }
}

/// Reconcile against controls actually drawn this pass (a hidden sidebar or a
/// read-only chat can remove stops). Menus and dialogs keep egui's own order.
pub fn finish(ctx: &Context, active: bool) {
    let target = ctx.data_mut(|data| {
        let order = data.get_temp_mut_or_default::<Order>(order_id());
        order.controls.sort_by_key(|(stop, _)| *stop);
        order
            .movement
            .and_then(|movement| next(&order.controls, movement))
    });
    if active
        && !egui::Popup::is_any_open(ctx)
        && let Some(target) = target
    {
        ctx.memory_mut(|memory| {
            memory.move_focus(FocusDirection::None);
            memory.request_focus(target);
        });
    }
}

fn next(controls: &[(Stop, Id)], movement: Move) -> Option<Id> {
    if controls.is_empty() {
        return None;
    }
    let index = if let Some(index) = controls
        .iter()
        .position(|(stop, _)| Some(*stop) == movement.from)
    {
        index as i32 + movement.steps
    } else if movement.steps < 0 {
        let before = movement.from.map_or(controls.len(), |from| {
            controls.partition_point(|(stop, _)| *stop < from)
        });
        before as i32 + movement.steps
    } else {
        let after = movement.from.map_or(0, |from| {
            controls.partition_point(|(stop, _)| *stop <= from)
        });
        after as i32 + movement.steps.saturating_sub(1)
    };
    Some(controls[index.rem_euclid(controls.len() as i32) as usize].1)
}

#[cfg(test)]
pub fn stops(ctx: &Context) -> Vec<(Stop, Id)> {
    ctx.data(|data| {
        data.get_temp::<Order>(order_id())
            .unwrap_or_default()
            .controls
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stops follow the window from the top left: the rail's, the chat
    /// list's, then the conversation's, and the composer's last.
    #[test]
    fn the_composer_comes_last() {
        let order = [
            Stop::Rail,
            Stop::ChatListActions,
            Stop::Search,
            Stop::Filters,
            Stop::ChatList,
            Stop::ChatActions,
            Stop::ChatSearchClose,
            Stop::Messages,
            Stop::Composer,
            Stop::MessageActions,
        ];
        assert!(order.is_sorted(), "{order:?}");
    }

    #[test]
    fn navigation_wraps_and_skips_missing_controls_in_both_directions() {
        let controls = [Stop::Search, Stop::Composer, Stop::MessageActions]
            .map(|stop| (stop, Id::new(format!("{stop:?}"))));
        for (from, steps, expected) in [
            (Some(Stop::Search), 1, 1),
            (Some(Stop::MessageActions), 1, 0),
            (Some(Stop::Search), -1, 2),
            (Some(Stop::ChatActions), 1, 1),
            (Some(Stop::ChatActions), -1, 0),
            (Some(Stop::ChatListActions), 1, 0),
            (None, 1, 0),
            (None, -1, 2),
            (Some(Stop::Search), 5, 2),
            (Some(Stop::Search), -5, 1),
        ] {
            assert_eq!(
                next(&controls, Move { from, steps }),
                Some(controls[expected].1)
            );
        }
        assert_eq!(
            next(
                &[],
                Move {
                    from: None,
                    steps: 1
                }
            ),
            None
        );
    }

    #[test]
    fn a_relayout_does_not_advance_tab_twice() {
        let ctx = Context::default();
        let draw = |ui: &mut egui::Ui| {
            begin(ui.ctx(), true);
            for stop in [Stop::Composer, Stop::MessageActions, Stop::ChatActions] {
                ui.button(format!("{stop:?}")).tab_stop(stop);
            }
            finish(ui.ctx(), true);
        };
        let mut output = ctx.run_ui(egui::RawInput::default(), draw);
        output.textures_delta.clear();
        let controls = stops(&ctx);
        ctx.memory_mut(|memory| memory.request_focus(controls[0].1));
        let mut passes = 0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::Tab,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| {
                draw(ui);
                passes += 1;
                if passes == 1 {
                    ui.ctx().request_discard("test re-layout");
                }
            },
        );
        output.textures_delta.clear();
        assert_eq!(passes, 2);
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(controls[1].1));
    }
}
