//! The account switcher: our own avatar at the foot of the navigation rail
//! opens a menu of the numbers linked here and a way to add another. The
//! settings keep their own button above it.

use egui::{Align2, CornerRadius, Rect, Sense, Vec2, pos2, vec2};

use crate::app::App;
use crate::i18n::gettext;
use crate::model::{AccountId, Action};
use crate::theme::{self, Icon};

use super::widgets;

/// Width of the switcher menu.
const MENU_WIDTH: f32 = 280.0;
/// Height of one account row in it.
const ROW: f32 = 48.0;
/// Avatar size in an account row.
const ROW_AVATAR: f32 = 32.0;

/// Stable id of the switcher button, for tests and the demo.
pub fn button_id() -> egui::Id {
    egui::Id::new("account-switcher")
}

/// The frame the keys chose an account with no chat open in: the switcher
/// drawn for that account takes focus.
fn refocus_id() -> egui::Id {
    egui::Id::new("account-switcher-refocus")
}

/// Our avatar as the switcher's button. A dot on it says another account has
/// unread chats.
pub fn avatar_button(app: &mut App, ui: &mut egui::Ui, size: f32) -> egui::Response {
    let palette = app.palette;
    let me = app.me.clone().unwrap_or_default();
    let name = app.me_name.clone().unwrap_or_else(|| "You".to_owned());
    let picture = app.avatar(&me);
    let label = gettext(app.locale, "Switch account");
    let response = ui
        .push_id(button_id(), |ui| {
            widgets::clickable_avatar(ui, &palette, &name, &me, size, picture.as_deref(), &label)
        })
        .inner;
    let ctx = ui.ctx();
    if let Some(chosen) = ctx.data_mut(|data| data.remove_temp::<u64>(refocus_id()))
        && ctx.cumulative_frame_nr() <= chosen + 2
    {
        response.request_focus();
    }
    let elsewhere = app.unread_chat_count_elsewhere();
    if elsewhere > 0 {
        // Ringed in the panel colour so it reads on any picture.
        let at = response.rect.right_top() + vec2(-3.0, 3.0);
        ui.painter().circle_filled(at, 6.0, palette.panel);
        ui.painter().circle_filled(at, 4.5, palette.accent);
    }
    let tooltip = match (&app.me_about, app.has_several_accounts()) {
        (_, true) => app.account().display_label(app.locale),
        (Some(about), false) => format!("{name}\n{about}"),
        (None, false) => name.clone(),
    };
    let response = response
        .on_hover_text(tooltip)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    menu(app, &response);
    response
}

/// What the switcher lists for one account.
struct Entry {
    id: AccountId,
    name: String,
    detail: String,
    me: String,
    picture: Option<std::path::PathBuf>,
    unread: u32,
    current: bool,
    linked: bool,
    /// Whether the account has a chat open, whose message field takes focus
    /// when it comes on screen.
    chat_open: bool,
}

fn entries(app: &mut App) -> Vec<Entry> {
    let locale = app.locale;
    let active = app.active;
    (0..app.accounts.len())
        .map(|index| {
            let account = &mut app.accounts[index];
            let me = account.me.clone().unwrap_or_default();
            let picture = (!me.is_empty()).then(|| account.avatar(&me)).flatten();
            let name = account.display_label(locale);
            let linked = account.is_linked();
            let detail = if linked {
                account
                    .phone()
                    .filter(|phone| *phone != name)
                    .unwrap_or_default()
            } else {
                gettext(locale, "Not linked").into_owned()
            };
            Entry {
                id: account.id.clone(),
                name,
                detail,
                me,
                picture,
                unread: account.unread_chat_count(),
                current: index == active,
                linked,
                chat_open: account.open_chat.is_some(),
            }
        })
        .collect()
}

fn menu(app: &mut App, button: &egui::Response) {
    let palette = app.palette;
    let ctx = button.ctx.clone();
    let popup_id = button.id.with("popup");
    // Asked for elsewhere, as by the linking screen's "Use another
    // account": from the keys, focus goes into it as below.
    let asked = std::mem::take(&mut app.account_menu);
    if asked {
        egui::Popup::open_id(&ctx, popup_id);
    }
    // Opened from the keys (Enter or Space on the button) or by a screen
    // reader, focus goes into the menu, stays in it, and returns to the
    // button when it closes, as in the other menus. The pointer's menu is
    // unchanged.
    let mut key_menu = super::list_nav::KeyMenu::for_button(&ctx, popup_id, button, asked)
        .menu(gettext(app.locale, "Switch account").into_owned());
    let entries = entries(app);
    let mut first = None;
    let mut chosen = false;
    // Whether what was chosen opens a linking screen: Add account, or an
    // account the phone has not linked.
    let mut linking = false;
    // Whether what was chosen takes focus itself: a linking screen, or an
    // account's open chat. Otherwise focus returns to the button.
    let mut leads = false;
    // egui gives a popup's background a focusable sense, a nameless Tab
    // stop; it only needs the clicks.
    let shown = egui::Popup::menu(button)
        .id(popup_id)
        .sense(Sense::CLICK)
        .width(MENU_WIDTH)
        .frame(widgets::menu_frame(&palette))
        .show(|ui| {
            for entry in &entries {
                let (clicked, row) = account_row(app, ui, entry);
                first.get_or_insert(row);
                if clicked && !entry.current {
                    app.actions.push(Action::SwitchAccount(entry.id.clone()));
                    chosen = true;
                    linking = !entry.linked;
                    leads = linking || entry.chat_open;
                    ui.close();
                }
            }
            widgets::menu_separator(ui, &palette);
            if widgets::menu_item(
                ui,
                &palette,
                Some(Icon::Plus),
                &gettext(app.locale, "Add account"),
            ) {
                app.actions.push(Action::AddAccount);
                chosen = true;
                linking = true;
                leads = true;
            }
        });
    // Another account, or a new one, takes focus where it opens: a linking
    // screen, a new account's or one the phone has not linked, on its first
    // link, which nothing else focuses. An account with no chat open has
    // nothing to take focus: the button takes it back once it is drawn for
    // that account, which can place it elsewhere in the rail, under another
    // id, so not by the menu's own return.
    if chosen {
        if linking && key_menu.from_keys() {
            ctx.data_mut(|data| data.insert_temp(login_focus_id(), true));
        }
        if !leads && key_menu.from_keys() {
            let frame = ctx.cumulative_frame_nr();
            ctx.data_mut(|data| data.insert_temp(refocus_id(), frame));
        }
        key_menu.moves_on();
    }
    key_menu.finish(
        &ctx,
        shown.map(|shown| shown.response.layer_id),
        false,
        first,
        button.id,
    );
}

/// One account in the switcher: its picture, name and number, its unread
/// chats, and a check on the one on screen. Returns whether it was clicked,
/// and its id.
fn account_row(app: &App, ui: &mut egui::Ui, entry: &Entry) -> (bool, egui::Id) {
    let palette = app.palette;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW), Sense::click());
    theme::reveal_focus(&response);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::RadioButton,
            ui.is_enabled(),
            entry.current,
            row_label(app, entry),
        )
    });
    if ui.is_rect_visible(rect) {
        if response.hovered() || entry.current {
            let fill = if entry.current && !response.hovered() {
                palette.surface
            } else {
                palette.surface_hover
            };
            ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
        }
        let avatar = Rect::from_center_size(
            pos2(rect.left() + 8.0 + ROW_AVATAR / 2.0, rect.center().y),
            Vec2::splat(ROW_AVATAR),
        );
        widgets::paint_avatar(
            ui,
            &palette,
            avatar,
            &entry.name,
            if entry.me.is_empty() {
                entry.id.as_str()
            } else {
                &entry.me
            },
            entry.picture.as_deref(),
        );
        // Room on the right for the check, or the unread count.
        let right = rect.right() - 10.0;
        let mut text_right = right;
        if entry.current {
            let check =
                Rect::from_center_size(pos2(right - 8.0, rect.center().y), Vec2::splat(16.0));
            Icon::Check.image(palette.accent, 16.0).paint_at(ui, check);
            text_right -= 24.0;
        } else if entry.unread > 0 {
            let width = widgets::badge(
                ui,
                &palette,
                pos2(right - 12.0, rect.center().y),
                entry.unread,
                false,
            );
            text_right -= width.max(24.0) + 4.0;
        }
        let x = avatar.right() + 10.0;
        let max_width = (text_right - x).max(0.0);
        let name = single_line(
            ui,
            &entry.name,
            theme::semibold(13.5),
            palette.text,
            max_width,
        );
        let detail = (!entry.detail.is_empty()).then(|| {
            single_line(
                ui,
                &entry.detail,
                theme::regular(12.0),
                palette.secondary,
                max_width,
            )
        });
        let height = name.size().y + detail.as_ref().map_or(0.0, |detail| detail.size().y + 1.0);
        let mut y = rect.center().y - height / 2.0;
        let name_height = name.size().y;
        ui.painter().galley(pos2(x, y), name, palette.text);
        y += name_height + 1.0;
        if let Some(detail) = detail {
            ui.painter().galley(pos2(x, y), detail, palette.secondary);
        }
    }
    let clicked = response.clicked();
    let id = response.id;
    if !entry.current {
        response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    (clicked, id)
}

/// What a screen reader says for an account row.
fn row_label(app: &App, entry: &Entry) -> String {
    let mut label = entry.name.clone();
    if !entry.detail.is_empty() {
        label.push_str(", ");
        label.push_str(&entry.detail);
    }
    if entry.unread > 0 {
        label.push_str(", ");
        label.push_str(
            &gettext(app.locale, "{count} unread chats")
                .replace("{count}", &entry.unread.to_string()),
        );
    }
    label
}

fn single_line(
    ui: &egui::Ui,
    text: &str,
    font: egui::FontId,
    color: egui::Color32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping {
        max_width,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('\u{2026}'),
    };
    crate::bidi::layout_job(ui, job)
}

/// Set when Add account, or an account that is not linked, was taken from
/// the keys: its linking screen puts focus on its way back once that is
/// drawn.
fn login_focus_id() -> egui::Id {
    egui::Id::new("account-switcher-login-focus")
}

/// The account being linked, when others are already here: a way back, and
/// for a number the phone unlinked, a way to remove it.
pub fn login_choices(app: &mut App, ui: &mut egui::Ui) {
    if !app.has_several_accounts() {
        return;
    }
    let palette = app.palette;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let back = if app.adding_account {
            "Cancel".to_owned()
        } else {
            "Use another account".to_owned()
        };
        let remove = (!app.adding_account).then_some("Remove this account");
        let width = ui
            .painter()
            .layout_no_wrap(back.clone(), theme::medium(13.0), palette.link)
            .size()
            .x
            + remove.map_or(0.0, |remove| {
                24.0 + ui
                    .painter()
                    .layout_no_wrap(remove.to_owned(), theme::medium(13.0), palette.link)
                    .size()
                    .x
            });
        ui.add_space((ui.available_width() - width).max(0.0) / 2.0);
        let back = theme::link(ui, &back, theme::medium(13.0), palette.link);
        if back.visible()
            && ui
                .ctx()
                .data(|data| data.get_temp::<bool>(login_focus_id()))
                .unwrap_or(false)
        {
            ui.ctx()
                .data_mut(|data| data.remove::<bool>(login_focus_id()));
            back.request_focus();
        }
        if back.clicked() {
            if app.adding_account {
                app.actions.push(Action::CancelAddAccount);
            } else {
                app.account_menu = true;
            }
        }
        if let Some(remove) = remove {
            ui.add_space(24.0);
            if theme::link(ui, remove, theme::medium(13.0), palette.danger).clicked() {
                let id = app.account().id.clone();
                app.actions.push(Action::ShowDialog(
                    crate::model::Dialog::ConfirmRemoveAccount(id),
                ));
            }
        }
    });
}

/// The switcher in the top-left corner of the linking screen, where there is
/// no chat list, once more than one account is here.
pub fn corner(app: &mut App, ctx: &egui::Context) {
    if !app.has_several_accounts() {
        return;
    }
    let inset = theme::traffic_light_inset(ctx);
    egui::Area::new(egui::Id::new("account-switcher-corner"))
        .anchor(Align2::LEFT_TOP, vec2(14.0 + inset, 12.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            avatar_button(app, ui, 34.0);
        });
}
