//! Sending a location: the spot the reader pastes, checked before it goes out.

use egui::{Align, Layout};

use crate::app::App;
use crate::geo;
use crate::model::Action;
use crate::theme::{self, Icon};

pub fn send(app: &mut App, ui: &mut egui::Ui, chat: &str) {
    let palette = app.palette;
    ui.horizontal(|ui| {
        theme::icon(ui, Icon::MapPin, 20.0, palette.accent);
        theme::text(
            ui,
            crate::i18n::gettext(app.locale, "Send location"),
            theme::bold(18.0),
            palette.text,
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if theme::icon_button(ui, Icon::X, 16.0, palette.secondary, palette.text, "Close")
                .clicked()
            {
                app.actions.push(Action::CloseDialog);
            }
        });
    });
    ui.add_space(8.0);
    let hint = crate::i18n::gettext(app.locale, "Paste a map link or coordinates");
    let field = ui.add(
        egui::TextEdit::singleline(&mut app.location_draft)
            .id_salt("send-location")
            .hint_text(hint.as_ref())
            .char_limit(2048)
            .font(theme::regular(14.0))
            .desired_width(f32::INFINITY),
    );
    // The placeholder is all the field shows of what it is for.
    super::widgets::name_field(&field, &hint);
    ui.add_space(6.0);
    // The spot is read again here and shown before it is sent: a pair read the
    // wrong way round is visible in this dialog, not in someone else's chat.
    let here = geo::spot(&app.location_draft);
    // What the check found, as the line under the field shows it: the
    // field's description, so a screen reader hears it with the field.
    let read = match here {
        Some(here) => Some(here.text()),
        None if app.location_draft.trim().is_empty() => None,
        None if geo::is_short_link(&app.location_draft) => Some(
            crate::i18n::gettext(
                app.locale,
                "Short links only open online. Paste the full map link or the coordinates.",
            )
            .into_owned(),
        ),
        None => Some(crate::i18n::gettext(app.locale, "That is not a location.").into_owned()),
    };
    describe(&field, read.as_deref());
    match here {
        Some(here) => {
            theme::text(ui, here.text(), theme::regular(13.0), palette.text);
            let open = crate::i18n::gettext(app.locale, "Open in a map");
            if theme::link(ui, open.as_ref(), theme::regular(12.5), palette.link).clicked() {
                app.actions.push(Action::OpenUrl(geo::map_url(here)));
            }
        }
        None if app.location_draft.trim().is_empty() => {}
        // A short link names its spot only on the server it redirects
        // through, and ZapFast asks no server, so it asks for the full link.
        None if geo::is_short_link(&app.location_draft) => {
            theme::text(
                ui,
                crate::i18n::gettext(
                    app.locale,
                    "Short links only open online. Paste the full map link or the coordinates.",
                ),
                theme::regular(12.0),
                palette.dim,
            );
        }
        None => {
            theme::text(
                ui,
                crate::i18n::gettext(app.locale, "That is not a location."),
                theme::regular(12.0),
                palette.dim,
            );
        }
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let cancel = crate::i18n::gettext(app.locale, "Cancel");
        if theme::soft_action_button(ui, &palette, None, cancel.as_ref(), false).clicked() {
            app.actions.push(Action::CloseDialog);
        }
        let connected = app.link.is_connected();
        ui.add_enabled_ui(here.is_some() && connected, |ui| {
            let send = crate::i18n::gettext(app.locale, "Send");
            let button = theme::pill_button(ui, &palette, send.as_ref(), true);
            // Unavailable, Send says why: offline, or nothing to send yet.
            let why = if !connected {
                Some(crate::i18n::gettext(app.locale, "Not connected to WhatsApp").into_owned())
            } else if here.is_none() {
                Some(read.clone().unwrap_or_else(|| hint.clone().into_owned()))
            } else {
                None
            };
            describe(&button, why.as_deref());
            if button.clicked()
                && let Some(here) = here
            {
                app.actions.push(Action::SendLocation {
                    chat: chat.to_owned(),
                    latitude: here.latitude,
                    longitude: here.longitude,
                    quoting: app.reply_to.clone(),
                });
            }
        });
    });
}

/// Gives `response`'s node `description`, or none.
fn describe(response: &egui::Response, description: Option<&str>) {
    let description = description.map(str::to_owned);
    response
        .ctx
        .accesskit_node_builder(response.id, |node| match description {
            Some(description) => node.set_description(description),
            None => node.clear_description(),
        });
}
