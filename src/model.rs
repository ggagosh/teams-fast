use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ChatSummary {
    pub id: String,
    pub title: String,
    pub preview: String,
    pub updated_at: String,
    pub preview_id: Option<String>,
    pub preview_mine: bool,
    /// Only an ordinary message preview can imply unread activity.
    pub preview_is_message: bool,
    pub hidden: bool,
    /// Absent from a successfully completed /me/chats listing; keep local user data.
    pub unavailable: bool,
    pub is_meeting: bool,
    /// None until a message is found or a complete Graph history proves it is activity-only.
    // Ignore v0.5.1's `has_messages`: unknownFutureValue system events poisoned that evidence.
    #[serde(rename = "has_chat_messages")]
    pub has_messages: Option<bool>,
    pub web_url: Option<String>,
    pub members: usize,
    pub avatar_user: Option<String>,
    /// The "Name (You)" chat, pinned first like Teams' Favorites.
    pub is_self: bool,
    pub read_at: Option<String>,
}

impl ChatSummary {
    pub fn keep_message_evidence(&mut self, previous: &Self) {
        if previous.has_messages == Some(true)
            || (self.has_messages.is_none()
                && self.preview_id == previous.preview_id
                && self.updated_at == previous.updated_at)
        {
            self.has_messages = previous.has_messages;
        }
    }

    pub fn old_meeting(&self) -> bool {
        // ponytail: local 30-day policy; replace if Graph exposes Teams' default-list eligibility.
        self.is_meeting
            && parse_time(&self.updated_at)
                .is_some_and(|at| OffsetDateTime::now_utc() - at >= time::Duration::days(30))
    }
}

pub(crate) struct Chat {
    pub summary: ChatSummary,
    pub messages: Vec<Message>,
    pub draft: String,
    pub loading: bool,
    pub loaded: bool,
    pub next_messages: Option<String>,
    pub send_error: Option<String>,
    pub unread: usize,
    pub revision: u64,
    pub message_versions: HashMap<String, u64>,
    pub pending: HashMap<String, PendingChange>,
    pub read_pending: Option<String>,
    pub read_attempted: Option<String>,
    pub read_revision: u64,
    pub history_cleared: u64,
    /// Last attempted history inspection, so failures wait for refresh instead of retrying forever.
    pub activity_check: Option<u64>,
    pub activity_loading: bool,
}

impl Chat {
    pub fn new(summary: ChatSummary) -> Self {
        Self {
            summary,
            messages: Vec::new(),
            draft: String::new(),
            loading: false,
            loaded: false,
            next_messages: None,
            send_error: None,
            unread: 0,
            revision: 0,
            message_versions: HashMap::new(),
            pending: HashMap::new(),
            read_pending: None,
            read_attempted: None,
            read_revision: 0,
            history_cleared: 0,
            activity_check: None,
            activity_loading: false,
        }
    }

    pub fn visible(&self) -> bool {
        !(self.summary.hidden || self.summary.unavailable) || self.has_local_work()
    }

    pub fn listed(&self, searching: bool) -> bool {
        self.visible()
            && (searching
                || !self.summary.old_meeting()
                || self.summary.has_messages != Some(false)
                || self.has_local_work())
    }

    fn has_local_work(&self) -> bool {
        !self.draft.is_empty() || self.messages.iter().any(|m| m.delivery != Delivery::Sent)
    }

    pub fn merge_messages(&mut self, messages: Vec<Message>) -> Vec<Message> {
        self.merge_messages_after(messages, None)
    }

    pub fn merge_messages_after(
        &mut self,
        messages: Vec<Message>,
        started_revision: Option<u64>,
    ) -> Vec<Message> {
        if messages
            .iter()
            .any(|message| message.delivery == Delivery::Sent && message.is_chat_message())
        {
            self.summary.has_messages = Some(true);
        }
        let mut indices: HashMap<String, usize> = self
            .messages
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id.clone(), i))
            .collect();
        let mut added = Vec::new();
        let mut changed = false;
        for message in messages {
            if started_revision.is_some_and(|start| {
                self.message_versions
                    .get(&message.id)
                    .is_some_and(|version| *version > start)
            }) {
                continue;
            }
            if let Some(index) = indices.get(&message.id) {
                let previous = &self.messages[*index];
                let previous_time = timestamp(if previous.modified_at.is_empty() {
                    &previous.created_at
                } else {
                    &previous.modified_at
                });
                let next_time = timestamp(if message.modified_at.is_empty() {
                    &message.created_at
                } else {
                    &message.modified_at
                });
                if next_time < previous_time
                    || (previous.deleted && !message.deleted && next_time == previous_time)
                {
                    continue;
                }
                changed |= self.messages[*index] != message;
                if self.messages[*index] != message {
                    self.message_versions
                        .insert(message.id.clone(), self.revision.wrapping_add(1));
                }
                self.messages[*index] = message;
            } else {
                changed = true;
                self.message_versions
                    .insert(message.id.clone(), self.revision.wrapping_add(1));
                indices.insert(message.id.clone(), self.messages.len());
                added.push(message.clone());
                self.messages.push(message);
            }
        }
        if !changed {
            return added;
        }
        self.revision = self.revision.wrapping_add(1);
        self.messages
            .sort_by_cached_key(|message| timestamp(&message.created_at));
        if let Some(latest) = self.messages.last()
            && timestamp(&latest.created_at) >= timestamp(&self.summary.updated_at)
        {
            if self.summary.has_messages == Some(false)
                && self.summary.preview_id.as_ref() != Some(&latest.id)
            {
                self.summary.has_messages = None;
            }
            self.summary.preview = latest.text.replace('\n', " ");
            self.summary.updated_at = latest.created_at.clone();
            self.summary.preview_id = Some(latest.id.clone());
            self.summary.preview_mine = latest.mine;
            self.summary.preview_is_message = latest.is_chat_message() && !latest.deleted;
        }
        added
    }

    /// Render optimistic changes without overwriting the latest server version.
    pub fn display_messages(&self) -> Vec<Message> {
        self.messages
            .iter()
            .filter(|message| {
                message.is_chat_message() || (message.system && !message.text.is_empty())
            })
            .cloned()
            .map(|mut message| {
                if let Some(pending) = self.pending.get(&message.id) {
                    pending.change.apply(&mut message);
                    message.updating = true;
                    message.update_uncertain = pending.uncertain;
                }
                message
            })
            .collect()
    }

    pub fn recount_unread(&mut self) {
        if self.summary.hidden || self.summary.unavailable || self.summary.is_self {
            self.unread = 0;
            return;
        }
        let Some(read_at) = self.read_pending.as_ref().or(self.summary.read_at.as_ref()) else {
            return; // Only known incoming messages can increment unread without a read marker.
        };
        let read_at = timestamp(read_at);
        self.unread = self
            .messages
            .iter()
            .filter(|m| {
                m.is_chat_message()
                    && !m.mine
                    && !m.deleted
                    && m.delivery == Delivery::Sent
                    && timestamp(&m.created_at) > read_at
            })
            .count();
        // Graph gives a read marker, not an unread count. History may be only partially cached.
        if self.summary.preview_is_message
            && !self.summary.preview_mine
            && timestamp(&self.summary.updated_at) > read_at
        {
            self.unread = self.unread.max(1);
        }
    }

    /// Replaces the local placeholder of a confirmed send with Microsoft's message.
    pub fn complete_send(&mut self, local_id: &str, message: Message) {
        self.remove_message(local_id);
        self.send_error = None;
        self.merge_messages(vec![message]);
    }

    pub fn remove_message(&mut self, id: &str) {
        let before = self.messages.len();
        self.messages.retain(|message| message.id != id);
        if self.messages.len() != before {
            self.revision = self.revision.wrapping_add(1);
            self.message_versions.insert(id.to_owned(), self.revision);
        }
    }
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Message {
    pub id: String,
    pub author: String,
    pub author_id: String,
    /// Plain text for previews, notifications, and copying.
    pub text: String,
    /// Display markdown for plain-text bodies; empty when `html` is used.
    pub markdown: String,
    /// Display HTML for formatted bodies (Teams tags already rewritten).
    pub html: String,
    /// https image sources inside `html`, fetched by the media lane.
    pub images: Vec<String>,
    /// First https link, shown as a link preview.
    pub link: Option<String>,
    pub files: Vec<File>,
    pub quote: Option<Quote>,
    pub reactions: Vec<Reaction>,
    pub created_at: String,
    pub modified_at: String,
    pub mine: bool,
    pub deleted: bool,
    #[serde(default)]
    pub system: bool,
    pub delivery: Delivery,
    #[serde(skip)]
    pub updating: bool,
    #[serde(skip)]
    pub update_uncertain: bool,
}

impl Message {
    pub fn is_chat_message(&self) -> bool {
        !self.system
            && (self.deleted
                || !self.text.trim().is_empty()
                || !self.images.is_empty()
                || !self.files.is_empty()
                || self.quote.is_some())
    }
}

/// Local send state only: `Sent` means accepted by Teams, not delivered/read by a recipient.
#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Delivery {
    #[default]
    Sent,
    Sending,
    /// The POST failed or timed out; Microsoft may still have accepted it.
    Unconfirmed,
}

#[derive(Clone)]
pub(crate) enum MessageChange {
    Reaction {
        kind: String,
        emoji: String,
        set: bool,
    },
    Edit(String),
    Delete,
}

pub(crate) struct PendingChange {
    pub id: u64,
    pub revision: u64,
    pub change: MessageChange,
    pub uncertain: bool,
}

impl MessageChange {
    pub fn apply(&self, message: &mut Message) {
        match self {
            Self::Reaction { kind, emoji, set } => {
                if let Some(i) = message.reactions.iter().position(|r| r.emoji == *emoji) {
                    let r = &mut message.reactions[i];
                    if r.mine != *set {
                        r.count = if *set {
                            r.count + 1
                        } else {
                            r.count.saturating_sub(1)
                        };
                        r.mine = *set;
                    }
                    if r.count == 0 {
                        message.reactions.remove(i);
                    }
                } else if *set {
                    message.reactions.push(Reaction {
                        kind: kind.clone(),
                        emoji: emoji.clone(),
                        count: 1,
                        mine: true,
                    });
                }
            }
            Self::Edit(text) => {
                message.text = text.clone();
                message.markdown.clear();
                message.html.clear();
                message.link = None;
            }
            Self::Delete => message.delete(),
        }
    }
}

impl Message {
    pub fn delete(&mut self) {
        self.deleted = true;
        self.text = "Message deleted".into();
        self.markdown = self.text.clone();
        self.html.clear();
        self.images.clear();
        self.link = None;
        self.files.clear();
        self.quote = None;
        self.reactions.clear();
    }

    pub fn time_label(&self, offset: UtcOffset) -> String {
        time_label(&self.created_at, offset)
    }
    pub fn grouped_after(&self, previous: &Self, offset: UtcOffset) -> bool {
        let same_author = if self.author_id.is_empty() {
            self.author == previous.author
        } else {
            self.author_id == previous.author_id
        };
        !self.system
            && !previous.system
            && same_author
            && self.mine == previous.mine
            && day_label(&self.created_at, offset) == day_label(&previous.created_at, offset)
            && match (
                parse_time(&self.created_at),
                parse_time(&previous.created_at),
            ) {
                (Some(a), Some(b)) => {
                    (a - b).whole_seconds() >= 0 && (a - b).whole_seconds() < 5 * 60
                }
                _ => false,
            }
    }
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct File {
    pub name: String,
    /// SharePoint/OneDrive web address; `None` opens the chat in Teams instead.
    pub url: Option<String>,
}

/// The message a reply quotes.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Quote {
    pub author: String,
    pub text: String,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Reaction {
    /// Graph `reactionType`, needed verbatim to remove a reaction.
    pub kind: String,
    pub emoji: String,
    pub count: usize,
    pub mine: bool,
}

/// Reactions offered in the message menu; Graph accepts the emoji itself as `reactionType`.
pub(crate) const QUICK_REACTIONS: [(&str, &str); 6] = [
    ("👍", "Like"),
    ("❤️", "Heart"),
    ("😆", "Laugh"),
    ("😮", "Surprised"),
    ("😢", "Sad"),
    ("😡", "Angry"),
];

/// Teams' legacy reaction names; newer reactions already are emoji.
pub(crate) fn reaction_emoji(kind: &str) -> &str {
    match kind {
        "like" => "👍",
        "heart" => "❤️",
        "laugh" => "😆",
        "surprised" => "😮",
        "sad" => "😢",
        "angry" => "😡",
        other => other,
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Preview {
    pub url: String,
    pub title: String,
    pub description: String,
    pub image: Option<String>,
    pub site: String,
}

#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Person {
    pub id: String,
    pub display_name: String,
    pub mail: Option<String>,
    pub user_principal_name: Option<String>,
}

pub(crate) fn escape_markdown(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        if character == '&' {
            escaped.push_str("&amp;");
            continue;
        }
        if r"\`*_{}[]()#+-.!|<>~".contains(character) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

pub(crate) fn message_markdown(text: &str) -> String {
    let mut result = String::new();
    let mut at = 0;
    let finder = linkify::LinkFinder::new();
    for link in finder
        .links(text)
        .filter(|link| link.kind() == &linkify::LinkKind::Url)
    {
        result.push_str(&escape_markdown(&text[at..link.start()]));
        let label = link_label(link.as_str());
        if reqwest::Url::parse(link.as_str())
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
        {
            let url = link.as_str().replace('(', "%28").replace(')', "%29");
            result.push_str(&format!("[{label}]({url})"));
        } else {
            result.push_str(&label);
        }
        at = link.end();
    }
    result.push_str(&escape_markdown(&text[at..]));
    result.replace('\n', "  \n")
}

pub(crate) fn link_label(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

pub(crate) fn parse_time(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339).ok()
}
pub(crate) fn timestamp(value: &str) -> i128 {
    parse_time(value).map_or(0, |time| time.unix_timestamp_nanos())
}
pub(crate) fn now_string() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default()
}
pub(crate) fn time_label(value: &str, offset: UtcOffset) -> String {
    parse_time(value)
        .map(|time| {
            let time = time.to_offset(offset);
            format!("{:02}:{:02}", time.hour(), time.minute())
        })
        .unwrap_or_default()
}
pub(crate) fn day_label(value: &str, offset: UtcOffset) -> String {
    parse_time(value)
        .map(|time| {
            let date = time.to_offset(offset).date();
            let today = OffsetDateTime::now_utc().to_offset(offset).date();
            if date == today {
                "Today".into()
            } else if Some(date) == today.previous_day() {
                "Yesterday".into()
            } else {
                format!("{} {}, {}", date.month(), date.day(), date.year())
            }
        })
        .unwrap_or_else(|| "Date unavailable".into())
}

#[derive(Default)]
pub(crate) struct NoticeHistory {
    ids: HashSet<(String, String)>,
    order: VecDeque<(String, String)>,
}
impl NoticeHistory {
    pub fn first(&mut self, chat: &str, message: &str) -> bool {
        let key = (chat.to_owned(), message.to_owned());
        if !self.ids.insert(key.clone()) {
            return false;
        }
        self.order.push_back(key);
        while self.order.len() > 2048 {
            if let Some(old) = self.order.pop_front() {
                self.ids.remove(&old);
            }
        }
        true
    }
}

pub(crate) fn demo_chats() -> Vec<Chat> {
    let mut chats: Vec<_> = [
        ("demo-design", "Product & design", 4, vec![
            ("Maya Chen", "I've updated the first chat screen. The sidebar should feel calmer now, with more room for the actual conversation.", false),
            ("Alex Morgan", "Looks good. Can we keep the composer visible when a message gets longer?", false),
            ("Alex Morgan", "And loading older messages should leave you exactly where you were reading.", false),
            ("You", "Yes. Clear navigation, a readable timeline, and drafts that stay with each conversation.", true),
            ("Maya Chen", "Perfect. I'll check the small-window layout next.\n\nThe API reference is here: https://learn.microsoft.com/graph/", false),
        ]),
        ("demo-engineering", "Engineering", 6, vec![
            ("Alex Morgan", "The relay forwards a change signal. Message content still comes directly from Microsoft.", false),
            ("You", "შეკითხვა", true),
            ("Alex Morgan", "We can test reconnection and notification routing with local demo data first.", false),
        ]),
        ("demo-maya", "Maya Chen", 2, vec![("Maya Chen", "The first review is ready whenever you are.", false)]),
        ("demo-release", "Release planning", 5, vec![("Nina Patel", "Let's check history, sending, and notifications together before the next build.", false)]),
        ("demo-notes", "Project notes", 2, vec![("TeamsFast", "These are local demo conversations. Connect your account to load your Teams chats.", false)]),
    ].into_iter().enumerate().map(|(chat_index, (id, title, members, messages))| {
        let mut chat = Chat::new(ChatSummary { id: id.into(), title: title.into(), members, ..Default::default() });
        chat.loaded = true;
        chat.merge_messages(messages.into_iter().enumerate().map(|(index, (author, text, mine))| Message {
            id: format!("{id}-{index}"), author: author.into(), author_id: author.into(), text: text.into(), mine,
            link: text.split_whitespace().find(|word| word.starts_with("https://")).map(str::to_owned),
            created_at: (OffsetDateTime::now_utc() - time::Duration::minutes(25 + (chat_index * 35) as i64 - index as i64)).format(&Rfc3339).unwrap_or_default(),
            ..Default::default()
        }).collect());
        chat
    }).collect();
    let at = |minutes| {
        (OffsetDateTime::now_utc() - time::Duration::minutes(minutes))
            .format(&Rfc3339)
            .unwrap_or_default()
    };
    let html = crate::html::teams_html(concat!(
        "<p>Release notes:</p>",
        "<ol><li><b>Formatted messages.</b> Teams HTML renders with bold, <i>italic</i>, lists, quotes, and <code>inline code</code>, keeping long items readable at full width.",
        "<ul><li>Nested bullets keep their indentation.</li><li>Mentions such as <at id=\"0\">Maya Chen</at> stand out.</li></ul></li>",
        "<li><b>Reactions</b> <emoji id=\"1f44d\" alt=\"👍\" title=\"Like\"></emoji> appear under the message.</li></ol>",
        "<p><img src=\"https://github.githubassets.com/images/modules/logos_page/GitHub-Mark.png\" width=\"96\" height=\"96\" alt=\"image\"></p>",
    ));
    let (images, link) = crate::html::media(&html);
    chats[0].merge_messages(vec![
        Message {
            id: "demo-design-formatted".into(),
            author: "Alex Morgan".into(),
            author_id: "Alex Morgan".into(),
            text: "Release notes draft, please review by Friday".into(),
            html,
            images,
            link,
            reactions: vec![
                Reaction { kind: "like".into(), emoji: "👍".into(), count: 2, mine: false },
                Reaction { kind: "❤️".into(), emoji: "❤️".into(), count: 1, mine: true },
            ],
            created_at: at(20),
            ..Default::default()
        },
        Message {
            id: "demo-design-file".into(),
            author: "Maya Chen".into(),
            author_id: "Maya Chen".into(),
            text: "Here is the spec we discussed.".into(),
            quote: Some(Quote {
                author: "You".into(),
                text: "Yes. Clear navigation, a readable timeline, and drafts that stay with each conversation.".into(),
            }),
            files: vec![File {
                name: "TeamsFast spec.pdf".into(),
                url: Some("https://example.com/teamsfast-spec.pdf".into()),
            }],
            created_at: at(19),
            ..Default::default()
        },
    ]);
    chats[0].next_messages = Some("demo-older".into());
    chats[1].unread = 2;
    chats
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_preserves_identity_order_and_a_draft_edited_during_send() {
        let mut chat = Chat::new(ChatSummary::default());
        let message = |id: &str, date: &str, text: &str| Message {
            id: id.into(),
            text: text.into(),
            created_at: date.into(),
            ..Default::default()
        };
        chat.merge_messages(vec![message("second", "2026-10-07T08:00:00.1Z", "old")]);
        chat.merge_messages(vec![
            message("first", "2026-10-07T12:00:00+04:00", "first"),
            message("second", "2026-10-07T08:00:00.1Z", "edited"),
        ]);
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[0].id, "first");
        assert_eq!(chat.messages[1].text, "edited");
        chat.merge_messages(vec![message(
            "local-1",
            "2026-10-07T09:00:00Z",
            "sent text",
        )]);
        chat.complete_send(
            "local-1",
            message("sent", "2026-10-07T09:00:00Z", "sent text"),
        );
        assert!(chat.messages.iter().all(|m| m.id != "local-1"));
        assert!(chat.messages.iter().any(|m| m.id == "sent"));
    }

    #[test]
    fn notifications_are_deduplicated_per_conversation_and_bounded() {
        let mut history = NoticeHistory::default();
        assert!(history.first("a", "1"));
        assert!(!history.first("a", "1"));
        assert!(history.first("b", "1"));
        for n in 0..3000 {
            history.first("a", &n.to_string());
        }
        assert!(history.ids.len() <= 2048);
    }
}
