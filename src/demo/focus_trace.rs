//! Frame-by-frame focus traces: what AccessKit is told has focus in every
//! frame of a keyboard scenario. A screen reader speaks each change of that
//! focus, so a frame that reports the window (the tree's root), or a key
//! that moves focus twice (to a node and on, or away and straight back), is
//! heard as "ZapFast window" or as an extra item, however briefly it lasts.

use egui::accesskit::{NodeId, TreeUpdate};

use super::tests::{app, press_key, render};
use crate::app::App;
use crate::model::Action;
use crate::ui::focus::{Stop, control};

/// One frame as a screen reader sees it.
#[derive(Clone, Debug)]
struct Frame {
    step: String,
    /// The user action this frame belongs to: its key frame and the quiet
    /// frames after it.
    action: usize,
    /// The tree's focus.
    focus: NodeId,
    /// Where the reader is: the focused node's active descendant, if it has
    /// one in this update, else the focused node.
    effective: NodeId,
    /// The effective node's ancestors, nearest first. A reader announces
    /// the ancestors that changed, so a node moved under another parent is
    /// heard again.
    ancestors: Vec<NodeId>,
    /// The effective node's role and label, for the failure dump.
    name: String,
    /// What a reader says of the effective node: a change while it keeps
    /// focus is read out again.
    spoken: String,
    /// The effective node's ancestors as `describe` names them, nearest
    /// first, such as the menu an item is in.
    ancestor_names: Vec<String>,
    /// What this frame's update says of the node the reader was on in the
    /// frame before, empty when it is gone. A screen reader takes a change
    /// to it in the update that moves focus away before the focus change,
    /// and reads that node out again first.
    left: String,
    /// How many nodes the frame's update carries.
    nodes: usize,
    /// The effective node's description. NVDA reads it when focus arrives
    /// and when asked, but does not speak a change to it, so it is not part
    /// of `spoken`.
    description: Option<String>,
    /// egui's focused widget after the frame.
    egui: Option<egui::Id>,
    /// The layer that widget is on, by its id.
    layer: Option<egui::Id>,
    /// egui's focused widget at the end of each pass's UI, before egui's own
    /// end of pass.
    passes: Vec<Option<egui::Id>>,
}

impl Frame {
    fn heard(&self) -> (NodeId, &[NodeId], &str) {
        (self.effective, &self.ancestors, &self.spoken)
    }
}

struct Trace {
    app: App,
    ctx: egui::Context,
    frames: Vec<Frame>,
    window_focused: bool,
    action: usize,
    time: f64,
}

fn root() -> NodeId {
    egui::accesskit_root_id().accesskit_id()
}

fn describe(tree: &TreeUpdate, id: NodeId) -> String {
    if id == root() {
        return "WINDOW".to_owned();
    }
    tree.nodes
        .iter()
        .find(|(found, _)| *found == id)
        .map_or_else(
            || "missing node".to_owned(),
            |(_, node)| {
                let label: String = node.label().unwrap_or_default().chars().take(24).collect();
                format!("{:?} {label:?}", node.role())
            },
        )
}

fn spoken(tree: &TreeUpdate, id: NodeId) -> String {
    tree.nodes
        .iter()
        .find(|(found, _)| *found == id)
        .map(|(_, node)| {
            format!(
                "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
                node.role(),
                node.label(),
                node.value(),
                node.is_selected(),
                node.toggled(),
                node.position_in_set(),
                node.size_of_set()
            )
        })
        .unwrap_or_default()
}

fn effective(tree: &TreeUpdate) -> NodeId {
    let node = |id: NodeId| tree.nodes.iter().find(|(found, _)| *found == id);
    node(tree.focus)
        .and_then(|(_, focused)| focused.active_descendant())
        .filter(|active| node(*active).is_some())
        .unwrap_or(tree.focus)
}

fn ancestors(tree: &TreeUpdate, mut id: NodeId) -> Vec<NodeId> {
    let mut chain = Vec::new();
    while let Some((parent, _)) = tree
        .nodes
        .iter()
        .find(|(_, node)| node.children().contains(&id))
    {
        chain.push(*parent);
        id = *parent;
        if chain.len() > 64 {
            break;
        }
    }
    chain
}

impl Trace {
    fn new(app: App) -> Self {
        let ctx = egui::Context::default();
        let mut app = app;
        app.attach(&ctx);
        ctx.enable_accesskit();
        let mut trace = Self {
            app,
            ctx,
            frames: Vec::new(),
            window_focused: true,
            action: 0,
            time: 0.0,
        };
        trace.settle("start", 4);
        trace
    }

    /// Runs one frame with `events`, as eframe does: the app's logic and its
    /// UI in every pass, with time moving on at 60 frames a second.
    fn frame(&mut self, step: &str, events: Vec<egui::Event>) {
        self.time += 1.0 / 60.0;
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1180.0, 780.0),
            )),
            time: Some(self.time),
            events,
            focused: self.window_focused,
            ..Default::default()
        };
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .expect("the root viewport")
            .focused = Some(self.window_focused);
        let mut passes = Vec::new();
        let app = &mut self.app;
        let mut output = self.ctx.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            app.background_frame(&ctx);
            app.frame_ui(ui);
            passes.push(ctx.memory(|memory| memory.focused()));
        });
        output.textures_delta.clear();
        let tree = output
            .platform_output
            .accesskit_update
            .expect("accessibility tree");
        let effective = effective(&tree);
        let ancestors = ancestors(&tree, effective);
        let left = self
            .frames
            .last()
            .map(|before| spoken(&tree, before.effective))
            .unwrap_or_default();
        let description = tree
            .nodes
            .iter()
            .find(|(found, _)| *found == effective)
            .and_then(|(_, node)| node.description().map(str::to_owned));
        self.frames.push(Frame {
            left,
            nodes: tree.nodes.len(),
            step: step.to_owned(),
            action: self.action,
            focus: tree.focus,
            effective,
            ancestor_names: ancestors.iter().map(|id| describe(&tree, *id)).collect(),
            ancestors,
            name: describe(&tree, effective),
            spoken: spoken(&tree, effective),
            description,
            egui: self.ctx.memory(|memory| memory.focused()),
            layer: self
                .ctx
                .memory(|memory| memory.focused())
                .and_then(|id| self.ctx.read_response(id))
                .map(|response| response.layer_id.id),
            passes,
        });
    }

    fn settle(&mut self, step: &str, frames: usize) {
        for _ in 0..frames {
            self.frame(step, Vec::new());
        }
    }

    /// A key pressed in one frame and released in the next, then a few
    /// quiet frames.
    fn press(&mut self, step: &str, key: egui::Key, modifiers: egui::Modifiers) {
        self.action += 1;
        self.frame(step, key_press(key, modifiers));
        self.frame(
            step,
            vec![egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: false,
                repeat: false,
                modifiers,
            }],
        );
        self.settle(step, 4);
    }

    /// A key held down: a press, then `repeats` repeats, every other frame
    /// as a keyboard repeats at about 30 a second, each its own action as
    /// the reader hears it.
    fn hold(&mut self, step: &str, key: egui::Key, repeats: usize) {
        for repeat in 0..=repeats {
            self.action += 1;
            self.frame(
                step,
                vec![egui::Event::Key {
                    key,
                    physical_key: Some(key),
                    pressed: true,
                    repeat: repeat > 0,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            self.frame(step, Vec::new());
        }
        self.settle(step, 4);
    }

    /// The window loses or regains focus, as winit reports it.
    fn window(&mut self, step: &str, focused: bool) {
        self.action += 1;
        self.window_focused = focused;
        self.frame(step, vec![egui::Event::WindowFocused(focused)]);
        self.settle(step, 4);
    }

    fn mark(&self) -> usize {
        self.frames.len()
    }

    fn last(&self) -> &Frame {
        self.frames.last().expect("a frame ran")
    }

    fn dump(&self, from: usize) -> String {
        self.frames[from..]
            .iter()
            .enumerate()
            .map(|(index, frame)| {
                format!(
                    "  #{:<3} {:<14} reader {} {:<40} up {:?} egui {:?} passes {:?}{}",
                    from + index,
                    frame.step,
                    frame.effective.0 % 100_000,
                    frame.name,
                    frame
                        .ancestors
                        .iter()
                        .map(|id| id.0 % 10_000)
                        .collect::<Vec<_>>(),
                    frame.egui,
                    frame.passes,
                    if frame.focus == root() {
                        "  <-- WINDOW"
                    } else {
                        ""
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Failures from frame `from` on: the window reported as focus, one
    /// action heard as more than one focus change, or the item focus is on
    /// given a new node id, which a reader hears as a move to it.
    fn failures(&self, from: usize) -> Vec<String> {
        let mut failures = Vec::new();
        for (index, frame) in self.frames.iter().enumerate().skip(from) {
            if frame.focus == root() {
                failures.push(format!(
                    "frame #{index} ({}): focus is the window",
                    frame.step
                ));
            }
        }
        let mut changes = 0;
        for index in from.max(1)..self.frames.len() {
            let (before, frame) = (&self.frames[index - 1], &self.frames[index]);
            if frame.action != before.action {
                changes = 0;
            }
            if frame.effective != before.effective
                && frame.spoken == before.spoken
                && frame.ancestors == before.ancestors
            {
                failures.push(format!(
                    "frame #{index} ({}): the same item under a new node id",
                    frame.step
                ));
            }
            if frame.heard() != before.heard() {
                changes += 1;
                if changes == 2 {
                    failures.push(format!(
                        "frame #{index} ({}): a second focus change for one action",
                        frame.step
                    ));
                }
            }
        }
        failures
    }

    fn assert_clean(&self, from: usize, scenario: &str) {
        let failures = self.failures(from);
        assert!(
            failures.is_empty(),
            "{scenario}:\n{}\n{}",
            failures.join("\n"),
            self.dump(from.saturating_sub(1))
        );
    }

    /// No focus change at all from frame `from` on.
    fn assert_unchanged(&self, from: usize, scenario: &str) {
        let before = &self.frames[from - 1];
        assert!(
            self.frames[from..]
                .iter()
                .all(|frame| frame.heard() == before.heard() && frame.focus != root()),
            "{scenario}: focus moved\n{}",
            self.dump(from - 1)
        );
    }
}

/// The events of a key press as Windows delivers them: Alt with a digit
/// also types the digit as text, which a shortcut must not let through.
fn key_press(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
    let mut events = vec![egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers,
    }];
    let digit = key.name();
    if modifiers.alt && digit.len() == 1 && digit.chars().all(|c| c.is_ascii_digit()) {
        events.push(egui::Event::Text(digit.to_owned()));
    }
    events
}

fn alt() -> egui::Modifiers {
    if cfg!(target_os = "macos") {
        egui::Modifiers::CTRL
    } else {
        egui::Modifiers::ALT
    }
}

fn long_chat(app: &mut App, count: usize) {
    let chat = app.open_chat.clone().expect("the demo opens a chat");
    let conversation = app.conversations.get_mut(&chat).expect("its messages");
    let template = conversation.messages.last().expect("a message").clone();
    conversation.messages = (0..count)
        .map(|index| crate::model::Message {
            id: format!("long-{index}"),
            timestamp: template.timestamp - (count - index) as i64 * 60,
            content: crate::model::Content::text(format!("Fixture line {index}")),
            ..template.clone()
        })
        .collect();
}

/// Gives focus to the main view's `stop` as the keys reach it: the lists
/// by their shortcuts, the message field as the demo starts, anything else
/// as Tab would.
fn focus_stop(trace: &mut Trace, stop: Stop) {
    match stop {
        Stop::ChatList => trace.press("Alt+1", egui::Key::Num1, alt()),
        Stop::Messages => trace.press("Alt+2", egui::Key::Num2, alt()),
        Stop::Composer => {}
        _ => {
            let id = control(&trace.ctx, stop).expect("drawn");
            trace.ctx.memory_mut(|memory| memory.request_focus(id));
            trace.settle("focus", 3);
        }
    }
    let id = control(&trace.ctx, stop).expect("drawn");
    assert_eq!(trace.last().egui, Some(id), "{stop:?} took focus");
}

/// The demo with no chat open and `stop` focused.
fn no_chat_open(stop: Stop) -> Trace {
    let mut trace = Trace::new(app());
    trace.app.actions.push(Action::CloseChat);
    trace.settle("close the chat", 3);
    focus_stop(&mut trace, stop);
    trace
}

#[test]
fn arrowing_the_message_list_never_reports_the_window() {
    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::Messages);
    let from = trace.mark();
    for _ in 0..12 {
        trace.press("Up", egui::Key::ArrowUp, egui::Modifiers::NONE);
    }
    for _ in 0..4 {
        trace.press("Down", egui::Key::ArrowDown, egui::Modifiers::NONE);
    }
    trace.assert_clean(from, "arrows in the message list");
}

/// In the message menu, the reactions are one stop of Up and Down: they
/// land on an emoji, Left and Right move between the emoji, and Up and Down
/// leave the row, which they come back to on the emoji left last.
#[test]
fn the_reactions_are_one_stop_of_the_message_menu() {
    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::Messages);
    let from = trace.mark();
    let heard = |trace: &mut Trace, step: &str, key: egui::Key| {
        trace.press(step, key, egui::Modifiers::NONE);
        trace.last().name.clone()
    };
    trace.press("Shift+F10", egui::Key::F10, egui::Modifiers::SHIFT);
    let reply = trace.last().name.clone();
    assert_eq!(reply, "MenuItem \"Reply\"", "{}", trace.dump(from));
    let first = heard(&mut trace, "Up", egui::Key::ArrowUp);
    assert!(first.starts_with("Button"), "Up lands on an emoji: {first}");
    let second = heard(&mut trace, "Right", egui::Key::ArrowRight);
    assert!(second.starts_with("Button") && second != first, "{second}");
    let third = heard(&mut trace, "Right", egui::Key::ArrowRight);
    assert!(third.starts_with("Button") && third != second, "{third}");
    assert_eq!(heard(&mut trace, "Left", egui::Key::ArrowLeft), second);
    // Up from the row, the first stop, wraps to the last item, not to the
    // emoji before.
    let last = heard(&mut trace, "Up", egui::Key::ArrowUp);
    assert!(
        last.starts_with("MenuItem") && last != reply,
        "Up leaves the row: {last}\n{}",
        trace.dump(from)
    );
    assert_eq!(
        heard(&mut trace, "Down", egui::Key::ArrowDown),
        second,
        "back on the emoji left last"
    );
    assert_eq!(heard(&mut trace, "Down", egui::Key::ArrowDown), reply);
    trace.assert_clean(from, "the reactions in the message menu");
}

/// Home and End reach messages far from the view, which are not laid out
/// yet, and the page keys move the rows laid out above the current one: the
/// reader still hears each message once.
#[test]
fn a_long_message_list_never_reports_the_window() {
    let mut app = app();
    long_chat(&mut app, 400);
    let mut trace = Trace::new(app);
    focus_stop(&mut trace, Stop::Messages);
    let from = trace.mark();
    for (step, key) in [
        ("Home", egui::Key::Home),
        ("Down", egui::Key::ArrowDown),
        ("Down", egui::Key::ArrowDown),
        ("End", egui::Key::End),
        ("Up", egui::Key::ArrowUp),
        ("PageUp", egui::Key::PageUp),
        ("PageUp", egui::Key::PageUp),
        ("PageDown", egui::Key::PageDown),
        ("Home", egui::Key::Home),
        ("Up", egui::Key::ArrowUp),
        ("End", egui::Key::End),
    ] {
        trace.press(step, key, egui::Modifiers::NONE);
    }
    // Held, the arrows move a message a frame and scroll as they go.
    trace.hold("Up held", egui::Key::ArrowUp, 40);
    trace.hold("Down held", egui::Key::ArrowDown, 20);
    trace.assert_clean(from, "Home, End and the page keys in a long chat");
}

#[test]
fn alt_with_a_digit_also_delivers_its_text() {
    assert_eq!(
        key_press(egui::Key::Num2, egui::Modifiers::ALT)[1],
        egui::Event::Text("2".to_owned())
    );
    assert_eq!(
        key_press(egui::Key::ArrowUp, egui::Modifiers::NONE).len(),
        1
    );
}

/// Alt+2 in the message field moves to the message list and types nothing,
/// though Windows delivers its digit as text too.
#[test]
fn alt_2_in_the_message_field_types_nothing() {
    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::Composer);
    let draft = trace.app.composer.clone();
    trace.press("Alt+2", egui::Key::Num2, alt());
    let chat = trace.app.open_chat.clone().expect("the demo opens a chat");
    assert_eq!(
        trace.last().egui,
        Some(crate::ui::conversation::messages_list_id(&chat))
    );
    assert_eq!(trace.app.composer, draft, "nothing typed");
}

/// The demo app with a second, linked account that has a chat open.
fn app_with_two_accounts() -> App {
    let mut app = app();
    let id = crate::model::AccountId::parse("2").expect("account id");
    let (mut work, _) = crate::account::Account::detached(
        &app.dirs,
        id,
        crate::settings::AccountSettings::default(),
    )
    .expect("a second account");
    work.me = Some("15550002222@s.whatsapp.net".into());
    work.me_name = Some("Studio".into());
    work.link = crate::backend::LinkStatus::Connected;
    let chat = "15550003333@s.whatsapp.net";
    work.chats
        .push(crate::model::Chat::new(chat.into(), "Grace".into()));
    work.open_chat = Some(chat.into());
    app.accounts.push(work);
    app
}

/// Choosing another account in the switcher from the keys moves focus once,
/// straight to that account's message field: never back to "Switch account"
/// first, and never to the window.
#[test]
fn choosing_an_account_from_the_keys_moves_focus_once() {
    let mut trace = Trace::new(app_with_two_accounts());
    let switcher = crate::ui::rail::buttons(&trace.ctx)
        .last()
        .copied()
        .expect("the rail's picture");
    trace
        .ctx
        .data_mut(|data| data.insert_temp(crate::theme::keyboard_focus_id(), true));
    trace
        .ctx
        .memory_mut(|memory| memory.request_focus(switcher));
    trace.settle("focus", 3);
    assert_eq!(trace.last().egui, Some(switcher));
    trace.press("Enter", egui::Key::Enter, egui::Modifiers::NONE);
    let first = trace.last().egui;
    assert_ne!(first, Some(switcher), "focus went into the menu");
    trace.press("Down", egui::Key::ArrowDown, egui::Modifiers::NONE);
    assert_ne!(trace.last().egui, first, "on the second account");
    let from = trace.mark();
    trace.press("Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(trace.app.account().id.as_str(), "2");
    assert_eq!(trace.last().egui, Some(egui::Id::new("composer-text")));
    trace.assert_clean(from, "choosing an account");
    assert!(
        trace.frames[from..]
            .iter()
            .all(|frame| frame.egui != Some(switcher)),
        "focus never went back to the switcher:
{}",
        trace.dump(from - 1)
    );
}

/// Add account taken from the keys moves focus once, to the new account's
/// linking screen and its way back, Cancel: never to the window.
#[test]
fn adding_an_account_from_the_keys_lands_on_its_linking_screen() {
    let mut trace = Trace::new(app());
    let switcher = crate::ui::rail::buttons(&trace.ctx)
        .last()
        .copied()
        .expect("the rail's picture");
    trace
        .ctx
        .data_mut(|data| data.insert_temp(crate::theme::keyboard_focus_id(), true));
    trace
        .ctx
        .memory_mut(|memory| memory.request_focus(switcher));
    trace.settle("focus", 3);
    trace.press("Enter", egui::Key::Enter, egui::Modifiers::NONE);
    trace.press("Down", egui::Key::ArrowDown, egui::Modifiers::NONE);
    assert_eq!(trace.last().name, "MenuItem \"Add account\"");
    let from = trace.mark();
    trace.press("Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert!(trace.app.adding_account);
    assert!(
        trace.last().name.ends_with("\"Cancel\""),
        "{}",
        trace.last().name
    );
    trace.assert_clean(from, "adding an account");
    // Focus is on the way back, Cancel: Enter takes it.
    trace.press("Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert!(!trace.app.adding_account, "{}", trace.dump(from - 1));
}

/// An account the phone has not linked, chosen from the keys, moves focus
/// once, as Add account does: to its linking screen and the way back from
/// it, "Use another account", never to the window.
#[test]
fn choosing_an_unlinked_account_from_the_keys_lands_on_its_linking_screen() {
    let mut app = app();
    let id = crate::model::AccountId::parse("2").expect("account id");
    let (pending, _) = crate::account::Account::detached(
        &app.dirs,
        id,
        crate::settings::AccountSettings::default(),
    )
    .expect("a second account");
    app.accounts.push(pending);
    let mut trace = Trace::new(app);
    let switcher = crate::ui::rail::buttons(&trace.ctx)
        .last()
        .copied()
        .expect("the rail's picture");
    trace
        .ctx
        .data_mut(|data| data.insert_temp(crate::theme::keyboard_focus_id(), true));
    trace
        .ctx
        .memory_mut(|memory| memory.request_focus(switcher));
    trace.settle("focus", 3);
    trace.press("Enter", egui::Key::Enter, egui::Modifiers::NONE);
    trace.press("Down", egui::Key::ArrowDown, egui::Modifiers::NONE);
    assert!(
        trace.last().name.contains("Not linked"),
        "{}",
        trace.last().name
    );
    let from = trace.mark();
    trace.press("Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(trace.app.account().id.as_str(), "2");
    assert!(!trace.app.adding_account);
    assert!(
        trace.last().name.ends_with("\"Use another account\""),
        "{}",
        trace.dump(from - 1)
    );
    trace.assert_clean(from, "choosing an unlinked account");
    // Focus is on the way back, "Use another account": Enter opens the
    // switcher with focus in it.
    trace.press("Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert!(
        trace.last().name.starts_with("MenuItemRadio"),
        "{}",
        trace.dump(from - 1)
    );
}

/// The linking screen's "Use another account", taken from the keys, opens
/// the switcher with focus in it, as Enter on the switcher does.
#[test]
fn the_switcher_asked_for_from_the_keys_takes_focus() {
    let mut trace = Trace::new(app_with_two_accounts());
    trace
        .ctx
        .data_mut(|data| data.insert_temp(crate::theme::keyboard_focus_id(), true));
    let from = trace.mark();
    trace.app.account_menu = true;
    trace.settle("Use another account", 4);
    let popup = crate::ui::rail::buttons(&trace.ctx)
        .last()
        .copied()
        .expect("the rail's picture")
        .with("popup");
    assert!(egui::Popup::is_id_open(&trace.ctx, popup));
    assert!(
        trace.last().name.starts_with("MenuItemRadio"),
        "{}",
        trace.last().name
    );
    trace.assert_clean(from, "the switcher asked for");
}

/// The day filter opened from the keys for the first time puts focus on
/// today at once: its first pass, drawn hidden to measure it, never leaves
/// the window focused.
#[test]
fn the_day_filter_opens_from_the_keys_without_a_window_frame() {
    let mut trace = Trace::new(app());
    trace.press("Ctrl+F", egui::Key::F, egui::Modifiers::COMMAND);
    focus_stop(&mut trace, Stop::ChatSearchDate);
    let button = trace.last().egui;
    let from = trace.mark();
    trace.press("Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert!(trace.app.chat_search_calendar, "Enter opens the filter");
    assert_ne!(trace.last().egui, button, "focus is on a day");
    trace.assert_clean(from, "the day filter opened");
}

/// Tab walks the open chat as it is laid out, top to bottom: from the chat
/// list to the chat's actions, its messages, the message field and Message
/// actions, the window's last stop, then wraps to its first, the rail.
/// Shift+Tab walks back. Each key is heard once.
#[test]
fn tab_reaches_the_composer_last() {
    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::ChatList);
    let from = trace.mark();
    for (modifiers, stops) in [
        (
            egui::Modifiers::NONE,
            [
                Stop::ChatActions,
                Stop::Messages,
                Stop::Composer,
                Stop::MessageActions,
                Stop::Rail,
            ],
        ),
        (
            egui::Modifiers::SHIFT,
            [
                Stop::MessageActions,
                Stop::Composer,
                Stop::Messages,
                Stop::ChatActions,
                Stop::ChatList,
            ],
        ),
    ] {
        for stop in stops {
            trace.press("Tab", egui::Key::Tab, modifiers);
            assert_eq!(
                trace.last().egui,
                control(&trace.ctx, stop),
                "{modifiers:?} to {stop:?}\n{}",
                trace.dump(from)
            );
        }
    }
    trace.assert_clean(from, "Tab through the open chat");
}

/// With the pointer, no keyboard navigation and no screen reader, Escape
/// lets go of focus as egui always did: a clicked field loses its caret.
#[test]
fn escape_with_the_pointer_lets_go_as_before() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.attach(&ctx);
    app.actions.push(Action::CloseChat);
    render(&mut app, &ctx);
    let search = control(&ctx, Stop::Search).expect("drawn");
    ctx.memory_mut(|memory| memory.request_focus(search));
    render(&mut app, &ctx);
    assert_eq!(ctx.memory(|memory| memory.focused()), Some(search));
    press_key(&mut app, &ctx, egui::Key::Escape, egui::Modifiers::NONE);
    assert_eq!(ctx.memory(|memory| memory.focused()), None);
}

/// Where focus can be with no chat open: Escape and Alt+2 are tried there.
const NO_CHAT_STOPS: &[Stop] = &[
    Stop::Search,
    Stop::ChatList,
    Stop::ChatListActions,
    Stop::Filters,
    Stop::Rail,
];

/// Escape with nothing to close is not heard at all.
#[test]
fn escape_with_nothing_open_changes_nothing() {
    for &stop in NO_CHAT_STOPS {
        let mut trace = no_chat_open(stop);
        let from = trace.mark();
        trace.press("Escape", egui::Key::Escape, egui::Modifiers::NONE);
        trace.assert_unchanged(from, &format!("Escape with nothing open on {stop:?}"));
    }
}

/// Alt+2 with no chat open says so, and focus stays put without a sound.
#[test]
fn alt_2_without_a_chat_only_announces() {
    for &stop in NO_CHAT_STOPS {
        let mut trace = no_chat_open(stop);
        let from = trace.mark();
        trace.press("Alt+2", egui::Key::Num2, alt());
        trace.assert_unchanged(from, &format!("Alt+2 without a chat on {stop:?}"));
        assert!(
            trace.app.announcer.spoken().contains(&"No chat open"),
            "{:?}",
            trace.app.announcer.spoken()
        );
    }
}

/// The current message deleted (Delete for me, confirmed), the list's
/// cursor goes to the message after it, not to the newest.
#[test]
fn a_deleted_current_message_leaves_the_cursor_on_the_next() {
    let mut trace = Trace::new(app());
    let (backend, _commands, events) = crate::backend::Backend::recording_with_events();
    trace.app.backend = backend;
    focus_stop(&mut trace, Stop::Messages);
    trace.press("Up", egui::Key::ArrowUp, egui::Modifiers::NONE);
    trace.press("Up", egui::Key::ArrowUp, egui::Modifiers::NONE);
    let chat = trace.app.open_chat.clone().expect("the demo opens a chat");
    let messages = &trace.app.conversations[&chat].messages;
    let current = trace.app.message_cursor[&chat].clone();
    let at = messages
        .iter()
        .position(|message| message.id == current)
        .expect("the current message");
    assert!(at + 2 < messages.len(), "not the newest two");
    let next = messages[at + 1].id.clone();
    events
        .send(crate::backend::Event::MessageDeleted {
            chat: chat.clone(),
            id: current,
        })
        .expect("the app listens");
    trace.settle("deleted", 4);
    assert_eq!(trace.app.message_cursor.get(&chat), Some(&next));
}

/// Where focus can be when the window goes away, one stop or more in each
/// section.
const REFOCUS_STOPS: &[Stop] = &[
    Stop::Search,
    Stop::Messages,
    Stop::Composer,
    Stop::ChatList,
    Stop::Filters,
    Stop::ChatActions,
];

/// Leaving the window and coming back puts the reader where they were, in
/// any section.
#[test]
fn coming_back_to_the_window_restores_focus() {
    for &stop in REFOCUS_STOPS {
        let mut trace = Trace::new(app());
        focus_stop(&mut trace, stop);
        let before = trace.last().egui;
        let from = trace.mark();
        trace.window("window lost", false);
        trace.window("window back", true);
        let scenario = format!("window refocus on {stop:?}");
        trace.assert_unchanged(from, &scenario);
        assert_eq!(trace.last().egui, before, "{scenario}");
    }
}

/// With nothing focused, coming back to the window lands in the message
/// field of the open chat.
#[test]
fn coming_back_to_the_window_with_nothing_focused_lands_in_the_field() {
    let mut trace = Trace::new(app());
    trace.ctx.memory_mut(|memory| {
        if let Some(id) = memory.focused() {
            memory.surrender_focus(id);
        }
    });
    trace.settle("nothing focused", 3);
    assert_eq!(trace.last().egui, None);
    trace.window("window lost", false);
    trace.window("window back", true);
    assert_eq!(trace.last().egui, Some(egui::Id::new("composer-text")));
}

/// Unlocking counts as coming back to the window: the lock screen's field
/// is gone, and the message field takes focus.
#[test]
fn unlocking_lands_in_the_message_field() {
    let mut app = app();
    app.settings.app_lock_hash = Some(crate::app_lock::verifier("fixture"));
    let mut trace = Trace::new(app);
    let from = trace.mark();
    trace.action += 1;
    trace.app.lock_app();
    trace.settle("locked", 3);
    assert_eq!(
        trace.last().egui,
        Some(egui::Id::new(crate::ui::lock::PASSWORD_ID))
    );
    trace.action += 1;
    trace.app.app_lock.unlocked(true);
    trace.settle("unlocked", 3);
    assert_eq!(trace.last().egui, Some(egui::Id::new("composer-text")));
    trace.assert_clean(from, "locking and unlocking");
}

/// A notification for the open chat, clicked while the window is away,
/// leaves a reader in its message list there.
#[test]
fn a_notification_clicked_while_away_keeps_the_message_list() {
    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::Messages);
    let before = trace.last().egui;
    let chat = trace.app.open_chat.clone().expect("the demo opens a chat");
    let message = trace.app.conversations[&chat].messages[1].id.clone();
    trace.window("window lost", false);
    trace
        .app
        .actions
        .push(Action::OpenMessage { chat, message });
    trace.window("window back", true);
    assert_eq!(trace.last().egui, before);
}

/// A key pressed in one frame and released in the next, then a few quiet
/// frames. Returns the press frame's index.
fn tap(trace: &mut Trace, step: &str, key: egui::Key, modifiers: egui::Modifiers) -> usize {
    trace.action += 1;
    let press = trace.mark();
    trace.frame(step, key_press(key, modifiers));
    trace.frame(
        step,
        vec![egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: false,
            repeat: false,
            modifiers,
        }],
    );
    trace.settle(step, 4);
    press
}

/// The demo's open chat with its newest message a photo whose file is
/// there, which Space on it opens in the preview.
fn photo_chat() -> App {
    let mut app = app();
    let (photo, _) = super::sample_files(&app);
    let chat = app.open_chat.clone().expect("the demo opens a chat");
    let conversation = app.conversations.get_mut(&chat).expect("its messages");
    let last = conversation.messages.last_mut().expect("a message");
    let mut media = super::media("image/jpeg", 120_000, Some(800), Some(600));
    media.path = Some(photo);
    last.content = crate::model::Content::Image {
        motion: None,
        caption: None,
        media,
    };
    app
}

/// Escape from an overlay opened from the message list: it closes, and
/// the reader is back on the message it was opened from, heard once.
fn escape_back_to_the_list(trace: &mut Trace, row: &Frame, scenario: &str) {
    let chat = trace.app.open_chat.clone().expect("the demo opens a chat");
    let from = tap(trace, "Escape", egui::Key::Escape, egui::Modifiers::NONE);
    trace.assert_clean(from, &format!("{scenario}, closed"));
    assert_eq!(
        trace.last().egui,
        Some(crate::ui::conversation::messages_list_id(&chat)),
        "{scenario}: back in the message list\n{}",
        trace.dump(from)
    );
    assert_eq!(
        trace.last().effective,
        row.effective,
        "{scenario}: on the message it was opened from"
    );
}

/// Dialogs opened from the message list's keys take focus in the first
/// pass that shows them, never by way of the window, and Escape puts the
/// reader back on the message: Delete's confirmation, Forward, New chat and
/// the shortcuts.
#[test]
fn dialogs_opened_from_the_message_list_give_focus_back() {
    for (step, key, modifiers) in [
        ("Delete", egui::Key::Delete, egui::Modifiers::NONE),
        ("F", egui::Key::F, egui::Modifiers::NONE),
        ("Ctrl+N", egui::Key::N, egui::Modifiers::COMMAND),
        ("Ctrl+/", egui::Key::Slash, egui::Modifiers::COMMAND),
    ] {
        let mut trace = Trace::new(app());
        focus_stop(&mut trace, Stop::Messages);
        let row = trace.last().clone();
        let press = tap(&mut trace, step, key, modifiers);
        let scenario = format!("{step} from the message list");
        assert!(trace.app.dialog.is_some(), "{scenario} opens a dialog");
        trace.assert_clean(press, &scenario);
        assert_eq!(
            trace.frames[press + 1].layer,
            Some(crate::ui::dialogs::modal_id()),
            "{scenario}: focus is in the dialog\n{}",
            trace.dump(press)
        );
        escape_back_to_the_list(&mut trace, &row, &scenario);
        assert!(trace.app.dialog.is_none(), "{scenario}");
    }
}

/// A shared sticker pack's dialog, a video across the window (in the media
/// viewer) and the sticker view, opened while the reader is in the message
/// list, take focus and give it back as well.
#[test]
fn the_sticker_pack_and_the_expanded_video_give_focus_back() {
    for overlay in ["sticker pack", "video", "sticker view"] {
        let mut app = app();
        if overlay == "video" {
            super::video_sample(&mut app, None);
            // The clip plays as the viewer opens: never through the speakers.
            app.video.silence();
        } else if overlay == "sticker view" {
            super::sample_files(&app);
        } else {
            super::shared_pack_sample(&mut app, false);
        }
        let mut trace = Trace::new(app);
        focus_stop(&mut trace, Stop::Messages);
        let row = trace.last().clone();
        let from = trace.mark();
        trace.action += 1;
        if overlay == "video" {
            let path = trace.app.dirs.media_cache_dir().join("demo-video.mp4");
            trace.app.actions.push(Action::ExpandVideo {
                message: "demo-video".into(),
                path,
            });
        } else if overlay == "sticker view" {
            let (_, sticker) = super::sample_files(&trace.app);
            trace.app.actions.push(Action::PreviewSticker(sticker));
        } else {
            super::sticker_sample(&mut trace.app, crate::model::StickerShelf::Recent, "");
            trace.app.picker = None;
            let ducks = trace.app.sticker_packs.first().cloned();
            trace.app.sticker_preview = ducks.map(|pack| (pack, "Ada Lovelace".to_owned()));
            trace
                .app
                .actions
                .push(Action::ShowDialog(crate::model::Dialog::StickerPack));
        }
        trace.settle(overlay, 4);
        trace.assert_clean(from, overlay);
        let layer = match overlay {
            "video" => crate::ui::image_preview::modal_id(),
            "sticker view" => crate::ui::image_preview::sticker_modal_id(),
            _ => crate::ui::dialogs::modal_id(),
        };
        // Opened at the end of the first frame, entered in the next.
        assert_eq!(
            trace.frames[from + 1].layer,
            Some(layer),
            "{overlay}: focus is in it\n{}",
            trace.dump(from)
        );
        escape_back_to_the_list(&mut trace, &row, overlay);
    }
}

/// With the pointer, no keyboard navigation and no screen reader, the
/// preview opens and closes as it always did: focus let go as it opens,
/// and the message field's once it closes.
#[test]
fn the_image_preview_with_the_pointer_is_as_before() {
    let mut app = photo_chat();
    let ctx = egui::Context::default();
    app.attach(&ctx);
    render(&mut app, &ctx);
    let chat = app.open_chat.clone().expect("the demo opens a chat");
    let list = crate::ui::conversation::messages_list_id(&chat);
    ctx.memory_mut(|memory| memory.request_focus(list));
    render(&mut app, &ctx);
    assert!(!crate::ui::keys::keyboard_or_reader(&ctx));
    let (photo, _) = super::sample_files(&app);
    let message = app.conversations[&chat]
        .messages
        .last()
        .expect("the photo")
        .id
        .clone();
    app.actions.push(Action::PreviewImage {
        path: photo,
        chat,
        message,
        card: None,
    });
    render(&mut app, &ctx);
    render(&mut app, &ctx);
    assert!(app.image_preview.is_some());
    assert_eq!(ctx.memory(|memory| memory.focused()), None, "let go");
    app.actions.push(Action::CloseImagePreview);
    render(&mut app, &ctx);
    render(&mut app, &ctx);
    assert_eq!(
        ctx.memory(|memory| memory.focused()),
        Some(egui::Id::new("composer-text"))
    );
}

/// A dialog the pointer opened leaves focus alone; a viewer the keys then
/// open over it takes focus, as one the keys open over the chat does.
#[test]
fn the_keys_viewer_over_a_pointers_dialog_takes_focus() {
    let mut app = photo_chat();
    let ctx = egui::Context::default();
    app.attach(&ctx);
    render(&mut app, &ctx);
    assert!(!crate::ui::keys::keyboard_or_reader(&ctx));
    app.actions
        .push(Action::ShowDialog(crate::model::Dialog::Shortcuts));
    render(&mut app, &ctx);
    render(&mut app, &ctx);
    assert!(app.dialog.is_some());
    assert_eq!(ctx.memory(|memory| memory.focused()), None, "left alone");
    ctx.data_mut(|data| data.insert_temp(crate::theme::keyboard_focus_id(), true));
    let chat = app.open_chat.clone().expect("the demo opens a chat");
    let (photo, _) = super::sample_files(&app);
    let message = app.conversations[&chat]
        .messages
        .last()
        .expect("the photo")
        .id
        .clone();
    app.actions.push(Action::PreviewImage {
        path: photo,
        chat,
        message,
        card: None,
    });
    for _ in 0..3 {
        render(&mut app, &ctx);
    }
    assert!(app.image_preview.is_some());
    let layer = ctx
        .memory(|memory| memory.focused())
        .and_then(|id| ctx.read_response(id))
        .map(|response| response.layer_id.id);
    assert_eq!(layer, Some(crate::ui::image_preview::modal_id()));
}

/// Arrows Down in an open menu until the reader is on the item `label`.
fn walk_to(trace: &mut Trace, label: &str) {
    for _ in 0..20 {
        if trace.last().name.contains(label) {
            return;
        }
        trace.press("Down", egui::Key::ArrowDown, egui::Modifiers::NONE);
    }
    panic!("no menu item {label}\n{}", trace.dump(trace.mark() - 1));
}

/// A message menu item that opens a dialog moves the reader once, from
/// the item into the dialog, not by way of the message list; Escape puts
/// them back on the message.
#[test]
fn message_menu_items_open_dialogs_with_one_focus_change() {
    for item in ["Forward", "Delete for me", "Message info"] {
        let mut trace = Trace::new(app());
        focus_stop(&mut trace, Stop::Messages);
        let row = trace.last().clone();
        trace.press("Shift+F10", egui::Key::F10, egui::Modifiers::SHIFT);
        walk_to(&mut trace, item);
        let press = tap(&mut trace, "Enter", egui::Key::Enter, egui::Modifiers::NONE);
        let scenario = format!("the message menu's {item}");
        assert!(trace.app.dialog.is_some(), "{scenario} opens a dialog");
        trace.assert_clean(press, &scenario);
        assert_eq!(
            trace.frames[press + 1].layer,
            Some(crate::ui::dialogs::modal_id()),
            "{scenario}\n{}",
            trace.dump(press)
        );
        escape_back_to_the_list(&mut trace, &row, &scenario);
    }
}

/// A primary click at `pos`, pressed in one frame and released in the next,
/// then a few quiet frames.
fn pointer_click(trace: &mut Trace, step: &str, pos: egui::Pos2) {
    let button = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    trace.action += 1;
    trace.frame(step, vec![egui::Event::PointerMoved(pos)]);
    trace.frame(step, vec![button(true), button(false)]);
    trace.settle(step, 4);
}

/// Where the first control on the layer `layer` was drawn last.
fn first_control(trace: &Trace, layer: egui::Id) -> egui::Pos2 {
    trace
        .ctx
        .viewport(|viewport| {
            viewport
                .prev_pass
                .widgets
                .layers()
                .filter(|(layer_id, _)| layer_id.id == layer)
                .flat_map(|(_, widgets)| widgets.iter())
                .find(|widget| widget.enabled && widget.visible && widget.sense.is_focusable())
                .map(|widget| widget.rect.center())
        })
        .expect("a control on the layer")
}

/// Chat info, chosen in the chat's toolbar, opens the info panel beside the
/// chat, whose controls come next in Tab order, and focus stays on the
/// button; Escape closes the panel, as upstream's own Escape does.
#[test]
fn chat_info_opens_the_panel_and_keeps_focus_on_its_button() {
    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::ChatActions);
    let button = trace.last().clone();
    assert!(button.name.contains("Chat info"), "{}", button.name);
    let press = tap(&mut trace, "Enter", egui::Key::Enter, egui::Modifiers::NONE);
    let chat = trace.app.open_chat.clone();
    assert_eq!(
        trace.app.info.as_ref().map(|info| info.chat.clone()),
        chat,
        "the chat's info panel"
    );
    trace.assert_clean(press, "chat info opened");
    assert_eq!(trace.last().egui, button.egui, "still on Chat info");
    let from = tap(
        &mut trace,
        "Escape",
        egui::Key::Escape,
        egui::Modifiers::NONE,
    );
    assert!(trace.app.info.is_none());
    trace.assert_clean(from, "chat info closed");
}

/// With a screen reader, or any other UIA client, attached, a mouse user
/// opening and closing the preview and a dialog with clicks gets upstream's
/// behaviour: no focus pushed into the overlay, and the caret back in the
/// message field once it closes.
#[test]
fn mouse_clicks_keep_upstream_overlay_focus_with_accesskit_on() {
    let mut trace = Trace::new(photo_chat());
    focus_stop(&mut trace, Stop::Composer);
    // The photo takes clicks once it is decoded and drawn.
    for _ in 0..300 {
        if !trace.ctx.has_pending_images() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        trace.settle("loading", 1);
    }
    trace.settle("loaded", 2);
    let chat = trace.app.open_chat.clone().expect("the demo opens a chat");
    let newest = trace.app.conversations[&chat]
        .messages
        .last()
        .expect("a message")
        .id
        .clone();
    let picture = trace
        .ctx
        .data(|data| {
            data.get_temp::<egui::Rect>(
                crate::ui::conversation::bubble_id(&chat, &newest).with("picture"),
            )
        })
        .expect("the photo is drawn");
    pointer_click(&mut trace, "click the photo", picture.center());
    assert!(trace.app.image_preview.is_some(), "the click opens it");
    assert_eq!(trace.last().egui, None, "focus let go, as upstream");
    let close = first_control(&trace, crate::ui::image_preview::modal_id());
    pointer_click(&mut trace, "click Close", close);
    assert!(trace.app.image_preview.is_none(), "the click closes it");
    assert_eq!(
        trace.last().egui,
        Some(crate::ui::conversation::composer_id()),
        "the caret is back in the message field"
    );

    let info = control(&trace.ctx, Stop::ChatActions)
        .and_then(|id| trace.ctx.read_response(id))
        .expect("Chat info is drawn")
        .rect
        .center();
    pointer_click(&mut trace, "click Chat info", info);
    assert!(trace.app.info.is_some(), "the click opens the info panel");
    let close = control(&trace.ctx, Stop::InfoClose)
        .and_then(|id| trace.ctx.read_response(id))
        .expect("the panel's Close is drawn")
        .rect
        .center();
    pointer_click(&mut trace, "click Close", close);
    assert!(trace.app.info.is_none(), "the click closes it");
    assert_eq!(
        trace.last().egui,
        Some(crate::ui::conversation::composer_id()),
        "the caret is back in the message field"
    );
}

/// A dialog open when ZapFast locks is there again after the unlock, as
/// upstream keeps it, and takes focus again: the field behind it cannot,
/// and the lock screen's field is gone.
#[test]
fn a_dialog_open_at_the_lock_takes_focus_again_after_the_unlock() {
    let mut app = app();
    app.settings.app_lock_hash = Some(crate::app_lock::verifier("fixture"));
    let mut trace = Trace::new(app);
    focus_stop(&mut trace, Stop::Messages);
    tap(
        &mut trace,
        "Ctrl+/",
        egui::Key::Slash,
        egui::Modifiers::COMMAND,
    );
    assert!(trace.app.dialog.is_some());
    trace.action += 1;
    trace.app.lock_app();
    trace.settle("locked", 3);
    let from = trace.mark();
    trace.action += 1;
    trace.app.app_lock.unlocked(true);
    trace.settle("unlocked", 3);
    assert!(trace.app.dialog.is_some(), "the dialog stays, as upstream");
    assert_eq!(
        trace.last().layer,
        Some(crate::ui::dialogs::modal_id()),
        "{}",
        trace.dump(from - 1)
    );
    trace.assert_clean(from, "unlocked");
}

/// The cropper over a staged photo, as the keys or a screen reader open it:
/// the demo's "crop" page, set up after the message list has focus.
fn open_cropper(trace: &mut Trace, target: crate::app::AttachmentId, photo: &std::path::Path) {
    let (width, height) = image::image_dimensions(photo).unwrap_or((4, 3));
    trace.app.picture_edit = Some(crate::model::PictureEdit::new(
        target,
        crate::model::PictureSource::File(photo.to_owned()),
        width,
        height,
    ));
    trace.action += 1;
    trace.settle("open the cropper", 4);
}

/// The cropper, opened from the keys, takes focus on "Keep this crop" and
/// keeps Tab among its own controls, never the chat under it. Enter presses
/// the focused button: Cancel leaves the picture as it was and gives focus
/// back to the message list, Turn left turns it, and Keep keeps the crop.
/// Before, the cropper took no focus and Enter kept the crop whatever was
/// focused.
#[test]
fn the_cropper_takes_focus_and_enter_presses_the_focused_button() {
    let mut app = app();
    let (photo, _) = super::sample_files(&app);
    let target = app.stage(crate::app::Pending::File(photo.clone()));
    let mut trace = Trace::new(app);
    focus_stop(&mut trace, Stop::Messages);
    let row = trace.last().clone();
    let editor = crate::ui::image_editor::area_id();
    let from = trace.mark();
    open_cropper(&mut trace, target, &photo);
    trace.assert_clean(from, "the cropper opened");
    assert_eq!(
        trace.last().name,
        "Button \"Keep this crop\"",
        "{}",
        trace.dump(from)
    );
    for _ in 0..7 {
        trace.press("Tab", egui::Key::Tab, egui::Modifiers::NONE);
        assert_eq!(
            trace.last().layer,
            Some(editor),
            "Tab stays in the cropper\n{}",
            trace.dump(from)
        );
        // The drag areas take the pointer only: no nameless stop.
        assert!(
            !trace.last().name.ends_with("\"\""),
            "a named control\n{}",
            trace.dump(from)
        );
    }
    // Enter on Cancel cancels: nothing is kept, and the reader is back on
    // the message list.
    for _ in 0..8 {
        if trace.last().name.contains("Cancel") {
            break;
        }
        trace.press("Tab", egui::Key::Tab, egui::Modifiers::NONE);
    }
    let from = tap(&mut trace, "Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert!(
        trace.app.picture_edit.is_none(),
        "Cancel closed the cropper"
    );
    assert!(
        trace.app.picture_applying.is_empty(),
        "Enter on Cancel keeps nothing"
    );
    trace.assert_clean(from, "the cropper cancelled");
    assert_eq!(trace.last().effective, row.effective, "back on the list");
    // Enter on Turn left turns the picture and stays.
    open_cropper(&mut trace, target, &photo);
    for _ in 0..8 {
        if trace.last().name.contains("Turn left") {
            break;
        }
        trace.press("Tab", egui::Key::Tab, egui::Modifiers::NONE);
    }
    tap(&mut trace, "Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(
        trace.app.picture_edit.as_ref().map(|edit| edit.turns),
        Some(3),
        "turned a quarter to the left, still open"
    );
    // Enter on Keep, where focus enters, keeps the crop.
    trace.app.picture_edit = None;
    trace.settle("closed", 2);
    open_cropper(&mut trace, target, &photo);
    tap(&mut trace, "Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert!(trace.app.picture_edit.is_none());
    assert!(
        trace.app.picture_applying.contains_key(&target),
        "Enter on Keep keeps the crop"
    );
}

/// A file's cropper opens frames after it was asked for, once the
/// picture's size is read. Asked for with the pointer, it leaves focus
/// where it was even with a screen reader attached; asked for from the
/// keys, it takes it on "Keep this crop".
#[test]
fn the_cropper_follows_focus_as_it_was_asked_for() {
    for pointer in [true, false] {
        let mut app = app();
        let (photo, _) = super::sample_files(&app);
        let target = app.stage(crate::app::Pending::File(photo.clone()));
        let mut trace = Trace::new(app);
        focus_stop(&mut trace, Stop::Messages);
        trace.app.actions.push(Action::EditPicture(target));
        trace.action += 1;
        if pointer {
            // The click is applied before this frame's UI, which would note
            // the pointer: noted already, as the frame of a real click does
            // before its UI asks for the cropper.
            trace
                .ctx
                .data_mut(|data| data.insert_temp(crate::theme::keyboard_focus_id(), false));
            let at = egui::pos2(600.0, 400.0);
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            trace.frame("ask with the pointer", vec![button(true), button(false)]);
        } else {
            trace.frame("ask from the keys", Vec::new());
        }
        assert!(
            trace.app.picture_edit.is_none(),
            "a file waits for its size"
        );
        let before = trace.last().egui;
        // The size arrives: the cropper opens, as `Event::PictureInspected`
        // opens it.
        open_cropper(&mut trace, target, &photo);
        if pointer {
            assert_eq!(trace.last().egui, before, "focus left alone");
        } else {
            assert_eq!(
                trace.last().name,
                "Button \"Keep this crop\"",
                "{}",
                trace.dump(1)
            );
        }
    }
}

/// The pickers take focus in the first pass that shows them, never by way
/// of the window, and keep their own way back: the reaction picker to its
/// message, the emoji picker to the message field its emoji go to.
#[test]
fn the_pickers_open_without_a_window_frame() {
    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::Messages);
    let row = trace.last().clone();
    let press = tap(&mut trace, "E", egui::Key::E, egui::Modifiers::NONE);
    assert!(trace.app.reaction_target.is_some());
    trace.assert_clean(press, "the reaction picker");
    assert_eq!(trace.last().name, "TextInput \"Search emoji\"");
    escape_back_to_the_list(&mut trace, &row, "the reaction picker");

    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::MessageActions);
    trace.press("Right", egui::Key::ArrowRight, egui::Modifiers::NONE);
    assert!(trace.last().name.contains("Emoji"), "{}", trace.last().name);
    let press = tap(&mut trace, "Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert!(trace.app.picker.is_some());
    trace.assert_clean(press, "the emoji picker");
    assert_eq!(trace.last().name, "TextInput \"Search emoji\"");
    let from = tap(
        &mut trace,
        "Escape",
        egui::Key::Escape,
        egui::Modifiers::NONE,
    );
    trace.assert_clean(from, "the emoji picker, closed");
    assert_eq!(trace.last().egui, Some(egui::Id::new("composer-text")));
}

/// A long chat whose newest message replies to message `target`.
fn replying_to(target: usize) -> App {
    let mut app = app();
    long_chat(&mut app, 400);
    let chat = app.open_chat.clone().expect("the demo opens a chat");
    let conversation = app.conversations.get_mut(&chat).expect("its messages");
    let quoted = conversation.messages[target].clone();
    conversation.messages[399].quoted = Some(crate::model::Quoted {
        chat: None,
        id: quoted.id.clone(),
        sender: quoted.sender.clone(),
        sender_name: None,
        summary: quoted.summary(),
        mentions: Vec::new(),
    });
    app
}

/// Fixes the clock at `now`, in UTC on a 24-hour clock, until the guard
/// drops. The demo dates its messages from now, so a message gains its day
/// in its name ("Today, ...") when a local midnight falls just before it.
fn clock_at(now: i64) -> crate::util::fixed_clock::Guard {
    crate::util::fixed_clock::set(crate::util::fixed_clock::Clock {
        now,
        zone: jiff::tz::TimeZone::UTC,
        twelve_hour: false,
    })
}

/// G on a reply lands on the replied message in the frame after the key,
/// which egui runs at once after a frame with input. A replied message far
/// up a long chat costs that frame no more than a near one: the rows laid
/// out only to place it are not handed to the screen reader. The clock is
/// fixed, and message 50 is replied to both inside a day and as the day's
/// first message, which the reader hears with its day.
#[test]
fn g_lands_on_the_replied_message_in_the_next_frame() {
    // 14:00 UTC on Wednesday 11 March 2026: the 400 messages, a minute
    // apart, all fall on that day.
    const AFTERNOON: i64 = 1_773_237_600;
    // A later time that places message 50 just after midnight.
    let day_start = {
        let _clock = clock_at(AFTERNOON);
        let app = replying_to(50);
        let chat = app.open_chat.as_ref().expect("the demo opens a chat");
        let sent = app.conversations[chat].messages[50].timestamp;
        let (_, midnight) = crate::util::day_key(sent)
            .and_then(crate::util::day_bounds)
            .expect("the message has a day");
        AFTERNOON + midnight + 30 - sent
    };
    for (target, now, day) in [
        (398, AFTERNOON, ""),
        (50, AFTERNOON, ""),
        (50, day_start, "Today, "),
    ] {
        let _clock = clock_at(now);
        let mut trace = Trace::new(replying_to(target));
        focus_stop(&mut trace, Stop::Messages);
        let before = trace.last().clone();
        let chat = trace.app.open_chat.clone().expect("the demo opens a chat");
        let heights = |trace: &Trace| -> Vec<f32> {
            let rows = &trace.app.conversations[&chat].rows;
            (target.saturating_sub(10)..target)
                .filter_map(|index| rows.get(&format!("long-{index}")).map(|row| row.height))
                .collect()
        };
        trace.action += 1;
        let press = trace.mark();
        trace.frame("G", key_press(egui::Key::G, egui::Modifiers::NONE));
        trace.frame(
            "G",
            vec![egui::Event::Key {
                key: egui::Key::G,
                physical_key: Some(egui::Key::G),
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let laid_out_hidden = heights(&trace);
        trace.settle("G", 4);
        let scenario = format!("G to message {target} at {now}");
        trace.assert_clean(press, &scenario);
        let landed = &trace.frames[press + 1];
        assert!(
            landed
                .spoken
                .contains(&format!("Some(\"{day}You, Fixture line {target},")),
            "{scenario}: the reader is on the replied message in the next frame, \
             not on {}\n{}",
            landed.spoken,
            trace.dump(press)
        );
        assert!(
            landed.nodes < before.nodes * 2,
            "{scenario}: the landing frame carries {} nodes, against {} before",
            landed.nodes,
            before.nodes
        );
        assert_eq!(
            laid_out_hidden,
            heights(&trace),
            "{scenario}: rows laid out hidden measure as shown"
        );
        if target == 50 {
            // Far from either end: centred in the view, as the jump asks.
            let bubble = trace
                .ctx
                .data(|data| {
                    data.get_temp::<egui::Rect>(
                        crate::ui::conversation::bubble_id(&chat, "long-50").with("rect"),
                    )
                })
                .expect("the replied message is drawn");
            let view = trace
                .app
                .selection_view
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .expect("the message view is drawn");
            assert!(
                (bubble.center().y - view.center().y).abs() <= 1.0,
                "{scenario}: {bubble:?} centred in {view:?}"
            );
        }
    }
}

/// Space on a photo in the message list opens the preview with the reader
/// on its Close button, in the first pass that shows it and never by way of
/// the window; Escape closes it, and the reader is back on the photo's
/// message, not in the message field.
#[test]
fn the_image_preview_takes_focus_and_gives_it_back() {
    let mut trace = Trace::new(photo_chat());
    focus_stop(&mut trace, Stop::Messages);
    let row = trace.last().clone();
    assert!(row.spoken.contains("Photo"), "{}", row.spoken);
    let press = tap(&mut trace, "Space", egui::Key::Space, egui::Modifiers::NONE);
    assert!(trace.app.image_preview.is_some());
    trace.assert_clean(press, "the image preview opened");
    assert_eq!(
        trace.frames[press + 1].name,
        "Button \"Close preview (Esc)\"",
        "on Close in the frame after the key\n{}",
        trace.dump(press)
    );
    escape_back_to_the_list(&mut trace, &row, "the image preview");
    assert!(trace.app.image_preview.is_none());
}

/// A screen reader's own activation of the preview's Close, an AccessKit
/// action rather than a pointer click, closes it and puts the reader back
/// on the message it was opened from, with no keyboard navigation noted.
#[test]
fn a_screen_readers_activation_of_close_gives_focus_back() {
    let mut trace = Trace::new(photo_chat());
    focus_stop(&mut trace, Stop::Messages);
    let row = trace.last().clone();
    tap(&mut trace, "Space", egui::Key::Space, egui::Modifiers::NONE);
    let close = trace.last().clone();
    assert_eq!(close.name, "Button \"Close preview (Esc)\"");
    trace
        .ctx
        .data_mut(|data| data.insert_temp(crate::theme::keyboard_focus_id(), false));
    let from = trace.mark();
    trace.action += 1;
    trace.frame(
        "activate Close",
        vec![egui::Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                action: egui::accesskit::Action::Click,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: close.effective,
                data: None,
            },
        )],
    );
    trace.settle("activate Close", 4);
    assert!(trace.app.image_preview.is_none(), "the action closes it");
    trace.assert_clean(from, "Close activated by the screen reader");
    assert_eq!(trace.last().effective, row.effective, "back on the message");
}

/// The media viewer shows the open chat's media and closes as the reader
/// moves to another chat, with no key: focus goes to that chat's message
/// field rather than back to the old chat's list.
#[test]
fn the_viewer_closed_by_a_chat_switch_leaves_focus_to_the_new_chat() {
    let mut trace = Trace::new(photo_chat());
    focus_stop(&mut trace, Stop::Messages);
    let old = trace.app.open_chat.clone().expect("the demo opens a chat");
    tap(&mut trace, "Space", egui::Key::Space, egui::Modifiers::NONE);
    assert!(trace.app.image_preview.is_some(), "Space opens the viewer");
    trace.action += 1;
    trace
        .app
        .actions
        .push(Action::OpenChat(super::SAMPLES[1].id.into()));
    trace.settle("another chat", 3);
    assert_eq!(trace.app.open_chat.as_deref(), Some(super::SAMPLES[1].id));
    assert!(
        trace.app.image_preview.is_none(),
        "the chat switch closes the viewer"
    );
    assert_ne!(
        trace.last().egui,
        Some(crate::ui::conversation::messages_list_id(&old)),
        "not the old chat's list"
    );
    assert_eq!(
        trace.last().egui,
        Some(crate::ui::conversation::composer_id()),
        "the new chat's message field"
    );
}

/// A dialog stays open as the reader moves to another chat, such as from a
/// notification. Escape closes it there, and focus goes to that chat's
/// message field rather than back to the old chat's list it opened from.
#[test]
fn a_dialog_closed_in_another_chat_leaves_focus_to_it() {
    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::Messages);
    let old = trace.app.open_chat.clone().expect("the demo opens a chat");
    tap(
        &mut trace,
        "Ctrl+/",
        egui::Key::Slash,
        egui::Modifiers::COMMAND,
    );
    assert!(trace.app.dialog.is_some(), "Ctrl+/ opens the shortcuts");
    trace.action += 1;
    trace
        .app
        .actions
        .push(Action::OpenChat(super::SAMPLES[1].id.into()));
    trace.settle("another chat", 3);
    assert_eq!(trace.app.open_chat.as_deref(), Some(super::SAMPLES[1].id));
    assert!(
        trace.app.dialog.is_some(),
        "the dialog is still open after the chat switch"
    );
    let from = tap(
        &mut trace,
        "Escape",
        egui::Key::Escape,
        egui::Modifiers::NONE,
    );
    assert!(trace.app.dialog.is_none(), "Escape closes the dialog");
    assert_ne!(
        trace.last().egui,
        Some(crate::ui::conversation::messages_list_id(&old)),
        "not the old chat's list\n{}",
        trace.dump(from)
    );
    assert_eq!(
        trace.last().egui,
        Some(crate::ui::conversation::composer_id()),
        "the new chat's message field\n{}",
        trace.dump(from)
    );
}

/// A clip opened in the viewer from the keys takes focus on its frame, so
/// Space plays or pauses it, as it always did, rather than pressing Close,
/// the first control, and closing the viewer.
#[test]
fn a_clip_opened_in_the_viewer_takes_focus_on_its_frame() {
    let mut app = app();
    super::video_sample(&mut app, None);
    // Nothing here may make a sound.
    app.video.silence();
    let mut trace = Trace::new(app);
    focus_stop(&mut trace, Stop::Messages);
    let from = trace.mark();
    trace.action += 1;
    let path = trace.app.dirs.media_cache_dir().join("demo-video.mp4");
    trace.app.actions.push(Action::ExpandVideo {
        message: "demo-video".into(),
        path,
    });
    trace.settle("open the clip", 4);
    trace.assert_clean(from, "the clip opened");
    assert_eq!(
        trace.last().name,
        "Button \"Play or pause the video\"",
        "{}",
        trace.dump(from)
    );
    tap(&mut trace, "Space", egui::Key::Space, egui::Modifiers::NONE);
    assert!(
        trace.app.image_preview.is_some(),
        "Space played or paused, the viewer stays"
    );
    trace.app.video.stop();
}

/// The message menu and the chat menu, opened from the keys, are heard in
/// the frame after the key: the menu's hidden first pass is redone in that
/// frame to show it, rather than shown a frame later.
#[test]
fn menus_opened_from_the_keys_are_heard_in_the_next_frame() {
    for (stop, step, key, modifiers) in [
        (
            Stop::Messages,
            "Shift+F10",
            egui::Key::F10,
            egui::Modifiers::SHIFT,
        ),
        (
            Stop::ChatList,
            "Shift+F10",
            egui::Key::F10,
            egui::Modifiers::SHIFT,
        ),
    ] {
        let mut trace = Trace::new(app());
        focus_stop(&mut trace, stop);
        let press = tap(&mut trace, step, key, modifiers);
        let scenario = format!("{step} on {stop:?}");
        trace.assert_clean(press, &scenario);
        // The item the menu settles on already has focus, on the menu's own
        // layer, in the frame after the key.
        let next = &trace.frames[press + 1];
        assert!(
            next.name.starts_with("MenuItem")
                && next.egui.is_some()
                && next.egui == trace.last().egui
                && next.layer != trace.frames[press].layer,
            "{scenario}: the menu is heard in the next frame\n{}",
            trace.dump(press)
        );
    }
}

/// Escape closes the open chat and puts the reader on its row in the chat
/// list, from the message field, the message list and the chat's toolbars
/// alike.
#[test]
fn escape_in_an_open_chat_lands_on_its_row() {
    for stop in [
        Stop::Composer,
        Stop::Messages,
        Stop::ChatActions,
        Stop::MessageActions,
    ] {
        let mut trace = Trace::new(app());
        let chat = trace.app.open_chat.clone().expect("the demo opens a chat");
        focus_stop(&mut trace, stop);
        let from = trace.mark();
        trace.press("Escape", egui::Key::Escape, egui::Modifiers::NONE);
        let scenario = format!("Escape in an open chat from {stop:?}");
        trace.assert_clean(from, &scenario);
        assert_eq!(trace.app.open_chat, None, "{scenario}");
        assert_eq!(
            trace.last().egui,
            Some(crate::ui::chats::chats_list_id()),
            "{scenario}\n{}",
            trace.dump(from)
        );
        let row = trace
            .ctx
            .data(|data| {
                data.get_temp::<egui::Id>(crate::ui::chats::chat_row_id(&chat).with("widget"))
            })
            .expect("the closed chat's row is drawn");
        assert_eq!(
            trace.last().effective,
            row.accesskit_id(),
            "{scenario}: the reader is on the closed chat's row\n{}",
            trace.dump(from)
        );
    }
}

/// Alt+1 in the message field moves to the chat list and types nothing.
#[test]
fn alt_1_in_the_message_field_types_nothing() {
    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::Composer);
    let draft = trace.app.composer.clone();
    trace.press("Alt+1", egui::Key::Num1, alt());
    assert_eq!(trace.last().egui, Some(crate::ui::chats::chats_list_id()));
    assert_eq!(trace.app.composer, draft, "nothing typed");
}

/// With the pointer, no keyboard navigation and no screen reader, Escape
/// closes the chat as it always did: no ring on its row in the chat list,
/// no scroll to it, and focus let go.
#[test]
fn escape_with_the_pointer_closes_the_chat_as_before() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.attach(&ctx);
    render(&mut app, &ctx);
    assert!(app.open_chat.is_some());
    press_key(&mut app, &ctx, egui::Key::Escape, egui::Modifiers::NONE);
    assert_eq!(app.open_chat, None);
    assert!(app.scroll_chat_into_view.is_none());
    assert_eq!(ctx.memory(|memory| memory.focused()), None);
    assert!(!ctx.data(|data| {
        data.get_temp::<bool>(crate::theme::keyboard_focus_id())
            .unwrap_or(false)
    }));
}

/// Enter on a chat row opens the chat and moves to its message field. The
/// row says "open" only once focus has left it: never while the reader is
/// on it, nor in the update that moves focus away, which a screen reader
/// reads before the focus change, so the row is not read out again. Back on
/// the row with Alt+1, it says "open".
#[test]
fn opening_a_chat_from_its_row_never_reads_the_row_again() {
    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::ChatList);
    trace.press("Down", egui::Key::ArrowDown, egui::Modifiers::NONE);
    let row = trace.last().clone();
    assert!(!row.spoken.contains(", open"), "{}", row.spoken);
    let from = trace.mark();
    trace.press("Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(trace.last().egui, Some(egui::Id::new("composer-text")));
    trace.assert_clean(from, "a chat opened from its row");
    for (index, frame) in trace.frames.iter().enumerate().skip(from) {
        let before = &trace.frames[index - 1];
        let renamed = (frame.effective == row.effective && frame.spoken != row.spoken)
            || (before.effective == row.effective
                && frame.effective != row.effective
                && frame.left != row.spoken);
        assert!(
            !renamed,
            "frame #{index}: the row changed while focus was on it or leaving it: {:?}\n{}",
            frame.left,
            trace.dump(from - 1)
        );
    }
    trace.press("Alt+1", egui::Key::Num1, alt());
    assert_eq!(trace.last().effective, row.effective, "back on the row");
    assert!(
        trace.last().spoken.contains(", open"),
        "{}",
        trace.last().spoken
    );
}

/// The chat list scrolls under the arrows: the reader hears each chat once.
#[test]
fn arrowing_the_chat_list_never_reports_the_window() {
    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::ChatList);
    let from = trace.mark();
    for _ in 0..super::sample_ids().len() {
        trace.press("Down", egui::Key::ArrowDown, egui::Modifiers::NONE);
    }
    for _ in 0..4 {
        trace.press("Up", egui::Key::ArrowUp, egui::Modifiers::NONE);
    }
    trace.press("Home", egui::Key::Home, egui::Modifiers::NONE);
    trace.press("End", egui::Key::End, egui::Modifiers::NONE);
    trace.assert_clean(from, "arrows in the chat list");
}

/// The chat menu's Delete chat asks in a dialog reached in one focus
/// change, and Escape puts the reader back on the chat's row.
#[test]
fn the_chat_menus_dialog_gives_focus_back_to_the_row() {
    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::ChatList);
    let row = trace.last().clone();
    trace.press("Shift+F10", egui::Key::F10, egui::Modifiers::SHIFT);
    walk_to(&mut trace, "Delete chat");
    let press = tap(&mut trace, "Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert!(trace.app.dialog.is_some());
    trace.assert_clean(press, "Delete chat");
    assert_eq!(
        trace.frames[press + 1].layer,
        Some(crate::ui::dialogs::modal_id())
    );
    let from = tap(
        &mut trace,
        "Escape",
        egui::Key::Escape,
        egui::Modifiers::NONE,
    );
    trace.assert_clean(from, "Delete chat, closed");
    assert_eq!(trace.last().egui, Some(crate::ui::chats::chats_list_id()));
    assert_eq!(trace.last().effective, row.effective, "back on the chat");
}

/// Where a menu item is in its menu, as a screen reader hears it: its
/// position (from 0) and the menu's size.
fn menu_position(frame: &Frame) -> (Option<usize>, Option<usize>) {
    let number = |part: Option<&str>| -> Option<usize> {
        part?.strip_prefix("Some(")?.strip_suffix(')')?.parse().ok()
    };
    let mut parts = frame.spoken.rsplitn(3, '|');
    let size = number(parts.next());
    let position = number(parts.next());
    (position, size)
}

/// A menu the keys opened from `opener` in the frame `press` behaves as
/// every keyboard menu does: it is heard in the frame after the key, as a
/// menu named `menu`, on its first item, whose role and name start with
/// `first`, said to be the first of its items; Up and Down walk its items
/// and wrap, Home and End go to its ends, Tab and Shift+Tab stay among its
/// items, and Escape closes it and puts the reader back on `opener`, each
/// key heard once.
fn walks_like_a_menu(
    trace: &mut Trace,
    opener: &Frame,
    press: usize,
    menu: &str,
    first: &str,
    scenario: &str,
) {
    trace.assert_clean(press, scenario);
    let opened = trace.frames[press + 1].clone();
    assert!(
        opened.name.starts_with(first),
        "{scenario}: on the first item, {first}, in the frame after the key\n{}",
        trace.dump(press)
    );
    assert!(
        opened
            .ancestor_names
            .iter()
            .any(|name| *name == format!("Menu {menu:?}")),
        "{scenario}: in a menu named {menu}: {:?}",
        opened.ancestor_names
    );
    let (position, size) = menu_position(&opened);
    assert_eq!(position, Some(0), "{scenario}: {}", opened.spoken);
    let size = size.unwrap_or_else(|| panic!("{scenario}: no size: {}", opened.spoken));
    let at = |trace: &Trace| menu_position(trace.last()).0;
    let from = trace.mark();
    trace.press("Up", egui::Key::ArrowUp, egui::Modifiers::NONE);
    assert_eq!(
        at(trace),
        Some(size - 1),
        "{scenario}: Up wraps to the last"
    );
    let last = trace.last().effective;
    trace.press("Down", egui::Key::ArrowDown, egui::Modifiers::NONE);
    assert_eq!(
        trace.last().effective,
        opened.effective,
        "{scenario}: Down wraps to the first"
    );
    trace.press("End", egui::Key::End, egui::Modifiers::NONE);
    assert_eq!(trace.last().effective, last, "{scenario}: End");
    trace.press("Home", egui::Key::Home, egui::Modifiers::NONE);
    assert_eq!(trace.last().effective, opened.effective, "{scenario}: Home");
    trace.press("Down", egui::Key::ArrowDown, egui::Modifiers::NONE);
    assert_eq!(at(trace), Some(1 % size), "{scenario}: Down");
    trace.press("Tab", egui::Key::Tab, egui::Modifiers::NONE);
    assert_eq!(
        at(trace),
        Some(2 % size),
        "{scenario}: Tab stays in the menu"
    );
    trace.press("Shift+Tab", egui::Key::Tab, egui::Modifiers::SHIFT);
    assert_eq!(at(trace), Some(1 % size), "{scenario}: Shift+Tab");
    trace.assert_clean(from, scenario);
    let escape = tap(trace, "Escape", egui::Key::Escape, egui::Modifiers::NONE);
    trace.assert_clean(escape, &format!("{scenario}, Escape"));
    assert!(!egui::Popup::is_any_open(&trace.ctx), "{scenario}: closed");
    assert_eq!(
        trace.last().egui,
        opener.egui,
        "{scenario}: Escape puts the reader back on the opener\n{}",
        trace.dump(escape)
    );
}

/// The chat menu, opened from the chat list's Shift+F10, walks as every
/// keyboard menu does, Home and End included.
#[test]
fn the_chat_menu_walks_like_the_other_menus() {
    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::ChatList);
    let row = trace.last().clone();
    let press = tap(
        &mut trace,
        "Shift+F10",
        egui::Key::F10,
        egui::Modifiers::SHIFT,
    );
    walks_like_a_menu(
        &mut trace,
        &row,
        press,
        "Chat",
        "MenuItem \"Mark as",
        "the chat menu",
    );
    assert_eq!(trace.last().effective, row.effective, "back on the chat");
}

/// Home and End reach the ends of the message menu: its first stop, the
/// reactions, on an emoji, and its last item.
#[test]
fn home_and_end_reach_the_ends_of_the_message_menu() {
    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::Messages);
    let row = trace.last().clone();
    trace.press("Shift+F10", egui::Key::F10, egui::Modifiers::SHIFT);
    let from = trace.mark();
    trace.press("End", egui::Key::End, egui::Modifiers::NONE);
    let (position, size) = menu_position(trace.last());
    assert!(
        trace.last().name.starts_with("MenuItem")
            && position.is_some()
            && position == size.map(|size| size - 1),
        "End: {}",
        trace.last().spoken
    );
    trace.press("Home", egui::Key::Home, egui::Modifiers::NONE);
    assert!(
        trace.last().name.starts_with("Button"),
        "Home lands on an emoji: {}",
        trace.last().name
    );
    trace.assert_clean(from, "Home and End in the message menu");
    escape_back_to_the_list(&mut trace, &row, "the message menu");
}

/// An item of a menu the keys opened from `opener`, taken with Enter after
/// `downs` presses of Down, opens a dialog in one focus change, and the
/// dialog's Escape puts the reader back on `opener`.
fn item_opens_a_dialog(trace: &mut Trace, opener: &Frame, downs: usize, scenario: &str) {
    for _ in 0..downs {
        trace.press("Down", egui::Key::ArrowDown, egui::Modifiers::NONE);
    }
    let press = tap(trace, "Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert!(trace.app.dialog.is_some(), "{scenario} opens a dialog");
    trace.assert_clean(press, scenario);
    assert_eq!(
        trace.frames[press + 1].layer,
        Some(crate::ui::dialogs::modal_id()),
        "{scenario}\n{}",
        trace.dump(press)
    );
    let from = tap(trace, "Escape", egui::Key::Escape, egui::Modifiers::NONE);
    assert!(trace.app.dialog.is_none(), "{scenario}, closed");
    trace.assert_clean(from, &format!("{scenario}, closed"));
    assert_eq!(
        trace.last().egui,
        opener.egui,
        "{scenario}: back on the opener\n{}",
        trace.dump(from)
    );
}

/// The chat header's More, reached from the keys: Chat info, then End.
fn header_more(trace: &mut Trace) -> Frame {
    focus_stop(trace, Stop::ChatActions);
    trace.press("End", egui::Key::End, egui::Modifiers::NONE);
    let more = trace.last().clone();
    assert_eq!(more.name, "Button \"More\"", "{}", trace.dump(0));
    more
}

/// The chat header's More opens its menu from Enter and Space as every
/// keyboard menu opens, and an item taken from it acts: Info opens the info
/// panel beside the chat, with the reader back on More in one focus change.
#[test]
fn the_chat_headers_menu_opens_from_the_keys_as_the_other_menus_do() {
    for (step, key) in [("Enter", egui::Key::Enter), ("Space", egui::Key::Space)] {
        let mut trace = Trace::new(app());
        let more = header_more(&mut trace);
        let press = tap(&mut trace, step, key, egui::Modifiers::NONE);
        walks_like_a_menu(
            &mut trace,
            &more,
            press,
            "Chat",
            "MenuItem \"Info\"",
            &format!("More, {step}"),
        );
    }
    let mut trace = Trace::new(app());
    let more = header_more(&mut trace);
    trace.press("Enter", egui::Key::Enter, egui::Modifiers::NONE);
    tap(&mut trace, "Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert!(trace.app.info.is_some(), "More, Info opens the info panel");
    // Not checked for a clean trace: the panel opening beside the chat lays
    // the conversation out again under new parent nodes, as it does for
    // anything focused in the chat.
    assert_eq!(trace.last().egui, more.egui, "back on More");
}

/// Offline, the header menu's Clear chat is still reached by the keys, and
/// says that it is unavailable and why; Enter on it does nothing.
#[test]
fn the_chat_headers_clear_chat_says_why_it_is_unavailable() {
    let mut app = app();
    app.link = crate::backend::LinkStatus::Connecting;
    let mut trace = Trace::new(app);
    header_more(&mut trace);
    trace.press("Enter", egui::Key::Enter, egui::Modifiers::NONE);
    walk_to(&mut trace, "Clear chat");
    let item = trace.last().clone();
    assert!(
        item.spoken
            .contains("Clear chat, unavailable: Connect to WhatsApp to clear the chat"),
        "{}",
        item.spoken
    );
    assert_eq!(menu_position(&item).0, Some(4), "{}", item.spoken);
    let press = tap(&mut trace, "Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert!(trace.app.dialog.is_none(), "nothing is cleared");
    trace.assert_unchanged(press, "Enter on Clear chat");
}

/// A screen reader's own activation of More, an AccessKit action rather
/// than a key, opens its menu with the reader in it, as the keys do.
#[test]
fn a_screen_readers_activation_of_more_puts_the_reader_in_its_menu() {
    let mut trace = Trace::new(app());
    let more = header_more(&mut trace);
    trace
        .ctx
        .data_mut(|data| data.insert_temp(crate::theme::keyboard_focus_id(), false));
    let from = trace.mark();
    trace.action += 1;
    trace.frame(
        "activate More",
        vec![egui::Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                action: egui::accesskit::Action::Click,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: more.effective,
                data: None,
            },
        )],
    );
    trace.settle("activate More", 4);
    trace.assert_clean(from, "More activated by the screen reader");
    assert_eq!(
        trace.last().name,
        "MenuItem \"Info\"",
        "{}",
        trace.dump(from)
    );
    let escape = tap(
        &mut trace,
        "Escape",
        egui::Key::Escape,
        egui::Modifiers::NONE,
    );
    trace.assert_clean(escape, "More activated, Escape");
    assert_eq!(trace.last().egui, more.egui, "back on More");
}

/// With a screen reader or another UIA client attached, a click on More
/// opens its menu as upstream does: no focus is pushed into it, and Tab is
/// not kept in it.
#[test]
fn the_chat_headers_menu_opened_with_the_pointer_is_as_before() {
    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::Composer);
    let more =
        *crate::ui::toolbar::buttons(&trace.ctx, crate::ui::conversation::header_toolbar_id())
            .last()
            .expect("More is drawn");
    let at = trace
        .ctx
        .read_response(more)
        .expect("More is drawn")
        .rect
        .center();
    pointer_click(&mut trace, "click More", at);
    let popup = more.with("popup");
    assert!(
        egui::Popup::is_id_open(&trace.ctx, popup),
        "the click opens it"
    );
    assert_ne!(trace.last().layer, Some(popup), "no focus pushed into it");
    assert_eq!(
        trace.ctx.memory(|memory| memory.top_modal_layer()),
        None,
        "Tab is not kept in it"
    );
}

/// Presses `key` until the reader is on a control whose name contains
/// `label`, and returns that frame.
fn move_to(trace: &mut Trace, key: egui::Key, label: &str) -> Frame {
    for _ in 0..60 {
        if trace.last().name.contains(label) {
            return trace.last().clone();
        }
        trace.press(key.name(), key, egui::Modifiers::NONE);
    }
    panic!("no control {label}\n{}", trace.dump(trace.mark() - 1));
}

/// The commands the app sent since the last call.
fn sent(
    inbox: &mut tokio::sync::mpsc::UnboundedReceiver<crate::backend::Command>,
) -> Vec<crate::backend::Command> {
    std::iter::from_fn(|| inbox.try_recv().ok()).collect()
}

/// The composer's plus, Attach, reached from the keys.
fn attach(trace: &mut Trace) -> Frame {
    focus_stop(trace, Stop::MessageActions);
    let plus = trace.last().clone();
    assert!(plus.name.contains("Attach"), "{}", plus.name);
    plus
}

/// The composer's plus opens its menu from the keys as every keyboard menu
/// opens, though it keeps its open state in the app rather than in egui:
/// Escape closes the menu, not the chat. Create poll opens its dialog,
/// whose Escape puts the reader back on the plus, and Send files asks for
/// the files with the reader back on the plus.
#[test]
fn the_composers_attach_menu_opens_from_the_keys_as_the_other_menus_do() {
    let mut trace = Trace::new(app());
    let plus = attach(&mut trace);
    let press = tap(&mut trace, "Enter", egui::Key::Enter, egui::Modifiers::NONE);
    walks_like_a_menu(
        &mut trace,
        &plus,
        press,
        "Attach",
        "MenuItem \"Send files\"",
        "Attach",
    );
    assert!(!trace.app.composer_tools_open, "Escape closes the menu");
    assert!(trace.app.open_chat.is_some(), "and not the chat");
    trace.press("Enter", egui::Key::Enter, egui::Modifiers::NONE);
    item_opens_a_dialog(&mut trace, &plus, 3, "Attach, Create poll");
    assert!(!trace.app.composer_tools_open);

    let mut trace = Trace::new(app());
    let (backend, mut inbox) = crate::backend::Backend::recording();
    trace.app.backend = backend;
    let plus = attach(&mut trace);
    trace.press("Enter", egui::Key::Enter, egui::Modifiers::NONE);
    let from = tap(&mut trace, "Enter", egui::Key::Enter, egui::Modifiers::NONE);
    trace.assert_clean(from, "Attach, Send files");
    assert!(
        sent(&mut inbox)
            .iter()
            .any(|command| matches!(command, crate::backend::Command::PickFiles { .. })),
        "Send files asks for the files"
    );
    assert!(!trace.app.composer_tools_open);
    assert_eq!(trace.last().egui, plus.egui, "back on the plus");
}

/// With a screen reader attached, a click on the plus opens its menu as
/// upstream does: kept in the app only, with no focus pushed into it.
#[test]
fn the_composers_attach_menu_opened_with_the_pointer_is_as_before() {
    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::Composer);
    let plus = control(&trace.ctx, Stop::MessageActions).expect("the plus is drawn");
    let at = trace
        .ctx
        .read_response(plus)
        .expect("the plus is drawn")
        .rect
        .center();
    pointer_click(&mut trace, "click Attach", at);
    assert!(trace.app.composer_tools_open, "the click opens it");
    let popup = plus.with("composer-tools");
    assert!(
        !egui::Popup::is_id_open(&trace.ctx, popup),
        "its open state stays in the app"
    );
    assert_ne!(trace.last().layer, Some(popup), "no focus pushed into it");
    assert_eq!(trace.ctx.memory(|memory| memory.top_modal_layer()), None);
}

/// The group photo in a group's info panel opens its menu from the keys as
/// every keyboard menu opens; Escape closes the menu and leaves the panel open,
/// and Change photo asks for the picture with the reader back on the photo.
#[test]
fn the_group_photos_menu_opens_from_the_keys_as_the_other_menus_do() {
    let mut app = app();
    super::apply_flags(&mut app, Some("group-info"));
    let mut trace = Trace::new(app);
    let (backend, mut inbox) = crate::backend::Backend::recording();
    trace.app.backend = backend;
    let photo = move_to(&mut trace, egui::Key::Tab, "Change group photo");
    let press = tap(&mut trace, "Enter", egui::Key::Enter, egui::Modifiers::NONE);
    walks_like_a_menu(
        &mut trace,
        &photo,
        press,
        "Change group photo",
        "MenuItem \"Change photo\"",
        "the group photo",
    );
    assert!(
        trace.app.info.is_some(),
        "Escape leaves the info panel open"
    );
    trace.press("Enter", egui::Key::Enter, egui::Modifiers::NONE);
    let from = tap(&mut trace, "Enter", egui::Key::Enter, egui::Modifiers::NONE);
    trace.assert_clean(from, "the group photo, Change photo");
    assert!(
        sent(&mut inbox)
            .iter()
            .any(|command| matches!(command, crate::backend::Command::PickGroupPicture(_))),
        "Change photo asks for the picture"
    );
    assert_eq!(trace.last().egui, photo.egui, "back on the photo");
}

/// The account switcher, reached and opened from the keys, walks as every
/// keyboard menu does.
#[test]
fn the_account_switcher_walks_like_the_other_menus() {
    let mut trace = Trace::new(app_with_two_accounts());
    let switcher = crate::ui::rail::buttons(&trace.ctx)
        .last()
        .copied()
        .expect("the rail's picture");
    trace
        .ctx
        .data_mut(|data| data.insert_temp(crate::theme::keyboard_focus_id(), true));
    trace
        .ctx
        .memory_mut(|memory| memory.request_focus(switcher));
    trace.settle("focus", 3);
    let opener = trace.last().clone();
    let press = tap(&mut trace, "Enter", egui::Key::Enter, egui::Modifiers::NONE);
    walks_like_a_menu(
        &mut trace,
        &opener,
        press,
        "Switch account",
        "MenuItemRadio",
        "the account switcher",
    );
}

/// A screen reader's own activation of the account switcher opens it with
/// the reader in it, as the keys do.
#[test]
fn a_screen_readers_activation_of_the_switcher_puts_the_reader_in_it() {
    let mut trace = Trace::new(app_with_two_accounts());
    let switcher = crate::ui::rail::buttons(&trace.ctx)
        .last()
        .copied()
        .expect("the rail's picture");
    trace
        .ctx
        .memory_mut(|memory| memory.request_focus(switcher));
    trace.settle("focus", 3);
    trace
        .ctx
        .data_mut(|data| data.insert_temp(crate::theme::keyboard_focus_id(), false));
    let from = trace.mark();
    trace.action += 1;
    trace.frame(
        "activate the switcher",
        vec![egui::Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                action: egui::accesskit::Action::Click,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: switcher.accesskit_id(),
                data: None,
            },
        )],
    );
    trace.settle("activate the switcher", 4);
    trace.assert_clean(from, "the switcher activated by the screen reader");
    assert!(
        trace.last().name.starts_with("MenuItemRadio"),
        "{}",
        trace.dump(from)
    );
}

/// Alt+3 when the first unread message is older than the loaded page: the
/// reader hears that it is loading, focus stays until the archive's older
/// page arrives, then moves once to that message.
#[test]
fn alt_3_loads_older_messages_up_to_the_first_unread() {
    let mut app = app();
    long_chat(&mut app, 400);
    let chat = app.open_chat.clone().expect("the demo opens a chat");
    let conversation = app.conversations.get_mut(&chat).expect("its messages");
    for message in &mut conversation.messages {
        message.from_me = false;
    }
    // Only the newest page is loaded; the first unread, long-250, is in the
    // archive.
    let older: Vec<_> = conversation
        .messages
        .drain(..400 - crate::app::PAGE)
        .collect();
    conversation.complete = false;
    app.unread_divider = Some(crate::app::UnreadDivider {
        chat: chat.clone(),
        count: 150,
        placed: true,
    });
    let mut trace = Trace::new(app);
    let (backend, mut commands, events) = crate::backend::Backend::recording_with_events();
    trace.app.backend = backend;
    focus_stop(&mut trace, Stop::Composer);
    let draft = trace.app.composer.clone();
    let from = trace.mark();
    trace.press("Alt+3", egui::Key::Num3, alt());
    trace.assert_unchanged(from, "Alt+3 while the unread messages load");
    assert!(
        trace
            .app
            .announcer
            .spoken()
            .contains(&"Loading unread messages"),
        "{:?}",
        trace.app.announcer.spoken()
    );
    assert!(
        std::iter::from_fn(|| commands.try_recv().ok())
            .any(|command| matches!(command, crate::backend::Command::LoadChat { .. })),
        "the archive is asked for older messages"
    );
    trace.action += 1;
    let from = trace.mark();
    events
        .send(crate::backend::Event::Messages {
            chat: chat.clone(),
            messages: older,
            older: true,
            complete: true,
        })
        .expect("the app listens");
    trace.settle("older page", 6);
    trace.assert_clean(from, "landing on the first unread message once loaded");
    assert_eq!(
        trace.last().egui,
        Some(crate::ui::conversation::messages_list_id(&chat))
    );
    assert_eq!(
        trace.app.message_cursor.get(&chat).map(String::as_str),
        Some("long-250")
    );
    assert!(
        trace.last().spoken.contains("Unread"),
        "the reader is on the unread message\n{}",
        trace.dump(from)
    );
    assert_eq!(trace.app.composer, draft, "nothing typed");
}

/// Typing after Alt+3 while the unread messages load ends the jump: the
/// older page arriving later leaves the reader in the message field, where
/// the letters went.
#[test]
fn typing_after_alt_3_keeps_focus_when_the_messages_load() {
    let mut app = app();
    long_chat(&mut app, 400);
    let chat = app.open_chat.clone().expect("the demo opens a chat");
    let conversation = app.conversations.get_mut(&chat).expect("its messages");
    for message in &mut conversation.messages {
        message.from_me = false;
    }
    let older: Vec<_> = conversation
        .messages
        .drain(..400 - crate::app::PAGE)
        .collect();
    conversation.complete = false;
    app.unread_divider = Some(crate::app::UnreadDivider {
        chat: chat.clone(),
        count: 150,
        placed: true,
    });
    let mut trace = Trace::new(app);
    let (backend, _commands, events) = crate::backend::Backend::recording_with_events();
    trace.app.backend = backend;
    focus_stop(&mut trace, Stop::Composer);
    let draft = trace.app.composer.clone();
    trace.press("Alt+3", egui::Key::Num3, alt());
    trace.action += 1;
    trace.frame("type", vec![egui::Event::Text("h".to_owned())]);
    trace.settle("typed", 2);
    let from = trace.mark();
    events
        .send(crate::backend::Event::Messages {
            chat: chat.clone(),
            messages: older,
            older: true,
            complete: true,
        })
        .expect("the app listens");
    trace.settle("older page", 6);
    trace.assert_unchanged(from, "the older page after typing");
    assert_eq!(trace.last().egui, Some(egui::Id::new("composer-text")));
    assert_eq!(trace.app.composer, format!("{draft}h"), "the letter typed");
}

/// Alt+3 with nothing unread says so: focus stays in the message field
/// without a sound, and nothing is typed.
#[test]
fn alt_3_with_nothing_unread_only_announces() {
    let mut trace = Trace::new(app());
    assert!(trace.app.unread_divider.is_none());
    focus_stop(&mut trace, Stop::Composer);
    let draft = trace.app.composer.clone();
    let from = trace.mark();
    trace.press("Alt+3", egui::Key::Num3, alt());
    trace.assert_unchanged(from, "Alt+3 in the message field");
    assert_eq!(trace.app.composer, draft, "nothing typed");
    assert!(
        trace.app.announcer.spoken().contains(&"No unread messages"),
        "{:?}",
        trace.app.announcer.spoken()
    );
}

/// Alt+3 with no chat open says so, and focus stays put without a sound.
#[test]
fn alt_3_without_a_chat_only_announces() {
    for &stop in NO_CHAT_STOPS {
        let mut trace = no_chat_open(stop);
        let from = trace.mark();
        trace.press("Alt+3", egui::Key::Num3, alt());
        trace.assert_unchanged(from, &format!("Alt+3 without a chat on {stop:?}"));
        assert!(
            trace.app.announcer.spoken().contains(&"No chat open"),
            "{:?}",
            trace.app.announcer.spoken()
        );
    }
}

/// Alt+3 from the message field goes to the first unread message, far above
/// the view: the message list takes focus with the reader on that message,
/// read as unread and heard once, and nothing is typed.
#[test]
fn alt_3_goes_to_the_first_unread_message() {
    let mut app = app();
    long_chat(&mut app, 400);
    let chat = app.open_chat.clone().expect("the demo opens a chat");
    for message in &mut app
        .conversations
        .get_mut(&chat)
        .expect("its messages")
        .messages
    {
        message.from_me = false;
    }
    // The reader has already been taken past the divider, to the newest.
    app.unread_divider = Some(crate::app::UnreadDivider {
        chat: chat.clone(),
        count: 150,
        placed: true,
    });
    let mut trace = Trace::new(app);
    focus_stop(&mut trace, Stop::Composer);
    let draft = trace.app.composer.clone();
    let from = trace.mark();
    trace.press("Alt+3", egui::Key::Num3, alt());
    trace.assert_clean(from, "Alt+3 to the first unread message");
    assert_eq!(
        trace.last().egui,
        Some(crate::ui::conversation::messages_list_id(&chat))
    );
    assert_eq!(
        trace.app.message_cursor.get(&chat).map(String::as_str),
        Some("long-250")
    );
    assert!(
        trace.last().spoken.contains("Unread"),
        "the reader is on the unread message\n{}",
        trace.dump(from)
    );
    assert_eq!(trace.app.composer, draft, "nothing typed");
    // From inside the list, back from the newest message.
    trace.press("End", egui::Key::End, egui::Modifiers::NONE);
    let from = trace.mark();
    trace.press("Alt+3", egui::Key::Num3, alt());
    trace.assert_clean(from, "Alt+3 in the message list");
    assert_eq!(
        trace.app.message_cursor.get(&chat).map(String::as_str),
        Some("long-250")
    );
    assert!(trace.last().spoken.contains("Unread"));
}

/// A Settings drop-down is heard once per key, never as the window: Down
/// on it closed says its new value, Enter opens its list on that value,
/// the arrows move in the list, and Tab leaves the list, closed and
/// unchanged, for the next control in the same frame.
#[test]
fn a_settings_dropdown_is_heard_once_per_key() {
    let mut trace = Trace::new(app());
    super::apply_flags(&mut trace.app, Some("settings"));
    trace.settle("settings", 3);
    let font = "ComboBox \"Font\"";
    trace.press("Ctrl+F", egui::Key::F, egui::Modifiers::COMMAND);
    for _ in 0..40 {
        if trace.last().name == font {
            break;
        }
        trace.press("Tab", egui::Key::Tab, egui::Modifiers::NONE);
    }
    assert_eq!(trace.last().name, font, "Tab reaches the Font drop-down");
    let from = trace.mark();
    trace.press("Down", egui::Key::ArrowDown, egui::Modifiers::NONE);
    assert_eq!(trace.last().name, font, "{}", trace.dump(from));
    assert!(trace.last().spoken.contains("Some(\"Inter\")"));
    trace.press("Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(trace.last().name, "ListBoxOption \"Inter\"");
    trace.press("Up", egui::Key::ArrowUp, egui::Modifiers::NONE);
    assert_eq!(trace.last().name, "ListBoxOption \"System\"");
    trace.press("Tab", egui::Key::Tab, egui::Modifiers::NONE);
    let left = trace.last().name.clone();
    assert!(
        left != font && !left.starts_with("ListBoxOption"),
        "Tab moves on: {left}\n{}",
        trace.dump(from)
    );
    assert_eq!(trace.app.settings.font, crate::settings::FontChoice::Inter);
    trace.assert_clean(from, "a Settings drop-down from the keys");
}

/// A privacy drop-down writes to the phone, which the demo never confirms:
/// Down on it is heard once, as its new value, and focus stays on it while
/// the write is pending rather than falling to the window.
#[test]
fn a_saving_privacy_dropdown_keeps_focus() {
    let mut trace = Trace::new(app());
    super::apply_flags(&mut trace.app, Some("settings"));
    trace.settle("settings", 3);
    let profile = "ComboBox \"Profile photo\"";
    trace.press("Ctrl+F", egui::Key::F, egui::Modifiers::COMMAND);
    for _ in 0..150 {
        if trace.last().name == profile {
            break;
        }
        trace.press("Tab", egui::Key::Tab, egui::Modifiers::NONE);
    }
    assert_eq!(trace.last().name, profile, "Tab reaches Profile photo");
    let from = trace.mark();
    trace.press("Down", egui::Key::ArrowDown, egui::Modifiers::NONE);
    trace.settle("saving", 6);
    assert!(
        trace
            .app
            .account_privacy
            .pending(crate::privacy::PrivacyKind::Profile)
    );
    assert_eq!(trace.last().name, profile, "{}", trace.dump(from));
    trace.press("Down", egui::Key::ArrowDown, egui::Modifiers::NONE);
    assert_eq!(trace.last().name, profile);
    trace.assert_clean(from, "a privacy drop-down while it saves");
}

/// Ctrl+Shift+R, Command+Shift+R on macOS: record, and send.
fn record_keys() -> egui::Modifiers {
    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT
}

/// The recorder's Send button, as last drawn.
fn recording_send(trace: &Trace) -> Option<egui::Id> {
    crate::ui::conversation::recording_send(&trace.ctx)
}

/// The demo with its sent commands kept, recording from the message field
/// by Ctrl+Shift+R: the reader is on Send.
fn recording(app: App) -> Trace {
    let mut app = app;
    app.backend.record_demo_commands();
    let mut trace = Trace::new(app);
    focus_stop(&mut trace, Stop::Composer);
    let from = trace.mark();
    trace.press("Ctrl+Shift+R", egui::Key::R, record_keys());
    assert!(trace.app.recording.is_some(), "Ctrl+Shift+R records");
    assert_eq!(trace.last().egui, recording_send(&trace));
    trace.assert_clean(from, "recording started");
    trace
}

/// Whether the reader hears `text` among the announcements shown now.
fn heard(trace: &Trace, text: &str) -> bool {
    trace.app.announcer.spoken().contains(&text)
}

/// Whether the demo backend was asked to send a voice message.
fn sent_voice(trace: &Trace) -> bool {
    trace
        .app
        .backend
        .take_demo_commands()
        .iter()
        .any(|command| matches!(command, crate::backend::Command::SendVoice { .. }))
}

/// Waits until the simulated recorder, which records in real time on its
/// own thread, holds more than the half second under which a recording is
/// dropped rather than sent. It waits for the samples themselves, not for a
/// fixed time, which a busy machine may not give the thread.
fn record_for_a_while(trace: &Trace) {
    let recorder = trace.app.recording.as_ref().expect("recording");
    // One level per 50 ms of samples: 12 levels are 0.6 s.
    let started = std::time::Instant::now();
    while recorder.levels().len() < 12 {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(20),
            "the simulated recorder made no progress"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Waits until `trace`'s recording has run for at least `time`.
fn recorded_for(trace: &Trace, time: std::time::Duration) {
    let recorder = trace.app.recording.as_ref().expect("recording");
    while recorder.elapsed() < time {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Starting a voice message from the keys puts the reader on Send, named
/// for what it sends, described by the time recorded, in the toolbar named
/// Recording: the toolbar's name says it once, and no announcement says it
/// again over the focus change. Ctrl+Shift+R again sends it, and the reader
/// is back in the message field. Each step is heard once, never as the
/// window.
#[test]
fn ctrl_shift_r_records_and_sends_a_voice_message() {
    let mut trace = recording(app());
    assert!(
        !heard(&trace, "Recording"),
        "{:?}",
        trace.app.announcer.spoken()
    );
    let send = trace.last().clone();
    assert_eq!(send.name, "Button \"Send voice message\"");
    assert_eq!(
        send.ancestor_names.first().map(String::as_str),
        Some("Toolbar \"Recording\""),
        "{:?}",
        send.ancestor_names
    );
    assert_eq!(
        send.description.as_deref(),
        Some("0:00"),
        "the time recorded is Send's description"
    );
    record_for_a_while(&trace);
    let from = trace.mark();
    trace.press("Ctrl+Shift+R", egui::Key::R, record_keys());
    assert!(trace.app.recording.is_none());
    assert_eq!(trace.last().egui, Some(egui::Id::new("composer-text")));
    assert!(sent_voice(&trace), "the voice message is sent");
    trace.assert_clean(from, "sent with Ctrl+Shift+R");
}

/// The time recorded moves on in Send's description while its name stays
/// as it was: a screen reader speaks a focused control's new name, so the
/// time there would be heard every second.
#[test]
fn the_time_recorded_changes_only_the_description() {
    let mut trace = recording(app());
    let before = trace.last().clone();
    recorded_for(&trace, std::time::Duration::from_millis(1100));
    trace.settle("a second later", 3);
    let after = trace.last().clone();
    assert_eq!(after.name, before.name);
    assert_eq!(after.effective, before.effective);
    assert_eq!(after.spoken, before.spoken, "nothing new is heard");
    // At least a second has gone by; a busy machine may have let more go.
    let time = after.description.as_deref().unwrap_or_default();
    assert!(
        time.starts_with("0:0") && time != "0:00",
        "the description moved on: {time:?}"
    );
}

/// Enter on Send sends the recording, and the reader is back in the
/// message field.
#[test]
fn enter_on_send_sends_the_recording() {
    let mut trace = recording(app());
    record_for_a_while(&trace);
    let from = trace.mark();
    trace.press("Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert!(trace.app.recording.is_none());
    assert_eq!(trace.last().egui, Some(egui::Id::new("composer-text")));
    assert!(sent_voice(&trace), "the voice message is sent");
    assert!(!heard(&trace, "Recording discarded"));
    trace.assert_clean(from, "sent with Enter");
}

/// Escape, or Enter on Discard (Left from Send), discards the recording:
/// the reader hears "Recording discarded" and is back in the message field,
/// and nothing is sent. Enter on Discard never sends.
#[test]
fn discarding_says_so_and_returns_to_the_field() {
    for keys in [
        &[egui::Key::Escape][..],
        &[egui::Key::ArrowLeft, egui::Key::Enter][..],
    ] {
        let mut trace = recording(app());
        record_for_a_while(&trace);
        let from = trace.mark();
        for key in keys {
            trace.press(&format!("{key:?}"), *key, egui::Modifiers::NONE);
            if *key == egui::Key::ArrowLeft {
                assert_eq!(trace.last().name, "Button \"Discard voice message\"");
            }
        }
        let scenario = format!("discarded with {keys:?}");
        assert!(trace.app.recording.is_none(), "{scenario}");
        assert!(
            heard(&trace, "Recording discarded"),
            "{scenario}: {:?}",
            trace.app.announcer.spoken()
        );
        assert_eq!(
            trace.last().egui,
            Some(egui::Id::new("composer-text")),
            "{scenario}\n{}",
            trace.dump(from)
        );
        assert!(!sent_voice(&trace), "{scenario}: nothing sent");
        trace.assert_clean(from, &scenario);
    }
}

/// While a voice message records, Tab walks the window's own stops, each
/// named, and comes back to the recorder: no unnamed control is reached, as
/// egui's own order reached them before. Shift+Tab, F6, Alt+1, Alt+2 and the
/// lists' arrows keep working, and F6 into the composer lands on the
/// recorder.
#[test]
fn the_window_keeps_its_order_while_recording() {
    let mut trace = recording(app());
    let send = recording_send(&trace);
    let from = trace.mark();
    let mut walked = Vec::new();
    // Every stop of the window once, the rail's included, and a few more.
    for _ in 0..24 {
        trace.press("Tab", egui::Key::Tab, egui::Modifiers::NONE);
        let frame = trace.last().clone();
        let stop = crate::ui::focus::stops(&trace.ctx)
            .into_iter()
            .find(|(_, id)| Some(*id) == frame.egui)
            .map(|(stop, _)| stop);
        assert!(
            stop.is_some(),
            "Tab reached a control that is not a stop: {}",
            frame.name
        );
        assert!(
            !frame.name.ends_with("None") && !frame.name.ends_with("\"\""),
            "Tab reached an unnamed control: {}",
            frame.name
        );
        walked.push(stop);
        if frame.egui == send {
            break;
        }
    }
    assert_eq!(walked.last(), Some(&Some(Stop::Recording)), "{walked:?}");
    assert!(walked.contains(&Some(Stop::Messages)), "{walked:?}");
    trace.press("Shift+Tab", egui::Key::Tab, egui::Modifiers::SHIFT);
    let chat = trace.app.open_chat.clone().expect("the demo opens a chat");
    let messages = crate::ui::conversation::messages_list_id(&chat);
    assert_eq!(trace.last().egui, Some(messages));
    trace.press("F6", egui::Key::F6, egui::Modifiers::NONE);
    assert_eq!(trace.last().egui, send, "F6 to the composer: the recorder");
    trace.press("Alt+1", egui::Key::Num1, alt());
    assert_eq!(trace.last().egui, Some(crate::ui::chats::chats_list_id()));
    trace.press("Down", egui::Key::ArrowDown, egui::Modifiers::NONE);
    trace.press("Alt+2", egui::Key::Num2, alt());
    assert_eq!(trace.last().egui, Some(messages));
    trace.press("Up", egui::Key::ArrowUp, egui::Modifiers::NONE);
    trace.press("Shift+F6", egui::Key::F6, egui::Modifiers::SHIFT);
    assert_eq!(trace.last().egui, Some(crate::ui::chats::chats_list_id()));
    assert!(trace.app.recording.is_some(), "still recording");
    trace.assert_clean(from, "moving around while recording");
}

/// Ctrl+Shift+R records nothing where the microphone button is not
/// offered, and says why when the composer shows or implies a reason: with
/// no chat open, in a chat that takes no messages, with a message typed or
/// being edited, or with a refused voice message waiting. Under a dialog it
/// says nothing at all.
#[test]
fn ctrl_shift_r_says_why_the_microphone_is_not_offered() {
    let open = |setup: &dyn Fn(&mut App)| {
        let mut trace = Trace::new(app());
        setup(&mut trace.app);
        trace.settle("set up", 3);
        trace
    };
    let chat = |app: &App| app.open_chat.clone().expect("the demo opens a chat");
    let cases: Vec<(&str, Trace, Option<&str>)> = vec![
        (
            "no chat",
            no_chat_open(Stop::ChatList),
            Some("No chat open"),
        ),
        (
            "a channel",
            open(&|app| super::apply_flags(app, Some("channel"))),
            Some("Channels are read-only in ZapFast"),
        ),
        (
            "an admins-only chat",
            open(&|app| {
                let id = chat(app);
                app.chat_mut(&id).expect("the open chat").read_only = true;
            }),
            Some("Only admins can send messages"),
        ),
        (
            "a draft",
            open(&|app| app.composer = "Fixture draft".to_owned()),
            Some("Send or clear the message to record"),
        ),
        (
            "an edit",
            open(&|app| {
                let id = chat(app);
                let last = app.conversations[&id]
                    .messages
                    .last()
                    .expect("a message")
                    .id
                    .clone();
                app.editing = Some(last);
                app.composer = "Fixture edit".to_owned();
            }),
            Some("Save or cancel the edit to record"),
        ),
        (
            "a refused voice message",
            open(&|app| app.unsent_voice = Some((chat(app), vec![0.0; 480]))),
            Some("Send or discard the unsent voice message to record"),
        ),
        (
            "a dialog",
            open(&|app| {
                app.actions
                    .push(Action::ShowDialog(crate::model::Dialog::Shortcuts));
            }),
            None,
        ),
    ];
    for (scenario, mut trace, reason) in cases {
        let before: Vec<String> = trace
            .app
            .announcer
            .spoken()
            .iter()
            .map(|text| text.to_string())
            .collect();
        trace.press("Ctrl+Shift+R", egui::Key::R, record_keys());
        assert!(trace.app.recording.is_none(), "{scenario}");
        assert!(!heard(&trace, "Recording"), "{scenario}");
        match reason {
            Some(reason) => assert!(
                heard(&trace, reason),
                "{scenario}: {:?}",
                trace.app.announcer.spoken()
            ),
            None => {
                assert_eq!(
                    trace.app.announcer.spoken().map(str::to_owned).to_vec(),
                    before,
                    "{scenario}"
                );
                assert!(trace.app.dialog.is_some(), "the dialog stays");
            }
        }
    }
}

/// A recording stopped under half a second is not sent, as before, and the
/// reader hears why; focus is back in the message field.
#[test]
fn a_recording_too_short_says_it_was_not_sent() {
    let mut trace = recording(app());
    let from = trace.mark();
    trace.press("Ctrl+Shift+R", egui::Key::R, record_keys());
    assert!(trace.app.recording.is_none());
    assert!(!sent_voice(&trace), "nothing sent");
    assert!(
        heard(&trace, "Recording too short, not sent"),
        "{:?}",
        trace.app.announcer.spoken()
    );
    assert_eq!(trace.last().egui, Some(egui::Id::new("composer-text")));
    trace.assert_clean(from, "a recording too short");
}

/// Opening another chat while recording keeps recording, as it always did,
/// now for that chat, and says so: from the chat list's Enter, Alt+Down and
/// Ctrl+Shift+] alike. Focus is on Send in the chat opened, and Ctrl+Shift+R
/// sends the recording there. Ctrl with a number opens no chat while
/// recording, as before.
#[test]
fn opening_another_chat_while_recording_says_where_it_goes() {
    let ways: [&[(&str, egui::Key, egui::Modifiers)]; 3] = [
        &[
            ("Alt+1", egui::Key::Num1, alt()),
            ("Down", egui::Key::ArrowDown, egui::Modifiers::NONE),
            ("Enter", egui::Key::Enter, egui::Modifiers::NONE),
        ],
        &[("Alt+Down", egui::Key::ArrowDown, egui::Modifiers::ALT)],
        &[("Ctrl+Shift+]", egui::Key::CloseBracket, record_keys())],
    ];
    for keys in ways {
        let mut trace = recording(app());
        let before = trace.app.open_chat.clone();
        let from = trace.mark();
        for (step, key, modifiers) in keys {
            trace.press(step, *key, *modifiers);
        }
        let scenario = format!("{:?}", keys.iter().map(|key| key.0).collect::<Vec<_>>());
        let opened = trace.app.open_chat.clone().expect("a chat is open");
        assert_ne!(
            Some(&opened),
            before.as_ref(),
            "{scenario} opened another chat"
        );
        assert!(trace.app.recording.is_some(), "{scenario}: still recording");
        let title = trace
            .app
            .chat(&opened)
            .map(|chat| trace.app.chat_title(chat))
            .expect("the opened chat");
        let said = format!("Recording continues in {title}");
        assert!(
            heard(&trace, &said),
            "{scenario}: {:?}",
            trace.app.announcer.spoken()
        );
        assert_eq!(trace.last().egui, recording_send(&trace), "{scenario}");
        trace.assert_clean(from, &scenario);
        record_for_a_while(&trace);
        trace.press("Ctrl+Shift+R", egui::Key::R, record_keys());
        assert!(
            trace
                .app
                .backend
                .take_demo_commands()
                .iter()
                .any(|command| matches!(
                    command,
                    crate::backend::Command::SendVoice { chat, .. } if *chat == opened
                )),
            "{scenario}: sent to the chat opened"
        );
    }
    let mut trace = recording(app());
    let before = trace.app.open_chat.clone();
    trace.press("Ctrl+2", egui::Key::Num2, egui::Modifiers::COMMAND);
    trace.press("Ctrl+3", egui::Key::Num3, egui::Modifiers::COMMAND);
    assert_eq!(trace.app.open_chat, before, "Ctrl with a number waits");
    assert!(trace.app.recording.is_some());
}

/// A microphone that cannot start says why, as the error it always was,
/// and the reader is back in the message field rather than on the window.
#[test]
fn a_microphone_that_fails_returns_to_the_field() {
    let mut trace = recording(app());
    let from = trace.mark();
    trace.action += 1;
    trace.app.recording = Some(crate::audio::Recorder::failed("No input device"));
    trace.settle("failed", 4);
    assert!(trace.app.recording.is_none());
    assert!(
        heard(&trace, "Could not record: No input device"),
        "{:?}",
        trace.app.announcer.spoken()
    );
    assert_eq!(trace.last().egui, Some(egui::Id::new("composer-text")));
    trace.assert_clean(from, "the microphone failed");
}

/// Locking while recording discards the recording, as it always did, with
/// nothing said about it; after the unlock the reader is in the message
/// field.
#[test]
fn locking_while_recording_lands_in_the_field_after_the_unlock() {
    let mut app = app();
    app.settings.app_lock_hash = Some(crate::app_lock::verifier("fixture"));
    let mut trace = recording(app);
    let from = trace.mark();
    trace.action += 1;
    trace.app.lock_app();
    trace.settle("locked", 3);
    assert!(trace.app.recording.is_none());
    assert_eq!(
        trace.last().egui,
        Some(egui::Id::new(crate::ui::lock::PASSWORD_ID))
    );
    assert!(!heard(&trace, "Recording discarded"));
    trace.action += 1;
    trace.app.app_lock.unlocked(true);
    trace.settle("unlocked", 3);
    assert_eq!(trace.last().egui, Some(egui::Id::new("composer-text")));
    trace.assert_clean(from, "locking while recording");
}

/// Where a recording went on names a chat: like a message, it is gone
/// from the tree once ZapFast locks or another account is on screen.
#[test]
fn where_a_recording_went_on_is_cleared_on_lock_and_account_switch() {
    let carried = |app: App| {
        let mut trace = recording(app);
        trace.press("Alt+Down", egui::Key::ArrowDown, egui::Modifiers::ALT);
        let opened = trace.app.open_chat.clone().expect("a chat is open");
        let title = trace
            .app
            .chat(&opened)
            .map(|chat| trace.app.chat_title(chat))
            .expect("the opened chat");
        assert!(heard(&trace, &format!("Recording continues in {title}")));
        trace
    };
    let mentions = |trace: &Trace| {
        trace
            .app
            .announcer
            .spoken()
            .iter()
            .any(|text| text.contains("continues in"))
    };
    let mut app = app();
    app.settings.app_lock_hash = Some(crate::app_lock::verifier("fixture"));
    let mut locked = carried(app);
    locked.app.lock_app();
    locked.settle("locked", 3);
    assert!(!mentions(&locked), "{:?}", locked.app.announcer.spoken());
    locked.app.app_lock.unlocked(true);
    locked.settle("unlocked", 3);
    assert!(!mentions(&locked), "not back after the unlock");
    let mut switched = carried(app_with_two_accounts());
    switched.app.actions.push(Action::SwitchAccount(
        crate::model::AccountId::parse("2").expect("account id"),
    ));
    switched.settle("switched", 3);
    assert_eq!(switched.app.account().id.as_str(), "2");
    assert!(
        !mentions(&switched),
        "{:?}",
        switched.app.announcer.spoken()
    );
    assert!(heard(&switched, "Recording discarded"));
}

/// Ctrl+Shift+R is exactly that: AltGr+Shift+R, which Windows sends as
/// Ctrl+Alt+Shift+R and some layouts type a character with, records
/// nothing. Ctrl+Shift+R as Windows sends it, Ctrl and Command, records.
#[test]
fn only_ctrl_shift_r_itself_records() {
    let mut trace = Trace::new(app());
    focus_stop(&mut trace, Stop::Composer);
    let altgr = egui::Modifiers {
        alt: true,
        ctrl: true,
        shift: true,
        mac_cmd: false,
        command: true,
    };
    trace.press("AltGr+Shift+R", egui::Key::R, altgr);
    assert!(trace.app.recording.is_none());
    let windows = egui::Modifiers {
        alt: false,
        ctrl: !cfg!(target_os = "macos"),
        shift: true,
        mac_cmd: cfg!(target_os = "macos"),
        command: true,
    };
    trace.press("Ctrl+Shift+R", egui::Key::R, windows);
    assert!(trace.app.recording.is_some());
}

/// Holding Ctrl+Shift+R does not toggle recording on every key repeat: the
/// first press starts a recording, and the repeats that follow while the
/// key is held neither send it nor start another.
#[test]
fn a_held_ctrl_shift_r_does_not_repeat() {
    let mut trace = Trace::new(app());
    trace.app.backend.record_demo_commands();
    focus_stop(&mut trace, Stop::Composer);
    let press = egui::Event::Key {
        key: egui::Key::R,
        physical_key: Some(egui::Key::R),
        pressed: true,
        repeat: false,
        modifiers: record_keys(),
    };
    trace.action += 1;
    trace.frame("Ctrl+Shift+R", vec![press.clone()]);
    trace.frame("Ctrl+Shift+R", Vec::new());
    assert!(trace.app.recording.is_some(), "the first press records");
    // egui marks a press of a key already down as a repeat.
    for _ in 0..3 {
        trace.frame("Ctrl+Shift+R held", vec![press.clone()]);
        trace.frame("Ctrl+Shift+R held", Vec::new());
    }
    trace.frame(
        "Ctrl+Shift+R released",
        vec![egui::Event::Key {
            key: egui::Key::R,
            physical_key: Some(egui::Key::R),
            pressed: false,
            repeat: false,
            modifiers: record_keys(),
        }],
    );
    trace.settle("released", 3);
    assert!(trace.app.recording.is_some(), "the repeats toggled nothing");
    assert!(!sent_voice(&trace), "nothing sent");
    assert!(!heard(&trace, "Recording too short, not sent"));
    assert_eq!(trace.last().egui, recording_send(&trace));
}

/// The linking screen has no chat to record in, and Ctrl+Shift+R there
/// says nothing about chats.
#[test]
fn ctrl_shift_r_is_silent_on_the_linking_screen() {
    let mut app = app();
    super::apply_flags(&mut app, Some("login"));
    let mut trace = Trace::new(app);
    assert!(!trace.app.is_linked());
    trace.press("Ctrl+Shift+R", egui::Key::R, record_keys());
    assert!(trace.app.recording.is_none());
    assert!(
        trace
            .app
            .announcer
            .spoken()
            .iter()
            .all(|text| text.is_empty()),
        "{:?}",
        trace.app.announcer.spoken()
    );
}

/// A recording carried into a chat that takes no messages says so with the
/// composer's own reason, and the reader lands in that chat's message list.
#[test]
fn a_recording_carried_into_a_read_only_chat_says_why() {
    let mut trace = recording(app());
    let open = trace.app.open_chat.clone().expect("a chat is open");
    let visible: Vec<String> = trace
        .app
        .visible_chats()
        .iter()
        .map(|chat| chat.id.clone())
        .collect();
    let index = visible.iter().position(|id| *id == open).expect("listed");
    let next = visible[(index + 1) % visible.len()].clone();
    trace.app.chat_mut(&next).expect("the next chat").read_only = true;
    let title = trace
        .app
        .chat(&next)
        .map(|chat| trace.app.chat_title(chat))
        .expect("the next chat");
    let from = trace.mark();
    trace.press("Alt+Down", egui::Key::ArrowDown, egui::Modifiers::ALT);
    assert_eq!(trace.app.open_chat.as_ref(), Some(&next));
    let said = format!("Recording continues in {title}. Only admins can send messages");
    assert!(heard(&trace, &said), "{:?}", trace.app.announcer.spoken());
    assert_eq!(
        trace.last().egui,
        Some(crate::ui::conversation::messages_list_id(&next))
    );
    trace.assert_clean(from, "into a read-only chat");
}

/// Switching to another account while recording drops the recording, as
/// it always did, and says "Recording discarded" there: focus moves once,
/// to the other account's message field, and nothing of the recording
/// follows it. Ctrl+Shift+R records afresh in that account.
#[test]
fn switching_accounts_while_recording_lands_in_the_other_field() {
    let mut trace = recording(app_with_two_accounts());
    let switcher = crate::ui::rail::buttons(&trace.ctx)
        .last()
        .copied()
        .expect("the rail's picture");
    trace
        .ctx
        .memory_mut(|memory| memory.request_focus(switcher));
    trace.settle("focus", 3);
    trace.press("Enter", egui::Key::Enter, egui::Modifiers::NONE);
    trace.press("Down", egui::Key::ArrowDown, egui::Modifiers::NONE);
    let from = trace.mark();
    trace.press("Enter", egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(trace.app.account().id.as_str(), "2");
    assert!(trace.app.recording.is_none());
    assert_eq!(trace.last().egui, Some(egui::Id::new("composer-text")));
    assert!(
        heard(&trace, "Recording discarded"),
        "{:?}",
        trace.app.announcer.spoken()
    );
    trace.assert_clean(from, "switching accounts while recording");
    trace.press("Ctrl+Shift+R", egui::Key::R, record_keys());
    assert!(trace.app.recording.is_some());
    assert_eq!(trace.last().egui, recording_send(&trace));
}

/// The first voice message of a session, on a fresh window, sent from the
/// keys: started from the message field, from the message list or from the
/// microphone button, and sent with Enter on Send or with Ctrl+Shift+R.
/// The reader goes from Send back to the message field once, never by way
/// of the window.
#[test]
fn the_first_recording_of_a_session_returns_to_the_field() {
    for start in ["field", "list", "button"] {
        for send in ["Enter", "Ctrl+Shift+R"] {
            let mut app = app();
            app.backend.record_demo_commands();
            let mut trace = Trace::new(app);
            match start {
                "field" => focus_stop(&mut trace, Stop::Composer),
                "list" => focus_stop(&mut trace, Stop::Messages),
                _ => {
                    focus_stop(&mut trace, Stop::MessageActions);
                    trace.press("End", egui::Key::End, egui::Modifiers::NONE);
                    assert_eq!(trace.last().name, "Button \"Record a voice message\"");
                }
            }
            let from = trace.mark();
            if start == "button" {
                trace.press("Enter", egui::Key::Enter, egui::Modifiers::NONE);
            } else {
                trace.press("Ctrl+Shift+R", egui::Key::R, record_keys());
            }
            let scenario = format!("recorded from the {start}, sent with {send}");
            assert!(trace.app.recording.is_some(), "{scenario}");
            assert_eq!(trace.last().egui, recording_send(&trace), "{scenario}");
            record_for_a_while(&trace);
            if send == "Enter" {
                trace.press("Enter", egui::Key::Enter, egui::Modifiers::NONE);
            } else {
                trace.press("Ctrl+Shift+R", egui::Key::R, record_keys());
            }
            assert!(trace.app.recording.is_none(), "{scenario}");
            assert!(sent_voice(&trace), "{scenario}: sent");
            assert_eq!(
                trace.last().egui,
                Some(egui::Id::new("composer-text")),
                "{scenario}\n{}",
                trace.dump(from)
            );
            trace.assert_clean(from, &scenario);
        }
    }
}
