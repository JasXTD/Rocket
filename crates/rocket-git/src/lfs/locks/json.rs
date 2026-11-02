//! Small, bounded JSON reader for Git LFS's machine-readable lock protocol.
//! Duplicate keys and invalid Unicode are rejected, rather than choosing a value.
use super::*;
#[derive(Debug)]
pub(super) enum Value {
    Null,
    Bool(bool),
    Number,
    String(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}
impl Value {
    pub fn field(&self, name: &str) -> Result<&Self> {
        if let Self::Object(o) = self {
            o.get(name).ok_or(Error::Lfs(LfsFailure::LockResponse))
        } else {
            fail(LfsFailure::LockResponse)
        }
    }
    pub fn string(&self) -> Result<&str> {
        if let Self::String(s) = self {
            Ok(s)
        } else {
            fail(LfsFailure::LockResponse)
        }
    }
    pub fn array(&self) -> Result<&[Self]> {
        if let Self::Array(a) = self {
            Ok(a)
        } else {
            fail(LfsFailure::LockResponse)
        }
    }
}
pub(super) fn parse(bytes: &[u8]) -> Result<Value> {
    if bytes.len() > 2 * 1024 * 1024 {
        return fail(LfsFailure::LockLimit);
    }
    std::str::from_utf8(bytes).map_err(|_| Error::Lfs(LfsFailure::LockResponse))?;
    let mut p = Parser {
        bytes,
        pos: 0,
        nodes: 0,
    };
    let value = p.value(0)?;
    p.space();
    if p.pos != bytes.len() {
        return fail(LfsFailure::LockResponse);
    }
    Ok(value)
}
struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    nodes: usize,
}
impl Parser<'_> {
    fn space(&mut self) {
        while self
            .bytes
            .get(self.pos)
            .is_some_and(|b| b" \n\t\r".contains(b))
        {
            self.pos += 1;
        }
    }
    fn take(&mut self, b: u8) -> bool {
        self.space();
        if self.bytes.get(self.pos) == Some(&b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn require(&mut self, b: u8) -> Result<()> {
        if self.take(b) {
            Ok(())
        } else {
            fail(LfsFailure::LockResponse)
        }
    }
    fn value(&mut self, depth: usize) -> Result<Value> {
        self.nodes += 1;
        if depth > 16 || self.nodes > 40_000 {
            return fail(LfsFailure::LockLimit);
        }
        self.space();
        match self.bytes.get(self.pos).copied() {
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b'{') => {
                self.pos += 1;
                let mut o = BTreeMap::new();
                if self.take(b'}') {
                    return Ok(Value::Object(o));
                }
                loop {
                    self.space();
                    let k = self.string()?;
                    self.require(b':')?;
                    if o.insert(k, self.value(depth + 1)?).is_some() {
                        return fail(LfsFailure::LockResponse);
                    }
                    if self.take(b'}') {
                        break;
                    }
                    self.require(b',')?;
                }
                Ok(Value::Object(o))
            }
            Some(b'[') => {
                self.pos += 1;
                let mut a = Vec::new();
                if self.take(b']') {
                    return Ok(Value::Array(a));
                }
                loop {
                    a.push(self.value(depth + 1)?);
                    if self.take(b']') {
                        break;
                    }
                    self.require(b',')?;
                }
                Ok(Value::Array(a))
            }
            Some(b'n') => {
                self.word(b"null")?;
                Ok(Value::Null)
            }
            Some(b't') => {
                self.word(b"true")?;
                Ok(Value::Bool(true))
            }
            Some(b'f') => {
                self.word(b"false")?;
                Ok(Value::Bool(false))
            }
            Some(b'-' | b'0'..=b'9') => {
                if self.bytes.get(self.pos) == Some(&b'-') {
                    self.pos += 1;
                }
                if self.bytes.get(self.pos) == Some(&b'0') {
                    self.pos += 1;
                } else {
                    self.digits()?;
                }
                if self.bytes.get(self.pos) == Some(&b'.') {
                    self.pos += 1;
                    self.digits()?;
                }
                if matches!(self.bytes.get(self.pos), Some(b'e' | b'E')) {
                    self.pos += 1;
                    if matches!(self.bytes.get(self.pos), Some(b'+' | b'-')) {
                        self.pos += 1;
                    }
                    self.digits()?;
                }
                Ok(Value::Number)
            }
            _ => fail(LfsFailure::LockResponse),
        }
    }
    fn digits(&mut self) -> Result<()> {
        let start = self.pos;
        while self.bytes.get(self.pos).is_some_and(u8::is_ascii_digit) {
            self.pos += 1;
        }
        if start == self.pos {
            fail(LfsFailure::LockResponse)
        } else {
            Ok(())
        }
    }
    fn word(&mut self, word: &[u8]) -> Result<()> {
        if self.bytes.get(self.pos..self.pos + word.len()) != Some(word) {
            return fail(LfsFailure::LockResponse);
        }
        self.pos += word.len();
        Ok(())
    }
    fn hex(&mut self) -> Result<u16> {
        let mut out = 0;
        for _ in 0..4 {
            let b = *self
                .bytes
                .get(self.pos)
                .ok_or(Error::Lfs(LfsFailure::LockResponse))?;
            self.pos += 1;
            out = out * 16
                + match b {
                    b'0'..=b'9' => u16::from(b - b'0'),
                    b'a'..=b'f' => u16::from(b - b'a' + 10),
                    b'A'..=b'F' => u16::from(b - b'A' + 10),
                    _ => return fail(LfsFailure::LockResponse),
                };
        }
        Ok(out)
    }
    fn string(&mut self) -> Result<String> {
        if self.bytes.get(self.pos) != Some(&b'"') {
            return fail(LfsFailure::LockResponse);
        }
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            let b = *self
                .bytes
                .get(self.pos)
                .ok_or(Error::Lfs(LfsFailure::LockResponse))?;
            self.pos += 1;
            match b {
                b'"' => {
                    return String::from_utf8(out)
                        .map_err(|_| Error::Lfs(LfsFailure::LockResponse));
                }
                0..=31 => return fail(LfsFailure::LockResponse),
                b'\\' => {
                    let b = *self
                        .bytes
                        .get(self.pos)
                        .ok_or(Error::Lfs(LfsFailure::LockResponse))?;
                    self.pos += 1;
                    match b {
                        b'"' | b'\\' | b'/' => out.push(b),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'n' => out.push(10),
                        b'r' => out.push(13),
                        b't' => out.push(9),
                        b'u' => {
                            let first = self.hex()?;
                            let code = if (0xD800..=0xDBFF).contains(&first) {
                                self.word(b"\\u")?;
                                let second = self.hex()?;
                                if !(0xDC00..=0xDFFF).contains(&second) {
                                    return fail(LfsFailure::LockResponse);
                                }
                                0x10000
                                    + ((u32::from(first) - 0xD800) << 10)
                                    + (u32::from(second) - 0xDC00)
                            } else {
                                u32::from(first)
                            };
                            let c =
                                char::from_u32(code).ok_or(Error::Lfs(LfsFailure::LockResponse))?;
                            let mut buf = [0; 4];
                            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                        }
                        _ => return fail(LfsFailure::LockResponse),
                    }
                }
                _ => out.push(b),
            }
            if out.len() > 16384 {
                return fail(LfsFailure::LockLimit);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json_unicode_and_escapes() {
        assert_eq!(
            parse(br#"{"path":"\u7d20\u6750/\ud83d\ude80 e\u0301\"\\"}"#)
                .unwrap()
                .field("path")
                .unwrap()
                .string()
                .unwrap(),
            "素材/🚀 é\"\\"
        );
        assert!(matches!(
            parse(b"[null,true,false,-1.2e+3]").unwrap(),
            Value::Array(_)
        ));
    }
    #[test]
    fn json_rejects_ambiguous_and_unbounded_inputs() {
        for bytes in [
            br#"{"x":1,"x":2}"#.as_slice(),
            br#""\ud800""#,
            br#""\udc00""#,
            br#""\x00""#,
            b"[1,]",
            b"01",
            b"1.",
            b"{}x",
            b"\"a\nb\"",
            b"{\"x\":true,}",
        ] {
            assert!(parse(bytes).is_err(), "{bytes:?}");
        }
        assert!(parse("[".repeat(18).as_bytes()).is_err());
        assert!(parse(&vec![b' '; 2 * 1024 * 1024 + 1]).is_err());
    }
}
