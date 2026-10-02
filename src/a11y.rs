//! Spoken labels for screen readers: the text NVDA reads for a message, built
//! from plain data so it can be tested without a UI.

// Task 5 wires these into the message list; until then only tests use them.
#![cfg_attr(not(test), allow(dead_code))]

use crate::i18n::{Locale, gettext, ngettext};
use crate::model::{Content, Delivery, Reaction};

/// Everything read for one message, in reading order.
pub struct MessageLabel<'a> {
    /// "Today", "Yesterday", weekday or date; only on a day's first message.
    pub day: Option<&'a str>,
    /// The notices drawn between the day and the message, such as who
    /// pinned a message then.
    pub notices: &'a [String],
    /// First unread message.
    pub unread: bool,
    /// Display name, or the translated "You".
    pub sender: &'a str,
    /// `content_description(..)`.
    pub body: &'a str,
    /// `crate::util::clock(timestamp)`.
    pub time: &'a str,
    /// `status_word(..)` for own messages.
    pub status: Option<&'a str>,
    /// Sender name and quoted plain text.
    pub reply_to: Option<(&'a str, &'a str)>,
    pub forwarded: bool,
    pub edited: bool,
    /// The star and pin marks beside the time.
    pub starred: bool,
    pub pinned: bool,
    /// (emoji, count), in display order.
    pub reactions: &'a [(String, usize)],
}

/// Joins the parts a screen reader announces for a message with ", ".
pub fn message_label(locale: Locale, label: &MessageLabel<'_>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(day) = label.day {
        parts.push(day.to_owned());
    }
    parts.extend(label.notices.iter().cloned());
    if label.unread {
        parts.push(gettext(locale, "Unread").into_owned());
    }
    parts.push(label.sender.to_owned());
    parts.push(label.body.to_owned());
    parts.push(label.time.to_owned());
    if let Some(status) = label.status {
        parts.push(status.to_owned());
    }
    if let Some((name, text)) = label.reply_to {
        parts.push(
            gettext(locale, "replying to {}: {}")
                .into_owned()
                .replacen("{}", name, 1)
                .replacen("{}", text, 1),
        );
    }
    if label.forwarded {
        parts.push(gettext(locale, "forwarded").into_owned());
    }
    if label.edited {
        parts.push(gettext(locale, "edited").into_owned());
    }
    if label.starred {
        parts.push(gettext(locale, "starred").into_owned());
    }
    if label.pinned {
        parts.push(gettext(locale, "pinned").into_owned());
    }
    if !label.reactions.is_empty() {
        let list = label
            .reactions
            .iter()
            .map(|(emoji, count)| format!("{emoji} {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(
            gettext(locale, "reactions: {}")
                .into_owned()
                .replacen("{}", &list, 1),
        );
    }
    parts.retain(|part| !part.is_empty());
    parts.join(", ")
}

/// An album, as its bubble draws it: how many photos and videos it holds,
/// then each caption. `captions` are plain text, as for
/// [`content_description`].
pub fn album_description(locale: Locale, photos: u32, videos: u32, captions: &[String]) -> String {
    let mut parts = Vec::new();
    if photos > 0 {
        parts.push(
            ngettext(locale, "{} photo", "{} photos", photos).replace("{}", &photos.to_string()),
        );
    }
    if videos > 0 {
        parts.push(
            ngettext(locale, "{} video", "{} videos", videos).replace("{}", &videos.to_string()),
        );
    }
    parts.extend(
        captions
            .iter()
            .filter(|caption| !caption.is_empty())
            .cloned(),
    );
    parts.join(", ")
}

/// What a message's content is, in words. `text` is the plain message text or
/// caption, with mentions and markup already resolved by the caller.
pub fn content_description(locale: Locale, content: &Content, text: &str) -> String {
    let join = |parts: Vec<String>| {
        parts
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let seconds = |seconds: &Option<u32>| seconds.map(crate::util::duration).unwrap_or_default();
    match content {
        Content::Text { .. } | Content::Interactive { .. } => text.to_owned(),
        Content::Image { .. } => join(vec![gettext(locale, "Photo").into_owned(), text.to_owned()]),
        Content::Video {
            seconds: length,
            gif,
            note,
            ..
        } => {
            let label = if *gif {
                gettext(locale, "GIF").into_owned()
            } else if *note {
                gettext(locale, "Video message").into_owned()
            } else {
                gettext(locale, "Video").into_owned()
            };
            join(vec![label, seconds(length), text.to_owned()])
        }
        Content::Audio {
            seconds: length,
            voice_note,
            ..
        } => {
            let label = if *voice_note {
                gettext(locale, "Voice message").into_owned()
            } else {
                gettext(locale, "Audio").into_owned()
            };
            join(vec![label, seconds(length)])
        }
        Content::Document { file_name, .. } => join(vec![
            gettext(locale, "Document").into_owned(),
            file_name.clone(),
            text.to_owned(),
        ]),
        Content::Sticker { .. } => gettext(locale, "Sticker").into_owned(),
        Content::StickerPack { name, .. } => join(vec![
            gettext(locale, "Sticker pack").into_owned(),
            name.clone(),
            text.to_owned(),
        ]),
        Content::Location { name, address, .. } => join(vec![
            gettext(locale, "Location").into_owned(),
            name.clone().unwrap_or_default(),
            address.clone().unwrap_or_default(),
        ]),
        Content::LiveLocation { ended: false, .. } => gettext(locale, "Live location").into_owned(),
        Content::Contact { display_name, .. } => join(vec![
            gettext(locale, "Contact").into_owned(),
            display_name.clone(),
        ]),
        Content::Poll {
            question, options, ..
        } => ngettext(
            locale,
            "Poll: {}, {} option",
            "Poll: {}, {} options",
            options.len() as u32,
        )
        .replacen("{}", question, 1)
        .replacen("{}", &options.len().to_string(), 1),
        // Placeholders and ended live locations already have a plain summary.
        Content::LiveLocation { .. }
        | Content::Revoked
        | Content::Unsupported { .. }
        | Content::PhoneOnly { .. } => content.summary(),
        // Only the message knows which way the call went: a row reads its
        // own through [`call_description`]. Without it, the call reads as
        // one made to this account.
        Content::Call {
            video,
            status,
            duration,
            group,
        } => call_description(
            locale,
            &crate::ui::calls::Entry {
                from_me: false,
                video: *video,
                status: *status,
                duration: *duration,
                group: *group,
            },
        ),
    }
}

/// A call in a chat, as its bubble draws it: the title, then the call's
/// length or what became of it.
pub fn call_description(locale: Locale, entry: &crate::ui::calls::Entry) -> String {
    let mut parts = vec![entry.title(locale)];
    parts.extend(entry.detail(locale));
    parts.join(", ")
}

/// Where an attachment stands when its file is not simply there to open, or
/// `None` once it is downloaded. `auto` says a download will start by itself,
/// so there is nothing to report yet. Failures read the text the bubble shows
/// for them, without the "click to retry" hint a pointer user needs.
pub fn attachment_state(locale: Locale, content: &Content, auto: bool) -> Option<String> {
    use crate::model::MediaState;
    let media = match content {
        Content::Image { media, .. }
        | Content::Video { media, .. }
        | Content::Audio { media, .. }
        | Content::Document { media, .. }
        | Content::Sticker { media, .. } => media,
        _ => return None,
    };
    if media.path.is_some() {
        return None;
    }
    match &media.state {
        MediaState::Downloading => Some(gettext(locale, "downloading").into_owned()),
        MediaState::Failed(error) => {
            let error = error.trim();
            Some(if error.is_empty() {
                gettext(locale, "download failed").into_owned()
            } else {
                error.to_owned()
            })
        }
        MediaState::Idle if !auto => Some(gettext(locale, "not downloaded").into_owned()),
        MediaState::Idle => None,
    }
}

/// The delivery state of an own message as a word; incoming messages have none.
pub fn status_word(locale: Locale, status: Delivery) -> Option<String> {
    Some(match status {
        Delivery::None => return None,
        Delivery::Pending => gettext(locale, "sending").into_owned(),
        Delivery::Sent => gettext(locale, "sent").into_owned(),
        Delivery::Delivered => gettext(locale, "delivered").into_owned(),
        Delivery::Read => gettext(locale, "read").into_owned(),
        Delivery::Played => gettext(locale, "played").into_owned(),
        Delivery::Failed => gettext(locale, "not sent").into_owned(),
    })
}

/// Reactions grouped by emoji, in order of first appearance.
pub fn reaction_counts(reactions: &[Reaction]) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for reaction in reactions {
        match counts
            .iter_mut()
            .find(|(emoji, _)| *emoji == reaction.emoji)
        {
            Some((_, count)) => *count += 1,
            None => counts.push((reaction.emoji.clone(), 1)),
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Locale;
    use crate::model::{Media, MediaState};

    fn media() -> Media {
        Media {
            mime: "application/octet-stream".into(),
            size: 1,
            width: None,
            height: None,
            path: None,
            state: MediaState::Idle,
            album: None,
        }
    }

    fn document(state: MediaState, path: Option<&str>) -> Content {
        Content::Document {
            media: Media {
                path: path.map(Into::into),
                state,
                ..media()
            },
            file_name: "report.pdf".into(),
            caption: None,
            pages: None,
        }
    }

    #[test]
    fn attachment_states_read_as_the_bubble_shows_them() {
        let en = Locale::default();
        let state = |content: &Content, auto| attachment_state(en, content, auto);
        assert_eq!(
            state(&document(MediaState::Idle, None), false).as_deref(),
            Some("not downloaded")
        );
        assert_eq!(
            state(&document(MediaState::Downloading, None), false).as_deref(),
            Some("downloading")
        );
        let limit = "This attachment is larger than the 64 MiB download limit";
        assert_eq!(
            state(&document(MediaState::Failed(limit.into()), None), false).as_deref(),
            Some(limit)
        );
        assert_eq!(
            state(&document(MediaState::Failed("  ".into()), None), false).as_deref(),
            Some("download failed")
        );
        // Downloaded, or about to download by itself: nothing to add.
        assert_eq!(
            state(&document(MediaState::Idle, Some("a.pdf")), false),
            None
        );
        assert_eq!(
            state(
                &document(MediaState::Failed("x".into()), Some("a.pdf")),
                false
            ),
            None
        );
        assert_eq!(state(&document(MediaState::Idle, None), true), None);
        // A text message has no attachment.
        assert_eq!(state(&Content::text("hi"), false), None);
    }

    fn base<'a>() -> MessageLabel<'a> {
        MessageLabel {
            day: None,
            notices: &[],
            unread: false,
            sender: "Ahmed",
            body: "see you at 5",
            time: "10:32 PM",
            status: None,
            reply_to: None,
            forwarded: false,
            edited: false,
            starred: false,
            pinned: false,
            reactions: &[],
        }
    }

    #[test]
    fn reads_sender_text_time_status_then_extras() {
        let en = Locale::default();
        assert_eq!(message_label(en, &base()), "Ahmed, see you at 5, 10:32 PM");
        let reactions = [("👍".to_owned(), 2), ("❤️".to_owned(), 1)];
        let notices = ["Ada pinned a message".to_owned()];
        let full = MessageLabel {
            day: Some("Yesterday"),
            notices: &notices,
            unread: true,
            sender: "You",
            status: Some("read"),
            reply_to: Some(("Sara", "dinner?")),
            forwarded: true,
            edited: true,
            starred: true,
            pinned: true,
            reactions: &reactions,
            ..base()
        };
        assert_eq!(
            message_label(en, &full),
            "Yesterday, Ada pinned a message, Unread, You, see you at 5, 10:32 PM, read, replying to Sara: dinner?, forwarded, edited, starred, pinned, reactions: 👍 2, ❤️ 1"
        );
    }

    #[test]
    fn describes_media_and_placeholders() {
        let en = Locale::default();
        let voice = Content::Audio {
            media: media(),
            seconds: Some(42),
            voice_note: true,
            waveform: vec![],
        };
        assert_eq!(content_description(en, &voice, ""), "Voice message, 0:42");
        let photo = Content::Image {
            caption: Some("beach".into()),
            media: media(),
            motion: None,
        };
        assert_eq!(content_description(en, &photo, "beach"), "Photo, beach");
        assert_eq!(
            content_description(en, &Content::Revoked, ""),
            "This message was deleted"
        );
        // An album counts its photos and videos, then reads each caption.
        let captions = ["beach".to_owned(), String::new()];
        assert_eq!(
            album_description(en, 3, 1, &captions),
            "3 photos, 1 video, beach"
        );
        assert_eq!(album_description(en, 2, 0, &[]), "2 photos");
    }

    #[test]
    fn describes_other_content() {
        let en = Locale::default();
        let video = Content::Video {
            caption: Some("hi".into()),
            media: media(),
            seconds: Some(42),
            gif: false,
            note: false,
        };
        assert_eq!(content_description(en, &video, "hi"), "Video, 0:42, hi");
        let document = Content::Document {
            media: media(),
            file_name: "plan.pdf".into(),
            caption: None,
            pages: None,
        };
        assert_eq!(content_description(en, &document, ""), "Document, plan.pdf");
        assert_eq!(
            content_description(en, &document, "draft"),
            "Document, plan.pdf, draft"
        );
        let poll = Content::Poll {
            question: "Lunch?".into(),
            options: vec!["Yes".into(), "No".into()],
            state: Default::default(),
        };
        assert_eq!(
            content_description(en, &poll, ""),
            "Poll: Lunch?, 2 options"
        );
        let text = Content::Text {
            text: "hello".into(),
            preview: None,
        };
        assert_eq!(content_description(en, &text, "hello"), "hello");
    }

    #[test]
    fn status_words() {
        let en = Locale::default();
        assert_eq!(status_word(en, Delivery::None), None);
        assert_eq!(
            status_word(en, Delivery::Pending).as_deref(),
            Some("sending")
        );
        assert_eq!(
            status_word(en, Delivery::Failed).as_deref(),
            Some("not sent")
        );
        assert_eq!(status_word(en, Delivery::Played).as_deref(), Some("played"));
    }

    #[test]
    fn groups_reactions_by_first_appearance() {
        let reaction = |emoji: &str| Reaction {
            sender: "x".into(),
            from_me: false,
            emoji: emoji.into(),
        };
        let all = [reaction("👍"), reaction("❤️"), reaction("👍")];
        assert_eq!(
            reaction_counts(&all),
            vec![("👍".to_owned(), 2), ("❤️".to_owned(), 1)]
        );
    }

    #[test]
    fn a_call_reads_as_its_bubble_draws_it() {
        use crate::model::CallStatus;
        use crate::ui::calls::Entry;
        let en = Locale::default();
        let call = |from_me, status, duration| Entry {
            from_me,
            video: false,
            status,
            duration,
            group: false,
        };
        assert_eq!(
            call_description(en, &call(false, CallStatus::Missed, 0)),
            "Missed voice call"
        );
        // An own call WhatsApp filed as missed is not one we missed.
        assert_eq!(
            call_description(en, &call(true, CallStatus::Missed, 0)),
            "Voice call"
        );
        assert_eq!(
            call_description(en, &call(true, CallStatus::Answered, 754)),
            "Voice call, 12 min 34 s"
        );
        assert_eq!(
            call_description(en, &call(true, CallStatus::NoAnswer, 0)),
            "Voice call, No answer"
        );
        let content = Content::Call {
            video: true,
            status: CallStatus::Declined,
            duration: 0,
            group: true,
        };
        assert_eq!(
            content_description(en, &content, ""),
            "Group video call, Declined"
        );
    }
}
