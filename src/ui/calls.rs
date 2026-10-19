//! Calls as the history shows them: the entry a call leaves among a chat's
//! messages, and the Calls tab, laid out as WhatsApp's: the call history in
//! the list pane (or the new-call list over it), and beside it the open chat
//! or, with none open, the way to start a call.
//!
//! Both read the same entries, which come from WhatsApp's own records (the
//! phone's call log and the call-log messages in history) and from the calls
//! ZapFast took part in.

use egui::{Align, Frame, Layout, Margin, Rect, Sense, Vec2, pos2, vec2};

use crate::app::App;
use crate::i18n::{Locale, gettext};
use crate::model::{Action, CallFilter, CallStatus, Content, Message};
use crate::theme::{self, Icon};
use crate::ui::focus::{Stop, TabStop};
use crate::ui::toolbar::{ToolbarButton, toolbar};
use crate::ui::widgets;

/// One call, as its entry reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Whether this account placed the call.
    pub from_me: bool,
    pub video: bool,
    pub status: CallStatus,
    /// Seconds the call was up.
    pub duration: u64,
    pub group: bool,
}

impl Entry {
    /// The entry of a call message, if it is one.
    pub fn of(message: &Message) -> Option<Self> {
        match message.content {
            Content::Call {
                video,
                status,
                duration,
                group,
            } => Some(Self {
                from_me: message.from_me,
                video,
                status,
                duration,
                group,
            }),
            _ => None,
        }
    }

    /// One glyph for the state: the arrow says which way an answered call
    /// went, the camera what it carried, and a call that never connected has
    /// the missed or the slashed handset.
    pub fn icon(&self) -> Icon {
        match self.status {
            CallStatus::Missed | CallStatus::NoAnswer => Icon::PhoneMissed,
            CallStatus::Declined
            | CallStatus::DeclinedElsewhere
            | CallStatus::Busy
            | CallStatus::Failed
            | CallStatus::ConnectionLost => Icon::PhoneOff,
            CallStatus::Answered | CallStatus::AnsweredElsewhere => {
                if self.video {
                    Icon::Video
                } else if self.from_me {
                    Icon::PhoneOutgoing
                } else {
                    Icon::PhoneIncoming
                }
            }
        }
    }

    /// Whether this was a call to us that nobody took.
    pub fn missed(&self) -> bool {
        self.status == CallStatus::Missed && !self.from_me
    }

    /// The history's glyph: which way the call went, and a missed call's
    /// own. What it carried shows in its title and its call-back button.
    pub fn direction_icon(&self) -> Icon {
        if self.missed() {
            Icon::PhoneMissed
        } else if self.from_me {
            Icon::PhoneOutgoing
        } else {
            Icon::PhoneIncoming
        }
    }

    /// What the call was: "Missed voice call", "Video call", "Group voice call".
    pub fn title(&self, locale: Locale) -> String {
        let text = match (self.missed(), self.group, self.video) {
            (true, false, false) => gettext(locale, "Missed voice call"),
            (true, false, true) => gettext(locale, "Missed video call"),
            (true, true, false) => gettext(locale, "Missed group voice call"),
            (true, true, true) => gettext(locale, "Missed group video call"),
            (false, false, false) => gettext(locale, "Voice call"),
            (false, false, true) => gettext(locale, "Video call"),
            (false, true, false) => gettext(locale, "Group voice call"),
            (false, true, true) => gettext(locale, "Group video call"),
        };
        text.into_owned()
    }

    /// How it went: its length when it was up, else what became of it. A
    /// missed call says it in its title.
    pub fn detail(&self, locale: Locale) -> Option<String> {
        let text = match self.status {
            CallStatus::Answered if self.duration > 0 => {
                return Some(crate::util::call_length(locale, self.duration));
            }
            CallStatus::Answered | CallStatus::Missed => return None,
            CallStatus::AnsweredElsewhere => gettext(locale, "Answered on another device"),
            CallStatus::Declined => gettext(locale, "Declined"),
            CallStatus::DeclinedElsewhere => gettext(locale, "Declined on another device"),
            CallStatus::Busy => gettext(locale, "Busy"),
            CallStatus::Failed => gettext(locale, "Could not connect"),
            CallStatus::NoAnswer => gettext(locale, "No answer"),
            CallStatus::ConnectionLost => gettext(locale, "Connection lost"),
        };
        Some(text.into_owned())
    }
}

/// Calls that share one history row: consecutive calls with the same chat
/// that went the same way and ended alike, as WhatsApp folds them into one
/// row with a count. `calls` runs newest first, and so do the rows; each
/// row's first call is its newest.
pub fn rows(calls: &[Message]) -> Vec<Vec<&Message>> {
    let mut rows: Vec<Vec<&Message>> = Vec::new();
    for message in calls {
        let Some(entry) = Entry::of(message) else {
            continue;
        };
        let joins = rows.last().and_then(|row| row.last()).is_some_and(|last| {
            Entry::of(last).is_some_and(|before| {
                last.chat == message.chat
                    && before.from_me == entry.from_me
                    && before.missed() == entry.missed()
                    && before.group == entry.group
            })
        });
        match rows.last_mut() {
            Some(row) if joins => row.push(message),
            _ => rows.push(vec![message]),
        }
    }
    rows
}

/// The calls the Calls tab lists: a locked chat's stay out while its folder
/// is closed.
fn visible_calls(app: &App) -> Vec<Message> {
    app.call_log
        .iter()
        .filter(|message| {
            app.chat(&message.chat)
                .is_none_or(|chat| !chat.locked || app.locked_folder_open())
        })
        .cloned()
        .collect()
}

/// The name a row of the history goes by: its chat's, or "Call link" for
/// the calls joined through one, which belong to no chat.
fn row_name(app: &App, chat: &str) -> String {
    if chat == crate::model::CALL_LINK_CHAT {
        gettext(app.locale, "Call link").into_owned()
    } else {
        app.display_name(chat)
    }
}

/// Whether `id`'s name or number matches `needle`, a `search_key`.
fn matches(app: &App, id: &str, needle: &str) -> bool {
    needle.is_empty()
        || crate::util::search_key(&row_name(app, id)).contains(needle)
        || crate::model::phone_of(id).is_some_and(|phone| phone.contains(needle))
}

/// The Calls tab's list pane, under the header the chat list's header
/// draws for it: the call history, or the new-call list over it.
pub fn list(app: &mut App, ui: &mut egui::Ui) {
    if app.new_call {
        new_call_list(app, ui);
    } else {
        history(app, ui);
    }
}

/// The call history's focusable list: one Tab stop in the list pane, the
/// chat list's (`Stop::ChatList`), whose arrows walk its rows.
pub fn history_list_id() -> egui::Id {
    egui::Id::new("call-history-list")
}

/// The new-call list's focusable list, in the same place.
pub fn new_call_list_id() -> egui::Id {
    egui::Id::new("new-call-list")
}

/// Where a row sits in its list, for its accessible node.
#[derive(Clone, Copy)]
struct Slot {
    index: usize,
    len: usize,
}

/// The rows the history lists, newest first, a row per run of alike calls,
/// as the chip and the search pick them. None while it shows an empty state
/// instead: no calls at all, or none missed under Missed.
fn history_rows(app: &App) -> Option<Vec<Vec<Message>>> {
    let calls = visible_calls(app);
    if calls.is_empty() {
        return None;
    }
    // The chip picks rows after they fold, so a row reads as it does
    // under All.
    let missed_only = app.call_filter == CallFilter::Missed;
    let shown =
        |row: &Vec<&Message>| !missed_only || Entry::of(row[0]).is_some_and(|entry| entry.missed());
    let folded = rows(&calls);
    if !folded.iter().any(shown) {
        return None;
    }
    let needle = crate::util::search_key(app.calls_search.trim());
    Some(
        folded
            .into_iter()
            .filter(|row| shown(row) && matches(app, &row[0].chat, &needle))
            .map(|row| row.into_iter().cloned().collect())
            .collect(),
    )
}

/// How many calls the history lists, as `history_rows` picks them, every
/// call a row folds counted; None while it shows an empty state instead.
pub(crate) fn history_count(app: &App) -> Option<usize> {
    history_rows(app).map(|rows| rows.iter().map(Vec::len).sum())
}

/// Where the history's current row is among `rows`: the cursor's row while
/// it is listed, else the picked row while that is, else the first. Never a
/// row the history no longer lists, after a chip, a search or a new call.
fn history_cursor(app: &App, rows: &[Vec<Message>]) -> Option<usize> {
    let find = |wanted: Option<&(String, String)>| {
        wanted.and_then(|(chat, id)| {
            rows.iter()
                .position(|row| row[0].chat == *chat && row[0].id == *id)
        })
    };
    find(app.call_cursor.as_ref())
        .or_else(|| find(app.picked_call.as_ref().filter(|_| app.call_picked())))
        .or_else(|| (!rows.is_empty()).then_some(0))
}

/// Every call of the account, newest first, a row per run of alike calls.
fn history(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let Some(rows) = history_rows(app) else {
        let (icon, title, body) = if visible_calls(app).is_empty() {
            let (title, body) = if app.call_log_loaded {
                (
                    gettext(app.locale, "No calls yet"),
                    gettext(
                        app.locale,
                        "Calls from your phone's call log and the ones you make here appear here.",
                    ),
                )
            } else {
                (
                    gettext(app.locale, "Loading…"),
                    gettext(app.locale, "Reading this account's calls."),
                )
            };
            (Icon::Phone, title, body)
        } else {
            (
                Icon::PhoneMissed,
                gettext(app.locale, "No missed calls"),
                gettext(app.locale, "Choose All to see every call."),
            )
        };
        Frame::new()
            .inner_margin(Margin::symmetric(24, 0))
            .show(ui, |ui| {
                widgets::empty_state(ui, &palette, icon, title.as_ref(), body.as_ref());
            });
        return;
    };
    let list = history_list_id();
    let label = if app.call_filter == CallFilter::Missed {
        gettext(app.locale, "Missed calls")
    } else {
        gettext(app.locale, "Calls")
    };
    let pass = ListPass::begin(ui, list, label.as_ref(), rows.len());
    let cursor = history_cursor(app, &rows);
    let mut active = None;
    let mut actions = Vec::new();
    egui::ScrollArea::vertical()
        .id_salt("call-history")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if rows.is_empty() {
                ui.add_space(12.0);
                ui.vertical_centered(|ui| {
                    theme::text(
                        ui,
                        gettext(app.locale, "No matching calls"),
                        theme::regular(13.0),
                        palette.secondary,
                    );
                });
            }
            for (index, row) in rows.iter().enumerate() {
                let slot = Slot {
                    index,
                    len: rows.len(),
                };
                let response = ui
                    .scope_builder(
                        egui::UiBuilder::new()
                            .id_salt(("call", &row[0].chat, &row[0].id))
                            .accessibility_parent(list),
                        |ui| history_row(app, ui, row, slot, &mut actions),
                    )
                    .inner;
                if Some(index) == cursor
                    && let Some(response) = response
                {
                    active = Some(pass.current(ui, &response));
                }
            }
            ui.add_space(12.0);
        });
    pass.finish(ui.ctx(), active);
    app.actions.extend(actions);
}

/// A list of the Calls page in the pass that draws it: one focusable
/// ListBox whose current row is its active descendant, as the chat list is.
struct ListPass {
    id: egui::Id,
    focused: bool,
    /// Whether the current row is to be scrolled into view: the list was
    /// reached by keyboard, or its keys moved the row, in this pass.
    reveal: bool,
}

impl ListPass {
    /// Creates the list's container before its rows, so their nodes can hang
    /// from its node. It senses no pointer input, so pointer users see the
    /// list exactly as before.
    fn begin(ui: &mut egui::Ui, id: egui::Id, label: &str, len: usize) -> Self {
        let rect = ui.available_rect_before_wrap();
        let response = super::list_nav::container(ui, id, rect, || label.to_owned(), len)
            .tab_stop(Stop::ChatList);
        // About a screen of rows, as PgUp and PgDn move.
        let page = (rect.height() / theme::ROW_HEIGHT).floor().max(1.0) as usize;
        ui.ctx()
            .data_mut(|data| data.insert_temp(id.with("page"), page));
        let keyboard = ui.ctx().data(|data| {
            data.get_temp::<bool>(theme::keyboard_focus_id())
                .unwrap_or(false)
        });
        let moved = ui.ctx().data(|data| data.get_temp::<u64>(id.with("moved")))
            == Some(ui.ctx().cumulative_frame_nr());
        let focused = response.has_focus();
        Self {
            id,
            focused,
            reveal: focused && ((response.gained_focus() && keyboard) || moved),
        }
    }

    /// The current row, `response`, drawn in this pass: it carries the
    /// list's focus ring, drawn only after keyboard navigation, and its node
    /// is the list's active descendant.
    fn current(&self, ui: &egui::Ui, response: &egui::Response) -> egui::Id {
        if self.focused {
            theme::focus_outline(ui, self.id, response.rect, f32::from(theme::RADIUS_SMALL));
            if self.reveal {
                response.scroll_to_me_animation(None, egui::style::ScrollAnimation::none());
            }
        }
        response.id
    }

    fn finish(self, ctx: &egui::Context, active: Option<egui::Id>) {
        super::list_nav::set_active(ctx, self.id, active);
    }
}

/// Notes that the keys moved the current row of `list` in this frame: its
/// passes bring the row into view.
fn note_moved(ctx: &egui::Context, list: egui::Id) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|data| data.insert_temp(list.with("moved"), frame));
}

/// About a screen of the list `list`'s rows, as PgUp and PgDn move.
fn page_rows(ctx: &egui::Context, list: egui::Id) -> usize {
    ctx.data(|data| data.get_temp::<usize>(list.with("page")))
        .unwrap_or(8)
}

/// Where the keys ask for a row's menu: the row, and the frame they asked
/// in, whose passes open it.
fn menu_request_id(list: egui::Id) -> egui::Id {
    list.with("menu-request")
}

/// Whether the keys asked for `row`'s menu in this frame.
fn menu_requested(ctx: &egui::Context, list: egui::Id, row: egui::Id) -> bool {
    let frame = ctx.cumulative_frame_nr();
    ctx.data(|data| data.get_temp::<(egui::Id, u64)>(menu_request_id(list)))
        .is_some_and(|(asked, at)| asked == row && at == frame)
}

/// Draws the keys' menu of the list row `response`, named `label`, when the
/// keys open it, or while it is open. Only the keys open it: a click on the
/// row does what it always did, and the pointer never sees this menu.
/// `items` draws its entries, as wide as `labels`, and returns its first,
/// where focus goes.
fn row_key_menu(
    ui: &egui::Ui,
    palette: &theme::Palette,
    response: &egui::Response,
    list: egui::Id,
    label: String,
    labels: &[&str],
    items: impl FnOnce(&mut egui::Ui) -> egui::Id,
) {
    let popup_id = egui::Popup::default_response_id(response);
    let force = menu_requested(ui.ctx(), list, response.id);
    if !force && !egui::Popup::is_id_open(ui.ctx(), popup_id) {
        return;
    }
    let key_menu = super::list_nav::KeyMenu::begin(ui.ctx(), popup_id, force, false).menu(label);
    let popup = egui::Popup::menu(response)
        // The background takes clicks, not focus: a focusable one is a
        // nameless Tab stop.
        .sense(Sense::CLICK)
        .open_memory(force.then_some(egui::SetOpenCommand::Bool(true)))
        .width(widgets::menu_width(ui, labels, true).max(160.0))
        .frame(widgets::menu_frame(palette))
        .at_position(response.rect.left_top() + vec2(12.0, 8.0));
    let mut first = None;
    let menu = popup.show(|ui| {
        let item = items(ui);
        // A popup's first pass only measures it, drawing nothing that could
        // take focus; only a pass that draws the menu names its item.
        if !ui.is_sizing_pass() {
            first = Some(item);
        }
    });
    key_menu.finish(
        ui.ctx(),
        menu.as_ref().map(|menu| menu.response.layer_id),
        false,
        first,
        list,
    );
}

/// Takes this frame's keys for the focused call history or new-call list:
/// the arrows, Home, End and the page keys move its current row; Enter opens
/// a history row's chat as a click does, and a person's menu; Shift+F10
/// opens the row's menu. Returns the actions they ask for.
pub fn list_keys(app: &mut App, ctx: &egui::Context) -> Vec<Action> {
    let history = ctx.memory(|memory| memory.has_focus(history_list_id()));
    let people_list = ctx.memory(|memory| memory.has_focus(new_call_list_id()));
    if !history && !people_list {
        return Vec::new();
    }
    let list = if history {
        history_list_id()
    } else {
        new_call_list_id()
    };
    if let Some(step) = super::list_nav::take_step(ctx, || page_rows(ctx, list)) {
        if history {
            step_history(app, ctx, step);
        } else {
            step_people(app, ctx, step);
        }
        note_moved(ctx, list);
        return Vec::new();
    }
    // While a voice message records, the history's Enter opens a chat as
    // the chat list's does, and the recording goes on there; nothing places
    // a call, which would meet the recording at the microphone.
    let recording = app.recording.is_some();
    if recording && !history {
        return Vec::new();
    }
    let (enter, menu) = ctx.input_mut(|input| {
        (
            super::keys::take_plain(input, egui::Key::Enter),
            !recording && super::keys::take_exact(input, egui::Modifiers::SHIFT, egui::Key::F10),
        )
    });
    if !enter && !menu {
        return Vec::new();
    }
    // Acting from the keyboard is keyboard navigation, as Tab is: the
    // focus the keys move, into a menu or a chat, is shown.
    ctx.data_mut(|data| data.insert_temp(theme::keyboard_focus_id(), true));
    let frame = ctx.cumulative_frame_nr();
    let mut actions = Vec::new();
    if history {
        let Some(rows) = history_rows(app) else {
            return actions;
        };
        let Some(row) = history_cursor(app, &rows).map(|index| &rows[index][0]) else {
            return actions;
        };
        if row.chat == crate::model::CALL_LINK_CHAT {
            // A call joined through a link has no chat to open and nobody
            // to call back: its row's click does nothing either.
            app.announcer.say(
                crate::announcer::Category::Status,
                gettext(app.locale, "Nothing to open for a call link"),
            );
        } else if enter {
            actions.push(Action::PickCall {
                chat: row.chat.clone(),
                message: row.id.clone(),
            });
        } else {
            let id = row_id(&row.chat, &row.id);
            ctx.data_mut(|data| data.insert_temp(menu_request_id(list), (id, frame)));
        }
    } else if let Some((frequent, id)) = people_cursor(app) {
        // A row alone starts nothing, so a stray key never rings anyone:
        // Enter opens the menu, as Shift+F10 does.
        let row = person_row_id(frequent, &id);
        ctx.data_mut(|data| data.insert_temp(menu_request_id(list), (row, frame)));
    }
    actions
}

/// Moves the history's current row by `step`. Up from the first row of a
/// search goes back to the search field, where Down came from.
fn step_history(app: &mut App, ctx: &egui::Context, step: super::list_nav::Step) {
    let Some(rows) = history_rows(app) else {
        return;
    };
    let current = history_cursor(app, &rows);
    if step == super::list_nav::Step::Previous
        && current == Some(0)
        && !app.calls_search.trim().is_empty()
    {
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new(HISTORY_SEARCH)));
        return;
    }
    if let Some(index) = super::list_nav::step(current, rows.len(), step) {
        app.call_cursor = Some((rows[index][0].chat.clone(), rows[index][0].id.clone()));
    }
}

/// The search field above the call history.
pub(crate) const HISTORY_SEARCH: &str = "calls-search";
/// The search field above the new-call list.
pub(crate) const NEW_CALL_SEARCH: &str = "new-call-search";

/// Down in the Calls page's focused search field, taken before the field
/// sees it: into the list under it, on its first row, as Down from the chat
/// list's search enters its results. Moving there from the keyboard is
/// keyboard navigation, as Tab is: the ring shows the first row. Nothing
/// happens while the list is empty.
pub fn search_keyboard(app: &mut App, ui: &egui::Ui) {
    let field = egui::Id::new(if app.new_call {
        NEW_CALL_SEARCH
    } else {
        HISTORY_SEARCH
    });
    if !ui.is_enabled()
        || !ui.memory(|memory| memory.has_focus(field))
        || !ui.input(|input| {
            input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key {
                        key: egui::Key::ArrowDown,
                        pressed: true,
                        modifiers,
                        ..
                    } if modifiers.is_none()
                )
            })
        })
    {
        return;
    }
    let list = if app.new_call {
        let Some(first) = people(app).into_iter().next() else {
            return;
        };
        app.new_call_cursor = Some(first);
        new_call_list_id()
    } else {
        let Some(first) = history_rows(app).and_then(|rows| rows.into_iter().next()) else {
            return;
        };
        app.call_cursor = Some((first[0].chat.clone(), first[0].id.clone()));
        history_list_id()
    };
    ui.input_mut(|input| super::keys::take_plain(input, egui::Key::ArrowDown));
    ui.ctx()
        .data_mut(|data| data.insert_temp(theme::keyboard_focus_id(), true));
    note_moved(ui.ctx(), list);
    ui.memory_mut(|memory| memory.request_focus(list));
}

/// Stable id of a call filter chip, for tests.
pub fn filter_chip_id(filter: CallFilter) -> egui::Id {
    egui::Id::new(("call-filter", filter as u8))
}

/// The chips under the Calls page's search field, drawn as the chat list's
/// are: every call, or the missed ones. They stay while a search is typed,
/// which looks among the chip's calls. A missed call has no unread state,
/// so Missed carries no count.
pub fn filter_chips(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    ui.add_space(8.0);
    // One toolbar, as the chat list's filters are.
    let label = gettext(app.locale, "Filters");
    toolbar(ui, filters_toolbar_id(), &label, Stop::Filters, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 6.0);
            for filter in CallFilter::EVERY {
                let selected = app.call_filter == filter;
                let chip = widgets::filter_chip(
                    ui,
                    &palette,
                    filter.label(app.locale).as_ref(),
                    0,
                    selected,
                )
                .in_toolbar(filters_toolbar_id());
                ui.ctx()
                    .data_mut(|data| data.insert_temp(filter_chip_id(filter), chip.rect));
                if chip.clicked() {
                    // A second click on Missed returns to every call, as the
                    // chat list's chips do.
                    let next = if selected { CallFilter::All } else { filter };
                    if next != app.call_filter {
                        app.actions.push(Action::SetCallFilter(next));
                    }
                }
            }
        });
    });
}

/// The Calls page's filter chips' toolbar.
pub(crate) fn filters_toolbar_id() -> egui::Id {
    egui::Id::new("call-filters-toolbar")
}

/// Stable id of a history row, for tests.
pub fn row_id(chat: &str, message: &str) -> egui::Id {
    egui::Id::new(("call-row", chat, message))
}

/// One history row, as WhatsApp draws it: the person, a count when calls
/// fold together, the way the newest went and what it was, when, and a
/// button to call back the same way. A click opens the chat beside the
/// list at that call. The row is the history list's item at `slot`.
fn history_row(
    app: &mut App,
    ui: &mut egui::Ui,
    row: &[Message],
    slot: Slot,
    actions: &mut Vec<Action>,
) -> Option<egui::Response> {
    let message = &row[0];
    let entry = Entry::of(message)?;
    let palette = app.palette;
    let locale = app.locale;
    // A call joined through a link has no chat to open or call back, and no
    // photo: its row only records the call.
    let link = message.chat == crate::model::CALL_LINK_CHAT;
    let name = row_name(app, &message.chat);
    let picture = if link {
        None
    } else {
        app.avatar(&message.chat)
    };
    let selected = app.call_picked()
        && app
            .picked_call
            .as_ref()
            .is_some_and(|(chat, id)| *chat == message.chat && *id == message.id);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, theme::ROW_HEIGHT), Sense::hover());
    // The row's own target goes first, so the call-back button inside keeps
    // its clicks. Clicks only, never focus: the keyboard reaches rows through
    // the history's one Tab stop and its arrows.
    let response = ui
        .interact(rect, row_id(&message.chat, &message.id), Sense::CLICK)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let mut label = name.clone();
    if row.len() > 1 {
        label = format!("{label} ({})", row.len());
    }
    let summary = summary(&entry, locale);
    let stamp = crate::util::chat_stamp(locale, message.timestamp);
    let spoken = format!("{label}, {summary}, {stamp}");
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            spoken.as_str(),
        )
    });
    super::list_nav::mark_row(
        ui.ctx(),
        response.id,
        spoken,
        slot.index,
        slot.len,
        rect,
        selected,
    );
    if response.clicked() && !link {
        actions.push(Action::PickCall {
            chat: message.chat.clone(),
            message: message.id.clone(),
        });
    }
    // One-to-one calls only: a group call has no way back from here.
    let button_rect = Rect::from_center_size(
        pos2(rect.right() - 30.0, rect.center().y),
        Vec2::splat(34.0),
    );
    if !entry.group && !link {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(button_rect)
                .layout(Layout::centered_and_justified(egui::Direction::LeftToRight))
                // Inside its row's item, not beside the list's items.
                .accessibility_parent(response.id),
        );
        let (icon, tooltip) = if entry.video {
            (Icon::Video, gettext(locale, "Video call"))
        } else {
            (Icon::Phone, gettext(locale, "Voice call"))
        };
        if theme::icon_button(
            &mut child,
            icon,
            18.0,
            palette.secondary,
            palette.text,
            tooltip.as_ref(),
        )
        .clicked()
        {
            actions.push(if entry.video {
                Action::StartVideoCall(message.chat.clone())
            } else {
                Action::StartCall(message.chat.clone())
            });
        }
    }
    if !link {
        history_menu(app, ui, &response, &message.chat, entry.group, actions);
    }
    if !ui.is_rect_visible(rect) {
        return Some(response);
    }
    if selected {
        widgets::row_highlight(ui, &palette, rect, palette.surface_active);
    } else if response.hovered() {
        widgets::row_highlight(ui, &palette, rect, palette.surface_hover);
    }
    let avatar_rect =
        Rect::from_center_size(pos2(rect.left() + 38.0, rect.center().y), Vec2::splat(48.0));
    if link {
        crate::ui::call::paint_link_avatar(ui, &palette, avatar_rect);
    } else {
        widgets::paint_avatar(
            ui,
            &palette,
            avatar_rect,
            &name,
            &message.chat,
            picture.as_deref(),
        );
    }
    let left = rect.left() + 76.0;
    let right = button_rect.left() - 6.0;
    let stamp_galley = ui
        .painter()
        .layout_no_wrap(stamp, theme::regular(11.5), palette.dim);
    let name_top = rect.top() + 14.0;
    ui.painter().galley(
        pos2(right - stamp_galley.size().x, name_top + 2.0),
        stamp_galley.clone(),
        palette.dim,
    );
    // A missed call's name is red, as on the phone.
    let name_color = if entry.missed() {
        palette.danger
    } else {
        palette.text
    };
    let name_line = widgets::line(
        ui,
        &label,
        theme::medium(14.5),
        name_color,
        (right - stamp_galley.size().x - 8.0 - left).max(0.0),
        1,
    );
    name_line.paint(ui, pos2(left, name_top), name_color);
    let glyph_rect = Rect::from_min_size(pos2(left, rect.top() + 39.0), Vec2::splat(15.0));
    theme::paint_icon(
        ui,
        entry.direction_icon(),
        glyph_rect,
        15.0,
        if entry.status.is_bad() {
            palette.danger
        } else {
            palette.accent
        },
    );
    let detail = widgets::line(
        ui,
        &summary,
        theme::regular(13.0),
        palette.secondary,
        (right - left - 21.0).max(0.0),
        1,
    );
    detail.paint(ui, pos2(left + 21.0, rect.top() + 38.0), palette.secondary);
    Some(response)
}

/// A history row's menu, which only the keys open (Shift+F10): calling the
/// row's person back by voice or video, as its button calls back the way the
/// call went. A group call has no way back from here, and its menu's one
/// row says why.
fn history_menu(
    app: &App,
    ui: &egui::Ui,
    response: &egui::Response,
    chat: &str,
    group: bool,
    actions: &mut Vec<Action>,
) {
    let palette = app.palette;
    let voice = gettext(app.locale, "Voice call");
    let video = gettext(app.locale, "Video call");
    let phone = gettext(app.locale, "Group calls happen on the phone");
    let labels: Vec<&str> = if group {
        vec![phone.as_ref()]
    } else {
        vec![voice.as_ref(), video.as_ref()]
    };
    row_key_menu(
        ui,
        &palette,
        response,
        history_list_id(),
        app.display_name(chat),
        &labels,
        |ui| {
            if group {
                // The menu's one row takes focus, unavailable, so the reader
                // hears why there is nothing to choose.
                return widgets::menu_item_focusable(
                    ui,
                    &palette,
                    Some(Icon::Phone),
                    &phone,
                    &phone,
                    false,
                    true,
                )
                .id;
            }
            let first = widgets::menu_item_response(ui, &palette, Some(Icon::Phone), &voice, true);
            if first.clicked() {
                actions.push(Action::StartCall(chat.to_owned()));
            }
            if widgets::menu_item(ui, &palette, Some(Icon::Video), &video) {
                actions.push(Action::StartVideoCall(chat.to_owned()));
            }
            first.id
        },
    );
}

/// What a call was and how it went, on one line: "Video call · 12 min 34 s".
fn summary(entry: &Entry, locale: Locale) -> String {
    match entry.detail(locale) {
        Some(detail) => format!("{} · {detail}", entry.title(locale)),
        None => entry.title(locale),
    }
}

/// How many people the new-call list offers as frequently contacted.
const FREQUENT: usize = 3;

/// The people called most often, most first, from the call history; the
/// newer call breaks a tie. Groups and ourselves stay out.
pub fn frequent(calls: &[Message], me: Option<&str>) -> Vec<String> {
    let mut counts: Vec<(String, usize, i64)> = Vec::new();
    for message in calls {
        if !matches!(
            crate::model::ChatKind::from_id(&message.chat),
            crate::model::ChatKind::Direct
        ) || me == Some(message.chat.as_str())
            || message.chat == crate::model::CALL_LINK_CHAT
        {
            continue;
        }
        match counts.iter_mut().find(|(chat, ..)| *chat == message.chat) {
            Some((_, count, newest)) => {
                *count += 1;
                *newest = (*newest).max(message.timestamp);
            }
            None => counts.push((message.chat.clone(), 1, message.timestamp)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));
    counts
        .into_iter()
        .take(FREQUENT)
        .map(|(chat, ..)| chat)
        .collect()
}

/// The people the new-call list offers while its search holds `needle`, a
/// `search_key`: the frequently contacted with no search, then every
/// contact the search finds.
fn people_lists(app: &App, needle: &str) -> (Vec<String>, Vec<crate::model::Contact>) {
    let contacts = super::dialogs::contact_choices(app, needle);
    let frequent: Vec<String> = if needle.is_empty() {
        let calls = visible_calls(app);
        frequent(&calls, app.me.as_deref())
            .into_iter()
            .filter(|chat| !app.chat(chat).is_some_and(|chat| chat.locked))
            .collect()
    } else {
        Vec::new()
    };
    (frequent, contacts)
}

/// The new-call list's rows in order, as its keys walk them: whether each
/// is among the frequently contacted, and the person.
fn people(app: &App) -> Vec<(bool, String)> {
    let needle = crate::util::search_key(app.new_call_search.trim());
    let (frequent, contacts) = people_lists(app, &needle);
    people_rows(&frequent, &contacts)
}

/// The new-call list's rows built from `people_lists`' two lists: the
/// frequently contacted first, then the contacts.
fn people_rows(frequent: &[String], contacts: &[crate::model::Contact]) -> Vec<(bool, String)> {
    frequent
        .iter()
        .map(|chat| (true, chat.clone()))
        .chain(contacts.iter().map(|contact| (false, contact.id.clone())))
        .collect()
}

/// Where the new-call list's current row is among `rows`: the cursor's
/// while it is listed, else the first.
fn people_index(app: &App, rows: &[(bool, String)]) -> Option<usize> {
    app.new_call_cursor
        .as_ref()
        .and_then(|cursor| rows.iter().position(|row| row == cursor))
        .or_else(|| (!rows.is_empty()).then_some(0))
}

/// The new-call list's current row, as the list resolves it.
fn people_cursor(app: &App) -> Option<(bool, String)> {
    let rows = people(app);
    people_index(app, &rows).map(|index| rows[index].clone())
}

/// Moves the new-call list's current row by `step`. Up from the first row
/// goes back to the search field above it.
fn step_people(app: &mut App, ctx: &egui::Context, step: super::list_nav::Step) {
    let rows = people(app);
    let current = people_index(app, &rows);
    if step == super::list_nav::Step::Previous && current == Some(0) {
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new(NEW_CALL_SEARCH)));
        return;
    }
    if let Some(index) = super::list_nav::step(current, rows.len(), step) {
        app.new_call_cursor = Some(rows[index].clone());
    }
}

/// Stable id of a row of the new-call list: among the frequently contacted
/// or among every contact, and the person.
pub fn person_row_id(frequent: bool, id: &str) -> egui::Id {
    egui::Id::new(("new-call-row", frequent, id))
}

/// WhatsApp's new-call list: the people called most often, then every
/// contact, each with a voice and a video button. A row alone starts
/// nothing, so a stray click never rings anyone.
fn new_call_list(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let needle = crate::util::search_key(app.new_call_search.trim());
    let (frequent, contacts) = people_lists(app, &needle);
    let rows = people_rows(&frequent, &contacts);
    let list = new_call_list_id();
    let label = gettext(app.locale, "New call");
    let pass = ListPass::begin(ui, list, label.as_ref(), rows.len());
    let cursor = people_index(app, &rows).map(|index| rows[index].clone());
    let mut active = None;
    let mut actions = Vec::new();
    let len = rows.len();
    egui::ScrollArea::vertical()
        .id_salt("new-call-contacts")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut index = 0;
            let mut row = |app: &mut App, ui: &mut egui::Ui, frequent: bool, id: &str| {
                let slot = Slot { index, len };
                index += 1;
                let response = ui
                    .scope_builder(
                        egui::UiBuilder::new()
                            .id_salt(if frequent {
                                egui::Id::new(("frequent", id))
                            } else {
                                egui::Id::new(id)
                            })
                            .accessibility_parent(list),
                        |ui| person(app, ui, id, frequent, slot, &mut actions),
                    )
                    .inner;
                if cursor
                    .as_ref()
                    .is_some_and(|(at, person)| *at == frequent && person == id)
                {
                    active = Some(pass.current(ui, &response));
                }
            };
            if !frequent.is_empty() {
                section(ui, &palette, &gettext(app.locale, "Frequently contacted"));
                for chat in &frequent {
                    row(app, ui, true, chat);
                }
            }
            if contacts.is_empty() {
                ui.add_space(12.0);
                ui.vertical_centered(|ui| {
                    theme::text(
                        ui,
                        gettext(app.locale, "No matching contacts"),
                        theme::regular(13.0),
                        palette.secondary,
                    );
                });
            } else {
                section(ui, &palette, &gettext(app.locale, "All contacts"));
            }
            for contact in &contacts {
                row(app, ui, false, &contact.id);
            }
            ui.add_space(12.0);
        });
    pass.finish(ui.ctx(), active);
    if !actions.is_empty() {
        actions.push(Action::ShowNewCall(false));
    }
    app.actions.extend(actions);
}

fn section(ui: &mut egui::Ui, palette: &theme::Palette, label: &str) {
    ui.add_space(10.0);
    Frame::new()
        .inner_margin(Margin {
            left: 16,
            right: 14,
            top: 4,
            bottom: 6,
        })
        .show(ui, |ui| {
            theme::text(ui, label, theme::medium(13.0), palette.accent);
        });
}

/// One person on the new-call list, with a voice and a video button: the
/// list's item at `slot`, among the frequently contacted or not.
fn person(
    app: &mut App,
    ui: &mut egui::Ui,
    id: &str,
    frequent: bool,
    slot: Slot,
    actions: &mut Vec<Action>,
) -> egui::Response {
    let palette = app.palette;
    let locale = app.locale;
    let name = app.display_name(id);
    let picture = app.avatar(id);
    let (rect, _) = ui.allocate_exact_size(
        vec2(ui.available_width(), theme::ROW_HEIGHT),
        Sense::hover(),
    );
    let response = ui.interact(rect, person_row_id(frequent, id), Sense::hover());
    let phone = crate::model::phone_of(id).map(crate::util::phone);
    let spoken = match &phone {
        Some(phone) => format!("{name}, {phone}"),
        None => name.clone(),
    };
    super::list_nav::mark_row(
        ui.ctx(),
        response.id,
        spoken,
        slot.index,
        slot.len,
        rect,
        false,
    );
    let buttons = Rect::from_min_max(
        pos2(rect.right() - 92.0, rect.top()),
        pos2(rect.right() - 12.0, rect.bottom()),
    );
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            widgets::row_highlight(ui, &palette, rect, palette.surface_hover);
        }
        let avatar_rect =
            Rect::from_center_size(pos2(rect.left() + 38.0, rect.center().y), Vec2::splat(48.0));
        widgets::paint_avatar(ui, &palette, avatar_rect, &name, id, picture.as_deref());
        let left = rect.left() + 76.0;
        let width = (buttons.left() - 8.0 - left).max(0.0);
        let name_line = widgets::line(ui, &name, theme::medium(14.5), palette.text, width, 1);
        name_line.paint(ui, pos2(left, rect.top() + 14.0), palette.text);
        if let Some(phone) = &phone {
            let phone_line = widgets::line(ui, phone, theme::regular(13.0), palette.dim, width, 1);
            phone_line.paint(ui, pos2(left, rect.top() + 38.0), palette.dim);
        }
    }
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(buttons)
            .layout(Layout::right_to_left(Align::Center))
            // Inside its row's item, not beside the list's items.
            .accessibility_parent(response.id),
    );
    let video = gettext(locale, "Video call {name}").replace("{name}", &name);
    if theme::icon_button(
        &mut child,
        Icon::Video,
        18.0,
        palette.secondary,
        palette.text,
        &video,
    )
    .clicked()
    {
        actions.push(Action::StartVideoCall(id.to_owned()));
    }
    child.add_space(4.0);
    let voice = gettext(locale, "Voice call {name}").replace("{name}", &name);
    if theme::icon_button(
        &mut child,
        Icon::Phone,
        18.0,
        palette.secondary,
        palette.text,
        &voice,
    )
    .clicked()
    {
        actions.push(Action::StartCall(id.to_owned()));
    }
    // The keys' menu: what the two buttons do.
    let voice = gettext(locale, "Voice call");
    let video = gettext(locale, "Video call");
    row_key_menu(
        ui,
        &palette,
        &response,
        new_call_list_id(),
        name,
        &[voice.as_ref(), video.as_ref()],
        |ui| {
            let first = widgets::menu_item_response(ui, &palette, Some(Icon::Phone), &voice, true);
            if first.clicked() {
                actions.push(Action::StartCall(id.to_owned()));
            }
            if widgets::menu_item(ui, &palette, Some(Icon::Video), &video) {
                actions.push(Action::StartVideoCall(id.to_owned()));
            }
            first.id
        },
    );
    response
}

/// The Calls tab's main pane when no chat is open beside the history:
/// WhatsApp's start of a call. An open chat shows there instead, the one
/// picked from the history or the one that was open before.
pub fn show(app: &mut App, ui: &mut egui::Ui) {
    start(app, ui);
}

/// The pane with no chat open: what WhatsApp shows there, a way to start a
/// call, and what ZapFast's calls are.
fn start(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    let locale = app.locale;
    let height = ui.available_height();
    ui.vertical_centered(|ui| {
        ui.add_space((height / 2.0 - 110.0).max(24.0));
        let label = gettext(locale, "Start call");
        let response = theme::circle_button(
            ui,
            Icon::PhonePlus,
            72.0,
            palette.accent,
            palette.accent_hover,
            palette.on_accent,
            label.as_ref(),
        );
        if response.clicked() {
            app.actions.push(Action::ShowNewCall(true));
        }
        ui.add_space(10.0);
        theme::text(ui, label.as_ref(), theme::semibold(15.0), palette.text);
        ui.add_space(18.0);
        ui.set_max_width(360.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(gettext(
                    locale,
                    "Call one person by voice or video, or choose a call in the list to see it.",
                ))
                .font(theme::regular(13.5))
                .color(palette.secondary),
            )
            .wrap()
            .selectable(false),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADA: &str = "4917000000001@s.whatsapp.net";
    const GRACE: &str = "4917000000002@s.whatsapp.net";

    fn call(chat: &str, id: &str, from_me: bool, timestamp: i64, status: CallStatus) -> Message {
        Message {
            id: id.to_owned(),
            chat: chat.to_owned(),
            sender: chat.to_owned(),
            sender_name: None,
            from_me,
            timestamp,
            content: Content::Call {
                video: false,
                status,
                duration: 0,
                group: false,
            },
            status: crate::model::Delivery::None,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: Vec::new(),
            history_order: None,
            edited: false,
            mentions: Vec::new(),
            forwarded: false,
            thumbnail: None,
        }
    }

    #[test]
    fn alike_calls_in_a_row_fold_into_one_history_row() {
        let log = [
            call(ADA, "1", false, 60, CallStatus::Missed),
            call(ADA, "2", false, 50, CallStatus::Missed),
            call(ADA, "3", false, 40, CallStatus::Answered),
            call(GRACE, "4", true, 30, CallStatus::Answered),
            call(ADA, "5", false, 20, CallStatus::Answered),
        ];
        let rows: Vec<Vec<&str>> = rows(&log)
            .iter()
            .map(|row| row.iter().map(|call| call.id.as_str()).collect())
            .collect();
        assert_eq!(rows, [vec!["1", "2"], vec!["3"], vec!["4"], vec!["5"]]);
    }

    #[test]
    fn the_new_call_list_offers_the_people_called_most() {
        let log = [
            call(GRACE, "1", true, 60, CallStatus::Answered),
            call(ADA, "2", false, 50, CallStatus::Missed),
            call(ADA, "3", false, 40, CallStatus::Answered),
            call(
                "120363000000000000@g.us",
                "4",
                false,
                30,
                CallStatus::Missed,
            ),
            call("me@s.whatsapp.net", "5", true, 20, CallStatus::Answered),
            call(
                crate::model::CALL_LINK_CHAT,
                "6",
                true,
                10,
                CallStatus::Answered,
            ),
        ];
        assert_eq!(frequent(&log, Some("me@s.whatsapp.net")), [ADA, GRACE]);
    }

    fn calls_app() -> (tempfile::TempDir, App, egui::Context) {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _events) = App::headless(
            crate::paths::AppDirs::under(directory.path()),
            crate::settings::Settings::default(),
        );
        app.page = crate::model::Page::Calls;
        app.call_log = vec![
            call(ADA, "1", false, 60, CallStatus::Missed),
            call(GRACE, "2", true, 30, CallStatus::Answered),
        ];
        app.call_log_loaded = true;
        let ctx = egui::Context::default();
        app.attach(&ctx);
        (directory, app, ctx)
    }

    fn frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 600.0))),
                events,
                ..Default::default()
            },
            |ui| list(app, ui),
        );
        output.textures_delta.clear();
    }

    fn click(at: egui::Pos2) -> Vec<egui::Event> {
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        vec![egui::Event::PointerMoved(at), press(true), press(false)]
    }

    #[test]
    fn a_history_row_opens_its_chat_and_its_button_calls_back() {
        let (_directory, mut app, ctx) = calls_app();
        frame(&mut app, &ctx, vec![]);
        let row = ctx.read_response(row_id(ADA, "1")).unwrap().rect;
        frame(&mut app, &ctx, click(row.left_center() + vec2(40.0, 0.0)));
        frame(&mut app, &ctx, vec![]);
        assert!(matches!(
            app.actions.as_slice(),
            [Action::PickCall { chat, message }] if chat == ADA && message == "1"
        ));
        app.actions.clear();
        // The call-back button sits at the row's right end.
        frame(&mut app, &ctx, click(row.right_center() - vec2(30.0, 0.0)));
        frame(&mut app, &ctx, vec![]);
        assert!(
            matches!(app.actions.as_slice(), [Action::StartCall(chat)] if chat == ADA),
            "{:?}",
            app.actions
        );
    }

    /// A call joined through a link is listed as "Call link", with nothing
    /// to open and nobody to call back: a click on it does nothing.
    #[test]
    fn a_call_link_row_only_records_the_call() {
        let (_directory, mut app, ctx) = calls_app();
        let link = crate::model::CALL_LINK_CHAT;
        app.call_log
            .insert(0, call(link, "9", true, 90, CallStatus::Answered));
        app.calls_search = "call link".to_owned();
        frame(&mut app, &ctx, vec![]);
        let row = ctx
            .read_response(row_id(link, "9"))
            .expect("the row is listed and found by its name")
            .rect;
        frame(&mut app, &ctx, click(row.left_center() + vec2(40.0, 0.0)));
        frame(&mut app, &ctx, click(row.right_center() - vec2(30.0, 0.0)));
        frame(&mut app, &ctx, vec![]);
        assert!(app.actions.is_empty(), "{:?}", app.actions);
    }

    #[test]
    fn a_call_from_the_new_call_list_closes_it() {
        let (_directory, mut app, ctx) = calls_app();
        app.new_call = true;
        app.chats.push(crate::model::Chat::new(
            GRACE.to_owned(),
            "Grace".to_owned(),
        ));
        frame(&mut app, &ctx, vec![]);
        // The voice button is the second from the right on each row: the
        // first one a click down the list meets calls.
        let mut found = None;
        for y in (0..600).step_by(4) {
            let at = egui::pos2(400.0 - 12.0 - 30.0 - 4.0 - 15.0, y as f32);
            app.actions.clear();
            frame(&mut app, &ctx, click(at));
            frame(&mut app, &ctx, vec![]);
            if !app.actions.is_empty() {
                found = Some(app.actions.clone());
                break;
            }
        }
        let actions = found.expect("a voice button answers a click");
        assert!(
            matches!(
                actions.as_slice(),
                [Action::StartCall(_), Action::ShowNewCall(false)]
            ),
            "{actions:?}"
        );
    }

    fn entry(from_me: bool, video: bool, status: CallStatus) -> Entry {
        Entry {
            from_me,
            video,
            status,
            duration: 0,
            group: false,
        }
    }

    #[test]
    fn the_states_a_reader_tells_apart_have_different_glyphs() {
        let glyphs = [
            entry(false, false, CallStatus::Answered).icon(),
            entry(true, false, CallStatus::Answered).icon(),
            entry(false, false, CallStatus::Missed).icon(),
            entry(false, false, CallStatus::Declined).icon(),
            entry(true, true, CallStatus::Answered).icon(),
        ];
        for (index, first) in glyphs.iter().enumerate() {
            for second in &glyphs[index + 1..] {
                assert_ne!(first, second);
            }
        }
    }

    #[test]
    fn an_entry_says_what_the_call_was_and_how_it_went() {
        let locale = Locale::default();
        let mut answered = entry(true, false, CallStatus::Answered);
        answered.duration = 221;
        assert_eq!(answered.title(locale), "Voice call");
        assert_eq!(answered.detail(locale).as_deref(), Some("3 min 41 s"));
        let missed = entry(false, true, CallStatus::Missed);
        assert_eq!(missed.title(locale), "Missed video call");
        assert_eq!(missed.detail(locale), None, "the title already says it");
        let refused = entry(true, false, CallStatus::NoAnswer);
        assert_eq!(refused.detail(locale).as_deref(), Some("No answer"));
    }
}
