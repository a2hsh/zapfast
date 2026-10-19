//! Phone linking with a QR code or pairing code.

use egui::{Align, CornerRadius, Frame, Layout, Margin, Stroke, Vec2};

use crate::app::App;
use crate::backend::LinkStatus;
use crate::model::{AccountId, Action, Dialog};
use crate::qr::Qr;
use crate::theme::{self, Icon};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let window = ui.max_rect();
    announce_status(app, ui);
    card(app, ui, "login", "A native WhatsApp client.", body);
    if app.qr_enlarged {
        enlarged_qr(app, ui, window);
    }
}

/// The linking QR code as a button: focus, Enter and Escape return here.
fn qr_button_id() -> egui::Id {
    egui::Id::new("login-qr")
}

/// Marks a line of text as a heading, which a screen reader can jump to.
fn heading(ui: &egui::Ui, response: &egui::Response) {
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_role(egui::accesskit::Role::Heading);
        node.set_level(1);
    });
}

/// A code as a screen reader spells it: "A B C D, E F G H", where the
/// screen shows "ABCD-EFGH". Read as a word, it would be guessed at.
fn spelled(code: &str) -> String {
    code.split('-')
        .flat_map(|part| {
            part.chars()
                .collect::<Vec<_>>()
                .chunks(4)
                .map(|group| {
                    group
                        .iter()
                        .map(char::to_string)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Says each change of the linking state once: the screen changes in
/// place, so a screen reader would otherwise hear nothing. The QR code is
/// not announced: focus lands on it, and its refreshes would only be noise.
fn announce_status(app: &mut App, ui: &egui::Ui) {
    use crate::announcer::Category;
    let id = egui::Id::new("login-status-said");
    let locale = app.locale;
    let t = |text: &'static str| crate::i18n::gettext(locale, text).into_owned();
    let (key, said, category) = match &app.link {
        LinkStatus::Starting | LinkStatus::Connecting => (
            "connecting".to_owned(),
            Some(t("Connecting to WhatsApp")),
            Category::Status,
        ),
        LinkStatus::Connected | LinkStatus::Disconnected { .. } => (
            "linked".to_owned(),
            Some(t("Linked. Waiting for your chats")),
            Category::Status,
        ),
        LinkStatus::LoggedOut => (
            "logged-out".to_owned(),
            Some(t(
                "This computer was unlinked from your phone. Requesting a new code.",
            )),
            Category::Status,
        ),
        LinkStatus::Failed(message) => (
            format!("failed:{message}"),
            Some(message.clone()),
            Category::Error,
        ),
        LinkStatus::Unlinked {
            pair_code: Some(code),
            ..
        } => (
            format!("code:{code}"),
            Some(
                t("Enter this code on your phone: {}. On your phone, open WhatsApp, tap Menu or Settings, then Linked devices, then Link a device, then Link with phone number instead.")
                    .replace("{}", &spelled(code)),
            ),
            Category::Status,
        ),
        LinkStatus::Unlinked {
            pairing_phone: Some(phone),
            ..
        } => (
            "requesting".to_owned(),
            Some(t("Requesting a code for +{}").replace("{}", phone)),
            Category::Status,
        ),
        LinkStatus::Unlinked { qr: Some(_), .. } => ("qr".to_owned(), None, Category::Status),
        LinkStatus::Unlinked { .. } => (
            "waiting".to_owned(),
            Some(t("Waiting for a code from WhatsApp")),
            Category::Status,
        ),
    };
    let last: Option<String> = ui.ctx().data(|data| data.get_temp(id));
    if last.as_deref() != Some(key.as_str()) {
        ui.ctx().data_mut(|data| data.insert_temp(id, key));
        if let Some(said) = said {
            app.announcer.say(category, said);
        }
    }
}

/// Once the account links, says so and, for keyboard and screen reader
/// users, puts focus on the chat list once it has chats.
pub fn follow_linking(app: &mut App, ctx: &egui::Context) {
    let was_id = egui::Id::new("login-was-linked");
    let waiting_id = egui::Id::new("login-chats-pending");
    let account = app.account().id.clone();
    let linked = app.is_linked();
    // Kept with the account it was seen for: switching from an account on
    // its linking screen to one already linked links nothing.
    let was: Option<(AccountId, bool)> = ctx.data(|data| data.get_temp(was_id));
    ctx.data_mut(|data| data.insert_temp(was_id, (account.clone(), linked)));
    if was.is_some_and(|(seen, was)| seen == account && !was) && linked {
        app.qr_enlarged = false;
        ctx.data_mut(|data| data.insert_temp(waiting_id, account.clone()));
    }
    let waiting = ctx
        .data(|data| data.get_temp::<AccountId>(waiting_id))
        .is_some_and(|waiting| waiting == account);
    if waiting && linked && !app.chats.is_empty() {
        ctx.data_mut(|data| data.remove::<AccountId>(waiting_id));
        app.announcer.say(
            crate::announcer::Category::Status,
            crate::i18n::gettext(app.locale, "Your chats are ready"),
        );
        if super::keys::keyboard_or_reader(ctx) {
            app.actions.push(Action::FocusChats);
        }
    }
}

/// The QR code filling the window, so a phone camera finds it easily.
/// Escape, Enter, Space or a click returns it to the card. It draws the
/// current code, so a refreshed code replaces it in place.
fn enlarged_qr(app: &mut App, ui: &mut egui::Ui, window: egui::Rect) {
    let code = match &app.link {
        LinkStatus::Unlinked {
            qr: Some(qr),
            pair_code: None,
            pairing_phone: None,
        } => qr.clone(),
        _ => {
            app.qr_enlarged = false;
            return;
        }
    };
    let locale = app.locale;
    let mut close = false;
    egui::Area::new(egui::Id::new("login-qr-enlarged"))
        .order(egui::Order::Foreground)
        .fixed_pos(window.min)
        .show(ui.ctx(), |ui| {
            ui.set_min_size(window.size());
            ui.painter().rect_filled(window, 0.0, egui::Color32::WHITE);
            // Room below the code for how to return, for pointer users.
            let hint_height = 40.0;
            let side = (window.width().min(window.height() - hint_height) - 48.0).max(64.0);
            let center = window.center() - egui::vec2(0.0, hint_height / 2.0);
            let rect = egui::Rect::from_center_size(center, Vec2::splat(side));
            if let Some(qr) = Qr::encode(&code) {
                qr.paint(ui, rect, egui::Color32::BLACK, egui::Color32::WHITE);
            }
            let hint = crate::i18n::gettext(locale, "Click anywhere or press Escape to return");
            ui.painter().text(
                egui::pos2(window.center().x, rect.bottom() + hint_height / 2.0 + 4.0),
                egui::Align2::CENTER_CENTER,
                hint,
                theme::regular(15.0),
                egui::Color32::from_gray(40),
            );
            let view = ui.interact(window, qr_button_id().with("enlarged"), egui::Sense::click());
            let label = crate::i18n::gettext(locale, "QR code enlarged").into_owned();
            view.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone())
            });
            let described = crate::i18n::gettext(
                locale,
                "Point your phone's camera at the screen, about a forearm's length away. Press Escape to return.",
            )
            .into_owned();
            ui.ctx().accesskit_node_builder(view.id, |node| {
                node.set_description(described);
            });
            if !view.has_focus() {
                view.request_focus();
            }
            // Escape is taken in `keys::handle`, before anything else could.
            close = view.clicked();
        });
    if close {
        shrink_qr(app, ui.ctx());
    }
}

/// Returns the enlarged QR code to the card, and focus to it there.
pub(super) fn shrink_qr(app: &mut App, ctx: &egui::Context) {
    app.qr_enlarged = false;
    ctx.memory_mut(|memory| memory.request_focus(qr_button_id()));
}

/// The centred card of the linking and lock screens: the logo, the name,
/// a line under it, and `body`.
pub(super) fn card(
    app: &mut App,
    ui: &mut egui::Ui,
    salt: &str,
    tagline: &str,
    body: impl FnOnce(&mut App, &mut egui::Ui),
) {
    let palette = app.palette;
    egui::CentralPanel::default()
        .frame(Frame::new().fill(palette.window))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            let top = theme::blend(palette.window, palette.accent, 0.10);
            super::widgets::paint_vertical_gradient(ui, rect, top, palette.window);
            let card_width = (460.0_f32.min(rect.width() - 24.0)).max(0.0);
            // Center the card using its previous height. Its content determines
            // the next frame's height.
            let height_id = ui.id().with(format!("{salt}-card-height"));
            let known_height = ui
                .ctx()
                .data(|data| data.get_temp::<f32>(height_id))
                .unwrap_or(560.0);
            let card_height = (known_height.min(rect.height() - 24.0)).max(0.0);
            let card =
                egui::Rect::from_center_size(rect.center(), Vec2::new(card_width, card_height));
            let mut card_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(card)
                    .layout(Layout::top_down(Align::Center)),
            );
            let shown = Frame::new()
                .fill(palette.panel)
                .stroke(Stroke::new(1.0, palette.outline))
                .corner_radius(CornerRadius::same(theme::RADIUS + 8))
                .inner_margin(Margin::same(32))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 16],
                    blur: 48,
                    spread: 0,
                    color: palette.shadow,
                })
                .show(&mut card_ui, |ui| {
                    ui.set_width((card_width - 64.0).max(0.0));
                    ui.spacing_mut().item_spacing.y = 8.0;
                    let (logo, _) = ui.allocate_exact_size(Vec2::splat(64.0), egui::Sense::hover());
                    theme::mark(ui, logo.center(), 64.0);
                    ui.add_space(4.0);
                    theme::text(ui, "ZapFast", theme::bold(28.0), palette.text);
                    theme::text(ui, tagline, theme::regular(14.5), palette.secondary);
                    ui.add_space(16.0);
                    body(app, ui);
                });
            let height = shown.response.rect.height();
            if (height - known_height).abs() > 2.0 {
                ui.ctx()
                    .data_mut(|data| data.insert_temp(height_id, height.round()));
                ui.ctx().request_repaint();
            }
        });
}

fn body(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    match app.link.clone() {
        LinkStatus::Starting | LinkStatus::Connecting => {
            busy(ui, palette.accent, "Connecting to WhatsApp…");
        }
        LinkStatus::Connected | LinkStatus::Disconnected { .. } => {
            busy(ui, palette.accent, "Linked. Waiting for your chats…");
        }
        LinkStatus::LoggedOut => {
            theme::icon(ui, Icon::Smartphone, 28.0, palette.warning);
            theme::paragraph(
                ui,
                "This computer was unlinked from your phone. Requesting a new code.",
                theme::regular(14.0),
                palette.text,
            );
            ui.add_space(8.0);
            busy(ui, palette.accent, "Requesting a new code…");
        }
        LinkStatus::Failed(message) => {
            let key_lost = archive_key_lost(&message);
            theme::icon(ui, Icon::CircleAlert, 28.0, palette.danger);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(message)
                        .font(theme::regular(13.5))
                        .color(palette.danger),
                )
                .wrap(),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if theme::pill_button(
                    ui,
                    &palette,
                    &crate::i18n::gettext(app.locale, "Try again"),
                    true,
                )
                .clicked()
                {
                    app.actions.push(Action::Reconnect);
                }
                if key_lost
                    && theme::pill_button(
                        ui,
                        &palette,
                        &crate::i18n::gettext(app.locale, "Start over…"),
                        false,
                    )
                    .clicked()
                {
                    app.actions
                        .push(Action::ShowDialog(crate::model::Dialog::ConfirmStartOver));
                }
            });
        }
        LinkStatus::Unlinked {
            qr,
            pair_code,
            pairing_phone,
        } => {
            if let Some(code) = pair_code {
                pair_code_view(app, ui, &code, pairing_phone.as_deref());
            } else if let Some(phone) = pairing_phone {
                busy(
                    ui,
                    palette.accent,
                    &format!("Requesting a code for +{phone}…"),
                );
                ui.add_space(10.0);
                change_number(app, ui, Some(&phone));
            } else if let Some(qr) = qr {
                qr_view(app, ui, &qr);
            } else {
                busy(ui, palette.accent, "Waiting for a code from WhatsApp…");
            }
        }
    }
    super::accounts::login_choices(app, ui);
    ui.add_space(18.0);
    theme::paragraph(
        ui,
        "Unofficial client. Using it may be against WhatsApp's terms of service.",
        theme::regular(11.5),
        palette.dim,
    );
}

pub(super) fn busy(ui: &mut egui::Ui, color: egui::Color32, label: &str) {
    ui.horizontal(|ui| {
        let width = 24.0
            + 8.0
            + ui.painter()
                .layout_no_wrap(label.to_owned(), theme::medium(14.0), color)
                .size()
                .x;
        ui.add_space((ui.available_width() - width).max(0.0) / 2.0);
        theme::spinner(ui, 18.0, color);
        theme::text(ui, label, theme::medium(14.0), ui.visuals().text_color());
    });
}

fn qr_view(app: &mut App, ui: &mut egui::Ui, code: &str) {
    let palette = app.palette;
    let title = theme::text(
        ui,
        "Link this computer",
        theme::semibold(16.0),
        palette.text,
    );
    heading(ui, &title);
    let side = 260.0;
    let (_, rect) = ui.allocate_space(Vec2::splat(side));
    let qr_button = ui.interact(rect, qr_button_id(), egui::Sense::click());
    match Qr::encode(code) {
        Some(qr) => qr.paint(ui, rect, egui::Color32::BLACK, egui::Color32::WHITE),
        None => {
            ui.painter().rect_filled(rect, 8.0, palette.surface);
            theme::paint_icon(ui, Icon::CircleAlert, rect, 32.0, palette.danger);
        }
    }
    ui.add_space(4.0);
    let steps = [
        "Open WhatsApp on your phone",
        "Tap Menu or Settings, then Linked devices",
        "Tap Link a device and point the phone at this code",
    ];
    step_list(app, ui, &steps);
    // A screen reader reads the steps with the button, and how to enlarge it.
    let locale = app.locale;
    let label = crate::i18n::gettext(locale, "QR code for linking").into_owned();
    qr_button
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone()));
    let mut described: Vec<String> = steps
        .iter()
        .enumerate()
        .map(|(index, step)| format!("{}. {}.", index + 1, crate::i18n::gettext(locale, step)))
        .collect();
    described.push(crate::i18n::gettext(locale, "Press Enter to enlarge.").into_owned());
    let described = described.join(" ");
    ui.ctx().accesskit_node_builder(qr_button.id, |node| {
        node.set_description(described);
    });
    theme::focus_outline(ui, qr_button.id, rect, 8.0);
    // Keyboard and screen reader users start here: both ways to link are
    // equally theirs, and Tab goes on to the phone number.
    if !app.qr_enlarged
        && app.dialog.is_none()
        && ui.ctx().memory(|memory| memory.focused().is_none())
        && super::keys::keyboard_or_reader(ui.ctx())
    {
        qr_button.request_focus();
    }
    if qr_button.clicked() {
        app.qr_enlarged = true;
    }
    ui.add_space(10.0);
    if theme::link(
        ui,
        "Link with phone number instead",
        theme::medium(13.0),
        palette.link,
    )
    .clicked()
    {
        app.actions.push(Action::ShowDialog(Dialog::PairWithPhone));
    }
}

/// How long a pairing code works, in seconds: whatsapp-rust's
/// `PAIR_CODE_VALIDITY_SECS`. The library's `PairingCode` event says what is
/// left of it, less the request's own time, but the worker hands on only the
/// code: it is timed from when its account got it
/// (`Account::pair_code_arrived`).
const CODE_VALID: f64 = 180.0;
/// One warning this long before a code runs out, for whoever types slowly.
const CODE_WARNING: f64 = 30.0;
// The screen's words say these times: "30 seconds left to enter the code" and
// "A code works for three minutes". A change here changes them too.
const _: () = assert!(CODE_VALID == 180.0 && CODE_WARNING == 30.0);

fn pair_code_view(app: &mut App, ui: &mut egui::Ui, code: &str, phone: Option<&str>) {
    let palette = app.palette;
    // Each code is timed from when its account got it, not from when a
    // window first shows it: a window opened later, or again, shows what is
    // left. The window's clock then counts on from there.
    let now = ui.input(|input| input.time);
    let clock_id = egui::Id::new("login-code-clock");
    let (started, warned) = match ui
        .ctx()
        .data(|data| data.get_temp::<(String, f64, bool)>(clock_id))
    {
        Some((seen, started, warned)) if seen == code => (started, warned),
        _ => {
            let waited = app
                .pair_code_arrived
                .as_ref()
                .filter(|(arrived, _)| arrived == code)
                .map_or(0.0, |(_, at)| at.elapsed().as_secs_f64());
            let started = now - waited;
            ui.ctx()
                .data_mut(|data| data.insert_temp(clock_id, (code.to_owned(), started, false)));
            (started, false)
        }
    };
    let left = CODE_VALID - (now - started);
    if left <= 0.0 {
        expired_view(app, ui, code, phone);
        return;
    }
    if left <= CODE_WARNING && !warned {
        ui.ctx()
            .data_mut(|data| data.insert_temp(clock_id, (code.to_owned(), started, true)));
        app.announcer.say(
            crate::announcer::Category::Status,
            crate::i18n::gettext(app.locale, "30 seconds left to enter the code"),
        );
    }
    let wake = if left > CODE_WARNING {
        left - CODE_WARNING
    } else {
        left
    };
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_secs_f64(wake.max(0.05)));
    let title = theme::text(
        ui,
        "Enter this code on your phone",
        theme::semibold(16.0),
        palette.text,
    );
    heading(ui, &title);
    if let Some(phone) = phone {
        theme::text(
            ui,
            format!("for +{phone}"),
            theme::regular(13.0),
            palette.secondary,
        );
    }
    ui.add_space(8.0);
    let shown = if code.len() == 8 && !code.contains('-') {
        format!("{}-{}", &code[..4], &code[4..])
    } else {
        code.to_owned()
    };
    Frame::new()
        .fill(palette.surface)
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(Margin::symmetric(22, 12))
        .show(ui, |ui| {
            // A read-only field, so a screen reader can read the code a
            // character at a time with the arrows. It looks as the text did.
            let font = theme::bold(30.0);
            let width = ui
                .painter()
                .layout_no_wrap(shown.clone(), font.clone(), palette.text)
                .size()
                .x
                + 4.0;
            let mut text: &str = &shown;
            let field = egui::TextEdit::singleline(&mut text)
                .id(code_field_id())
                .font(font)
                .text_color(palette.text)
                .frame(Frame::NONE)
                .margin(Margin::ZERO)
                .horizontal_align(Align::Center)
                .desired_width(width)
                .show(ui)
                .response;
            let label = crate::i18n::gettext(app.locale, "Pairing code").into_owned();
            let described = crate::i18n::gettext(
                app.locale,
                "Read it with the arrow keys. Press Escape to change the phone number.",
            )
            .into_owned();
            ui.ctx().accesskit_node_builder(field.id, |node| {
                node.set_label(label);
                node.set_description(described);
                node.set_read_only();
            });
            // A new code takes focus once, for keyboard and screen reader
            // users: it was announced, and here it can be read again.
            let focused_id = egui::Id::new("login-code-focused");
            let done = ui
                .ctx()
                .data(|data| data.get_temp::<String>(focused_id))
                .is_some_and(|done| done == code);
            if !done && super::keys::keyboard_or_reader(ui.ctx()) {
                ui.ctx()
                    .data_mut(|data| data.insert_temp(focused_id, code.to_owned()));
                field.request_focus();
            }
        });
    ui.add_space(8.0);
    let steps = [
        "Open WhatsApp on your phone",
        "Tap Menu or Settings, then Linked devices",
        "Tap Link a device, then Link with phone number instead",
    ];
    step_list(app, ui, &steps);
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.add_space((ui.available_width() - 200.0).max(0.0) / 2.0);
        let copy = theme::soft_action_button(ui, &palette, Some(Icon::Copy), "Copy code", false);
        if copy.clicked() {
            app.actions.push(Action::CopyText(code.to_owned()));
        }
    });
    ui.add_space(6.0);
    change_number(app, ui, phone);
}

/// A code that ran out: it would only fail on the phone, so it gives way to
/// a new one for the same number, or one step back to the number.
fn expired_view(app: &mut App, ui: &mut egui::Ui, code: &str, phone: Option<&str>) {
    let palette = app.palette;
    let title = theme::text(ui, "This code expired", theme::semibold(16.0), palette.text);
    heading(ui, &title);
    theme::paragraph(
        ui,
        "A code works for three minutes. Get a new one, then enter it on your phone.",
        theme::regular(13.0),
        palette.secondary,
    );
    ui.add_space(10.0);
    let mut again = None;
    ui.horizontal(|ui| {
        ui.add_space((ui.available_width() - 160.0).max(0.0) / 2.0);
        again = Some(theme::pill_button(ui, &palette, "Get a new code", true));
    });
    let Some(again) = again else {
        return;
    };
    // Said and focused once per code that ran out.
    let said_id = egui::Id::new("login-code-expired");
    let said = ui
        .ctx()
        .data(|data| data.get_temp::<String>(said_id))
        .is_some_and(|said| said == code);
    if !said {
        ui.ctx()
            .data_mut(|data| data.insert_temp(said_id, code.to_owned()));
        app.announcer.say(
            crate::announcer::Category::Status,
            crate::i18n::gettext(app.locale, "This code expired. Get a new code."),
        );
        if super::keys::keyboard_or_reader(ui.ctx()) {
            again.request_focus();
        }
    }
    if again.clicked()
        && let Some(phone) = phone
    {
        app.actions.push(Action::PairWithPhone(phone.to_owned()));
    }
    ui.add_space(6.0);
    change_number(app, ui, phone);
}

/// The pairing code as a read-only field: the arrows read it.
fn code_field_id() -> egui::Id {
    egui::Id::new("login-pair-code")
}

/// One step back from a pairing code to the phone number, kept as it was,
/// which Escape also takes; from there Escape goes on to the QR code.
fn change_number(app: &mut App, ui: &mut egui::Ui, phone: Option<&str>) {
    let palette = app.palette;
    let link = theme::link(ui, "Change phone number", theme::medium(13.0), palette.link);
    // Which number the code is for, heard before deciding to change it.
    if let Some(phone) = phone {
        let current = crate::i18n::gettext(app.locale, "Current number: +{}").replace("{}", phone);
        ui.ctx()
            .accesskit_node_builder(link.id, |node| node.set_description(current));
    }
    if link.clicked() {
        app.actions.push(Action::CancelPairing);
    }
}

/// Numbered steps, drawn as before, that a screen reader finds as a list.
fn step_list(app: &App, ui: &mut egui::Ui, steps: &[&'static str]) {
    let palette = app.palette;
    let locale = app.locale;
    let len = steps.len();
    ui.scope(|ui| {
        let label = crate::i18n::gettext(locale, "Steps").into_owned();
        ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
            node.set_role(egui::accesskit::Role::List);
            node.set_label(label);
            node.set_size_of_set(len);
        });
        for (index, step) in steps.iter().enumerate() {
            ui.horizontal(|ui| {
                let item = format!("{}. {}", index + 1, crate::i18n::gettext(locale, step));
                ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
                    node.set_role(egui::accesskit::Role::ListItem);
                    node.set_label(item);
                    node.set_position_in_set(index);
                    node.set_size_of_set(len);
                });
                ui.add_space(12.0);
                theme::text(
                    ui,
                    format!("{}.", index + 1),
                    theme::semibold(13.0),
                    palette.accent,
                );
                theme::text(ui, *step, theme::regular(13.0), palette.secondary);
            });
        }
    });
}

/// Whether the archive's key is gone for good, rather than the keyring being
/// locked or unavailable, which "Try again" can fix.
fn archive_key_lost(message: &str) -> bool {
    message.contains("OS keyring key is missing")
        || message.contains("archive key in the OS keyring is invalid")
        || message.contains("could not be unlocked with its OS keyring key")
}

#[cfg(test)]
mod start_over_tests {
    #[test]
    fn only_a_lost_key_offers_to_start_over() {
        for lost in [
            "The archive is encrypted but its OS keyring key is missing. Restore the original keyring; the archive has not been changed",
            "The archive key in the OS keyring is invalid",
            "The archive could not be unlocked with its OS keyring key: file is not a database",
        ] {
            assert!(super::archive_key_lost(lost), "{lost}");
        }
        for recoverable in [
            "Unlock your OS keyring and restart ZapFast",
            "The OS keyring could not open ZapFast's archive key",
        ] {
            assert!(!super::archive_key_lost(recoverable), "{recoverable}");
        }
    }
}
