//! Spoken announcements for screen readers: two hidden live-region nodes
//! take turns, so a repeated message is still a change worth reading.

use egui::accesskit::{Live, Role};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    /// A new message in the open chat.
    Message,
    /// Someone started typing or recording in the open chat.
    Typing,
    /// One of our messages was sent or could not be.
    Sent,
    /// Confirmations, connection and history state.
    Status,
    /// Errors: read before anything else waiting.
    Error,
}

impl Category {
    /// Categories that would reveal who wrote or what, hidden while locked.
    fn private(self) -> bool {
        matches!(self, Self::Message | Self::Typing)
    }
}

#[derive(Default)]
pub struct Announcer {
    queue: Vec<(Category, String)>,
    slots: [(String, bool); 2],
    next: usize,
}

pub fn announcer_id(slot: usize) -> egui::Id {
    egui::Id::new(("announcer", slot))
}

impl Announcer {
    pub fn say(&mut self, category: Category, text: impl Into<String>) {
        let text = text.into();
        if !text.trim().is_empty() {
            self.queue.push((category, text));
        }
    }

    /// Emits both announcer nodes; call once per frame.
    pub fn show(&mut self, ctx: &egui::Context, private_allowed: bool) {
        let items: Vec<_> = std::mem::take(&mut self.queue)
            .into_iter()
            .filter(|(category, _)| private_allowed || !category.private())
            .collect();
        if !items.is_empty() {
            let assertive = items
                .iter()
                .any(|(category, _)| *category == Category::Error);
            let mut text = items
                .into_iter()
                .map(|(_, text)| text)
                .collect::<Vec<_>>()
                .join(". ");
            let slot = self.next;
            self.next = 1 - self.next;
            // A screen reader only speaks a live region whose text changed,
            // so an identical text gets an invisible zero-width space.
            if self.slots[slot].0 == text {
                text.push('\u{200B}');
            }
            self.slots[slot] = (text, assertive);
        }
        let ppp = f64::from(ctx.pixels_per_point());
        for (slot, (text, assertive)) in self.slots.iter().enumerate() {
            ctx.accesskit_node_builder(announcer_id(slot), |node| {
                node.set_role(Role::Label);
                node.set_value(text.clone());
                node.set_live(if *assertive {
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
        [&self.slots[0].0, &self.slots[1].0]
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
        assert_eq!(node.value(), Some("Copied. Could not send"));
        assert_eq!(node.live(), Some(egui::accesskit::Live::Assertive));
    }

    #[test]
    fn locked_app_drops_private_announcements() {
        let mut announcer = Announcer::default();
        announcer.say(Category::Message, "Ada: secret");
        announcer.say(Category::Typing, "Ada is typing");
        announcer.say(Category::Status, "Connection lost");
        frame(&mut announcer, false);
        assert_eq!(announcer.spoken()[0], "Connection lost");
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
}
