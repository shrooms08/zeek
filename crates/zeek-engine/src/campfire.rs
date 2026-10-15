//! Campfire: the messages carried by digs and shouts (rules v4), read from
//! the vault's received notes.
//!
//! Messages are public on chain and permanent; the engine never deletes
//! one. It applies a **display filter** only: a configurable wordlist and a
//! few spam heuristics. A filtered message is still returned, with
//! `hidden: true`, the reason, and its text omitted unless the caller asks
//! for hidden text (`include_hidden=1`).
//!
//! Nickname syntax: a message that starts with `[name] `, where `name` is 1
//! to 16 ASCII letters, digits, `_` or `-`, is shown with that nickname and
//! the rest as its text. Nicknames are self-chosen and unverified.

use std::collections::VecDeque;
use std::path::Path;

use serde::{Deserialize, Serialize};
use zeek_core::memo::ZeekMemo;
use zeek_core::params::GameParams;
use zeek_core::rounds;

use crate::db::{Db, NoteRow};
use crate::error::EngineError;

/// Environment variable naming the filter file.
pub const FILTER_VAR: &str = "ZEEK_CAMPFIRE_FILTER";
/// Default filter file.
pub const DEFAULT_FILTER_PATH: &str = "config/campfire-filter.json";
/// Most messages one request returns.
pub const MAX_LIMIT: usize = 100;

/// Where a part of the wordlist comes from (attribution).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterSource {
    pub name: String,
    pub url: String,
    pub license: String,
    pub words: usize,
}

/// The display filter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filter {
    /// Attribution of the wordlist.
    #[serde(default)]
    pub sources: Vec<FilterSource>,
    /// Words and phrases (case-insensitive, matched as whole words) that
    /// hide a message.
    pub words: Vec<String>,
    /// Hide messages containing links (`http://`, `https://`, `www.`).
    pub hide_links: bool,
    /// Hide messages repeating one character more than this many times in a row.
    pub max_repeated_char: usize,
    /// Hide messages whose letters are more than this percent upper case...
    pub max_uppercase_percent: usize,
    /// ...when they have at least this many letters.
    pub uppercase_min_letters: usize,
    /// Hide a message repeating one of this many newer messages verbatim.
    pub duplicate_window: usize,
}

impl Default for Filter {
    fn default() -> Self {
        Filter {
            sources: Vec::new(),
            words: Vec::new(),
            hide_links: true,
            max_repeated_char: 10,
            max_uppercase_percent: 80,
            uppercase_min_letters: 16,
            duplicate_window: 20,
        }
    }
}

impl Filter {
    /// Loads the filter from `path` (JSON).
    pub fn load(path: &Path) -> Result<Self, EngineError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| EngineError::Config(format!("campfire filter {}: {e}", path.display())))?;
        serde_json::from_str(&text)
            .map_err(|e| EngineError::Config(format!("campfire filter {}: {e}", path.display())))
    }

    /// The filter named by [`FILTER_VAR`], else the default file if present,
    /// else an empty wordlist with the default heuristics.
    pub fn from_env() -> Result<Self, EngineError> {
        match std::env::var(FILTER_VAR) {
            Ok(path) => Self::load(Path::new(&path)),
            Err(_) if Path::new(DEFAULT_FILTER_PATH).exists() => {
                Self::load(Path::new(DEFAULT_FILTER_PATH))
            }
            Err(_) => Ok(Self::default()),
        }
    }

    /// Why `text` is hidden, if it is.
    pub fn reason(&self, text: &str) -> Option<String> {
        // Invisible and direction-changing characters can make a message
        // read differently from what it says, or slip past the word list
        // (security review L4).
        if text.chars().any(is_invisible_or_bidi) {
            return Some("contains invisible or text-direction characters".into());
        }
        let lower = text.to_lowercase();
        if let Some(word) = self
            .words
            .iter()
            .find(|w| !w.is_empty() && contains_word(&lower, &w.to_lowercase()))
        {
            return Some(format!("contains a filtered word ({word})"));
        }
        if self.hide_links
            && ["http://", "https://", "www."]
                .iter()
                .any(|l| lower.contains(l))
        {
            return Some("contains a link".into());
        }
        let mut run = 0usize;
        let mut previous = None;
        for c in text.chars() {
            run = if Some(c) == previous { run + 1 } else { 1 };
            previous = Some(c);
            if run > self.max_repeated_char {
                return Some(format!(
                    "repeats one character more than {} times",
                    self.max_repeated_char
                ));
            }
        }
        let letters: Vec<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
        if letters.len() >= self.uppercase_min_letters {
            let upper = letters.iter().filter(|c| c.is_uppercase()).count();
            if upper * 100 > letters.len() * self.max_uppercase_percent {
                return Some("mostly capital letters".into());
            }
        }
        None
    }
}

/// Unicode format characters a reader cannot see: zero-width characters, the
/// bidirectional embeddings, overrides and isolates, the word joiner and
/// invisible operators, the byte order mark and the tag characters.
pub fn is_invisible_or_bidi(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{E0000}'..='\u{E007F}'
    )
}

/// Whether `needle` occurs in `haystack` as a whole word or phrase: the
/// characters around the match are not letters or digits (so a short entry
/// never hides a longer innocent word, e.g. "ass" does not hide "class").
pub fn contains_word(haystack: &str, needle: &str) -> bool {
    haystack.match_indices(needle).any(|(start, _)| {
        let before = haystack[..start].chars().next_back();
        let after = haystack[start + needle.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// One Campfire message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampfireMessage {
    /// `txid:action` of the first output carrying it, unique.
    pub id: String,
    pub round: u32,
    /// `"dig"` or `"shout"`.
    pub kind: String,
    /// The tiles dug by this transaction with this message (one dig
    /// transaction may cover several tiles, each memo carrying the same
    /// message); empty for a shout.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tiles: Vec<u8>,
    pub height: u32,
    /// Block time (Unix seconds).
    pub time: u64,
    /// Sum of the note values carrying it (public through the vault's
    /// viewing key, like the board).
    pub value_zat: u64,
    /// Self-chosen nickname, if the message uses the nickname syntax.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    /// The message (without the nickname prefix); omitted when hidden
    /// unless hidden text was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Splits a `[name] ` prefix off a message.
pub fn split_nickname(message: &str) -> (Option<String>, String) {
    if let Some(rest) = message.strip_prefix('[') {
        if let Some(end) = rest.find("] ") {
            let name = &rest[..end];
            let valid = (1..=16).contains(&name.len())
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if valid {
                return (Some(name.to_owned()), rest[end + 2..].to_owned());
            }
        }
    }
    (None, message.to_owned())
}

/// The message of one stored note, if it carries a valid one and the note
/// counts (a dig of at least `min_dig_zat`, a shout of at least
/// `min_shout_zat`).
fn message_of(params: &GameParams, note: &NoteRow) -> Option<CampfireMessage> {
    if !params.campfire_active(note.height) {
        return None;
    }
    let memo = ZeekMemo::decode_at(params, note.height, &note.memo).ok()?;
    let (kind, tile, minimum) = match &memo {
        ZeekMemo::Dig(d) => ("dig", Some(d.tile), params.min_dig_zat),
        ZeekMemo::Shout(_) => (
            "shout",
            None,
            params
                .campfire
                .as_ref()
                .map_or(u64::MAX, |c| c.min_shout_zat),
        ),
    };
    if note.value < minimum {
        return None;
    }
    let message = memo.message()?;
    let (nickname, text) = split_nickname(message);
    Some(CampfireMessage {
        id: format!("{}:{}", note.txid_hex(), note.action_index),
        round: rounds::round_for_height(params, note.height)?,
        kind: kind.to_owned(),
        tiles: tile.into_iter().collect(),
        height: note.height,
        time: note.block_time,
        value_zat: note.value,
        nickname,
        text: Some(text),
        hidden: false,
        reason: None,
    })
}

/// Messages newest first, mined below `before` (exclusive; `None` for the
/// newest), at most `limit`.
pub fn messages(
    db: &Db,
    params: &GameParams,
    filter: &Filter,
    before: Option<u32>,
    limit: usize,
    include_hidden: bool,
) -> Result<Vec<CampfireMessage>, EngineError> {
    let Some(campfire) = &params.campfire else {
        return Ok(Vec::new());
    };
    let limit = limit.clamp(1, MAX_LIMIT);
    // Scan down from the cursor, or from just above the stored tip.
    let tip_end = db.tip()?.map_or(0, |b| b.height.saturating_add(1));
    let mut end = before.map_or(tip_end, |b| b.min(tip_end));
    let mut out = Vec::new();
    // Recent texts (newest first) for the duplicate heuristic: a message is
    // a duplicate of a newer one, so the first copy stays visible.
    let mut seen: VecDeque<String> = VecDeque::new();
    let step = 2_000u32;
    while out.len() < limit && end > campfire.activation_height {
        let start = end.saturating_sub(step).max(campfire.activation_height);
        let mut notes = db.notes_in_range(start, end)?;
        notes.sort_by_key(|n| std::cmp::Reverse((n.height, n.action_index)));
        // One transaction's digs that carry the same message are one entry.
        let mut grouped: Vec<(String, CampfireMessage)> = Vec::new();
        for note in &notes {
            let Some(message) = message_of(params, note) else {
                continue;
            };
            let txid = note.txid_hex();
            match grouped.iter_mut().find(|(t, m)| {
                *t == txid
                    && m.kind == message.kind
                    && m.text == message.text
                    && m.nickname == message.nickname
            }) {
                Some((_, entry)) => {
                    entry.tiles.extend(message.tiles);
                    entry.tiles.sort_unstable();
                    entry.value_zat = entry.value_zat.saturating_add(message.value_zat);
                    if message.id < entry.id {
                        entry.id = message.id;
                    }
                }
                None => grouped.push((txid, message)),
            }
        }
        for (_, mut message) in grouped {
            let text = message.text.clone().unwrap_or_default();
            let mut reason = filter.reason(&text);
            if reason.is_none()
                && seen
                    .iter()
                    .take(filter.duplicate_window)
                    .any(|t| *t == text)
            {
                reason = Some("repeats a recent message".into());
            }
            seen.push_front(text);
            seen.truncate(filter.duplicate_window.max(1));
            if let Some(reason) = reason {
                message.hidden = true;
                message.reason = Some(reason);
                if !include_hidden {
                    message.text = None;
                }
            }
            out.push(message);
            if out.len() == limit {
                break;
            }
        }
        end = start;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nicknames_and_filters() {
        assert_eq!(
            split_nickname("[ember] hi"),
            (Some("ember".into()), "hi".into())
        );
        assert_eq!(
            split_nickname("[no space]hi"),
            (None, "[no space]hi".into())
        );
        assert_eq!(split_nickname("[way_too_long_nickname] x").0, None);
        let filter = Filter {
            words: vec!["giveaway".into()],
            ..Filter::default()
        };
        assert_eq!(filter.reason("warm night"), None);
        assert_eq!(
            filter.reason("FREE Giveaway now").as_deref(),
            Some("contains a filtered word (giveaway)")
        );
        assert_eq!(
            filter.reason("see https://x.y").as_deref(),
            Some("contains a link")
        );
        assert!(filter.reason("aaaaaaaaaaaaaaaa").is_some());
        assert_eq!(
            filter.reason("THIS IS ALL SHOUTING HERE").as_deref(),
            Some("mostly capital letters")
        );
        assert_eq!(filter.reason("Short CAPS"), None);
        for sneaky in [
            "gi\u{200B}veaway",
            "paid \u{202E}3 ZEC",
            "\u{2066}x\u{2069}",
            "a\u{FEFF}b",
        ] {
            assert_eq!(
                filter.reason(sneaky).as_deref(),
                Some("contains invisible or text-direction characters"),
                "{sneaky:?}"
            );
        }
        assert_eq!(
            filter.reason("café ☕ 你好"),
            None,
            "ordinary non-ASCII text is fine"
        );
        assert_eq!(filter.reason("giveaways galore"), None, "whole words only");
        assert!(contains_word("a free giveaway!", "giveaway"));
        assert!(!contains_word("classic", "ass"));
        assert!(contains_word("dm me now", "dm me"));
        // The shipped wordlist loads, cites its sources and hides abuse
        // without hiding ordinary words.
        let shipped = Filter::load(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/campfire-filter.json"
        )))
        .expect("shipped filter");
        assert!(shipped.words.len() > 400);
        assert!(shipped
            .sources
            .iter()
            .any(|s| s.license.starts_with("CC BY 4.0")));
        assert!(shipped
            .reason("what a classic assessment of the scunthorpe pass")
            .is_none());
        assert!(shipped.reason("free giveaway").is_some());
    }
}
