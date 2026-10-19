//! Spoken announcements for screen readers: two hidden live-region nodes
//! take turns, so a repeated message is still a change worth reading. Their
//! text leaves the tree a few seconds after it is set, so object navigation
//! does not find old announcements as if they were current.

use std::time::{Duration, Instant};

use egui::accesskit::{Live, Role};

/// How long an announcement waits to be spoken: as long as an info toast
/// stays on screen. eframe draws no frame for a minimized, covered or
/// hidden window, so nothing is spoken then; what is older than this when
/// a frame comes is stale and dropped. An error waits, as its toast stays
/// on screen until it is closed.
const FRESH: Duration = crate::app::INFO_TOAST_LIFETIME;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    /// A new message in the open chat.
    Message,
    /// Someone started typing or recording in the open chat.
    Typing,
    /// One of our messages was sent or could not be.
    Sent,
    /// Connection and history state.
    Status,
    /// A confirmation shown as a toast.
    Toast,
    /// Errors: read before anything else waiting.
    Error,
    /// What the message list's keys reach inside a message: a link, a
    /// phone number, a card's text, a file's name or a poll option.
    Item,
    /// Status that names a chat, such as where a recording goes on.
    ChatStatus,
}

/// Whether a `category` item said at `at` is still worth speaking `now`.
fn fresh(category: Category, at: Instant, now: Instant) -> bool {
    category == Category::Error || now.saturating_duration_since(at) < FRESH
}

impl Category {
    /// Categories hidden while locked and cleared once another account is
    /// on screen: messages, typing, a message's items and chat status reveal
    /// who wrote or what, or with whom, and toasts, errors included, are not
    /// drawn on the lock screen and can name a contact or an account.
    fn private(self) -> bool {
        matches!(
            self,
            Self::Message
                | Self::Typing
                | Self::Item
                | Self::ChatStatus
                | Self::Toast
                | Self::Error
        )
    }
}

/// How long an announcement stays in the tree before its node is emptied.
/// NVDA reads a live region's text when it handles the change event, not
/// when it finishes speaking, so this only has to outlast that delay: it is
/// usually a few milliseconds, but NVDA waiting on a busy window has been
/// seen to fall about two seconds behind (G17). Five seconds covers that
/// with room to spare, and is short enough that object navigation soon
/// stops finding the text.
pub const CLEAR_AFTER: Duration = Duration::from_secs(5);

/// One live-region node's announcement.
#[derive(Default)]
struct Slot {
    text: String,
    assertive: bool,
    /// Whether it came from a private category.
    private: bool,
    /// Who it says is typing, by chat and sender, when typing is all it
    /// says: it goes once they stop.
    typers: Vec<(String, String)>,
    /// When it was set, so it is emptied `CLEAR_AFTER` later.
    since: Option<Instant>,
}

/// An announcement waiting for the next `show`.
struct Item {
    category: Category,
    text: String,
    /// The chat and sender of a typing announcement.
    typer: Option<(String, String)>,
    /// When it was said, so it can go stale.
    at: Instant,
}

#[derive(Default)]
pub struct Announcer {
    queue: Vec<Item>,
    slots: [Slot; 2],
    next: usize,
}

pub fn announcer_id(slot: usize) -> egui::Id {
    egui::Id::new(("announcer", slot))
}

impl Announcer {
    pub fn say(&mut self, category: Category, text: impl Into<String>) {
        self.push(category, text.into(), None);
    }

    /// Says that `sender` is typing or recording in `chat`; the text goes
    /// from the tree as soon as `typing_stopped` says they stopped.
    pub fn say_typing(&mut self, chat: &str, sender: &str, text: impl Into<String>) {
        let typer = Some((chat.to_owned(), sender.to_owned()));
        self.push(Category::Typing, text.into(), typer);
    }

    fn push(&mut self, category: Category, text: String, typer: Option<(String, String)>) {
        if !text.trim().is_empty() {
            let now = Instant::now();
            // While no frame speaks them, stale items must not pile up.
            self.queue.retain(|item| fresh(item.category, item.at, now));
            self.queue.push(Item {
                category,
                text,
                typer,
                at: now,
            });
        }
    }

    /// `sender` stopped typing in `chat`: "Ada is typing" is no longer
    /// true, so it is not spoken if still waiting, and a node saying only
    /// that someone types is emptied (any other typer it names was already
    /// heard). A node that also says something else keeps it until it is
    /// emptied with time, since changing it would have it read again.
    pub fn typing_stopped(&mut self, chat: &str, sender: &str) {
        let stopped = |typer: &(String, String)| typer.0 == chat && typer.1 == sender;
        self.queue
            .retain(|item| !item.typer.as_ref().is_some_and(stopped));
        for slot in &mut self.slots {
            if slot.typers.iter().any(stopped) {
                *slot = Slot::default();
            }
        }
    }

    /// Drops the private text waiting to be spoken and empties the slots
    /// that hold some, so it is no longer readable in the tree: behind the
    /// lock screen, or once another account is on screen.
    pub fn forget_private(&mut self) {
        self.queue.retain(|item| !item.category.private());
        for slot in &mut self.slots {
            if slot.private {
                *slot = Slot::default();
            }
        }
    }

    /// Emits both announcer nodes; call once per frame.
    pub fn show(&mut self, ctx: &egui::Context, private_allowed: bool) {
        self.show_at(ctx, private_allowed, Instant::now());
    }

    /// `show` at `now`, which tests pass so they need no sleeping.
    pub fn show_at(&mut self, ctx: &egui::Context, private_allowed: bool, now: Instant) {
        let items: Vec<_> = std::mem::take(&mut self.queue)
            .into_iter()
            .filter(|item| {
                fresh(item.category, item.at, now) && (private_allowed || !item.category.private())
            })
            .collect();
        if !private_allowed {
            // Text spoken before the lock must not stay readable in the tree
            // behind the lock screen, nor come back after unlocking.
            self.forget_private();
        }
        // Text read long enough ago leaves the tree.
        for slot in &mut self.slots {
            if slot
                .since
                .is_some_and(|since| now.saturating_duration_since(since) >= CLEAR_AFTER)
            {
                *slot = Slot::default();
            }
        }
        if !items.is_empty() {
            // Errors first, each category in the order it was said.
            let mut items = items;
            items.sort_by_key(|item| item.category != Category::Error);
            let private = items.iter().any(|item| item.category.private());
            let assertive = items.iter().any(|item| item.category == Category::Error);
            let typers = if items.iter().all(|item| item.category == Category::Typing) {
                items.iter().filter_map(|item| item.typer.clone()).collect()
            } else {
                Vec::new()
            };
            let mut text = items
                .into_iter()
                .map(|item| item.text)
                .collect::<Vec<_>>()
                .join(". ");
            let slot = self.next;
            self.next = 1 - self.next;
            // A screen reader only speaks a live region whose text changed,
            // so an identical text gets an invisible zero-width space.
            if self.slots[slot].text == text {
                text.push('\u{200B}');
            }
            self.slots[slot] = Slot {
                text,
                assertive,
                private,
                typers,
                since: Some(now),
            };
        }
        // Wake the window to empty a node once its time is up: nothing else
        // may come to draw a frame then.
        if let Some(since) = self.slots.iter().filter_map(|slot| slot.since).min() {
            ctx.request_repaint_after((since + CLEAR_AFTER).saturating_duration_since(now));
        }
        let ppp = f64::from(ctx.pixels_per_point());
        for (index, slot) in self.slots.iter().enumerate() {
            ctx.accesskit_node_builder(announcer_id(index), |node| {
                node.set_role(Role::Label);
                // An empty node has no value at all, not an empty one. A
                // label's value is its name, and AccessKit raises a live
                // region change only for a node that has a name, so emptying
                // the node is silent, and the next text, whatever it is,
                // still reads as a change.
                if !slot.text.is_empty() {
                    node.set_value(slot.text.clone());
                }
                node.set_live(if slot.assertive {
                    Live::Assertive
                } else {
                    Live::Polite
                });
                node.set_bounds(egui::accesskit::Rect {
                    x0: 0.0,
                    y0: 0.0,
                    x1: ppp,
                    y1: ppp,
                });
            });
        }
    }

    #[cfg(test)]
    pub fn spoken(&self) -> [&str; 2] {
        [&self.slots[0].text, &self.slots[1].text]
    }

    /// Whether each slot is assertive, as errors are.
    #[cfg(test)]
    pub fn assertive(&self) -> [bool; 2] {
        [self.slots[0].assertive, self.slots[1].assertive]
    }
}

/// How long a chat must stay quiet before its held messages are spoken.
pub const QUIET: Duration = Duration::from_millis(1000);
/// The longest a message waits, so a steady stream is still heard.
pub const MAX_HOLD: Duration = Duration::from_millis(2500);
/// How long a search must stay unchanged before its result count is spoken:
/// long enough to cover the gap between two typed letters.
pub const SEARCH_QUIET: Duration = Duration::from_millis(400);

/// A run of events, such as messages arriving or letters typed, that has
/// settled once `quiet` has passed since the latest of them, or `cap` since
/// the first, so a steady stream is still heard. Time comes from the caller,
/// so tests need no sleeping.
#[derive(Clone, Copy, Debug)]
pub struct Settle {
    first: Instant,
    last: Instant,
}

impl Settle {
    pub fn new(now: Instant) -> Self {
        Self {
            first: now,
            last: now,
        }
    }

    /// Another event of the run.
    pub fn touch(&mut self, now: Instant) {
        self.last = now;
    }

    /// When the run counts as settled.
    pub fn deadline(&self, quiet: Duration, cap: Duration) -> Instant {
        (self.last + quiet).min(self.first + cap)
    }
}

/// Incoming messages for the chat being read, held until the chat goes
/// quiet so that a burst is spoken as one line rather than one per message.
/// Time comes from the caller, so tests need no sleeping.
#[derive(Default)]
pub struct MessageHold {
    held: Option<Held>,
}

struct Held {
    chat: String,
    /// The sender's id while only one person wrote, with their spoken name.
    sender: Option<(String, String)>,
    /// The full line for the first message, spoken as is if no other comes.
    line: String,
    count: u32,
    settle: Settle,
}

/// What a hold has to say once it is due.
#[derive(Debug, PartialEq, Eq)]
pub enum Burst {
    /// A single message: its full line.
    One(String),
    /// Several messages: how many, and the sender's name if only one wrote.
    Many { count: u32, sender: Option<String> },
}

impl MessageHold {
    /// Holds a message for `chat`. A message for another chat replaces, and
    /// so drops, whatever the previous chat still held.
    pub fn push(&mut self, chat: &str, sender: &str, name: String, line: String, now: Instant) {
        if let Some(held) = &mut self.held
            && held.chat == chat
        {
            held.count += 1;
            held.settle.touch(now);
            if held.sender.as_ref().is_some_and(|(id, _)| id != sender) {
                held.sender = None;
            }
            return;
        }
        self.held = Some(Held {
            chat: chat.to_owned(),
            sender: Some((sender.to_owned(), name)),
            line,
            count: 1,
            settle: Settle::new(now),
        });
    }

    /// The held burst, once `now` has reached its `deadline`. A burst left
    /// unspoken for as long as an announcement stays fresh after that is
    /// dropped instead, as any stale announcement is: no frame is drawn for
    /// a minimized, covered or hidden window to speak it in time.
    pub fn due(&mut self, now: Instant) -> Option<Burst> {
        let deadline = self.deadline()?;
        if now < deadline {
            return None;
        }
        let held = self.held.take()?;
        if now.saturating_duration_since(deadline) >= FRESH {
            return None;
        }
        Some(if held.count == 1 {
            Burst::One(held.line)
        } else {
            Burst::Many {
                count: held.count,
                sender: held.sender.map(|(_, name)| name),
            }
        })
    }

    /// When `due` will next return something: `QUIET` after the latest
    /// message, or `MAX_HOLD` after the first, whichever comes sooner.
    pub fn deadline(&self) -> Option<Instant> {
        self.held
            .as_ref()
            .map(|held| held.settle.deadline(QUIET, MAX_HOLD))
    }

    /// The chat whose messages are held.
    pub fn chat(&self) -> Option<&str> {
        self.held.as_ref().map(|held| held.chat.as_str())
    }

    pub fn clear(&mut self) {
        self.held = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(announcer: &mut Announcer, private: bool) -> egui::FullOutput {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            announcer.show(ui.ctx(), private)
        });
        // Dropping unapplied texture deltas would panic.
        output.textures_delta.clear();
        output
    }

    impl Announcer {
        /// Makes everything waiting look `by` older.
        pub(crate) fn age(&mut self, by: Duration) {
            for item in &mut self.queue {
                item.at = item.at.checked_sub(by).unwrap_or(item.at);
            }
        }
    }

    #[test]
    fn stale_announcements_are_dropped_but_errors_wait() {
        let mut announcer = Announcer::default();
        announcer.say(Category::Status, "Connection lost");
        announcer.say(Category::Toast, "Copied");
        announcer.say(Category::Error, "Could not send");
        announcer.age(FRESH + Duration::from_millis(1));
        announcer.say(Category::Status, "Back online");
        frame(&mut announcer, true);
        assert_eq!(announcer.spoken()[0], "Could not send. Back online");
    }

    #[test]
    fn stale_items_do_not_pile_up_while_no_frame_speaks() {
        let mut announcer = Announcer::default();
        for _ in 0..100 {
            announcer.say(Category::Toast, "Copied");
            announcer.age(FRESH);
        }
        announcer.say(Category::Toast, "Copied");
        assert_eq!(announcer.queue.len(), 1);
    }

    fn frame_at(announcer: &mut Announcer, now: Instant) -> egui::FullOutput {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        // A context's first frames ask for more at once; the last, which
        // changes nothing, shows the wake-up asked for.
        let mut output = egui::FullOutput::default();
        for index in 0..3 {
            let input = egui::RawInput {
                time: Some(f64::from(index)),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 300.0),
                )),
                ..Default::default()
            };
            output = ctx.run_ui(input, |ui| announcer.show_at(ui.ctx(), true, now));
            output.textures_delta.clear();
        }
        output
    }

    /// Each announcer node's value in `output`'s tree, and so its name.
    fn values(output: &egui::FullOutput) -> [Option<String>; 2] {
        let tree = output.platform_output.accesskit_update.as_ref().unwrap();
        [0, 1].map(|slot| {
            let (_, node) = tree
                .nodes
                .iter()
                .find(|(id, _)| *id == announcer_id(slot).accesskit_id())
                .expect("both nodes every frame");
            assert_eq!(node.label(), None, "named by its value alone");
            node.value().map(str::to_owned)
        })
    }

    /// The wake-up `output` asks for, with the frame egui takes off it to
    /// be ready in time added back.
    fn repaint_delay(output: &egui::FullOutput) -> Duration {
        let delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        let frame = Duration::from_secs_f64(1.0 / 60.0);
        // Rounded to the millisecond, past egui's float arithmetic.
        let ms = (delay.saturating_add(frame).as_secs_f64() * 1000.0).round();
        if ms > 1e12 {
            Duration::MAX
        } else {
            Duration::from_millis(ms as u64)
        }
    }

    /// G19: an announcement leaves the tree `CLEAR_AFTER` after it was set,
    /// so object navigation does not find it later; one wake-up is asked
    /// for then, and none once both nodes are empty.
    #[test]
    fn text_leaves_the_tree_a_few_seconds_after_it_is_set() {
        let t0 = Instant::now();
        let mut announcer = Announcer::default();
        announcer.say(Category::Status, "Copied");
        let output = frame_at(&mut announcer, t0);
        assert_eq!(values(&output), [Some("Copied".into()), None]);
        assert_eq!(repaint_delay(&output), CLEAR_AFTER);
        let output = frame_at(&mut announcer, t0 + CLEAR_AFTER - ms(1));
        assert_eq!(values(&output), [Some("Copied".into()), None]);
        assert!(repaint_delay(&output) <= ms(17), "due at once");
        let output = frame_at(&mut announcer, t0 + CLEAR_AFTER);
        assert_eq!(values(&output), [None, None]);
        assert_eq!(announcer.spoken(), ["", ""]);
        assert_eq!(
            repaint_delay(&output),
            Duration::MAX,
            "nothing left to wake for"
        );
    }

    /// G19: emptying a node gives it no value, and so no name, which
    /// AccessKit never raises a live region change for: nothing is spoken.
    /// The other node is left as it is, so it is not read again, and the
    /// same text said again after the clear still reads as a change.
    #[test]
    fn emptying_a_node_speaks_nothing_and_reads_nothing_again() {
        let t0 = Instant::now();
        let mut announcer = Announcer::default();
        announcer.say(Category::Status, "Copied");
        frame_at(&mut announcer, t0);
        announcer.say(Category::Status, "Saved");
        let output = frame_at(&mut announcer, t0 + ms(3000));
        assert_eq!(
            values(&output),
            [Some("Copied".into()), Some("Saved".into())]
        );
        assert_eq!(
            repaint_delay(&output),
            ms(2000),
            "the older one is due first"
        );
        let output = frame_at(&mut announcer, t0 + CLEAR_AFTER);
        assert_eq!(
            values(&output),
            [None, Some("Saved".into())],
            "only the older node changed, and only to no value"
        );
        assert_eq!(repaint_delay(&output), ms(3000));
        let output = frame_at(&mut announcer, t0 + ms(8000));
        assert_eq!(values(&output), [None, None]);
        announcer.say(Category::Status, "Copied");
        // Said just before that frame, not 9 seconds earlier.
        announcer.queue[0].at = t0 + ms(9000);
        let output = frame_at(&mut announcer, t0 + ms(9000));
        assert_eq!(
            values(&output),
            [Some("Copied".into()), None],
            "no zero-width space needed after an empty node"
        );
    }

    /// G19: "Ada is typing" goes from the tree as soon as Ada stops, not
    /// when someone else does; if she stops before it is spoken, it is not
    /// spoken. A node that also says something else keeps it.
    #[test]
    fn typing_leaves_the_tree_when_the_typer_stops() {
        let t0 = Instant::now();
        let mut announcer = Announcer::default();
        announcer.say_typing("family", "ada", "Ada is typing");
        let output = frame_at(&mut announcer, t0);
        assert_eq!(values(&output), [Some("Ada is typing".into()), None]);
        announcer.typing_stopped("family", "grace");
        announcer.typing_stopped("work", "ada");
        let output = frame_at(&mut announcer, t0 + ms(100));
        assert_eq!(values(&output), [Some("Ada is typing".into()), None]);
        announcer.typing_stopped("family", "ada");
        let output = frame_at(&mut announcer, t0 + ms(200));
        assert_eq!(values(&output), [None, None]);

        // Stopped before the frame that would speak it.
        announcer.say_typing("family", "grace", "Grace is typing");
        announcer.typing_stopped("family", "grace");
        let output = frame_at(&mut announcer, t0 + ms(300));
        assert_eq!(values(&output), [None, None]);

        // Said together with a message: the message stays until its time.
        announcer.say(Category::Message, "Ada: hi");
        announcer.say_typing("family", "grace", "Grace is typing");
        let output = frame_at(&mut announcer, t0 + ms(400));
        let both = Some("Ada: hi. Grace is typing".to_owned());
        assert_eq!(values(&output)[1], both);
        announcer.typing_stopped("family", "grace");
        let output = frame_at(&mut announcer, t0 + ms(500));
        assert_eq!(values(&output)[1], both);
        let output = frame_at(&mut announcer, t0 + ms(400) + CLEAR_AFTER);
        assert_eq!(values(&output), [None, None]);
    }

    #[test]
    fn alternates_slots_and_repeats_identical_text() {
        let mut announcer = Announcer::default();
        announcer.say(Category::Status, "Copied");
        frame(&mut announcer, true);
        announcer.say(Category::Status, "Copied");
        frame(&mut announcer, true);
        announcer.say(Category::Status, "Copied");
        frame(&mut announcer, true);
        let [a, b] = announcer.spoken();
        assert_eq!(b, "Copied");
        assert_ne!(
            a, "Copied",
            "third repeat must differ from slot 0's old text"
        );
        assert_eq!(a.trim_end_matches('\u{200B}'), "Copied");
    }

    #[test]
    fn joins_a_frames_items_and_escalates_errors() {
        let mut announcer = Announcer::default();
        announcer.say(Category::Status, "Copied");
        announcer.say(Category::Error, "Could not send");
        let output = frame(&mut announcer, true);
        let tree = output.platform_output.accesskit_update.unwrap();
        let node = tree
            .nodes
            .iter()
            .find(|(id, _)| *id == announcer_id(0).accesskit_id())
            .map(|(_, node)| node)
            .unwrap();
        assert_eq!(node.value(), Some("Could not send. Copied"));
        assert_eq!(node.live(), Some(egui::accesskit::Live::Assertive));
    }

    #[test]
    fn locked_app_drops_private_announcements() {
        let mut announcer = Announcer::default();
        announcer.say(Category::Message, "Ada: secret");
        announcer.say(Category::Typing, "Ada is typing");
        announcer.say(Category::Toast, "Studio: Contact saved");
        announcer.say(Category::Error, "Could not send to Ada");
        announcer.say(Category::Status, "Connection lost");
        frame(&mut announcer, false);
        assert_eq!(announcer.spoken()[0], "Connection lost");
    }

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    #[test]
    fn a_single_held_message_is_spoken_once_the_chat_is_quiet() {
        let t0 = Instant::now();
        let mut hold = MessageHold::default();
        hold.push("ada", "ada", "Ada".into(), "Ada: hi".into(), t0);
        assert_eq!(hold.deadline(), Some(t0 + QUIET));
        assert_eq!(hold.due(t0 + ms(500)), None);
        assert_eq!(hold.due(t0 + ms(1000)), Some(Burst::One("Ada: hi".into())));
        assert_eq!(hold.due(t0 + ms(5000)), None);
        assert_eq!(hold.deadline(), None);
    }

    /// Frames stop while the window is minimized, covered or hidden: a
    /// burst whose time came that long ago is dropped, not spoken late.
    #[test]
    fn a_held_message_not_spoken_in_time_is_dropped() {
        let t0 = Instant::now();
        let mut hold = MessageHold::default();
        hold.push("ada", "ada", "Ada".into(), "Ada: hi".into(), t0);
        let deadline = hold.deadline().unwrap();
        assert_eq!(hold.due(deadline + FRESH), None);
        assert_eq!(hold.chat(), None, "the hold is gone");
        hold.push("ada", "ada", "Ada".into(), "Ada: hi".into(), t0);
        assert_eq!(
            hold.due(deadline + FRESH - ms(1)),
            Some(Burst::One("Ada: hi".into()))
        );
    }

    #[test]
    fn a_burst_waits_for_quiet_up_to_the_cap() {
        let t0 = Instant::now();
        let mut hold = MessageHold::default();
        // Five messages 300 ms apart: each one pushes the deadline back.
        for index in 0..5 {
            let at = t0 + ms(300 * index);
            assert_eq!(hold.due(at), None);
            hold.push("ada", "ada", "Ada".into(), format!("Ada: {index}"), at);
        }
        assert_eq!(hold.due(t0 + ms(1900)), None);
        assert_eq!(
            hold.due(t0 + ms(2200)),
            Some(Burst::Many {
                count: 5,
                sender: Some("Ada".into())
            })
        );

        // A steady stream is cut off MAX_HOLD after its first message, and
        // what follows starts a new hold.
        let mut hold = MessageHold::default();
        let mut spoken = Vec::new();
        for index in 0..10 {
            let at = t0 + ms(400 * index);
            spoken.extend(hold.due(at).map(|burst| (index, burst)));
            hold.push("ada", "ada", "Ada".into(), "Ada: more".into(), at);
        }
        assert_eq!(
            spoken,
            [(
                7,
                Burst::Many {
                    count: 7,
                    sender: Some("Ada".into())
                }
            )]
        );
        assert_eq!(hold.deadline(), Some(t0 + ms(3600) + QUIET));
    }

    #[test]
    fn mixed_senders_are_counted_and_another_chat_drops_the_hold() {
        let t0 = Instant::now();
        let mut hold = MessageHold::default();
        hold.push("family", "ada", "Ada".into(), "Ada in Family: a".into(), t0);
        hold.push("family", "grace", "Grace".into(), "x".into(), t0);
        hold.push("family", "ada", "Ada".into(), "x".into(), t0);
        assert_eq!(
            hold.due(t0 + QUIET),
            Some(Burst::Many {
                count: 3,
                sender: None
            })
        );
        hold.push("family", "ada", "Ada".into(), "Ada in Family: a".into(), t0);
        hold.push(
            "work",
            "grace",
            "Grace".into(),
            "Grace in Work: b".into(),
            t0,
        );
        assert_eq!(hold.chat(), Some("work"));
        assert_eq!(
            hold.due(t0 + QUIET),
            Some(Burst::One("Grace in Work: b".into()))
        );
    }

    #[test]
    fn a_run_settles_after_quiet_or_at_the_cap() {
        let t0 = Instant::now();
        let mut run = Settle::new(t0);
        assert_eq!(run.deadline(SEARCH_QUIET, MAX_HOLD), t0 + SEARCH_QUIET);
        run.touch(t0 + ms(300));
        assert_eq!(run.deadline(SEARCH_QUIET, MAX_HOLD), t0 + ms(700));
        run.touch(t0 + ms(2400));
        assert_eq!(run.deadline(SEARCH_QUIET, MAX_HOLD), t0 + MAX_HOLD);
    }

    #[test]
    fn nothing_new_keeps_both_nodes_every_frame() {
        let mut announcer = Announcer::default();
        let output = frame(&mut announcer, true);
        let tree = output.platform_output.accesskit_update.unwrap();
        for slot in 0..2 {
            assert!(
                tree.nodes
                    .iter()
                    .any(|(id, _)| *id == announcer_id(slot).accesskit_id())
            );
        }
    }

    #[test]
    fn locking_clears_text_spoken_before_it() {
        let mut announcer = Announcer::default();
        announcer.say(Category::Message, "Ada: secret");
        frame(&mut announcer, true);
        assert_eq!(announcer.spoken()[0], "Ada: secret");
        let output = frame(&mut announcer, false);
        assert_eq!(values(&output), [None, None], "emptied, silently");
        let tree = output.platform_output.accesskit_update.unwrap();
        assert!(
            tree.nodes
                .iter()
                .all(|(_, node)| node.value() != Some("Ada: secret"))
        );
        assert!(
            announcer
                .spoken()
                .iter()
                .all(|text| !text.contains("secret"))
        );
        // Unlocking does not bring it back.
        frame(&mut announcer, true);
        assert!(
            announcer
                .spoken()
                .iter()
                .all(|text| !text.contains("secret"))
        );
    }
}
