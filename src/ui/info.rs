//! The info panel beside the open chat: a contact's, group's or channel's
//! picture, name and actions, its members, and the pictures, videos,
//! documents and links shared in it. It takes a column at the right, and
//! the conversation narrows beside it; when the chat list leaves too little
//! room, the list steps aside. In the narrow window mode it is a page of its
//! own over the chat, with Back.

use egui::{Align, Align2, Color32, CornerRadius, Frame, Margin, Rect, Sense, Stroke, Vec2};
use egui::{pos2, vec2};

use crate::app::{App, InfoFocus};
use crate::model::{Action, ChatMedia, Content, Dialog, InfoView, MediaTab, Message};
use crate::theme::{self, Icon};

use super::focus::{Stop, TabStop};
use super::pane::{self, Docking};
use super::widgets;

/// The panel's side margin.
const MARGIN: f32 = 14.0;
/// Space between the cells of the picture grid and strip.
const GAP: f32 = 4.0;
/// A document or link row.
const ROW_HEIGHT: f32 = 60.0;
/// The panel's Tab stops after its close button, in reading order: the
/// group photo, the rename pencil, the name editor's check, the action
/// buttons, the media row, the media view's tabs, Leave, and Try again.
const PHOTO_STOP: u8 = 0;
const PENCIL_STOP: u8 = 1;
const NAME_CHECK_STOP: u8 = 2;
const ACTION_STOP: u8 = 3;
pub const MEDIA_ROW_STOP: u8 = 10;
pub const TAB_STOP: u8 = 20;
/// The open tab's list in the media view, after its tabs.
const LIST_STOP: u8 = 23;
/// A group's members, after the media row.
const MEMBERS_STOP: u8 = 24;
const LEAVE_STOP: u8 = 30;
const RETRY_STOP: u8 = 31;

/// The media view's title, and the overview row that opens it.
fn media_title(app: &App) -> String {
    crate::i18n::gettext(app.locale, "Media, links and docs").into_owned()
}

/// The texture name a listed picture's preview is drawn under: the panel's
/// own, so freeing it when the panel closes leaves the bubbles' alone.
pub(crate) fn thumbnail_uri(chat: &str, id: &str) -> String {
    format!(
        "bytes://info-{}-{}",
        chat.chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>(),
        id
    )
}

/// Frees the pictures the panel no longer lists. Called on every frame,
/// the lock and login screens included, so a closed panel's pictures do
/// not wait for the chats to be drawn again.
pub fn release_thumbnails(app: &mut App, ctx: &egui::Context) {
    for uri in std::mem::take(&mut app.info_released) {
        app.info_textures.remove(&uri);
        crate::image_cache::forget(ctx, &uri);
    }
}

/// Docks the panel when there is room, before the conversation is laid out.
/// Otherwise returns the conversation's rect for [`show_overlay`], drawn
/// after the conversation so it lies on top.
pub fn show(app: &mut App, ui: &mut egui::Ui) -> Option<Rect> {
    if !app.info_visible() {
        ui.ctx().data_mut(|data| data.remove::<bool>(docked_id()));
        // The thumbnails go with the panel, not with the resident window.
        for uri in std::mem::take(&mut app.info_textures) {
            crate::image_cache::forget(ui.ctx(), &uri);
        }
        return None;
    }
    // A narrow window shows the panel as a page of its own over the chat.
    if super::narrow(ui.ctx()) {
        return Some(ui.available_rect_before_wrap());
    }
    // Otherwise it takes a column beside the conversation, which narrows.
    let wanted = app.settings.info_pane_width;
    match pane::docked(
        app,
        ui,
        "chat-info-pane",
        wanted,
        pane::INFO_MIN,
        pane::INFO_CONVERSATION_MIN,
        |app, ui| contents(app, ui, false),
    ) {
        Docking::Docked { width, wanted } => {
            // Only a drag moves the edge off the width asked for.
            if (width - wanted).abs() > 1.0 {
                app.settings.info_pane_width = width;
                app.actions.push(Action::SettingsChanged);
            }
            ui.ctx()
                .data_mut(|data| data.insert_temp(docked_id(), true));
            None
        }
        Docking::Overlay(region) => Some(region),
    }
}

/// The panel as a page over the whole conversation in a narrow window.
pub fn show_overlay(app: &mut App, ctx: &egui::Context, region: Rect) {
    let width = region.width();
    pane::overlay(app, ctx, "chat-info-overlay", width, region, |app, ui| {
        contents(app, ui, true)
    });
    ctx.data_mut(|data| data.insert_temp(docked_id(), false));
}

/// Whether the panel was docked (`true`) or laid over the chat (`false`)
/// this frame, for interaction tests.
pub fn docked_id() -> egui::Id {
    egui::Id::new("chat-info-docked")
}

/// The panel's contents. Over a narrow conversation (`overlay`) an item
/// that goes to a message closes the panel, so the message can be seen.
fn contents(app: &mut App, ui: &mut egui::Ui, overlay: bool) {
    let (chat, view) = match &app.info {
        Some(info) => (info.chat.clone(), info.view),
        None => return,
    };
    // A region named by what the panel is about, whichever view it shows: a
    // screen reader says it as focus moves in from the conversation, and
    // not again between the overview and the media view.
    let region = panel_title(app, &chat);
    ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
        node.set_role(egui::accesskit::Role::Region);
        node.set_label(region);
    });
    match view {
        InfoView::Overview => overview(app, ui, &chat, overlay),
        // A locked chat's media is never drawn: the overview shows instead.
        InfoView::Media(_) if !app.info_media_allowed() => {
            app.actions.push(Action::ShowInfo(InfoView::Overview));
            overview(app, ui, &chat, overlay);
        }
        InfoView::Media(tab) => media_view(app, ui, &chat, tab, overlay),
    }
}

/// The pane-style header: a button at the left, then the title. `focus`
/// says whether the keyboard moves to the button this frame.
fn header(
    app: &mut App,
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    title: &str,
    action: Action,
    focus: bool,
) {
    let palette = app.palette;
    Frame::new()
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            let room = pane::caption_header(ui, vec2(14.0, 10.0));
            ui.horizontal(|ui| {
                ui.set_min_height(32.0);
                ui.add_space(room.left);
                let button =
                    theme::icon_button(ui, icon, 18.0, palette.secondary, palette.text, label)
                        .tab_stop(Stop::InfoClose);
                if focus {
                    button.request_focus();
                }
                if button.clicked() {
                    app.actions.push(action);
                }
                ui.add_space(6.0);
                let title = theme::text(ui, title, theme::semibold(16.0), palette.text);
                heading(ui, &title, 2);
            });
        });
    ui.painter().hline(
        ui.max_rect().x_range(),
        ui.cursor().top(),
        Stroke::new(1.0, palette.outline),
    );
}

/// Marks a line of text as a heading of `level`, which a screen reader
/// names as one and can move by: the panel's title, then its sections.
fn heading(ui: &egui::Ui, response: &egui::Response, level: usize) {
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_role(egui::accesskit::Role::Heading);
        node.set_level(level);
        // egui gives a label's text as its value; a heading is named by it.
        if let Some(text) = node.value().map(str::to_owned) {
            node.set_label(text);
            node.clear_value();
        }
    });
}

/// The overview's title, which names the panel too: "Group info",
/// "Channel info" or "Contact info", by the chat it shows.
fn panel_title(app: &App, id: &str) -> String {
    let (group, channel) = match app.chat(id) {
        Some(chat) => (chat.is_group(), chat.is_channel()),
        None => (
            crate::model::ChatKind::from_id(id) == crate::model::ChatKind::Group,
            id.ends_with("@newsletter"),
        ),
    };
    if group {
        crate::i18n::gettext(app.locale, "Group info")
    } else if channel {
        crate::i18n::gettext(app.locale, "Channel info")
    } else {
        crate::i18n::gettext(app.locale, "Contact info")
    }
    .into_owned()
}

/// What the overview shows of the chat, read once per frame from the chat
/// list without copying its member list.
struct Shown {
    id: String,
    /// The name the chat list shows.
    name: String,
    /// The group's subject as the phone named it, for the rename editor.
    subject: String,
    subject_known: bool,
    has_chat: bool,
    group: bool,
    channel: bool,
    phone: Option<String>,
    members: usize,
    can_leave: bool,
    can_edit_info: bool,
    muted_until: Option<i64>,
    muted: bool,
    pinned: bool,
    archived: bool,
    /// A group's description, when it has one.
    description: Option<String>,
    /// The disappearing-message timer line, when the timer is known.
    timer: Option<String>,
}

impl Shown {
    fn of(app: &App, id: &str) -> Self {
        // Group members may not have an existing chat.
        let fallback;
        let (chat, has_chat) = match app.chat(id) {
            Some(chat) => (chat, true),
            None => {
                fallback = crate::model::Chat::new(id.to_owned(), app.display_name(id));
                (&fallback, false)
            }
        };
        let ours = app.our_ids();
        Self {
            id: chat.id.clone(),
            name: app.chat_title(chat),
            subject: chat.name.clone(),
            subject_known: chat.group_subject_known,
            has_chat,
            group: chat.is_group(),
            channel: chat.is_channel(),
            phone: chat.phone().map(str::to_owned),
            members: chat.participants.len(),
            can_leave: chat.can_leave(&ours),
            can_edit_info: chat.can_edit_info(),
            muted_until: chat.muted_until,
            muted: chat.muted(crate::util::now()),
            pinned: chat.pinned,
            archived: chat.archived,
            description: chat
                .group_description
                .clone()
                .filter(|text| chat.is_group() && !text.trim().is_empty()),
            timer: crate::util::disappearing_line(app.locale, chat.ephemeral_expiration),
        }
    }
}

/// The chat's picture, name, details, actions and members.
fn overview(app: &mut App, ui: &mut egui::Ui, id: &str, overlay: bool) {
    let shown = Shown::of(app, id);
    let title = panel_title(app, id);
    // As a page of its own, the panel goes back to the chat it covers.
    let (icon, close) = if overlay {
        (Icon::ChevronLeft, crate::i18n::gettext(app.locale, "Back"))
    } else {
        (Icon::X, crate::i18n::gettext(app.locale, "Close"))
    };
    header(app, ui, icon, &close, &title, Action::CloseInfo, false);
    egui::ScrollArea::vertical()
        .id_salt(("chat-info", id))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            Frame::new()
                .inner_margin(Margin::symmetric(MARGIN as i8, 12))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    identity(app, ui, &shown, overlay);
                });
        });
}

/// The picture, name, details, actions and members, top to bottom.
fn identity(app: &mut App, ui: &mut egui::Ui, shown: &Shown, overlay: bool) {
    let palette = app.palette;
    let id = shown.id.as_str();
    let name = shown.name.as_str();
    // The photo takes the panel's width, up to a portrait size.
    let window = ui.ctx().content_rect().height();
    let photo = (ui.available_width() * 0.55).clamp(120.0, 200.0);
    let picture = app.avatar_full(id).or_else(|| app.avatar(id));
    let mine = app.me.as_deref() == Some(id);
    let editable = shown.phone.is_some() && !mine;
    // Saving or cancelling leaves the editor buffer checked out.
    let mut editing = app.contact_edit.take().filter(|_| editable);
    let mut saved = None;
    let mut leave = false;
    // A group's name and photo, when WhatsApp lets us change them. Nothing is
    // offered while a change is on its way.
    let saving = app.group_saving.contains(id);
    let group_editable = shown.can_edit_info && !saving;
    let mut renaming = app.group_name_edit.take().filter(|_| group_editable);
    let mut group_action = None;
    ui.vertical_centered(|ui| {
        if group_editable {
            group_action = group_photo(app, ui, name, id, photo, picture.as_deref());
        } else {
            widgets::avatar(ui, &palette, name, id, photo, picture.as_deref());
        }
        ui.add_space(6.0);
        if let Some(draft) = renaming.as_mut() {
            match group_name_field(app, ui, draft) {
                Some(true) => {
                    let typed = draft.trim();
                    // An empty or unchanged name closes the editor and sends nothing.
                    group_action = Some(if typed.is_empty() || typed == shown.subject {
                        Action::CloseGroupName
                    } else {
                        Action::SetGroupName {
                            chat: id.to_owned(),
                            name: typed.to_owned(),
                        }
                    });
                }
                Some(false) => group_action = Some(Action::CloseGroupName),
                None => {}
            }
        } else if group_editable {
            let edit = crate::i18n::gettext(app.locale, "Edit group name");
            // Center the name and its pencil together.
            let button = 15.0 + 12.0;
            let spacing = ui.spacing().item_spacing.x;
            let available = ui.available_width();
            let text_width = widgets::line(
                ui,
                name,
                theme::bold(19.0),
                palette.text,
                (available - button - spacing).max(40.0),
                1,
            )
            .size()
            .x;
            ui.horizontal(|ui| {
                ui.add_space(((available - text_width - spacing - button) / 2.0).max(0.0));
                ui.allocate_ui(vec2(text_width + 1.0, 30.0), |ui| {
                    widgets::selectable_rich_text(ui, name, theme::bold(19.0), palette.text);
                });
                let pencil = theme::icon_button(
                    ui,
                    Icon::Pencil,
                    15.0,
                    palette.secondary,
                    palette.text,
                    &edit,
                )
                .tab_stop(Stop::InfoControl(PENCIL_STOP));
                ui.ctx()
                    .data_mut(|data| data.insert_temp(group_name_button_id(), pencil.rect));
                if pencil.clicked() {
                    // The field appears next frame; egui keeps a focus
                    // request that long.
                    ui.memory_mut(|memory| memory.request_focus(group_name_field_id()));
                    // An unnamed group starts empty rather than from its
                    // members' summary, the title `chat_title` falls back to.
                    let unnamed = shown.subject.trim().is_empty()
                        || (shown.subject == "Group" && !shown.subject_known);
                    group_action = Some(Action::EditGroupName(if unnamed {
                        String::new()
                    } else {
                        shown.subject.clone()
                    }));
                }
            });
        } else if let Some((first, last)) = editing.as_mut() {
            let mut submit = false;
            ui.horizontal(|ui| {
                ui.add_space((ui.available_width() - 288.0).max(0.0) / 2.0);
                let name_field = |ui: &mut egui::Ui, buffer: &mut String, salt: &str, hint| {
                    let format = egui::TextFormat::simple(theme::semibold(15.0), palette.text);
                    let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
                        crate::bidi::layout_field(ui, text.as_str(), &format, wrap)
                    };
                    let align = if crate::bidi::base_rtl(buffer) {
                        Align::RIGHT
                    } else {
                        Align::LEFT
                    };
                    let field = Frame::new()
                        .fill(palette.surface)
                        .corner_radius(CornerRadius::same(theme::RADIUS))
                        .inner_margin(Margin::symmetric(10, 5))
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::singleline(buffer)
                                    .id(egui::Id::new(salt))
                                    .hint_text(
                                        egui::RichText::new(hint)
                                            .color(palette.dim)
                                            .font(theme::semibold(15.0)),
                                    )
                                    .font(theme::semibold(15.0))
                                    .text_color(palette.text)
                                    .frame(Frame::NONE)
                                    .desired_width(108.0)
                                    .horizontal_align(align)
                                    .layouter(&mut layouter),
                            )
                        });
                    theme::focus_outline(
                        ui,
                        field.inner.id,
                        field.response.rect,
                        f32::from(theme::RADIUS),
                    );
                    field.inner
                };
                let first_field = name_field(ui, first, "contact-first", "First name");
                let last_field = name_field(ui, last, "contact-last", "Surname");
                // The placeholders are all the fields show of what they are for.
                widgets::name_field(
                    &first_field,
                    &crate::i18n::gettext(app.locale, "First name"),
                );
                widgets::name_field(&last_field, &crate::i18n::gettext(app.locale, "Surname"));
                if ui.memory(|memory| memory.focused().is_none()) {
                    first_field.request_focus();
                }
                submit = (first_field.lost_focus() || last_field.lost_focus())
                    && ui.input(|input| input.key_pressed(egui::Key::Enter));
                if theme::icon_button(
                    ui,
                    Icon::Check,
                    18.0,
                    palette.secondary,
                    palette.accent,
                    "Save name (Enter)",
                )
                .tab_stop(Stop::InfoControl(NAME_CHECK_STOP))
                .clicked()
                {
                    submit = true;
                }
            });
            if submit && !(first.trim().is_empty() && last.trim().is_empty()) {
                saved = Some((first.trim().to_owned(), last.trim().to_owned()));
            }
        } else {
            widgets::selectable_rich_text(ui, name, theme::bold(19.0), palette.text);
        }
        if let Some(phone) = &shown.phone {
            theme::selectable_text(
                ui,
                crate::util::phone(phone),
                theme::regular(13.5),
                palette.secondary,
            );
        }
        if saving {
            theme::text(
                ui,
                crate::i18n::gettext(app.locale, "Saving…"),
                theme::regular(12.5),
                palette.dim,
            );
        }
        if shown.group && shown.members > 0 {
            theme::text(
                ui,
                crate::i18n::ngettext(app.locale, "{} member", "{} members", shown.members as u32)
                    .replace("{}", &shown.members.to_string()),
                theme::regular(13.5),
                palette.secondary,
            );
        }
        if let Some(presence) = app.presence.get(id) {
            let status = if presence.online {
                crate::i18n::gettext(app.locale, "online").into_owned()
            } else if let Some(seen) = presence.last_seen {
                crate::util::last_seen(app.locale, seen)
            } else {
                String::new()
            };
            if !status.is_empty() {
                theme::text(ui, status, theme::regular(12.5), palette.dim);
            }
        }
        if let Some(until) = shown.muted_until {
            theme::text(
                ui,
                if until == 0 {
                    crate::i18n::gettext(app.locale, "Muted").into_owned()
                } else {
                    crate::i18n::gettext(app.locale, "Muted until {time}")
                        .replace("{time}", &crate::util::chat_stamp(app.locale, until))
                },
                theme::regular(12.5),
                palette.secondary,
            );
        }
        // The chat's disappearing-message timer, read-only. A timer WhatsApp
        // turned off reads as "Off", which is a fact; a chat whose timer
        // nobody has mentioned draws nothing rather than guessing.
        // Recorded every frame, drawn or not, so a test reading the marker
        // after the row went away sees nothing instead of the last frame.
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                super::dialogs::disappearing_timer_id(),
                shown.timer.as_deref().map(|text| super::dialogs::TimerRow {
                    text: text.to_owned(),
                    rect: Rect::NOTHING,
                }),
            )
        });
        if let Some(text) = shown.timer.as_deref() {
            let drawn = super::dialogs::disappearing_timer_row(ui, &palette, text);
            ui.ctx().data_mut(|data| {
                data.insert_temp(super::dialogs::disappearing_timer_id(), Some(drawn))
            });
        }
    });
    if let Some((first, last)) = saved {
        editing = None;
        app.actions.push(Action::SaveContact {
            id: id.to_owned(),
            first,
            last,
        });
    }
    app.contact_edit = editing;
    app.group_name_edit = renaming;
    if let Some(action) = group_action {
        app.actions.push(action);
    }
    ui.add_space(4.0);
    actions(app, ui, shown, mine, editable);
    if let Some(description) = shown.description.as_deref() {
        ui.add_space(8.0);
        let label = theme::text(
            ui,
            crate::i18n::gettext(app.locale, "Description"),
            theme::medium(12.5),
            palette.secondary,
        );
        heading(ui, &label, 3);
        ui.add_space(4.0);
        // A long description scrolls in place, so the media and the members
        // stay close under it.
        egui::ScrollArea::vertical()
            .id_salt("group-description")
            .max_height(120.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                theme::selectable_text(ui, description, theme::regular(13.5), palette.text);
            });
    }
    // A locked chat's media stays in the locked folder: the section is not
    // there, rather than empty.
    if app.info_media_allowed() {
        ui.add_space(8.0);
        media_row(app, ui, id, overlay);
    }
    if shown.group && shown.members > 0 {
        ui.add_space(8.0);
        members(app, ui, id, window, photo);
    }
    if shown.can_leave {
        ui.add_space(8.0);
        let leave_label = if shown.channel {
            crate::i18n::gettext(app.locale, "Leave channel")
        } else {
            crate::i18n::gettext(app.locale, "Leave group")
        };
        ui.vertical_centered(|ui| {
            if super::dialogs::danger_button(ui, app, leave_label.as_ref())
                .tab_stop(Stop::InfoControl(LEAVE_STOP))
                .clicked()
            {
                leave = true;
            }
        });
    }
    if leave {
        app.actions
            .push(Action::ShowDialog(Dialog::ConfirmLeaveGroup(id.to_owned())));
    }
}

/// The actions the chat offers, as many per row as fit, centred.
fn actions(app: &mut App, ui: &mut egui::Ui, shown: &Shown, mine: bool, editable: bool) {
    let palette = app.palette;
    let id = shown.id.as_str();
    let mut buttons: Vec<(Icon, &str, Vec<Action>)> = Vec::new();
    if !shown.group && !mine {
        buttons.push((
            Icon::MessageCircle,
            "Message",
            vec![
                Action::StartChat {
                    id: id.to_owned(),
                    name: shown.name.clone(),
                    // The panel is not a dialog: whatever dialog is up stays.
                    dismiss_dialog: false,
                },
                Action::CloseInfo,
            ],
        ));
    }
    if editable {
        let known = app
            .contacts
            .get(id)
            .and_then(|contact| contact.full_name.as_deref())
            .is_some_and(|full| !full.is_empty());
        buttons.push((
            Icon::User,
            if known { "Rename" } else { "Add to contacts" },
            vec![Action::EditContact {
                id: id.to_owned(),
                name: shown.name.trim_start_matches('~').to_owned(),
            }],
        ));
    }
    if let Some(phone) = &shown.phone {
        buttons.push((
            Icon::Copy,
            "Copy number",
            vec![Action::CopyText(format!("+{phone}"))],
        ));
    }
    if shown.has_chat {
        buttons.push(if shown.muted {
            (
                Icon::Bell,
                "Unmute",
                vec![Action::SetMuted(id.to_owned(), None)],
            )
        } else {
            (
                Icon::BellOff,
                "Mute",
                vec![Action::SetMuted(id.to_owned(), Some(0))],
            )
        });
        buttons.push((
            if shown.pinned {
                Icon::PinOff
            } else {
                Icon::Pin
            },
            if shown.pinned { "Unpin" } else { "Pin" },
            vec![Action::SetPinned(id.to_owned(), !shown.pinned)],
        ));
        buttons.push((
            Icon::Archive,
            if shown.archived {
                "Unarchive"
            } else {
                "Archive"
            },
            vec![
                Action::SetArchived(id.to_owned(), !shown.archived),
                Action::CloseInfo,
            ],
        ));
    }
    let spacing = ui.spacing().item_spacing.x;
    let available = ui.available_width();
    let mut fired: Option<Vec<Action>> = None;
    let mut start = 0;
    while start < buttons.len() {
        // Fit and center as many buttons as each row allows.
        let mut end = start;
        let mut total = 0.0;
        while end < buttons.len() {
            let width = theme::soft_button_width(ui, buttons[end].1, true);
            let grown = if end == start {
                width
            } else {
                total + spacing + width
            };
            if end > start && grown > available {
                break;
            }
            total = grown;
            end += 1;
        }
        ui.horizontal(|ui| {
            ui.add_space((available - total).max(0.0) / 2.0);
            for (index, (icon, label, actions)) in buttons[start..end].iter().enumerate() {
                if theme::soft_action_button(ui, &palette, Some(*icon), label, false)
                    .tab_stop(Stop::InfoControl(ACTION_STOP + (start + index) as u8))
                    .clicked()
                {
                    fired = Some(actions.clone());
                }
            }
        });
        start = end;
    }
    if let Some(actions) = fired {
        app.actions.extend(actions);
    }
}

/// The group's members, each opening their own panel. They are sorted once
/// per change of the chat list or the contacts, not every frame. The list
/// is one Tab stop whose arrows walk the members, as the chat list's do:
/// only the rows near the view are laid out, so the keys bring the current
/// one into view.
fn members(app: &mut App, ui: &mut egui::Ui, id: &str, window: f32, photo: f32) {
    let palette = app.palette;
    if app.info_members.as_ref().is_none_or(|(chat, _)| chat != id) {
        let list = app
            .chat(id)
            .map(|chat| app.participant_list(chat))
            .unwrap_or_default();
        app.info_members = Some((id.to_owned(), list));
    }
    // Taken for the frame rather than cloned: the rows only push actions.
    let Some((_, members)) = app.info_members.take() else {
        return;
    };
    let label = theme::text(
        ui,
        crate::i18n::gettext(app.locale, "Members ({count})")
            .replace("{count}", &members.len().to_string()),
        theme::medium(12.5),
        palette.secondary,
    );
    heading(ui, &label, 3);
    ui.add_space(4.0);
    // Limit the visible rows because groups can have thousands of members.
    let row_height = 30.0;
    let rows = ((window - photo - 300.0) / row_height)
        .floor()
        .clamp(2.0, 8.0);
    let mut open = None;
    // The list's own rect, as the scroll area below takes it.
    let height = (members.len() as f32 * row_height).min(row_height * rows);
    let rect = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), height));
    let label = crate::i18n::gettext(app.locale, "Members").into_owned();
    let pass = ListPass::begin(
        ui,
        InfoList::Members,
        rect,
        label,
        members.len(),
        row_height,
    );
    let keys: Vec<String> = members.iter().map(|(member, _)| member.clone()).collect();
    let current = cursor_index(app, InfoList::Members, &keys);
    let mut active = None;
    let mut current_rect = None;
    // `show_rows` reads the spacing from outside its closure.
    ui.spacing_mut().item_spacing.y = 0.0;
    let scroll = egui::ScrollArea::vertical()
        .id_salt(("members", id))
        .max_height(row_height * rows)
        .auto_shrink([false, true]);
    let scroll = pass.scroll(ui.ctx(), scroll, current, row_height, 0.0);
    let output = scroll.show_rows(ui, row_height, members.len(), |ui, range| {
        let first = range.start;
        for (offset, (member, name)) in members[range].iter().enumerate() {
            let index = first + offset;
            // Clicks only, never focus: the keyboard reaches the members
            // through the list's one stop and its arrows.
            let (rect, response) = ui
                .scope_builder(
                    super::list_nav::row_builder(ui, ("member", member), pass.id),
                    |ui| {
                        ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), row_height),
                            Sense::CLICK,
                        )
                    },
                )
                .inner;
            // Named by the member whose info it opens.
            let spoken = crate::a11y::without_push_marker(name);
            widgets::name_button(&response, spoken);
            super::list_nav::mark_row(
                ui.ctx(),
                response.id,
                spoken.to_owned(),
                index,
                members.len(),
                rect,
                false,
            );
            if Some(index) == current {
                active = Some(pass.current(ui, &response, 6.0));
                current_rect = Some(rect);
            }
            if ui.is_rect_visible(rect) {
                if response.hovered() {
                    ui.painter().rect_filled(rect, 6.0, palette.surface_hover);
                }
                let picture = app.avatar(member);
                let avatar = egui::Rect::from_center_size(
                    egui::pos2(rect.left() + 16.0, rect.center().y),
                    egui::Vec2::splat(24.0),
                );
                widgets::paint_avatar(
                    ui,
                    &palette,
                    avatar,
                    name.trim_start_matches('~'),
                    member,
                    picture.as_deref(),
                );
                let line = widgets::line(
                    ui,
                    name,
                    theme::regular(13.0),
                    palette.text,
                    rect.width() - 40.0,
                    1,
                );
                line.paint(
                    ui,
                    egui::pos2(rect.left() + 34.0, rect.center().y - line.size().y / 2.0),
                    palette.text,
                );
            }
            if response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                open = Some(member.clone());
            }
        }
    });
    pass.remember(ui.ctx(), &output);
    // The overview scrolls too: the current member comes into its view as
    // well, asked for outside the list's own scroll area, which took its
    // own request.
    if pass.reveal
        && let Some(rect) = current_rect
    {
        ui.scroll_to_rect_animation(rect, None, egui::style::ScrollAnimation::none());
    }
    pass.finish(ui.ctx(), active);
    app.info_members = Some((id.to_owned(), members));
    if let Some(member) = open {
        open_member(app, &member);
    }
}

/// Opens a member's own panel, as clicking their row does. Our own row opens
/// nothing.
fn open_member(app: &mut App, member: &str) {
    if Some(member) != app.me.as_deref() {
        app.actions.push(Action::OpenInfo(member.to_owned()));
    }
}

/// The panel's lists. Each is one Tab stop, a ListBox whose arrows walk its
/// rows and whose current row is its active descendant, as the chat list's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfoList {
    /// A group's members, in the overview.
    Members,
    /// The media view's pictures and videos, in reading order: the grid's
    /// rows from the top, each from the left, which is newest first.
    Media,
    /// The media view's documents, newest first.
    Docs,
    /// The media view's links, newest first.
    Links,
}

impl InfoList {
    const EVERY: [InfoList; 4] = [
        InfoList::Members,
        InfoList::Media,
        InfoList::Docs,
        InfoList::Links,
    ];

    /// Where the list's current row is kept in `App::info_cursor`.
    fn slot(self) -> usize {
        self as usize
    }

    fn stop(self) -> Stop {
        match self {
            InfoList::Members => Stop::InfoControl(MEMBERS_STOP),
            InfoList::Media | InfoList::Docs | InfoList::Links => Stop::InfoControl(LIST_STOP),
        }
    }
}

/// The key a link's row is kept under: a message can hold several links.
fn link_key(link: &crate::model::ChatLink) -> String {
    format!("{}\n{}", link.message, link.url)
}

/// The listing of the chat the panel shows, once it is in.
fn shown_listing(app: &App) -> Option<&ChatMedia> {
    let shown = app.info.as_ref()?;
    app.info_media
        .as_ref()
        .filter(|listing| listing.chat == shown.chat)
}

/// The focusable list widget of `list`.
pub fn list_id(list: InfoList) -> egui::Id {
    egui::Id::new(("info-list", list as u8))
}

/// The list of the panel that has focus, if one has.
fn focused_list(ctx: &egui::Context) -> Option<InfoList> {
    InfoList::EVERY
        .into_iter()
        .find(|list| ctx.memory(|memory| memory.has_focus(list_id(*list))))
}

/// The rows `list` shows, each by the key its current row is kept under.
fn row_keys(app: &App, list: InfoList) -> Vec<String> {
    let shown = app.info.as_ref().map(|info| info.chat.as_str());
    match list {
        InfoList::Members => app
            .info_members
            .as_ref()
            .filter(|(chat, _)| Some(chat.as_str()) == shown)
            .map(|(_, members)| members.iter().map(|(member, _)| member.clone()).collect())
            .unwrap_or_default(),
        InfoList::Media | InfoList::Docs => shown_listing(app)
            .map(|listing| {
                let rows = if list == InfoList::Media {
                    &listing.media
                } else {
                    &listing.docs
                };
                rows.iter().map(|row| row.id.clone()).collect()
            })
            .unwrap_or_default(),
        InfoList::Links => shown_listing(app)
            .map(|listing| listing.links.iter().map(link_key).collect())
            .unwrap_or_default(),
    }
}

/// Where `list`'s current row is among its rows `keys`: the one the keys
/// reached while it is listed, else the first.
fn cursor_index(app: &App, list: InfoList, keys: &[String]) -> Option<usize> {
    app.info_cursor[list.slot()]
        .as_ref()
        .and_then(|wanted| keys.iter().position(|key| key == wanted))
        .or_else(|| (!keys.is_empty()).then_some(0))
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

/// Takes this frame's keys for the panel's focused list: the arrows, Home,
/// End and the page keys move its current row, and Enter or Space acts on
/// it: a member opens their panel and a document goes to its message, as a
/// click does, a link opens, and a picture or video opens in the viewer.
pub fn list_keys(app: &mut App, ctx: &egui::Context) {
    let Some(list) = focused_list(ctx).filter(|_| app.info_visible()) else {
        return;
    };
    let id = list_id(list);
    let keys = row_keys(app, list);
    let current = cursor_index(app, list, &keys);
    if let Some(step) = super::list_nav::take_step(ctx, || page_rows(ctx, id)) {
        if let Some(index) = super::list_nav::step(current, keys.len(), step) {
            app.info_cursor[list.slot()] = Some(keys[index].clone());
        }
        note_moved(ctx, id);
        return;
    }
    let activate = ctx.input_mut(|input| {
        super::keys::take_plain(input, egui::Key::Enter)
            | super::keys::take_plain(input, egui::Key::Space)
    });
    let Some(index) = current.filter(|_| activate) else {
        return;
    };
    // Acting from the keyboard is keyboard navigation, as Tab is: the focus
    // it moves, into the viewer or a member's panel, is shown.
    ctx.data_mut(|data| data.insert_temp(theme::keyboard_focus_id(), true));
    let key = keys[index].clone();
    app.info_cursor[list.slot()] = Some(key.clone());
    let docked = ctx.data(|data| data.get_temp::<bool>(docked_id()));
    let Some(listing) = shown_listing(app) else {
        if list == InfoList::Members {
            open_member(app, &key);
        }
        return;
    };
    match list {
        InfoList::Members => open_member(app, &key),
        // The viewer rather than the message: it reads the item and steps
        // through the chat's album, where "Show in the chat" goes on to the
        // message. A click still goes to the message, as upstream.
        InfoList::Media => {
            let message = &listing.media[index];
            let open = Action::OpenListedMedia {
                chat: message.chat.clone(),
                message: message.id.clone(),
            };
            app.actions.push(open);
        }
        InfoList::Docs => {
            let message = listing.docs[index].clone();
            go_to(app, &message, docked == Some(false));
            // A reader goes on to the message, the open chat's current
            // message now, as from the Starred list.
            if crate::a11y::screen_reader_active(ctx, egui::accesskit_root_id()) {
                app.actions.push(Action::FocusMessages);
            }
        }
        InfoList::Links => {
            let url = listing.links[index].url.clone();
            app.actions.push(Action::OpenUrl(url));
        }
    }
}

/// One of the panel's lists in the pass that draws it.
struct ListPass {
    id: egui::Id,
    focused: bool,
    /// Whether the current row is to be brought into view: the list was
    /// reached by keyboard, or its keys moved the row, in this pass.
    reveal: bool,
}

impl ListPass {
    /// Creates the list's container over `rect`, before its rows, so their
    /// nodes can hang from its node. It senses no pointer input, so pointer
    /// users see the list exactly as before. `pitch` is a row's height, for
    /// the page keys.
    fn begin(
        ui: &mut egui::Ui,
        list: InfoList,
        rect: Rect,
        label: String,
        len: usize,
        pitch: f32,
    ) -> Self {
        let id = list_id(list);
        let response =
            super::list_nav::container(ui, id, rect, || label, len).tab_stop(list.stop());
        // About a screen of rows, as PgUp and PgDn move.
        let page = (rect.height() / pitch).floor().max(1.0) as usize;
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

    /// `scroll`, moved to show row `row` when the current row is to be
    /// brought into view: a row the scroll area does not lay out has no
    /// node to name, nor a place for the focus ring. `height` and `spacing`
    /// are the rows' as `show_rows` lays them out.
    fn scroll(
        &self,
        ctx: &egui::Context,
        scroll: egui::ScrollArea,
        row: Option<usize>,
        height: f32,
        spacing: f32,
    ) -> egui::ScrollArea {
        let Some(row) = row.filter(|_| self.reveal) else {
            return scroll;
        };
        let Some((offset, viewport)) =
            ctx.data(|data| data.get_temp::<(f32, f32)>(self.id.with("scroll")))
        else {
            return scroll;
        };
        scroll.vertical_scroll_offset(super::chats::row_scroll_offset(
            offset, viewport, row, height, spacing,
        ))
    }

    /// Keeps where the scroll area stood and how tall it was, for
    /// [`ListPass::scroll`] in the next frame.
    fn remember<R>(&self, ctx: &egui::Context, output: &egui::scroll_area::ScrollAreaOutput<R>) {
        let at = (output.state.offset.y, output.inner_rect.height());
        ctx.data_mut(|data| data.insert_temp(self.id.with("scroll"), at));
    }

    /// The current row, `response`, drawn in this pass: it carries the
    /// list's focus ring, drawn only after keyboard navigation, and its node
    /// is the list's active descendant.
    fn current(&self, ui: &egui::Ui, response: &egui::Response, radius: f32) -> egui::Id {
        if self.focused {
            theme::focus_outline(ui, self.id, response.rect, radius);
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

/// The "Media, links and docs" row: its count once the listing is in, a
/// chevron, and the newest pictures under it. The row opens the media view;
/// a picture goes to its message.
fn media_row(app: &mut App, ui: &mut egui::Ui, chat: &str, overlay: bool) {
    let palette = app.palette;
    // Taken for the frame rather than cloned: the cells only push actions.
    let listing = app.info_media.take();
    let uris = std::mem::take(&mut app.info_uris);
    let shown = listing.as_ref().filter(|media| media.chat == chat);
    let count = shown.map(|media| {
        (
            media.media.len() + media.docs.len() + media.links.len(),
            media.truncated(),
        )
    });
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
    let response = response.tab_stop(Stop::InfoControl(MEDIA_ROW_STOP));
    if app.info_focus == Some(InfoFocus::MediaRow) {
        app.info_focus = None;
        response.request_focus();
    }
    theme::reveal_focus(&response);
    theme::focus_outline(ui, response.id, rect, 6.0);
    let title = media_title(app);
    // A "+" says the lists were cut, so the count is a floor.
    let count = count.map(|(count, cut)| {
        if cut {
            crate::i18n::gettext(app.locale, "{}+").replace("{}", &count.to_string())
        } else {
            count.to_string()
        }
    });
    // Named with the count it shows.
    let spoken = match &count {
        Some(count) => format!("{title}, {count}"),
        None => title.clone(),
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &spoken));
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            // The label sits on the panel's margin, in line with the members
            // header and the pictures under it; the highlight reaches past.
            ui.painter()
                .rect_filled(rect.expand2(vec2(6.0, 0.0)), 6.0, palette.surface_hover);
        }
        ui.painter().text(
            pos2(rect.left(), rect.center().y),
            Align2::LEFT_CENTER,
            &title,
            theme::regular(13.5),
            palette.secondary,
        );
        let chevron = Rect::from_center_size(
            pos2(rect.right() - 15.0, rect.center().y),
            Vec2::splat(18.0),
        );
        theme::paint_icon(ui, Icon::ChevronRight, chevron, 18.0, palette.dim);
        if let Some(count) = count {
            ui.painter().text(
                pos2(chevron.left() - 2.0, rect.center().y),
                Align2::RIGHT_CENTER,
                count,
                theme::regular(13.0),
                palette.dim,
            );
        }
    }
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        app.actions
            .push(Action::ShowInfo(InfoView::Media(MediaTab::Media)));
    }
    if app.info_media_failed {
        failure_line(app, ui);
    }
    if let Some(media) = shown
        && !media.media.is_empty()
    {
        let cell = ((ui.available_width() - 2.0 * GAP) / 3.0).min(110.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for (index, message) in media.media.iter().take(3).enumerate() {
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(cell), Sense::click());
                media_cell(
                    app,
                    ui,
                    rect,
                    response,
                    message,
                    uris.get(index).map(String::as_str),
                    overlay,
                );
            }
        });
    }
    app.info_uris = uris;
    app.info_media = listing;
}

/// Says the listing could not be made, with a button that asks again.
fn failure_line(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    ui.horizontal_wrapped(|ui| {
        theme::text(
            ui,
            crate::i18n::gettext(app.locale, "Could not list the media."),
            theme::regular(12.5),
            palette.dim,
        );
        if theme::soft_action_button(
            ui,
            &palette,
            Some(Icon::Refresh),
            &crate::i18n::gettext(app.locale, "Try again"),
            false,
        )
        .tab_stop(Stop::InfoControl(RETRY_STOP))
        .clicked()
        {
            app.actions.push(Action::RetryInfoMedia);
        }
    });
}

/// The media view: a back header, the three tabs, and the tab's list.
fn media_view(app: &mut App, ui: &mut egui::Ui, chat: &str, tab: MediaTab, overlay: bool) {
    let palette = app.palette;
    let back = crate::i18n::gettext(app.locale, "Back");
    let title = media_title(app);
    let focus_back = app.info_focus == Some(InfoFocus::Back);
    if focus_back {
        app.info_focus = None;
    }
    header(
        app,
        ui,
        Icon::ChevronLeft,
        &back,
        &title,
        Action::ShowInfo(InfoView::Overview),
        focus_back,
    );
    // Taken for the frame rather than cloned: the rows only push actions.
    let listing = app.info_media.take();
    let uris = std::mem::take(&mut app.info_uris);
    let shown = listing.as_ref().filter(|media| media.chat == chat);
    Frame::new()
        .inner_margin(Margin::symmetric(MARGIN as i8, 10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // The tabs are a tab list, named by the view they switch.
                let title = media_title(app);
                ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
                    node.set_role(egui::accesskit::Role::TabList);
                    node.set_label(title);
                });
                ui.spacing_mut().item_spacing.x = 6.0;
                let tabs = [MediaTab::Media, MediaTab::Docs, MediaTab::Links];
                for (index, choice) in tabs.into_iter().enumerate() {
                    let count = shown.map_or(0, |media| match choice {
                        MediaTab::Media => media.media.len(),
                        MediaTab::Docs => media.docs.len(),
                        MediaTab::Links => media.links.len(),
                    });
                    let label = tab_label(app, choice);
                    let chip = widgets::filter_chip(ui, &palette, &label, count, choice == tab)
                        .tab_stop(Stop::InfoControl(TAB_STOP + index as u8));
                    // A tab, selected or not, with the count it shows: a
                    // filter chip would say "pressed" and call it unread.
                    let spoken = if count > 0 {
                        format!("{label}, {count}")
                    } else {
                        label.clone()
                    };
                    ui.ctx().accesskit_node_builder(chip.id, |node| {
                        node.set_role(egui::accesskit::Role::Tab);
                        node.set_label(spoken);
                        node.clear_toggled();
                        node.set_selected(choice == tab);
                        node.set_position_in_set(index);
                        node.set_size_of_set(tabs.len());
                    });
                    if chip
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        app.actions.push(Action::ShowInfo(InfoView::Media(choice)));
                    }
                }
            });
        });
    ui.painter().hline(
        ui.max_rect().x_range(),
        ui.cursor().top(),
        Stroke::new(1.0, palette.outline),
    );
    match shown {
        Some(media) => {
            let cut = match tab {
                MediaTab::Media => media.media_truncated,
                MediaTab::Docs => media.docs_truncated,
                MediaTab::Links => media.links_truncated,
            };
            if cut {
                truncated_note(app, ui);
            }
            match tab {
                MediaTab::Media => media_grid(app, ui, media, &uris, overlay),
                MediaTab::Docs => docs_list(app, ui, media, overlay),
                MediaTab::Links => links_list(app, ui, media),
            }
            if cut {
                // The note above the list, which the list's node does not
                // hold, describes it.
                let note = crate::i18n::gettext(app.locale, "Only the newest items are listed.");
                let list = list_id(match tab {
                    MediaTab::Media => InfoList::Media,
                    MediaTab::Docs => InfoList::Docs,
                    MediaTab::Links => InfoList::Links,
                });
                ui.ctx().accesskit_node_builder(list, |node| {
                    if node.role() == egui::accesskit::Role::ListBox {
                        node.set_description(note.into_owned());
                    }
                });
            }
        }
        None if app.info_media_failed => {
            ui.add_space(24.0);
            ui.horizontal(|ui| {
                ui.add_space(MARGIN);
                failure_line(app, ui);
            });
        }
        None if app.info_media_pending => {
            ui.add_space(32.0);
            ui.vertical_centered(|ui| theme::spinner(ui, 22.0, palette.secondary));
        }
        None => {}
    }
    app.info_uris = uris;
    app.info_media = listing;
}

/// The chat's pictures and videos, newest first, in square cells as wide as
/// the panel allows. Only the rows on screen are laid out, so only their
/// thumbnails are decoded. To the keyboard the grid is one list, in reading
/// order, named as the viewer names its items.
fn media_grid(
    app: &mut App,
    ui: &mut egui::Ui,
    listing: &ChatMedia,
    uris: &[String],
    overlay: bool,
) {
    let palette = app.palette;
    if listing.media.is_empty() {
        widgets::empty_state(
            ui,
            &palette,
            Icon::Image,
            &crate::i18n::gettext(app.locale, "No media"),
            &crate::i18n::gettext(
                app.locale,
                "Photos and videos shared in this chat will show here.",
            ),
        );
        return;
    }
    let width = ui.available_width() - 2.0 * MARGIN;
    let columns = if width >= 4.0 * 84.0 + 3.0 * GAP {
        4
    } else {
        3
    };
    let cell = (width - (columns - 1) as f32 * GAP) / columns as f32;
    let rows = listing.media.len().div_ceil(columns);
    ui.add_space(6.0);
    let len = listing.media.len();
    let label = tab_label(app, MediaTab::Media);
    let rect = ui.available_rect_before_wrap();
    let pass = ListPass::begin(ui, InfoList::Media, rect, label, len, cell + GAP);
    // The list is in reading order, so about a screen of it, as PgUp and
    // PgDn move, is a screen of the grid's rows times its columns.
    let page = pass.id.with("page");
    ui.ctx().data_mut(|data| {
        let rows = data.get_temp::<usize>(page).unwrap_or(1);
        data.insert_temp(page, rows * columns);
    });
    let keys: Vec<String> = listing.media.iter().map(|row| row.id.clone()).collect();
    let current = cursor_index(app, InfoList::Media, &keys);
    let mut active = None;
    // `show_rows` reads the spacing from outside its closure.
    ui.spacing_mut().item_spacing = vec2(GAP, GAP);
    let scroll = egui::ScrollArea::vertical()
        .id_salt(("info-media-grid", listing.chat.as_str()))
        .auto_shrink([false, false]);
    let current_row = current.map(|index| index / columns);
    let scroll = pass.scroll(ui.ctx(), scroll, current_row, cell + GAP, GAP);
    let output = scroll.show_rows(ui, cell + GAP, rows, |ui, range| {
        for row in range {
            ui.horizontal(|ui| {
                ui.add_space(MARGIN);
                for column in 0..columns {
                    let index = row * columns + column;
                    let Some(message) = listing.media.get(index) else {
                        break;
                    };
                    // Clicks only, never focus: the keyboard reaches the
                    // cells through the list's one stop and its arrows.
                    let (rect, response) = ui
                        .scope_builder(
                            super::list_nav::row_builder(ui, ("media", &message.id), pass.id),
                            |ui| ui.allocate_exact_size(Vec2::splat(cell), Sense::CLICK),
                        )
                        .inner;
                    let id = response.id;
                    if Some(index) == current {
                        active = Some(pass.current(ui, &response, 6.0));
                    }
                    media_cell(
                        app,
                        ui,
                        rect,
                        response,
                        message,
                        uris.get(index).map(String::as_str),
                        overlay,
                    );
                    super::list_nav::mark_row(
                        ui.ctx(),
                        id,
                        app.listed_media_label(message, index, len),
                        index,
                        len,
                        rect,
                        false,
                    );
                }
            });
        }
    });
    pass.remember(ui.ctx(), &output);
    pass.finish(ui.ctx(), active);
}

/// One square picture or video: its preview cropped to the square, with a
/// play mark and the length on a video. Clicking it goes to the message.
fn media_cell(
    app: &mut App,
    ui: &mut egui::Ui,
    rect: Rect,
    response: egui::Response,
    message: &Message,
    uri: Option<&str>,
    overlay: bool,
) {
    let palette = app.palette;
    let video = matches!(message.content, Content::Video { .. });
    let seconds = match &message.content {
        Content::Video { seconds, .. } => *seconds,
        _ => None,
    };
    let stamp = crate::util::chat_stamp(app.locale, message.timestamp);
    response.widget_info(|| {
        let what = if video {
            crate::i18n::gettext(app.locale, "Video")
        } else {
            crate::i18n::gettext(app.locale, "Photo")
        };
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{what}, {stamp}"))
    });
    if ui.is_rect_visible(rect) {
        if !thumbnail(ui, &palette, rect, message, uri) {
            ui.painter().rect_filled(rect, 6.0, palette.surface);
            let icon = if video { Icon::Video } else { Icon::Image };
            theme::paint_icon(ui, icon, rect, 22.0, palette.dim);
        }
        if video {
            let disc = Rect::from_center_size(rect.center(), Vec2::splat(30.0));
            ui.painter()
                .circle_filled(disc.center(), 15.0, Color32::from_black_alpha(140));
            theme::paint_icon(ui, Icon::Play, disc, 14.0, Color32::WHITE);
            if let Some(seconds) = seconds {
                let galley = ui.painter().layout_no_wrap(
                    crate::util::duration(seconds),
                    theme::medium(11.0),
                    Color32::WHITE,
                );
                let pad = vec2(4.0, 2.0);
                let label = Rect::from_min_max(
                    rect.max - galley.size() - pad * 2.0 - Vec2::splat(4.0),
                    rect.max - Vec2::splat(4.0),
                );
                ui.painter()
                    .rect_filled(label, 3.0, Color32::from_black_alpha(140));
                ui.painter().galley(label.min + pad, galley, Color32::WHITE);
            }
        }
        if response.hovered() {
            ui.painter()
                .rect_filled(rect, 6.0, Color32::from_white_alpha(24));
        }
    }
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        go_to(app, message, overlay);
    }
}

/// Goes to a listed message. Over a narrow chat the panel would hide it,
/// so the panel closes, as the search pane folds.
fn go_to(app: &mut App, message: &Message, overlay: bool) {
    app.actions.push(Action::OpenMessage {
        chat: message.chat.clone(),
        message: message.id.clone(),
    });
    if overlay {
        app.actions.push(Action::CloseInfo);
    }
}

/// Paints the message's preview cropped to fill `rect`, and reports whether
/// there was one. The archived JPEG preview needs no download; a downloaded
/// picture without one goes through egui's file loader, as in the bubble. A
/// video without one keeps its placeholder: its file is not a picture.
fn thumbnail(
    ui: &egui::Ui,
    palette: &theme::Palette,
    rect: Rect,
    message: &Message,
    uri: Option<&str>,
) -> bool {
    let image = match (&message.thumbnail, uri, &message.content) {
        (Some(bytes), Some(uri), _) => {
            crate::image_cache::include(ui.ctx(), uri, bytes);
            egui::Image::new(uri.to_owned())
        }
        (_, _, Content::Image { media, .. }) if media.path.is_some() => {
            let path = media.path.as_deref().expect("checked");
            widgets::file_image(ui, path)
        }
        _ => return false,
    };
    // The texture's shape is known once it is decoded; until then the cell
    // is a plain surface rather than a stretched placeholder.
    let Ok(egui::load::TexturePoll::Ready { texture }) = image.load_for_size(ui.ctx(), rect.size())
    else {
        ui.painter().rect_filled(rect, 6.0, palette.surface);
        return true;
    };
    // Cover the square: the longer side is cropped at both ends.
    let (width, height) = (texture.size.x.max(1.0), texture.size.y.max(1.0));
    let uv = if width > height {
        let cut = (1.0 - height / width) / 2.0;
        Rect::from_min_max(pos2(cut, 0.0), pos2(1.0 - cut, 1.0))
    } else {
        let cut = (1.0 - width / height) / 2.0;
        Rect::from_min_max(pos2(0.0, cut), pos2(1.0, 1.0 - cut))
    };
    image.uv(uv).corner_radius(6.0).paint_at(ui, rect);
    true
}

/// The chat's documents, newest first: a file icon, the name, and its
/// pages, size and day. A row goes to its message.
fn docs_list(app: &mut App, ui: &mut egui::Ui, listing: &ChatMedia, overlay: bool) {
    let palette = app.palette;
    if listing.docs.is_empty() {
        widgets::empty_state(
            ui,
            &palette,
            Icon::FileText,
            &crate::i18n::gettext(app.locale, "No documents"),
            &crate::i18n::gettext(app.locale, "Files shared in this chat will show here."),
        );
        return;
    }
    let len = listing.docs.len();
    let label = tab_label(app, MediaTab::Docs);
    let rect = ui.available_rect_before_wrap();
    let pass = ListPass::begin(ui, InfoList::Docs, rect, label, len, ROW_HEIGHT);
    let keys: Vec<String> = listing.docs.iter().map(|row| row.id.clone()).collect();
    let current = cursor_index(app, InfoList::Docs, &keys);
    let mut active = None;
    // `show_rows` reads the spacing from outside its closure.
    ui.spacing_mut().item_spacing.y = 0.0;
    let scroll = egui::ScrollArea::vertical()
        .id_salt(("info-docs", listing.chat.as_str()))
        .auto_shrink([false, false]);
    let scroll = pass.scroll(ui.ctx(), scroll, current, ROW_HEIGHT, 0.0);
    let output = scroll.show_rows(ui, ROW_HEIGHT, len, |ui, range| {
        let first = range.start;
        for (offset, message) in listing.docs[range].iter().enumerate() {
            let index = first + offset;
            let Content::Document {
                media,
                file_name,
                pages,
                ..
            } = &message.content
            else {
                continue;
            };
            let mut detail = Vec::new();
            if let Some(pages) = pages {
                detail.push(
                    crate::i18n::ngettext(app.locale, "{} page", "{} pages", *pages)
                        .replace("{}", &pages.to_string()),
                );
            }
            detail.push(crate::util::bytes(media.size));
            detail.push(crate::util::chat_stamp(app.locale, message.timestamp));
            let row = ListRow {
                pass: &pass,
                salt: ("doc", message.id.as_str()),
                index,
                len,
                current: Some(index) == current,
                spoken: None,
            };
            let response = item_row(
                app,
                ui,
                Icon::FileText,
                file_name,
                &detail,
                row,
                &mut active,
            );
            if response.clicked() {
                go_to(app, message, overlay);
            }
        }
    });
    pass.remember(ui.ctx(), &output);
    pass.finish(ui.ctx(), active);
}

/// The web links written in the chat, newest first: the preview's title or
/// the address, and the site. A row opens the address in the browser.
fn links_list(app: &mut App, ui: &mut egui::Ui, listing: &ChatMedia) {
    let palette = app.palette;
    if listing.links.is_empty() {
        widgets::empty_state(
            ui,
            &palette,
            Icon::ExternalLink,
            &crate::i18n::gettext(app.locale, "No links"),
            &crate::i18n::gettext(app.locale, "Links shared in this chat will show here."),
        );
        return;
    }
    let len = listing.links.len();
    let label = tab_label(app, MediaTab::Links);
    let rect = ui.available_rect_before_wrap();
    let pass = ListPass::begin(ui, InfoList::Links, rect, label, len, ROW_HEIGHT);
    let keys: Vec<String> = listing.links.iter().map(link_key).collect();
    let current = cursor_index(app, InfoList::Links, &keys);
    let mut active = None;
    // `show_rows` reads the spacing from outside its closure.
    ui.spacing_mut().item_spacing.y = 0.0;
    let scroll = egui::ScrollArea::vertical()
        .id_salt(("info-links", listing.chat.as_str()))
        .auto_shrink([false, false]);
    let scroll = pass.scroll(ui.ctx(), scroll, current, ROW_HEIGHT, 0.0);
    let output = scroll.show_rows(ui, ROW_HEIGHT, len, |ui, range| {
        let first = range.start;
        for (offset, link) in listing.links[range].iter().enumerate() {
            let index = first + offset;
            let title = link.title.as_deref().unwrap_or(&link.url);
            let stamp = crate::util::chat_stamp(app.locale, link.timestamp);
            // A call link joins a call here, as in the message list: it says
            // so, rather than its address with the call's token.
            let spoken = crate::safety::call_link(&link.url).map(|(_, video)| {
                let kind = if video {
                    // Translators: a call link in the info panel's Links
                    // list, read in place of its address.
                    crate::i18n::gettext(app.locale, "Call link, video call")
                } else {
                    // Translators: a call link in the info panel's Links
                    // list, read in place of its address.
                    crate::i18n::gettext(app.locale, "Call link, voice call")
                };
                std::iter::once(kind.into_owned())
                    .chain(link.title.clone())
                    .chain(std::iter::once(stamp.clone()))
                    .collect::<Vec<_>>()
                    .join(", ")
            });
            let detail = [domain(&link.url), stamp];
            let key = link_key(link);
            let row = ListRow {
                pass: &pass,
                salt: ("link", key.as_str()),
                index,
                len,
                current: Some(index) == current,
                spoken,
            };
            let response = item_row(
                app,
                ui,
                Icon::ExternalLink,
                title,
                &detail,
                row,
                &mut active,
            );
            if response.clicked() {
                app.actions.push(Action::OpenUrl(link.url.clone()));
            }
        }
    });
    pass.remember(ui.ctx(), &output);
    pass.finish(ui.ctx(), active);
}

/// Where a document or link row sits in its list, and the list's pass.
struct ListRow<'a> {
    pass: &'a ListPass,
    /// What the row's ids come from: its kind and its key.
    salt: (&'static str, &'a str),
    index: usize,
    len: usize,
    /// Whether it is the list's current row.
    current: bool,
    /// What a screen reader hears in place of the title and the detail
    /// line, when they would not say what the row is.
    spoken: Option<String>,
}

/// One document or link row: an icon in a tile, a title, and a detail line
/// of `detail`'s parts. To the keyboard it is an item of its list, `row`,
/// which it names `active` when it is the current one; to the pointer a
/// target, as before.
fn item_row(
    app: &mut App,
    ui: &mut egui::Ui,
    icon: Icon,
    title: &str,
    detail: &[String],
    row: ListRow<'_>,
    active: &mut Option<egui::Id>,
) -> egui::Response {
    let palette = app.palette;
    // Clicks only, never focus: the keyboard reaches the rows through the
    // list's one stop and its arrows.
    let (rect, response) = ui
        .scope_builder(
            super::list_nav::row_builder(ui, row.salt, row.pass.id),
            |ui| ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::CLICK),
        )
        .inner;
    let detail_line = detail.join(" · ");
    // Read with commas between the parts the line separates with dots.
    let spoken = row.spoken.clone().unwrap_or_else(|| {
        std::iter::once(title)
            .chain(detail.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(", ")
    });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &spoken));
    super::list_nav::mark_row(
        ui.ctx(),
        response.id,
        spoken.clone(),
        row.index,
        row.len,
        rect,
        false,
    );
    if row.current {
        *active = Some(row.pass.current(ui, &response, 0.0));
    }
    let detail = detail_line.as_str();
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter().rect_filled(rect, 0.0, palette.surface_hover);
        }
        let tile = Rect::from_center_size(
            pos2(rect.left() + MARGIN + 20.0, rect.center().y),
            Vec2::splat(40.0),
        );
        ui.painter().rect_filled(tile, 8.0, palette.surface);
        theme::paint_icon(ui, icon, tile, 20.0, palette.secondary);
        let left = tile.right() + 12.0;
        let width = rect.right() - MARGIN - left;
        let name = widgets::line(ui, title, theme::medium(13.5), palette.text, width, 1);
        name.paint(ui, pos2(left, rect.top() + 12.0), palette.text);
        let line = widgets::line(ui, detail, theme::regular(12.0), palette.dim, width, 1);
        line.paint(ui, pos2(left, rect.top() + 33.0), palette.dim);
        ui.painter().hline(
            left..=rect.right() - MARGIN,
            rect.bottom() - 0.5,
            Stroke::new(1.0, palette.outline),
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Says the open tab's list was cut, above it, when the archive held more
/// than it carries.
fn truncated_note(app: &App, ui: &mut egui::Ui) {
    let palette = app.palette;
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(MARGIN);
        let line = widgets::line(
            ui,
            &crate::i18n::gettext(app.locale, "Only the newest items are listed."),
            theme::regular(12.0),
            palette.dim,
            ui.available_width() - MARGIN,
            2,
        );
        let (rect, _) = ui.allocate_exact_size(line.size(), Sense::hover());
        line.paint(ui, rect.min, palette.dim);
    });
    ui.add_space(4.0);
}

/// A tab's label. The words are short, so each carries the context that
/// tells a translator where it sits.
pub fn tab_label(app: &App, tab: MediaTab) -> String {
    match tab {
        MediaTab::Media => crate::i18n::pgettext(app.locale, "info tab", "Media"),
        MediaTab::Docs => crate::i18n::pgettext(app.locale, "info tab", "Docs"),
        MediaTab::Links => crate::i18n::pgettext(app.locale, "info tab", "Links"),
    }
    .into_owned()
}

/// The host an address opens, without a leading `www.`. It is parsed, not
/// sliced, so userinfo such as `bank.example@` cannot pose as the site.
pub fn domain(url: &str) -> String {
    let parsed = reqwest::Url::parse(url)
        .or_else(|_| reqwest::Url::parse(&format!("https://{url}")))
        .ok();
    match parsed.as_ref().and_then(reqwest::Url::host_str) {
        Some(host) => host.trim_start_matches("www.").to_owned(),
        None => url.to_owned(),
    }
}

/// Where the pencil that renames a group was drawn, for interaction tests.
pub fn group_name_button_id() -> egui::Id {
    egui::Id::new("group-name-edit")
}

/// Where the group photo that opens its menu was drawn, for interaction tests.
pub fn group_photo_id() -> egui::Id {
    egui::Id::new("group-photo")
}

/// The group's big photo, which opens a menu to change or remove it.
fn group_photo(
    app: &App,
    ui: &mut egui::Ui,
    name: &str,
    id: &str,
    size: f32,
    picture: Option<&std::path::Path>,
) -> Option<Action> {
    let palette = app.palette;
    let label = crate::i18n::gettext(app.locale, "Change group photo");
    let response = widgets::clickable_avatar(ui, &palette, name, id, size, picture, &label)
        .tab_stop(Stop::InfoControl(PHOTO_STOP))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label.as_ref());
    ui.ctx()
        .data_mut(|data| data.insert_temp(group_photo_id(), response.rect));
    let change = crate::i18n::gettext(app.locale, "Change photo");
    let remove = crate::i18n::gettext(app.locale, "Remove photo");
    let width = widgets::menu_width(ui, &[change.as_ref(), remove.as_ref()], true);
    let mut chosen = None;
    // Opened from the keys (Enter or Space on the photo) or by a screen
    // reader, focus goes into the menu, stays in it, and returns to the
    // photo when it closes, as in the other menus. The pointer's menu is
    // unchanged.
    let key_menu = super::list_nav::KeyMenu::for_button(
        ui.ctx(),
        egui::Popup::default_response_id(&response),
        &response,
        false,
    )
    .menu(label.to_string());
    let mut first = None;
    // The background takes clicks, not focus: a focusable one is a nameless
    // Tab stop.
    let shown = egui::Popup::menu(&response)
        .sense(egui::Sense::CLICK)
        .width(width)
        .frame(widgets::menu_frame(&palette))
        .show(|ui| {
            let item = widgets::menu_item_response(ui, &palette, Some(Icon::Image), &change, true);
            // A popup's first pass only measures it, drawing nothing that
            // could take focus.
            first = (!ui.is_sizing_pass()).then_some(item.id);
            if item.clicked() {
                chosen = Some(Action::PickGroupPicture(id.to_owned()));
                ui.close();
            }
            // Only a photo that is there can be removed.
            if picture.is_some() && widgets::menu_item(ui, &palette, Some(Icon::Trash), &remove) {
                chosen = Some(Action::RemoveGroupPicture(id.to_owned()));
                ui.close();
            }
        });
    key_menu.finish(
        ui.ctx(),
        shown.map(|shown| shown.response.layer_id),
        false,
        first,
        response.id,
    );
    chosen
}

/// The group name being typed. Returns `Some(true)` when Enter or the check
/// submits it, `Some(false)` when the cross cancels it.
fn group_name_field(app: &App, ui: &mut egui::Ui, draft: &mut String) -> Option<bool> {
    let palette = app.palette;
    let mut outcome = None;
    ui.horizontal(|ui| {
        let buttons = 2.0 * (18.0 + 12.0) + 2.0 * ui.spacing().item_spacing.x;
        let width = 240.0_f32.min(ui.available_width() - buttons);
        ui.add_space(((ui.available_width() - width - buttons) / 2.0).max(0.0));
        let format = egui::TextFormat::simple(theme::semibold(15.0), palette.text);
        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap: f32| {
            crate::bidi::layout_field(ui, text.as_str(), &format, wrap)
        };
        let align = if crate::bidi::base_rtl(draft) {
            Align::RIGHT
        } else {
            Align::LEFT
        };
        let hint = crate::i18n::gettext(app.locale, "Group name");
        let field = Frame::new()
            .fill(palette.surface)
            .corner_radius(CornerRadius::same(theme::RADIUS))
            .inner_margin(Margin::symmetric(10, 5))
            .show(ui, |ui| {
                let field = ui.add(
                    egui::TextEdit::singleline(draft)
                        .id(group_name_field_id())
                        .hint_text(
                            egui::RichText::new(hint.as_ref())
                                .color(palette.dim)
                                .font(theme::semibold(15.0)),
                        )
                        .font(theme::semibold(15.0))
                        .text_color(palette.text)
                        .frame(Frame::NONE)
                        // WhatsApp refuses longer names.
                        .char_limit(crate::model::GROUP_NAME_LIMIT)
                        .desired_width(width - 20.0)
                        .horizontal_align(align)
                        .layouter(&mut layouter),
                );
                // The placeholder is all the field shows of what it is for.
                widgets::name_field(&field, &hint);
                field
            });
        theme::focus_outline(
            ui,
            field.inner.id,
            field.response.rect,
            f32::from(theme::RADIUS),
        );
        // Read before focus is requested again below: egui answers
        // `lost_focus` from the current focus, not from this frame's input.
        if field.inner.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            outcome = Some(true);
        } else if ui.memory(|memory| memory.focused().is_none()) {
            field.inner.request_focus();
        }
        if theme::icon_button(
            ui,
            Icon::Check,
            18.0,
            palette.secondary,
            palette.accent,
            &crate::i18n::gettext(app.locale, "Save name (Enter)"),
        )
        .clicked()
        {
            outcome = Some(true);
        }
        if theme::icon_button(
            ui,
            Icon::X,
            18.0,
            palette.secondary,
            palette.text,
            &crate::i18n::gettext(app.locale, "Cancel (Escape)"),
        )
        .clicked()
        {
            outcome = Some(false);
        }
    });
    outcome
}

/// The group name editor's text field.
pub fn group_name_field_id() -> egui::Id {
    egui::Id::new("group-name-field")
}

#[cfg(test)]
mod tests {
    use super::domain;

    #[test]
    fn a_links_site_drops_the_scheme_the_www_and_the_path() {
        assert_eq!(domain("https://spotifast.rocks/"), "spotifast.rocks");
        assert_eq!(
            domain("https://www.rust-lang.org/learn?x=1"),
            "rust-lang.org"
        );
        assert_eq!(domain("engine.rocks#top"), "engine.rocks");
    }

    #[test]
    fn a_links_site_is_the_host_the_browser_opens_not_its_userinfo() {
        assert_eq!(
            domain("https://bank.example@attacker.example/path"),
            "attacker.example"
        );
        assert_eq!(
            domain("https://user:pass@www.attacker.example:8080/"),
            "attacker.example"
        );
    }
}
