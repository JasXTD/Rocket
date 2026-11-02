//! A small, bounded JSON reader for GitHub API responses. The backend keeps no
//! third-party dependencies, so this accepts exactly RFC 8259 JSON and refuses
//! deep nesting instead of recursing without limit.

const MAX_DEPTH: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Json {
    Null,
    Bool(bool),
    /// The original numeric text; GitHub IDs can exceed f64 precision.
    Number(String),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    pub(crate) fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Self::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub(crate) fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Self::String(s) => Some(s),
            _ => None,
        }
    }
    pub(crate) fn bool(&self, key: &str) -> Option<bool> {
        match self.get(key)? {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub(crate) fn u64(&self, key: &str) -> Option<u64> {
        match self.get(key)? {
            Self::Number(n) => n.parse().ok(),
            _ => None,
        }
    }
    /// A number or string ID, as text.
    pub(crate) fn id(&self, key: &str) -> Option<String> {
        match self.get(key)? {
            Self::Number(n) if n.bytes().all(|b| b.is_ascii_digit()) => Some(n.clone()),
            Self::String(s) if !s.is_empty() && s.len() <= 64 => Some(s.clone()),
            _ => None,
        }
    }
    pub(crate) fn array(&self) -> Option<&[Json]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }
}

pub(crate) fn parse(bytes: &[u8]) -> Option<Json> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut parser = Parser {
        bytes: text.as_bytes(),
        at: 0,
    };
    parser.space();
    let value = parser.value(0)?;
    parser.space();
    (parser.at == parser.bytes.len()).then_some(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }
    fn space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }
    fn literal(&mut self, word: &[u8], value: Json) -> Option<Json> {
        self.bytes[self.at..].starts_with(word).then(|| {
            self.at += word.len();
            value
        })
    }
    fn value(&mut self, depth: usize) -> Option<Json> {
        if depth > MAX_DEPTH {
            return None;
        }
        match self.peek()? {
            b'{' => self.object(depth),
            b'[' => self.array(depth),
            b'"' => self.string().map(Json::String),
            b't' => self.literal(b"true", Json::Bool(true)),
            b'f' => self.literal(b"false", Json::Bool(false)),
            b'n' => self.literal(b"null", Json::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }
    fn object(&mut self, depth: usize) -> Option<Json> {
        self.at += 1;
        let mut fields = Vec::new();
        self.space();
        if self.peek()? == b'}' {
            self.at += 1;
            return Some(Json::Object(fields));
        }
        loop {
            self.space();
            if self.peek()? != b'"' {
                return None;
            }
            let key = self.string()?;
            self.space();
            if self.peek()? != b':' {
                return None;
            }
            self.at += 1;
            self.space();
            fields.push((key, self.value(depth + 1)?));
            self.space();
            match self.peek()? {
                b',' => self.at += 1,
                b'}' => {
                    self.at += 1;
                    return Some(Json::Object(fields));
                }
                _ => return None,
            }
        }
    }
    fn array(&mut self, depth: usize) -> Option<Json> {
        self.at += 1;
        let mut items = Vec::new();
        self.space();
        if self.peek()? == b']' {
            self.at += 1;
            return Some(Json::Array(items));
        }
        loop {
            self.space();
            items.push(self.value(depth + 1)?);
            self.space();
            match self.peek()? {
                b',' => self.at += 1,
                b']' => {
                    self.at += 1;
                    return Some(Json::Array(items));
                }
                _ => return None,
            }
        }
    }
    fn number(&mut self) -> Option<Json> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        match self.peek()? {
            b'0' => self.at += 1,
            b'1'..=b'9' => self.digits(),
            _ => return None,
        }
        if self.peek() == Some(b'.') {
            self.at += 1;
            if !self.peek()?.is_ascii_digit() {
                return None;
            }
            self.digits();
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if !self.peek()?.is_ascii_digit() {
                return None;
            }
            self.digits();
        }
        // The input was validated as UTF-8 and this slice is ASCII.
        Some(Json::Number(
            std::str::from_utf8(&self.bytes[start..self.at])
                .ok()?
                .into(),
        ))
    }
    fn digits(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.at += 1;
        }
    }
    fn hex4(&mut self) -> Option<u32> {
        let digits = self.bytes.get(self.at..self.at + 4)?;
        self.at += 4;
        u32::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()
    }
    fn string(&mut self) -> Option<String> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let start = self.at;
            while let Some(b) = self.peek() {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.at += 1;
            }
            // Splitting only at ASCII bytes keeps UTF-8 boundaries intact.
            out.push_str(std::str::from_utf8(&self.bytes[start..self.at]).ok()?);
            match self.peek()? {
                b'"' => {
                    self.at += 1;
                    return Some(out);
                }
                b'\\' => {
                    self.at += 1;
                    let escape = self.peek()?;
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let high = self.hex4()?;
                            let code = if (0xd800..0xdc00).contains(&high)
                                && self.bytes[self.at..].starts_with(b"\\u")
                            {
                                let rewind = self.at;
                                self.at += 2;
                                let low = self.hex4()?;
                                if (0xdc00..0xe000).contains(&low) {
                                    0x10000 + ((high - 0xd800) << 10) + (low - 0xdc00)
                                } else {
                                    // A lone high surrogate; re-read the next escape.
                                    self.at = rewind;
                                    high
                                }
                            } else {
                                high
                            };
                            out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                        }
                        _ => return None,
                    }
                }
                _ => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn values_escapes_unicode_and_numbers() {
        let v = parse(br#" {"a":[1,-2.5e3,true,false,null],"s":"\u4e2d\u6587 \ud83d\ude80 \"q\" \\ \/ \n","big":12345678901234567890,"e":{}} "#)
            .unwrap();
        assert_eq!(
            v.get("a").unwrap().array().unwrap(),
            &[
                Json::Number("1".into()),
                Json::Number("-2.5e3".into()),
                Json::Bool(true),
                Json::Bool(false),
                Json::Null
            ]
        );
        assert_eq!(v.str("s").unwrap(), "中文 🚀 \"q\" \\ / \n");
        assert_eq!(v.id("big").unwrap(), "12345678901234567890");
        assert_eq!(v.get("e"), Some(&Json::Object(Vec::new())));
        assert_eq!(
            parse("\"é 界\"".as_bytes()),
            Some(Json::String("é 界".into()))
        );
        // Lone surrogates become U+FFFD rather than invalid text.
        assert_eq!(
            parse(br#""\ud800x\udc00""#),
            Some(Json::String("\u{fffd}x\u{fffd}".into()))
        );
    }
    #[test]
    fn malformed_and_too_deep_input_is_refused() {
        for bad in [
            &b""[..],
            b"{",
            b"[1,]",
            b"{\"a\" 1}",
            b"01",
            b"1.",
            b"-",
            b"\"\x01\"",
            b"\"\\x\"",
            b"tru",
            b"[1] 2",
            b"\xff",
        ] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
        let deep = "[".repeat(MAX_DEPTH + 2) + &"]".repeat(MAX_DEPTH + 2);
        assert_eq!(parse(deep.as_bytes()), None);
        let ok = "[".repeat(MAX_DEPTH) + &"]".repeat(MAX_DEPTH);
        assert!(parse(ok.as_bytes()).is_some());
    }
}
