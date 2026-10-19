//! Keyboard and screen-reader navigation for long lists such as the messages.
//!
//! A list is one focusable container: Tab reaches it once, and the arrows
//! move a cursor among its items. The current item is exposed to AccessKit as
//! the container's active descendant rather than by moving egui focus, so
//! focus never names a row that a later frame skips laying out.

use egui::accesskit::{Rect as AkRect, Role};

/// A move of the cursor in a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Previous,
    Next,
    First,
    Last,
    /// Up by about a page of rows.
    PageUp(usize),
    /// Down by about a page of rows.
    PageDown(usize),
}

/// New index for `step` in a list of `len` items from `current`; None for an
/// empty list. Without a cursor the list starts on its last (newest) item.
pub fn step(current: Option<usize>, len: usize, step: Step) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let last = len - 1;
    // A cursor left past the end by removed items counts from the end.
    let Some(current) = current.map(|index| index.min(last)) else {
        return Some(last);
    };
    Some(match step {
        Step::Previous => current.saturating_sub(1),
        Step::Next => (current + 1).min(last),
        Step::First => 0,
        Step::Last => last,
        Step::PageUp(rows) => current.saturating_sub(rows.max(1)),
        Step::PageDown(rows) => (current + rows.max(1)).min(last),
    })
}

/// A rect in AccessKit's physical pixels.
fn bounds(rect: egui::Rect, ppp: f32) -> AkRect {
    AkRect {
        x0: f64::from(rect.min.x * ppp),
        y0: f64::from(rect.min.y * ppp),
        x1: f64::from(rect.max.x * ppp),
        y1: f64::from(rect.max.y * ppp),
    }
}

/// The focusable list widget: one Tab stop whose node is a labelled ListBox.
///
/// It senses focus only, never clicks, hovers or drags, so every widget in
/// the list keeps its pointer behaviour. Its node is created here, before the
/// rows, so rows that name it as their accessibility parent attach to it.
/// `label` is asked only while a screen reader is attached and the list is
/// drawn visibly, after `interact` has given the node its place in the tree:
/// a node made before that would hang from the window, and one made in a
/// hidden pass would have no role.
pub fn container(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: egui::Rect,
    label: impl FnOnce() -> String,
    len: usize,
) -> egui::Response {
    let response = ui.interact(rect, id, egui::Sense::focusable_noninteractive());
    outline_only(ui.ctx(), id);
    if ui.ctx().accesskit_node_builder(id, |_| ()).is_some() {
        let label = label();
        let ppp = ui.ctx().pixels_per_point();
        ui.ctx().accesskit_node_builder(id, |node| {
            node.set_role(Role::ListBox);
            node.set_label(label);
            node.set_size_of_set(len);
            node.set_bounds(bounds(rect, ppp));
        });
    }
    if response.has_focus() {
        // The arrows move the cursor, not egui's focus. Tab and Escape keep
        // their usual meaning.
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: false,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: false,
                },
            );
        });
    }
    response
}

/// The builder of a row's `Ui` in the list `list`: the row's node is an item
/// of the list's node rather than of the containers in between, and the
/// row's ids come from `salt` alone, as with `id_salt`. Unlike `id_salt`,
/// which leaves the row's own node id counting the rows drawn before it, the
/// node id is fixed too: a list that lays out only the rows near the view
/// would otherwise rename its current row whenever that count changes, and
/// a screen reader would hear the same item again.
pub fn row_builder(
    ui: &egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    list: egui::Id,
) -> egui::UiBuilder {
    // `id_salt` hashes its salt into an `IdSalt` before combining it with
    // the `Ui`'s id: the same here keeps the row's `Ui` id, and every id
    // made from it, as it was. Automatic ids inside the row now come from
    // the fixed id too, so they no longer shift either.
    egui::UiBuilder::new()
        .id(ui.make_persistent_id(egui::IdSalt::new(salt)))
        .accessibility_parent(list)
}

/// Sets `active_descendant` on the container to `row`. Callers pass only a
/// row whose node was created in this pass: AccessKit must never be told
/// about a node that is not in the tree it receives.
pub fn set_active(ctx: &egui::Context, container: egui::Id, row: Option<egui::Id>) {
    let Some(row) = row else {
        return;
    };
    ctx.accesskit_node_builder(container, |node| {
        node.set_active_descendant(row.accesskit_id());
    });
}

/// Turns a row's node into a ListBoxOption at `index` of `len`. Only the
/// `selected` item, such as the open chat, carries a selection state; every
/// other item carries none, neither selected nor toggled. A row that said
/// "not selected", or that kept the toggle state egui gives a selectable
/// widget, would have a screen reader say "not selected" or "not checked"
/// on every arrow press. Messages are never selected.
pub fn mark_row(
    ctx: &egui::Context,
    row: egui::Id,
    label: String,
    index: usize,
    len: usize,
    rect: egui::Rect,
    selected: bool,
) {
    let ppp = ctx.pixels_per_point();
    ctx.accesskit_node_builder(row, |node| {
        node.set_role(Role::ListBoxOption);
        node.set_label(label);
        node.set_position_in_set(index);
        node.set_size_of_set(len);
        node.set_bounds(bounds(rect, ppp));
        node.clear_toggled();
        if selected {
            node.set_selected(true);
        } else {
            node.clear_selected();
        }
    });
}

fn outline_only_id(id: egui::Id) -> egui::Id {
    id.with("outline-only")
}

/// Marks `id` as a widget whose focus ring is drawn only through
/// `theme::focus_outline`, around its current item, and never around the
/// widget's own rect. Registered each frame the widget is drawn.
pub fn outline_only(ctx: &egui::Context, id: egui::Id) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|data| data.insert_temp(outline_only_id(id), frame));
}

/// Whether `id` was registered with [`outline_only`] this frame.
pub fn is_outline_only(ctx: &egui::Context, id: egui::Id) -> bool {
    ctx.data(|data| data.get_temp::<u64>(outline_only_id(id))) == Some(ctx.cumulative_frame_nr())
}

/// Whether `id` was registered with [`outline_only`] this frame or the one
/// before: what a check at the start of a frame, before the widget is drawn
/// again, can see.
pub fn was_outline_only(ctx: &egui::Context, id: egui::Id) -> bool {
    let frame = ctx.cumulative_frame_nr();
    ctx.data(|data| data.get_temp::<u64>(outline_only_id(id)))
        .is_some_and(|registered| registered + 1 >= frame)
}

/// The keys a focused list consumes: the arrows, Home, End, PgUp and PgDn
/// without modifiers. Returns this frame's first one as a step, and cancels
/// the focus move egui chose for an arrow at the start of the frame. `page`
/// gives the rows a page key moves; it is asked only when PgUp or PgDn was
/// taken, so the frames without one do not work it out.
pub fn take_step(ctx: &egui::Context, page: impl FnOnce() -> usize) -> Option<Step> {
    let mut found = None;
    ctx.input_mut(|input| {
        for (key, step) in [
            (egui::Key::ArrowUp, Step::Previous),
            (egui::Key::ArrowDown, Step::Next),
            (egui::Key::Home, Step::First),
            (egui::Key::End, Step::Last),
            (egui::Key::PageUp, Step::PageUp(0)),
            (egui::Key::PageDown, Step::PageDown(0)),
        ] {
            if found.is_none() && crate::ui::keys::take_plain(input, key) {
                found = Some(step);
            }
        }
    });
    if found.is_some() {
        ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
    }
    // Outside the input lock: `page` may read the context.
    found.map(|step| match step {
        Step::PageUp(_) => Step::PageUp(page()),
        Step::PageDown(_) => Step::PageDown(page()),
        other => other,
    })
}

/// Whether focus last moved by keyboard.
fn keyboard(ctx: &egui::Context) -> bool {
    ctx.data(|data| {
        data.get_temp::<bool>(crate::theme::keyboard_focus_id())
            .unwrap_or(false)
    })
}

/// Marks `row` as a row that opens a submenu, so a keyboard-opened menu can
/// open it with Right as well as Enter. Registered each frame it is drawn.
pub fn mark_submenu_row(ctx: &egui::Context, row: egui::Id) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|data| data.insert_temp(row.with("submenu-row"), frame));
}

/// Whether `row` was registered with [`mark_submenu_row`] this frame or the
/// one before.
pub(crate) fn is_submenu_row(ctx: &egui::Context, row: egui::Id) -> bool {
    let frame = ctx.cumulative_frame_nr();
    ctx.data(|data| data.get_temp::<u64>(row.with("submenu-row")))
        .is_some_and(|registered| registered + 1 >= frame)
}

fn row_of_id(item: egui::Id) -> egui::Id {
    item.with("menu-row-of")
}

fn row_last_id(row: egui::Id) -> egui::Id {
    row.with("menu-row-last")
}

/// Makes the `Ui` `ui` a row of `items` inside a menu, such as the quick
/// reactions: a Toolbar named `label` that a keyboard-opened menu walks as
/// one stop with Up and Down, and within with Left and Right. Registered
/// each frame it is drawn, after its items.
pub fn menu_row(ui: &egui::Ui, label: &str, items: &[egui::Id]) {
    let ctx = ui.ctx();
    let row = ui.unique_id();
    ctx.accesskit_node_builder(row, |node| {
        node.set_role(Role::Toolbar);
        node.set_label(label);
    });
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|data| {
        for item in items {
            data.insert_temp(row_of_id(*item), (frame, row));
        }
    });
}

/// The menu row `item` was registered in with [`menu_row`] this frame or
/// the one before.
fn row_of(ctx: &egui::Context, item: egui::Id) -> Option<egui::Id> {
    let frame = ctx.cumulative_frame_nr();
    ctx.data(|data| data.get_temp::<(u64, egui::Id)>(row_of_id(item)))
        .filter(|(registered, _)| registered + 1 >= frame)
        .map(|(_, row)| row)
}

/// One stop of a menu as its keys walk it: an item, or a row's items.
struct MenuStop {
    row: Option<egui::Id>,
    items: Vec<egui::Id>,
}

/// A menu's `items`, in order, as stops: each item on its own, but the
/// items of one row together.
fn menu_stops(ctx: &egui::Context, items: &[egui::Id]) -> Vec<MenuStop> {
    let mut stops: Vec<MenuStop> = Vec::new();
    for &item in items {
        let row = row_of(ctx, item);
        match stops.last_mut() {
            Some(last) if row.is_some() && last.row == row => last.items.push(item),
            _ => stops.push(MenuStop {
                row,
                items: vec![item],
            }),
        }
    }
    stops
}

/// The item that takes focus when the keys reach `stop`: a row's item
/// focused there last, else its first.
fn landing(ctx: &egui::Context, stop: &MenuStop) -> Option<egui::Id> {
    stop.row
        .and_then(|row| ctx.data(|data| data.get_temp::<egui::Id>(row_last_id(row))))
        .filter(|last| stop.items.contains(last))
        .or_else(|| stop.items.first().copied())
}

/// The popup `popup` and the submenus open in it, outermost first. Each is
/// a layer of its own, named by its id.
fn menu_layers(ctx: &egui::Context, popup: egui::Id) -> Vec<egui::Id> {
    use egui::containers::menu::MenuState;
    let mut layers = vec![popup];
    // Menus nest a few levels at most; the bound guards a broken chain.
    while layers.len() < 8 {
        let last = layers[layers.len() - 1];
        match MenuState::from_id(ctx, last, |state| state.open_item) {
            Some(open) => layers.push(open),
            None => break,
        }
    }
    layers
}

/// The items the last frame drew on the popup layer `layer`, in order: its
/// enabled focusable widgets, without the layer's own background. Empty
/// until a frame has drawn the items, which a popup's first, measuring pass
/// does not.
fn layer_items(ctx: &egui::Context, layer: egui::Id) -> Vec<egui::Id> {
    let background = layer.with("move");
    ctx.viewport(|viewport| {
        viewport
            .prev_pass
            .widgets
            .layers()
            .find(|(layer_id, _)| layer_id.id == layer)
            .map(|(_, widgets)| {
                widgets
                    .iter()
                    .filter(|widget| {
                        widget.enabled && widget.sense.is_focusable() && widget.id != background
                    })
                    .map(|widget| widget.id)
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// The entries this pass drew on the popup layer `layer`, in order, each
/// with whether it is enabled: the widgets that take a click, without the
/// layer's own background. A disabled menu item takes clicks without being
/// focusable. None when the layer was not drawn in this pass.
fn drawn_entries(ctx: &egui::Context, layer: egui::Id) -> Option<Vec<(egui::Id, bool)>> {
    let background = layer.with("move");
    ctx.viewport(|viewport| {
        viewport
            .this_pass
            .widgets
            .layers()
            .find(|(layer_id, _)| layer_id.id == layer)
            .map(|(_, widgets)| {
                widgets
                    .iter()
                    .filter(|widget| widget.sense.senses_click() && widget.id != background)
                    .map(|widget| (widget.id, widget.enabled && widget.sense.is_focusable()))
                    .collect()
            })
    })
}

/// Exposes the popup `popup` and its open submenus, drawn in this pass, as
/// AccessKit menus, so a screen reader says "menu" and where each item is
/// in it ("2 of 9"), as for a desktop menu. Only the stops the keys walk
/// count, in the same order: a row is one stop, its items counted within
/// it, and a disabled item, which the keys skip, has no position. A
/// submenu takes the name of its row, which says it opens one.
fn describe_menu(ctx: &egui::Context, popup: egui::Id, label: &str) {
    use egui::accesskit::HasPopup;
    use egui::containers::menu::SubMenu;
    let layers = menu_layers(ctx, popup);
    let mut parent_items: Vec<egui::Id> = Vec::new();
    for layer in layers {
        let Some(entries) = drawn_entries(ctx, layer) else {
            break;
        };
        let enabled: Vec<egui::Id> = entries
            .iter()
            .filter(|(_, enabled)| *enabled)
            .map(|(id, _)| *id)
            .collect();
        let stops = menu_stops(ctx, &enabled);
        // A popup's first pass only measures it: nothing is enabled yet.
        if stops.is_empty() {
            break;
        }
        let name = if layer == popup {
            Some(label.to_owned())
        } else {
            parent_items
                .iter()
                .find(|row| SubMenu::id_from_widget_id(**row) == layer)
                .and_then(|row| {
                    ctx.accesskit_node_builder(*row, |node| node.label().map(str::to_owned))
                        .flatten()
                })
        };
        let len = stops.len();
        // The layer's root `Ui` has the layer's id: its node holds the items.
        ctx.accesskit_node_builder(layer, |node| {
            node.set_role(Role::Menu);
            if let Some(name) = name {
                node.set_label(name);
            }
            node.set_size_of_set(len);
        });
        for (index, stop) in stops.iter().enumerate() {
            match stop.row {
                // The row's items stay buttons, counted within the row, whose
                // node holds the row's size.
                Some(row) => {
                    let count = stop.items.len();
                    ctx.accesskit_node_builder(row, |node| node.set_size_of_set(count));
                    for (position, item) in stop.items.iter().enumerate() {
                        ctx.accesskit_node_builder(*item, |node| {
                            node.set_position_in_set(position);
                            node.set_size_of_set(count);
                        });
                    }
                }
                None => {
                    let item = stop.items[0];
                    let submenu = is_submenu_row(ctx, item);
                    ctx.accesskit_node_builder(item, |node| {
                        // A choice of one among the items, such as an account
                        // in the switcher, keeps saying whether it is chosen.
                        let role = if node.role() == Role::RadioButton {
                            Role::MenuItemRadio
                        } else {
                            Role::MenuItem
                        };
                        node.set_role(role);
                        node.set_position_in_set(index);
                        node.set_size_of_set(len);
                        if submenu {
                            node.set_has_popup(HasPopup::Menu);
                        }
                    });
                }
            }
        }
        for (item, _) in entries.iter().filter(|(_, enabled)| !*enabled) {
            if row_of(ctx, *item).is_none() {
                ctx.accesskit_node_builder(*item, |node| node.set_role(Role::MenuItem));
            }
        }
        parent_items = enabled;
    }
}

/// Where keyboard focus is relative to a menu.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuFocus {
    /// On nothing drawn, as between two items while Tab moves on.
    Nowhere,
    /// On a widget outside the menu and its submenus.
    Elsewhere,
    /// On an item of the menu or of one of its open submenus.
    In,
}

fn menu_focus(ctx: &egui::Context, layers: &[egui::Id]) -> MenuFocus {
    match ctx
        .memory(|memory| memory.focused())
        .and_then(|id| ctx.read_response(id))
    {
        None => MenuFocus::Nowhere,
        Some(response) if layers.contains(&response.layer_id.id) => MenuFocus::In,
        Some(_) => MenuFocus::Elsewhere,
    }
}

/// The context menu of a list's current item, as the list's keys open it
/// (Shift+F10), or a phone number's menu, which Space or Enter on the number
/// opens in the message list. Opened that way, focus goes to its first item,
/// Tab stays among its items, Up and Down walk them in order (a row of
/// items, such as the reactions, is one stop, walked within by Left and
/// Right), Home and End go to the first and the last, focus leaving closes
/// it, and Escape or a chosen item returns focus to the list. A submenu
/// opens with Right or Enter on its row, with focus on its first item, and
/// Left or Escape closes it and returns to its row. A menu opened with the
/// pointer is left as it was.
///
/// [`KeyMenu::begin`] runs before the popup is shown, [`KeyMenu::finish`]
/// after it.
pub struct KeyMenu {
    popup: egui::Id,
    pending: egui::Id,
    /// Some(true) while the first item is still to be focused, Some(false)
    /// once it was; None for a menu the keys did not open.
    state: Option<bool>,
    was_open: bool,
    /// The name a screen reader hears the menu by; None for a popup that is
    /// not a menu, such as a drop-down's list.
    label: Option<std::borrow::Cow<'static, str>>,
    /// Whether focus goes back to where the menu came from when it closes.
    hand_back: bool,
}

impl KeyMenu {
    /// `requested` is true in the pass the list asks for the menu: it then
    /// opens once, and from there on closes as any menu does. `hold` keeps
    /// focus on a layer beside the menu, such as the reaction picker.
    pub fn begin(ctx: &egui::Context, popup: egui::Id, requested: bool, hold: bool) -> Self {
        // Only after keyboard navigation: a demo page that opens the menu
        // moves no focus.
        Self::start(ctx, popup, requested && keyboard(ctx), hold)
    }

    /// As [`KeyMenu::begin`], for the menu a button opens, such as the chat
    /// header's More: Enter or Space on the button opens it as the keys open
    /// a list's menu, and so does a screen reader's own activation, which
    /// reaches egui as a click with no pointer event (`overlay::followed`).
    /// A click opens the pointer's menu, as it always did. `asked` is a
    /// request for the menu from elsewhere, such as the linking screen's
    /// "Use another account", taken as the keys' after keyboard navigation.
    pub fn for_button(
        ctx: &egui::Context,
        popup: egui::Id,
        button: &egui::Response,
        asked: bool,
    ) -> Self {
        let activated = button.clicked()
            && !ctx.input(|input| input.pointer.any_click())
            && super::overlay::followed(ctx);
        Self::start(ctx, popup, (asked && keyboard(ctx)) || activated, false)
    }

    /// A menu opened in this pass from the keys when `opened`.
    fn start(ctx: &egui::Context, popup: egui::Id, opened: bool, hold: bool) -> Self {
        let pending = popup.with("from-keys");
        let was_open = egui::Popup::is_id_open(ctx, popup);
        if opened {
            ctx.data_mut(|data| data.insert_temp(pending, true));
        }
        let state = ctx.data(|data| data.get_temp::<bool>(pending));
        if state == Some(false) && was_open && !hold {
            Self::keys(ctx, popup);
        }
        Self {
            popup,
            pending,
            state,
            was_open,
            label: None,
            hand_back: true,
        }
    }

    /// Makes the popup a menu named `label` to a screen reader.
    pub fn menu(mut self, label: impl Into<std::borrow::Cow<'static, str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// An item that takes the reader elsewhere was chosen, such as another
    /// account: focus moves once, straight to where that item leads, not
    /// back to the menu's opener first.
    pub fn moves_on(&mut self) {
        self.hand_back = false;
    }

    /// The keys that move within a menu the keys opened, taken before egui
    /// sees them.
    fn keys(ctx: &egui::Context, popup: egui::Id) {
        use egui::containers::menu::{MenuState, SubMenu};
        let layers = menu_layers(ctx, popup);
        // egui drops focus on Escape before anything is drawn: the item
        // focused at the end of the last frame says where Escape came from.
        let escape = ctx.input(|input| input.key_pressed(egui::Key::Escape));
        let Some(focused) = ctx.memory(|memory| memory.focused()).or_else(|| {
            escape
                .then(|| ctx.data(|data| data.get_temp::<egui::Id>(popup.with("last-focus"))))
                .flatten()
        }) else {
            return;
        };
        let Some(depth) = ctx.read_response(focused).and_then(|response| {
            layers
                .iter()
                .position(|layer| *layer == response.layer_id.id)
        }) else {
            return;
        };
        let opening = popup.with("submenu-opening");
        // A submenu opened from its row: once a frame has drawn its items,
        // the first takes focus.
        if let Some(row) = ctx.data(|data| data.get_temp::<egui::Id>(opening)) {
            let submenu = SubMenu::id_from_widget_id(row);
            if focused != row {
                ctx.data_mut(|data| data.remove::<egui::Id>(opening));
            } else if layers.contains(&submenu)
                && let Some(&first) = layer_items(ctx, submenu).first()
            {
                ctx.memory_mut(|memory| memory.request_focus(first));
                ctx.data_mut(|data| data.remove::<egui::Id>(opening));
                return;
            }
        }
        let submenu_row = is_submenu_row(ctx, focused);
        // The menu is open, so its items were drawn last frame.
        let items = layer_items(ctx, layers[depth]);
        let stops = menu_stops(ctx, &items);
        let current = stops.iter().position(|stop| stop.items.contains(&focused));
        let row = current.and_then(|index| stops[index].row);
        if let Some(row) = row {
            // Where the keys come back to in this row.
            ctx.data_mut(|data| data.insert_temp(row_last_id(row), focused));
        }
        let (down, up, right, enter, back, along) = ctx.input_mut(|input| {
            let down = super::keys::take_plain(input, egui::Key::ArrowDown)
                || super::keys::take_plain(input, egui::Key::Tab);
            let up = !down
                && (super::keys::take_plain(input, egui::Key::ArrowUp)
                    || super::keys::take_exact(input, egui::Modifiers::SHIFT, egui::Key::Tab));
            // Left and Right move along a row, such as the reactions.
            let along = if row.is_none() {
                None
            } else if super::keys::take_plain(input, egui::Key::ArrowLeft) {
                Some(Step::Previous)
            } else if super::keys::take_plain(input, egui::Key::ArrowRight) {
                Some(Step::Next)
            } else {
                None
            };
            let right = submenu_row && super::keys::take_plain(input, egui::Key::ArrowRight);
            // Left, and Escape inside a submenu, close that submenu only:
            // taken here, before the whole menu would close on Escape.
            let back = depth > 0
                && ((row.is_none() && super::keys::take_plain(input, egui::Key::ArrowLeft))
                    || super::keys::take_plain(input, egui::Key::Escape));
            let enter = input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key {
                        key: egui::Key::Enter,
                        pressed: true,
                        modifiers,
                        ..
                    } if modifiers.is_none()
                )
            });
            (down, up, right, enter, back, along)
        });
        // Home and End go to the menu's first and last item, as in a
        // desktop menu: egui has no move of its own for them.
        let (home, end) = ctx.input_mut(|input| {
            let home = super::keys::take_plain(input, egui::Key::Home);
            (
                home,
                !home && super::keys::take_plain(input, egui::Key::End),
            )
        });
        if let (Some(direction), Some(row), Some(index)) = (along, row, current) {
            // egui already read the arrow as a spatial move of its own. The
            // row ends where it ends, as a toolbar does.
            let row_items = &stops[index].items;
            let target = row_items
                .iter()
                .position(|item| *item == focused)
                .and_then(|at| step(Some(at), row_items.len(), direction))
                .map(|at| row_items[at]);
            ctx.memory_mut(|memory| {
                memory.move_focus(egui::FocusDirection::None);
                if let Some(target) = target {
                    memory.request_focus(target);
                }
            });
            if let Some(target) = target {
                ctx.data_mut(|data| data.insert_temp(row_last_id(row), target));
            }
            return;
        }
        if right || back {
            // egui already read the arrow, or Escape, as a move of its own
            // at the start of the frame: a spatial move would take focus
            // anywhere on screen.
            ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
        }
        if back {
            // The submenu's row is in the layer that opened it.
            MenuState::from_id(ctx, layers[depth - 1], |state| state.open_item = None);
            let row = ctx.data(|data| data.get_temp::<egui::Id>(layers[depth].with("row")));
            if let Some(row) = row {
                ctx.memory_mut(|memory| memory.request_focus(row));
            }
            return;
        }
        if (right || enter) && submenu_row {
            if right {
                // Activates the row as Enter does, which opens its submenu.
                ctx.input_mut(|input| {
                    input.events.push(egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    });
                });
            }
            ctx.data_mut(|data| {
                data.insert_temp(opening, focused);
                data.insert_temp(SubMenu::id_from_widget_id(focused).with("row"), focused);
            });
            return;
        }
        if down || up || home || end {
            // Moving on from a row whose submenu is open closes the submenu
            // first, so the keys do not walk into it.
            if layers.len() > depth + 1 {
                MenuState::from_id(ctx, layers[depth], |state| state.open_item = None);
            }
            // Down and Tab, Up and Shift+Tab walk the items of the menu or
            // submenu focus is in, in order, and wrap, as in any desktop
            // menu. egui's own moves would go elsewhere: an arrow to the
            // nearest widget on screen, which can be one under the menu,
            // and Tab past the last item onto the menu's own background,
            // which has no name, or from a submenu into its parent menu.
            // A row is one stop, entered on its item focused last.
            let next = match current {
                _ if home => (!stops.is_empty()).then_some(0),
                _ if end => stops.len().checked_sub(1),
                Some(index) if down => Some((index + 1) % stops.len()),
                Some(index) => Some((index + stops.len() - 1) % stops.len()),
                // Not on an item (on the background): the first or the last.
                None if down => (!stops.is_empty()).then_some(0),
                None => stops.len().checked_sub(1),
            };
            let target = next.and_then(|next| landing(ctx, &stops[next]));
            ctx.memory_mut(|memory| {
                memory.move_focus(egui::FocusDirection::None);
                if let Some(target) = target {
                    memory.request_focus(target);
                }
            });
        }
    }

    /// Whether the menu was open before this pass.
    pub fn was_open(&self) -> bool {
        self.was_open
    }

    /// Whether the keys opened this menu (Enter or Shift+F10 after keyboard
    /// navigation), as opposed to the pointer.
    pub fn from_keys(&self) -> bool {
        self.state.is_some()
    }

    /// After the popup was shown on `layer` (None when it was not drawn),
    /// with `first` its first item drawn in this pass. Focus returns to
    /// `list` when the menu closes. Returns whether the menu is open now.
    pub fn finish(
        self,
        ctx: &egui::Context,
        layer: Option<egui::LayerId>,
        hold: bool,
        first: Option<egui::Id>,
        list: egui::Id,
    ) -> bool {
        // However it was opened: a menu is a menu to a screen reader. Only a
        // drawn popup has a node to probe.
        if let Some(label) = &self.label
            && layer.is_some()
            && crate::a11y::screen_reader_active(ctx, self.popup)
        {
            describe_menu(ctx, self.popup, label);
        }
        let open = egui::Popup::is_id_open(ctx, self.popup);
        let Some(focus_pending) = self.state else {
            return open;
        };
        let focus = if open {
            menu_focus(ctx, &menu_layers(ctx, self.popup))
        } else {
            MenuFocus::Nowhere
        };
        if focus == MenuFocus::In {
            // Where Escape comes from next frame, after egui drops focus.
            if let Some(focused) = ctx.memory(|memory| memory.focused()) {
                ctx.data_mut(|data| data.insert_temp(self.popup.with("last-focus"), focused));
            }
        }
        if open && !focus_pending && !hold && focus == MenuFocus::Elsewhere {
            // A shortcut (Ctrl+L, Ctrl+K, Alt+1, Alt+2) took focus out of
            // the menu: it closes, as a desktop menu does when focus leaves,
            // rather than staying open unseen and holding on to the arrows,
            // Tab and the next Escape. Focus stays where the shortcut put it.
            egui::Popup::close_id(ctx, self.popup);
            ctx.data_mut(|data| data.remove::<bool>(self.pending));
            return false;
        }
        if !open {
            ctx.data_mut(|data| data.remove::<bool>(self.pending));
            // Closed in this pass by Escape or a chosen item: back to the
            // list, drawn earlier in this pass. A click elsewhere keeps the
            // focus it gave. A menu that went away unseen, its item not drawn
            // (another chat, Settings), returns nothing: focus has moved on
            // since.
            if self.was_open
                && self.hand_back
                && !ctx.input(|input| input.pointer.any_pressed() || input.pointer.any_click())
            {
                let item = ctx.memory(|memory| memory.focused());
                super::overlay::hand_back(ctx, list, item);
            }
        } else if let Some(layer) = layer
            && !hold
        {
            // Tab stays among the menu's items, its submenus' included:
            // they are layers above it. Not beside the reaction picker,
            // whose own layer must keep its focus.
            ctx.memory_mut(|memory| memory.set_modal_layer(layer));
            // The first item, drawn in this pass, takes focus once. A
            // popup's first pass is drawn hidden and gives no widget focus:
            // the item takes it in the pass that shows it. That pass is
            // asked for at once, in this frame, rather than in the next: the
            // reader hears the menu a frame sooner, and the pointer's menus,
            // which never wait for focus, keep egui's own pace.
            if focus_pending
                && let Some(first) = first
                && ctx
                    .read_response(first)
                    .is_none_or(|response| response.visible())
            {
                ctx.memory_mut(|memory| memory.request_focus(first));
                ctx.data_mut(|data| data.insert_temp(self.pending, false));
            } else if focus_pending && !self.was_open {
                ctx.request_discard("a menu the keys opened takes focus in the pass that shows it");
            }
        }
        open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list's node hangs from the node of the Ui around it, and its
    /// label is worked out then; a hidden list has no node and no label.
    #[test]
    fn the_container_node_has_its_place_and_none_while_hidden() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let id = egui::Id::new("list");
        let list = id.accesskit_id();
        let run = |hidden: bool| {
            let mut asked = false;
            let mut outer = None;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let builder = egui::UiBuilder::new().id_salt("outer");
                let builder = if hidden { builder.invisible() } else { builder };
                ui.scope_builder(builder, |ui| {
                    // A node of its own, as the conversation around the
                    // message list has.
                    outer = Some(ui.unique_id().accesskit_id());
                    ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
                        node.set_role(Role::GenericContainer);
                    });
                    let rect = ui.max_rect();
                    container(
                        ui,
                        id,
                        rect,
                        || {
                            asked = true;
                            "Messages".to_owned()
                        },
                        3,
                    );
                });
            });
            output.textures_delta.clear();
            let tree = output
                .platform_output
                .accesskit_update
                .expect("accessibility tree");
            (tree, asked, outer.expect("drawn"))
        };
        let (tree, asked, outer) = run(false);
        assert!(asked);
        let parent = tree
            .nodes
            .iter()
            .find(|(_, node)| node.children().contains(&list))
            .map(|(id, _)| *id);
        assert_eq!(parent, Some(outer), "under the Ui around it");
        let (_, node) = tree.nodes.iter().find(|(node, _)| *node == list).unwrap();
        assert_eq!(node.role(), Role::ListBox);
        assert_eq!(node.label(), Some("Messages"));
        let (tree, asked, _) = run(true);
        assert!(!asked, "no label for a hidden list");
        assert!(
            tree.nodes.iter().all(|(node, _)| *node != list),
            "no node for a hidden list"
        );
    }

    /// The page size is worked out only for a page key.
    #[test]
    fn only_a_page_key_asks_for_the_page_size() {
        let ctx = egui::Context::default();
        let press = |key| {
            let mut asked = 0;
            let mut step = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ui| {
                    step = take_step(ui.ctx(), || {
                        asked += 1;
                        7
                    });
                },
            );
            output.textures_delta.clear();
            (step, asked)
        };
        assert_eq!(press(egui::Key::ArrowDown), (Some(Step::Next), 0));
        assert_eq!(press(egui::Key::End), (Some(Step::Last), 0));
        assert_eq!(press(egui::Key::PageDown), (Some(Step::PageDown(7)), 1));
        assert_eq!(press(egui::Key::PageUp), (Some(Step::PageUp(7)), 1));
        assert_eq!(press(egui::Key::A), (None, 0));
    }

    #[test]
    fn steps_clamp_and_handle_empty_lists() {
        assert_eq!(step(None, 0, Step::Next), None);
        assert_eq!(
            step(None, 5, Step::Next),
            Some(4),
            "no cursor starts at the newest"
        );
        assert_eq!(step(Some(0), 5, Step::Previous), Some(0));
        assert_eq!(step(Some(4), 5, Step::Next), Some(4));
        assert_eq!(step(Some(2), 5, Step::First), Some(0));
        assert_eq!(step(Some(2), 5, Step::Last), Some(4));
        assert_eq!(step(Some(4), 5, Step::PageUp(3)), Some(1));
        assert_eq!(step(Some(1), 5, Step::PageDown(10)), Some(4));
        assert_eq!(
            step(Some(9), 5, Step::Previous),
            Some(3),
            "a stale index is clamped first"
        );
    }
}
