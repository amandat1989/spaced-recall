//! JSON persistence for a deck of named cards.
//!
//! The on-disk shape is fixed and small (an object mapping card names to
//! card state), so this hand-rolls just enough of JSON to read and write
//! that shape rather than pulling in a general-purpose JSON crate.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

use crate::Card;

/// A named collection of cards, backed by a JSON file on disk.
///
/// `BTreeMap` keeps card names in a stable sorted order, so saving the same
/// deck twice produces byte-identical output and diffs cleanly in git.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Deck {
    pub cards: BTreeMap<String, Card>,
}

#[derive(Debug)]
pub enum DeckError {
    Io(std::io::Error),
    Parse(String),
    InvalidName(String),
}

impl fmt::Display for DeckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeckError::Io(e) => write!(f, "{e}"),
            DeckError::Parse(msg) => write!(f, "malformed deck file: {msg}"),
            DeckError::InvalidName(name) => write!(
                f,
                "card name \"{name}\" contains a tab or newline, which the plain text format uses as a field separator"
            ),
        }
    }
}

impl Error for DeckError {}

impl From<std::io::Error> for DeckError {
    fn from(e: std::io::Error) -> Self {
        DeckError::Io(e)
    }
}

impl Deck {
    pub fn new() -> Self {
        Deck::default()
    }

    /// Loads a deck from `path`. A missing file is treated as an empty deck,
    /// since a deck that has never been saved yet is not an error.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, DeckError> {
        match fs::read_to_string(path) {
            Ok(text) => Deck::from_json(&text).map_err(DeckError::Parse),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Deck::new()),
            Err(e) => Err(DeckError::Io(e)),
        }
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), DeckError> {
        fs::write(path, self.to_json())?;
        Ok(())
    }

    pub fn to_json(&self) -> String {
        let mut out = String::from("{\"cards\":{");
        for (i, (name, card)) in self.cards.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('"');
            escape_into(name, &mut out);
            out.push_str("\":{\"interval_days\":");
            out.push_str(&card.interval_days.to_string());
            out.push_str(",\"repetitions\":");
            out.push_str(&card.repetitions.to_string());
            out.push_str(",\"ease\":");
            out.push_str(&card.ease.to_string());
            out.push_str(",\"due_on\":");
            out.push_str(&card.due_on.to_string());
            out.push('}');
        }
        out.push_str("}}");
        out
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        let mut parser = JsonParser::new(text);
        let value = parser.parse_value()?;
        parser.skip_whitespace();
        if !parser.at_end() {
            return Err("trailing data after the top-level object".to_string());
        }
        deck_from_value(value)
    }

    /// Loads a deck from a plain text file, one card per line. See
    /// `to_text` for the format.
    pub fn import_text(path: impl AsRef<Path>) -> Result<Self, DeckError> {
        let text = fs::read_to_string(path)?;
        Deck::from_text(&text)
    }

    /// Saves a deck to a plain text file. See `to_text` for the format.
    pub fn export_text(&self, path: impl AsRef<Path>) -> Result<(), DeckError> {
        let text = self.to_text()?;
        fs::write(path, text)?;
        Ok(())
    }

    /// Renders the deck as plain text: one card per line, fields separated
    /// by tabs, in `name interval_days repetitions ease due_on` order.
    ///
    /// This exists alongside the JSON format for decks that get hand-edited
    /// or diffed line by line - a tab-separated line is easier to skim and
    /// patch in an editor than a JSON object is.
    pub fn to_text(&self) -> Result<String, DeckError> {
        let mut out = String::new();
        for (name, card) in &self.cards {
            if name.contains(['\t', '\n', '\r']) {
                return Err(DeckError::InvalidName(name.clone()));
            }
            out.push_str(name);
            out.push('\t');
            out.push_str(&card.interval_days.to_string());
            out.push('\t');
            out.push_str(&card.repetitions.to_string());
            out.push('\t');
            out.push_str(&card.ease.to_string());
            out.push('\t');
            out.push_str(&card.due_on.to_string());
            out.push('\n');
        }
        Ok(out)
    }

    /// Parses the plain text format written by `to_text`. Blank lines are
    /// skipped so a trailing newline at end of file is not an error.
    pub fn from_text(text: &str) -> Result<Self, DeckError> {
        let mut cards = BTreeMap::new();
        for (i, line) in text.lines().enumerate() {
            if line.is_empty() {
                continue;
            }
            let line_no = i + 1;
            let fields: Vec<&str> = line.split('\t').collect();
            let [name, interval_days, repetitions, ease, due_on] = fields.as_slice() else {
                return Err(DeckError::Parse(format!(
                    "line {line_no}: expected 5 tab-separated fields, got {}",
                    fields.len()
                )));
            };
            if name.is_empty() {
                return Err(DeckError::Parse(format!("line {line_no}: card name is empty")));
            }
            let card = Card {
                interval_days: parse_text_field(interval_days, "interval_days", line_no)?,
                repetitions: parse_text_field(repetitions, "repetitions", line_no)?,
                ease: parse_text_field(ease, "ease", line_no)?,
                due_on: parse_text_field(due_on, "due_on", line_no)?,
            };
            if cards.insert(name.to_string(), card).is_some() {
                return Err(DeckError::Parse(format!(
                    "line {line_no}: duplicate card name \"{name}\""
                )));
            }
        }
        Ok(Deck { cards })
    }
}

fn parse_text_field<T: std::str::FromStr>(
    field: &str,
    name: &str,
    line_no: usize,
) -> Result<T, DeckError> {
    field
        .parse()
        .map_err(|_| DeckError::Parse(format!("line {line_no}: field \"{name}\" is malformed")))
}

fn escape_into(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
}

enum JsonValue {
    Object(BTreeMap<String, JsonValue>),
    Number(f64),
    String(String),
}

fn deck_from_value(value: JsonValue) -> Result<Deck, String> {
    let mut top = match value {
        JsonValue::Object(map) => map,
        _ => return Err("expected the top-level value to be an object".to_string()),
    };
    let cards_value = top
        .remove("cards")
        .ok_or_else(|| "missing \"cards\" key".to_string())?;
    let raw_cards = match cards_value {
        JsonValue::Object(map) => map,
        _ => return Err("\"cards\" must be an object".to_string()),
    };

    let mut cards = BTreeMap::new();
    for (name, card_value) in raw_cards {
        cards.insert(name, card_from_value(card_value)?);
    }
    Ok(Deck { cards })
}

fn card_from_value(value: JsonValue) -> Result<Card, String> {
    let mut fields = match value {
        JsonValue::Object(map) => map,
        _ => return Err("expected a card to be an object".to_string()),
    };

    Ok(Card {
        interval_days: take_u32(&mut fields, "interval_days")?,
        repetitions: take_u32(&mut fields, "repetitions")?,
        ease: take_f64(&mut fields, "ease")?,
        due_on: take_u32(&mut fields, "due_on")?,
    })
}

fn take_f64(fields: &mut BTreeMap<String, JsonValue>, key: &str) -> Result<f64, String> {
    match fields.remove(key) {
        Some(JsonValue::Number(n)) => Ok(n),
        Some(_) => Err(format!("field \"{key}\" must be a number")),
        None => Err(format!("missing field \"{key}\"")),
    }
}

fn take_u32(fields: &mut BTreeMap<String, JsonValue>, key: &str) -> Result<u32, String> {
    let n = take_f64(fields, key)?;
    if n.fract() != 0.0 || n < 0.0 || n > u32::MAX as f64 {
        return Err(format!("field \"{key}\" must be a whole number that fits in u32"));
    }
    Ok(n as u32)
}

/// A minimal recursive-descent parser covering the subset of JSON this
/// module needs: objects, strings, and numbers. Arrays, booleans, and null
/// are not part of the deck schema, so they are not accepted.
struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(text: &'a str) -> Self {
        JsonParser {
            bytes: text.as_bytes(),
            pos: 0,
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at byte offset {}", byte as char, self.pos))
        }
    }

    fn parse_value(&mut self) -> Result<JsonValue, String> {
        self.skip_whitespace();
        match self.peek() {
            Some(b'{') => self.parse_object(),
            Some(b'"') => Ok(JsonValue::String(self.parse_string()?)),
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            Some(other) => Err(format!("unexpected character '{}'", other as char)),
            None => Err("unexpected end of input".to_string()),
        }
    }

    fn parse_object(&mut self) -> Result<JsonValue, String> {
        self.expect(b'{')?;
        let mut map = BTreeMap::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonValue::Object(map));
        }
        loop {
            self.skip_whitespace();
            let key = self.parse_string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            let value = self.parse_value()?;
            map.insert(key, value);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(format!("expected ',' or '}}' at byte offset {}", self.pos)),
            }
        }
        Ok(JsonValue::Object(map))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.skip_whitespace();
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err("unterminated string".to_string()),
                Some(b'"') => {
                    self.pos += 1;
                    break;
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let escaped = self
                        .peek()
                        .ok_or_else(|| "unterminated escape sequence".to_string())?;
                    self.pos += 1;
                    match escaped {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000c}'),
                        b'u' => out.push(self.parse_unicode_escape()?),
                        other => return Err(format!("invalid escape '\\{}'", other as char)),
                    }
                }
                Some(_) => {
                    // Strings are ASCII-clean JSON syntax around whatever's
                    // in them, so it's safe to walk the underlying UTF-8
                    // directly rather than re-decoding one codepoint at a
                    // time.
                    let rest = std::str::from_utf8(&self.bytes[self.pos..])
                        .map_err(|_| "invalid utf-8 in string".to_string())?;
                    let ch = rest.chars().next().unwrap();
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
        Ok(out)
    }

    fn parse_unicode_escape(&mut self) -> Result<char, String> {
        let high = self.parse_hex4()?;
        if (0xD800..=0xDBFF).contains(&high) {
            if self.peek() != Some(b'\\') || self.bytes.get(self.pos + 1) != Some(&b'u') {
                return Err("unpaired utf-16 surrogate".to_string());
            }
            self.pos += 2;
            let low = self.parse_hex4()?;
            if !(0xDC00..=0xDFFF).contains(&low) {
                return Err("invalid low surrogate".to_string());
            }
            let combined = 0x10000 + (((high - 0xD800) as u32) << 10) + (low - 0xDC00) as u32;
            char::from_u32(combined).ok_or_else(|| "invalid surrogate pair".to_string())
        } else {
            char::from_u32(high as u32).ok_or_else(|| "invalid unicode escape".to_string())
        }
    }

    fn parse_hex4(&mut self) -> Result<u16, String> {
        let s = self
            .bytes
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| "truncated unicode escape".to_string())?;
        let s = std::str::from_utf8(s).map_err(|_| "invalid unicode escape".to_string())?;
        let n = u16::from_str_radix(s, 16).map_err(|_| "invalid unicode escape".to_string())?;
        self.pos += 4;
        Ok(n)
    }

    fn parse_number(&mut self) -> Result<JsonValue, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos]).unwrap();
        text.parse::<f64>()
            .map(JsonValue::Number)
            .map_err(|_| format!("invalid number literal '{text}'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn scratch_path(label: &str) -> std::path::PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        env::temp_dir().join(format!("spaced-recall-test-{label}-{n}.json"))
    }

    #[test]
    fn round_trips_a_deck_through_json() {
        let mut deck = Deck::new();
        deck.cards.insert(
            "capital of peru".to_string(),
            Card {
                interval_days: 6,
                repetitions: 2,
                ease: 2.5,
                due_on: 110,
            },
        );
        deck.cards.insert(
            "\"quoted\" and \\ backslash".to_string(),
            Card::new(0),
        );

        let json = deck.to_json();
        let restored = Deck::from_json(&json).unwrap();
        assert_eq!(restored, deck);
    }

    #[test]
    fn saves_and_loads_from_disk() {
        let path = scratch_path("roundtrip");
        let mut deck = Deck::new();
        deck.cards.insert("front".to_string(), Card::new(5));

        deck.save(&path).unwrap();
        let loaded = Deck::load(&path).unwrap();
        fs::remove_file(&path).unwrap();

        assert_eq!(loaded, deck);
    }

    #[test]
    fn loading_a_missing_file_gives_an_empty_deck() {
        let path = scratch_path("missing");
        let loaded = Deck::load(&path).unwrap();
        assert_eq!(loaded, Deck::new());
    }

    #[test]
    fn rejects_a_card_missing_a_field() {
        let err = Deck::from_json(r#"{"cards":{"x":{"interval_days":1,"repetitions":0,"ease":2.5}}}"#)
            .unwrap_err();
        assert!(err.contains("due_on"));
    }

    #[test]
    fn rejects_malformed_json() {
        let err = Deck::from_json("not json").unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn rejects_a_non_integer_due_date() {
        let err = Deck::from_json(
            r#"{"cards":{"x":{"interval_days":1,"repetitions":0,"ease":2.5,"due_on":1.5}}}"#,
        )
        .unwrap_err();
        assert!(err.contains("due_on"));
    }

    #[test]
    fn round_trips_a_deck_through_plain_text() {
        let mut deck = Deck::new();
        deck.cards.insert(
            "capital of peru".to_string(),
            Card {
                interval_days: 6,
                repetitions: 2,
                ease: 2.5,
                due_on: 110,
            },
        );
        deck.cards.insert("second card".to_string(), Card::new(0));

        let text = deck.to_text().unwrap();
        let restored = Deck::from_text(&text).unwrap();
        assert_eq!(restored, deck);
    }

    #[test]
    fn exports_and_imports_plain_text_from_disk() {
        let path = scratch_path("text-roundtrip");
        let mut deck = Deck::new();
        deck.cards.insert("front".to_string(), Card::new(5));

        deck.export_text(&path).unwrap();
        let loaded = Deck::import_text(&path).unwrap();
        fs::remove_file(&path).unwrap();

        assert_eq!(loaded, deck);
    }

    #[test]
    fn plain_text_skips_blank_lines() {
        let deck = Deck::from_text("front\t1\t1\t2.5\t10\n\n\n").unwrap();
        assert_eq!(deck.cards.len(), 1);
    }

    #[test]
    fn plain_text_rejects_a_line_with_the_wrong_number_of_fields() {
        let err = Deck::from_text("front\t1\t1\t2.5\n").unwrap_err();
        match err {
            DeckError::Parse(msg) => assert!(msg.contains("5 tab-separated fields")),
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn plain_text_rejects_a_malformed_number_field() {
        let err = Deck::from_text("front\tnot-a-number\t1\t2.5\t10\n").unwrap_err();
        match err {
            DeckError::Parse(msg) => assert!(msg.contains("interval_days")),
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn plain_text_rejects_a_duplicate_card_name() {
        let err = Deck::from_text("front\t1\t1\t2.5\t10\nfront\t2\t2\t2.5\t20\n").unwrap_err();
        match err {
            DeckError::Parse(msg) => assert!(msg.contains("duplicate")),
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn plain_text_export_rejects_a_name_with_a_tab() {
        let mut deck = Deck::new();
        deck.cards.insert("bad\tname".to_string(), Card::new(0));

        let err = deck.to_text().unwrap_err();
        match err {
            DeckError::InvalidName(name) => assert_eq!(name, "bad\tname"),
            other => panic!("expected an invalid name error, got {other:?}"),
        }
    }
}
