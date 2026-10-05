//! The main window's sections: the chat list with the rail beside it, the
//! open conversation, the chat info panel beside it while it is open, and
//! the conversation's composer. F6 and Shift+F6 move to the next and
//! previous section drawn now, landing where focus was last in the chat
//! list, the conversation and the info panel, and on the message field in
//! the composer. On the Calls page the chat list's place holds the call
//! history or the new-call list. Alt+1, Alt+2
//! and Alt+3 (Control on macOS, see `keys.rs`) go straight to the chat
//! list, the message list and the open chat's first unread message.
//!
//! A section is made of the Tab stops of `focus.rs`: it exists while one of
//! its stops is drawn, so a read-only chat has no composer section, and with
//! no chat open only the chat list's is left.

use egui::{Context, Id, Key};

use super::focus::{self, Stop};
use crate::app::App;
use crate::model::Action;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Chats,
    Conversation,
    /// The chat info panel, beside the conversation.
    Info,
    Composer,
}

/// F6 order.
const ORDER: [Section; 4] = [
    Section::Chats,
    Section::Conversation,
    Section::Info,
    Section::Composer,
];

impl Section {
    fn index(self) -> usize {
        ORDER
            .iter()
            .position(|section| *section == self)
            .unwrap_or_default()
    }

    /// Where the section is entered when nothing in it was focused yet: the
    /// chat list and the conversation on their list, the info panel on its
    /// Close button. The composer is always entered on the message field.
    fn entry(self) -> Stop {
        match self {
            Section::Chats => Stop::ChatList,
            Section::Conversation => Stop::Messages,
            Section::Info => Stop::InfoClose,
            Section::Composer => Stop::Composer,
        }
    }
}

/// Which section a Stop belongs to.
pub fn of(stop: Stop) -> Section {
    match stop {
        // The rail beside the list goes with it: its avatars are the list
        // while the list is hidden.
        Stop::Rail | Stop::ChatListActions | Stop::Search | Stop::Filters | Stop::ChatList => {
            Section::Chats
        }
        // The chat's search pane docks beside its messages, and searches them.
        Stop::ChatActions
        | Stop::PinBanner
        | Stop::ChatSearchClose
        | Stop::ChatSearchDate
        | Stop::ChatSearchField
        | Stop::ChatSearchDay
        | Stop::ChatSearchResults
        | Stop::Messages => Section::Conversation,
        Stop::InfoClose | Stop::InfoControl(_) => Section::Info,
        // The recorder takes the composer's place while a voice message
        // records.
        Stop::Composer | Stop::Recording | Stop::MessageActions => Section::Composer,
    }
}

/// The sections with a stop among `stops`, in F6 order.
pub fn present(stops: &[(Stop, Id)]) -> Vec<Section> {
    ORDER
        .into_iter()
        .filter(|section| stops.iter().any(|(stop, _)| of(*stop) == *section))
        .collect()
}

#[derive(Clone, Copy, Default)]
struct Memory {
    /// The stop focused last in each section, by `Section::index`.
    stops: [Option<Stop>; ORDER.len()],
    /// The section focus was in last, for F6 from a control that is not a
    /// stop, or from nothing at all.
    section: Option<Section>,
}

fn memory_id() -> Id {
    Id::new("main-sections")
}

/// Remembers `stop` as where its section was left.
pub fn remember(ctx: &Context, stop: Stop) {
    let section = of(stop);
    ctx.data_mut(|data| {
        let memory = data.get_temp_mut_or_default::<Memory>(memory_id());
        memory.stops[section.index()] = Some(stop);
        memory.section = Some(section);
    });
}

/// The stop focused last in `section`, if any.
pub fn target(ctx: &Context, section: Section) -> Option<Stop> {
    ctx.data(|data| data.get_temp::<Memory>(memory_id()))
        .and_then(|memory| memory.stops[section.index()])
}

/// Remembers where focus is, then takes F6 and Shift+F6. Runs before
/// `focus::begin`, while the stops drawn in the last pass are still known.
/// `active` is the main view's own Tab navigation: under a dialog, a menu or
/// a picker F6 is left alone, not consumed.
pub fn handle(app: &mut App, ctx: &Context, active: bool) {
    let stops = focus::stops(ctx);
    let focused = ctx.memory(|memory| memory.focused());
    let at = stops
        .iter()
        .find(|(_, id)| Some(*id) == focused)
        .map(|(stop, _)| *stop);
    if let Some(stop) = at {
        remember(ctx, stop);
    }
    if !active {
        return;
    }
    let mut steps = 0;
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            if let egui::Event::Key {
                key: Key::F6,
                pressed: true,
                modifiers,
                ..
            } = event
                && (!modifiers.any() || modifiers.shift_only())
            {
                steps += if modifiers.shift { -1 } else { 1 };
                false
            } else {
                true
            }
        });
    });
    if steps == 0 {
        return;
    }
    let current = at.map(of).or_else(|| {
        ctx.data(|data| data.get_temp::<Memory>(memory_id()))
            .and_then(|memory| memory.section)
    });
    if let Some(section) = next(&present(&stops), current, steps) {
        land(app, ctx, section, &stops);
    }
}

/// The present section `steps` away from `current` (by its sign only),
/// skipping absent ones. With no current section F6 starts at the first and
/// Shift+F6 at the last.
fn next(present: &[Section], current: Option<Section>, steps: i32) -> Option<Section> {
    let Some(current) = current else {
        return if steps > 0 {
            present.first().copied()
        } else {
            present.last().copied()
        };
    };
    let direction = steps.signum();
    (1..=ORDER.len() as i32)
        .map(|offset| {
            ORDER[(current.index() as i32 + offset * direction).rem_euclid(ORDER.len() as i32)
                as usize]
        })
        .find(|section| present.contains(section))
}

/// Moves focus into `section`: to its stop focused last, else its entry,
/// else its first stop drawn. The composer is always entered on its entry,
/// or on the recorder, its only stop while a voice message records.
fn land(app: &mut App, ctx: &Context, section: Section, stops: &[(Stop, Id)]) {
    // On the Calls page the list's place holds the call history or the
    // new-call list, which take focus as any stop does.
    let calls = app.page == crate::model::Page::Calls;
    // An empty chat list has no chat to stand on; Alt+1 only says so.
    let usable = |stop: &Stop| {
        stops.iter().any(|(drawn, _)| drawn == stop)
            && (*stop != Stop::ChatList || calls || app.current_list_chat().is_some())
    };
    // F6 to the composer is a way back to typing: the message field, not a
    // button of Message actions left there before.
    let remembered = (section != Section::Composer)
        .then(|| target(ctx, section))
        .flatten();
    let Some(stop) = remembered
        .filter(usable)
        .or(Some(section.entry()).filter(usable))
        .or_else(|| {
            stops
                .iter()
                .map(|(stop, _)| *stop)
                .find(|stop| of(*stop) == section && usable(stop))
        })
    else {
        return;
    };
    // Remembered now: a list takes focus only when it is next drawn, and a
    // second F6 before then moves on from here.
    remember(ctx, stop);
    match stop {
        // The lists take focus the way Alt+1 and Alt+2 give it, when they
        // are drawn, never on a row. Alt+1 leaves the Calls page for the
        // chats: there F6 stays, on the list drawn in that place.
        Stop::ChatList if !calls => app.actions.push(Action::FocusChats),
        Stop::Messages => app.actions.push(Action::FocusMessages),
        _ => {
            if let Some((_, id)) = stops.iter().find(|(drawn, _)| *drawn == stop) {
                ctx.memory_mut(|memory| memory.request_focus(*id));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stop_has_its_section() {
        for (stop, section) in [
            (Stop::Rail, Section::Chats),
            (Stop::ChatListActions, Section::Chats),
            (Stop::Search, Section::Chats),
            (Stop::Filters, Section::Chats),
            (Stop::ChatList, Section::Chats),
            (Stop::ChatActions, Section::Conversation),
            (Stop::ChatSearchField, Section::Conversation),
            (Stop::PinBanner, Section::Conversation),
            (Stop::InfoClose, Section::Info),
            (Stop::InfoControl(0), Section::Info),
            (Stop::Messages, Section::Conversation),
            (Stop::Composer, Section::Composer),
            (Stop::Recording, Section::Composer),
            (Stop::MessageActions, Section::Composer),
        ] {
            assert_eq!(of(stop), section, "{stop:?}");
        }
    }

    #[test]
    fn present_sections_follow_the_drawn_stops_in_f6_order() {
        let drawn = |stops: &[Stop]| {
            stops
                .iter()
                .map(|stop| (*stop, Id::new(format!("{stop:?}"))))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            present(&drawn(&[Stop::Composer, Stop::Messages, Stop::Search])),
            [Section::Chats, Section::Conversation, Section::Composer]
        );
        assert_eq!(
            present(&drawn(&[Stop::Search, Stop::ChatList])),
            [Section::Chats]
        );
        assert!(present(&[]).is_empty());
    }

    #[test]
    fn next_wraps_and_skips_absent_sections() {
        use Section::*;
        let all = [Chats, Conversation, Composer];
        for (present, current, steps, expected) in [
            (&all[..], Some(Chats), 1, Some(Conversation)),
            (&all[..], Some(Composer), 1, Some(Chats)),
            (&all[..], Some(Chats), -1, Some(Composer)),
            (&all[..], None, 1, Some(Chats)),
            (&all[..], None, -1, Some(Composer)),
            (
                &[Chats, Conversation][..],
                Some(Conversation),
                1,
                Some(Chats),
            ),
            // Gone since: the next one after where it was.
            (&[Chats, Conversation][..], Some(Composer), 1, Some(Chats)),
            (
                &[Chats, Conversation][..],
                Some(Composer),
                -1,
                Some(Conversation),
            ),
            (&[Chats][..], Some(Chats), 1, Some(Chats)),
            (&[Chats][..], Some(Chats), -1, Some(Chats)),
            // The info panel, between the conversation and the composer.
            (
                &[Chats, Conversation, Info, Composer][..],
                Some(Conversation),
                1,
                Some(Info),
            ),
            (
                &[Chats, Conversation, Info, Composer][..],
                Some(Info),
                1,
                Some(Composer),
            ),
            (
                &[Chats, Conversation, Info, Composer][..],
                Some(Composer),
                -1,
                Some(Info),
            ),
            (&[][..], Some(Chats), 1, None),
            (&[][..], None, 1, None),
        ] {
            assert_eq!(
                next(present, current, steps),
                expected,
                "{present:?} from {current:?} by {steps}"
            );
        }
    }
}
