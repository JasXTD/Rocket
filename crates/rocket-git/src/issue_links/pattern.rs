//! A small regular-expression subset for issue IDs, run as a Pike VM: time is
//! linear in the text for a given pattern, with no backtracking blow-up.
//!
//! Supported: literals; `.`; classes `[a-z0-9_-]`, `[^…]`; `\d \D \w \W \s \S`
//! (ASCII, like RE2); `\b \B`; `^ $` (line start/end); groups `( )` and `(?: )`;
//! `|`; `* + ? {n} {n,} {n,m}` (bounds up to 100), each optionally lazy with `?`;
//! a leading `(?i)` for case-insensitive matching. Lookaround, backreferences and
//! other flags are refused with an explanation rather than misread.

pub(crate) const MAX_PATTERN_CHARS: usize = 256;
const MAX_PROGRAM: usize = 2000;
const MAX_REPEAT: u32 = 100;
pub(crate) const MAX_GROUPS: usize = 9;

#[derive(Clone, Debug)]
pub(crate) struct Pattern {
    program: Vec<Inst>,
    classes: Vec<Class>,
    groups: usize,
    fold: bool,
}

/// Byte ranges of one match: index 0 is the whole match, then each group.
pub(crate) type Captures = Vec<Option<(usize, usize)>>;

#[derive(Clone, Debug)]
enum Inst {
    Char(char),
    Any,
    Class(usize),
    LineStart,
    LineEnd,
    Boundary(bool),
    Save(usize),
    Split(usize, usize),
    Jump(usize),
    Match,
}

#[derive(Clone, Debug)]
struct Class {
    negated: bool,
    items: Vec<Item>,
}

#[derive(Clone, Copy, Debug)]
enum Item {
    Range(char, char),
    Digit(bool),
    Word(bool),
    Space(bool),
}

#[derive(Debug)]
enum Node {
    Empty,
    Char(char),
    Any,
    Class(Class),
    LineStart,
    LineEnd,
    Boundary(bool),
    Group(Box<Node>, Option<usize>),
    Concat(Vec<Node>),
    Alternate(Vec<Node>),
    Repeat {
        node: Box<Node>,
        min: u32,
        max: Option<u32>,
        greedy: bool,
    },
}

fn word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}
fn space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c')
}
/// Simple one-character case variants; anything longer keeps its own case.
fn lower(c: char) -> char {
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}
fn upper(c: char) -> char {
    let mut upper = c.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(u), None) => u,
        _ => c,
    }
}

impl Item {
    fn matches(self, c: char) -> bool {
        match self {
            Self::Range(a, b) => a <= c && c <= b,
            Self::Digit(positive) => c.is_ascii_digit() == positive,
            Self::Word(positive) => word(c) == positive,
            Self::Space(positive) => space(c) == positive,
        }
    }
}
impl Class {
    fn matches(&self, c: char, fold: bool) -> bool {
        let hit = |c| self.items.iter().any(|i| i.matches(c));
        let found = hit(c) || (fold && (hit(lower(c)) || hit(upper(c))));
        found != self.negated
    }
}

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    groups: usize,
}

impl Parser<'_> {
    fn alternation(&mut self, depth: usize) -> Result<Node, String> {
        if depth > 50 {
            return Err("Groups are nested too deeply".into());
        }
        let mut branches = vec![self.concatenation(depth)?];
        while self.chars.peek() == Some(&'|') {
            self.chars.next();
            branches.push(self.concatenation(depth)?);
        }
        Ok(if branches.len() == 1 {
            branches.pop().unwrap()
        } else {
            Node::Alternate(branches)
        })
    }

    fn concatenation(&mut self, depth: usize) -> Result<Node, String> {
        let mut items = Vec::new();
        while let Some(&c) = self.chars.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom(depth)?;
            items.push(self.quantified(atom)?);
        }
        Ok(match items.len() {
            0 => Node::Empty,
            1 => items.pop().unwrap(),
            _ => Node::Concat(items),
        })
    }

    fn atom(&mut self, depth: usize) -> Result<Node, String> {
        let c = self.chars.next().unwrap();
        Ok(match c {
            '.' => Node::Any,
            '^' => Node::LineStart,
            '$' => Node::LineEnd,
            '[' => Node::Class(self.class()?),
            '\\' => self
                .escape(false)?
                .map_or_else(Node::Boundary, |item| match item {
                    Item::Range(a, _) => Node::Char(a),
                    item => Node::Class(Class {
                        negated: false,
                        items: vec![item],
                    }),
                }),
            '(' => {
                let capture = if self.chars.peek() == Some(&'?') {
                    self.chars.next();
                    match self.chars.next() {
                        Some(':') => None,
                        Some('i') => {
                            return Err("(?i) is allowed only at the start of the pattern".into());
                        }
                        Some('=' | '!' | '<') => {
                            return Err("Lookaround and named groups are not supported".into());
                        }
                        _ => return Err("Unsupported group syntax after (?".into()),
                    }
                } else {
                    self.groups += 1;
                    if self.groups > MAX_GROUPS {
                        return Err(format!("At most {MAX_GROUPS} capturing groups"));
                    }
                    Some(self.groups)
                };
                let inner = self.alternation(depth + 1)?;
                if self.chars.next() != Some(')') {
                    return Err("Missing )".into());
                }
                Node::Group(Box::new(inner), capture)
            }
            ')' => return Err("Unmatched )".into()),
            '*' | '+' | '?' => return Err(format!("Nothing to repeat before {c}")),
            '{' if self.chars.peek().is_some_and(char::is_ascii_digit) => {
                return Err("Nothing to repeat before {".into());
            }
            c => Node::Char(c),
        })
    }

    /// `Ok(Ok(item))` for a character or set, `Ok(Err(boundary))` for `\b`/`\B`.
    fn escape(&mut self, in_class: bool) -> Result<Result<Item, bool>, String> {
        let c = self.chars.next().ok_or("Pattern ends with \\")?;
        Ok(Ok(match c {
            'd' => Item::Digit(true),
            'D' => Item::Digit(false),
            'w' => Item::Word(true),
            'W' => Item::Word(false),
            's' => Item::Space(true),
            'S' => Item::Space(false),
            't' => Item::Range('\t', '\t'),
            'n' => Item::Range('\n', '\n'),
            'r' => Item::Range('\r', '\r'),
            'b' if !in_class => return Ok(Err(true)),
            'B' if !in_class => return Ok(Err(false)),
            c if c.is_ascii_punctuation() || c == ' ' => Item::Range(c, c),
            c if c.is_ascii_digit() => return Err("Backreferences are not supported".into()),
            c => return Err(format!("Unsupported escape \\{c}")),
        }))
    }

    fn class(&mut self) -> Result<Class, String> {
        let negated = self.chars.peek() == Some(&'^');
        if negated {
            self.chars.next();
        }
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let c = self.chars.next().ok_or("Missing ]")?;
            if c == ']' && !first {
                break;
            }
            first = false;
            let start = if c == '\\' {
                match self.escape(true)? {
                    Ok(Item::Range(a, _)) => a,
                    Ok(item) => {
                        items.push(item);
                        continue;
                    }
                    Err(_) => unreachable!("no boundaries inside a class"),
                }
            } else {
                c
            };
            let mut ahead = self.chars.clone();
            if ahead.next() == Some('-') && ahead.peek().is_some_and(|&n| n != ']') {
                self.chars.next();
                let end = match self.chars.next().unwrap() {
                    '\\' => match self.escape(true)? {
                        Ok(Item::Range(a, _)) => a,
                        _ => return Err("A class range must end with a character".into()),
                    },
                    end => end,
                };
                if end < start {
                    return Err(format!("Range {start}-{end} is out of order"));
                }
                items.push(Item::Range(start, end));
            } else {
                items.push(Item::Range(start, start));
            }
        }
        Ok(Class { negated, items })
    }

    fn quantified(&mut self, atom: Node) -> Result<Node, String> {
        let (min, max) = match self.chars.peek() {
            Some('*') => (0, None),
            Some('+') => (1, None),
            Some('?') => (0, Some(1)),
            Some('{') => {
                let mut ahead = self.chars.clone();
                ahead.next();
                if !ahead.peek().is_some_and(char::is_ascii_digit) {
                    return Ok(atom); // A literal brace, as in most engines.
                }
                self.chars.next();
                let min = self.number()?;
                let max = match self.chars.next() {
                    Some('}') => Some(min),
                    Some(',') if self.chars.peek() == Some(&'}') => {
                        self.chars.next();
                        None
                    }
                    Some(',') => {
                        let max = self.number()?;
                        if self.chars.next() != Some('}') {
                            return Err("Missing } in a repetition".into());
                        }
                        Some(max)
                    }
                    _ => return Err("Malformed {n,m} repetition".into()),
                };
                if max.is_some_and(|max| max < min) {
                    return Err("A repetition's maximum is below its minimum".into());
                }
                if matches!(atom, Node::LineStart | Node::LineEnd | Node::Boundary(_)) {
                    return Err("An anchor cannot be repeated".into());
                }
                return self.finish_repeat(atom, min, max);
            }
            _ => return Ok(atom),
        };
        self.chars.next();
        if matches!(atom, Node::LineStart | Node::LineEnd | Node::Boundary(_)) {
            return Err("An anchor cannot be repeated".into());
        }
        self.finish_repeat(atom, min, max)
    }

    fn finish_repeat(&mut self, atom: Node, min: u32, max: Option<u32>) -> Result<Node, String> {
        let greedy = if self.chars.peek() == Some(&'?') {
            self.chars.next();
            false
        } else {
            true
        };
        if matches!(self.chars.peek(), Some('*' | '+')) {
            return Err("Possessive or doubled repetition is not supported".into());
        }
        Ok(Node::Repeat {
            node: Box::new(atom),
            min,
            max,
            greedy,
        })
    }

    fn number(&mut self) -> Result<u32, String> {
        let mut value = 0u32;
        let mut digits = 0;
        while let Some(d) = self.chars.peek().and_then(|c| c.to_digit(10)) {
            self.chars.next();
            digits += 1;
            value = value.saturating_mul(10).saturating_add(d);
        }
        if digits == 0 {
            return Err("Expected a number in a repetition".into());
        }
        if value > MAX_REPEAT {
            return Err(format!("Repetition bounds are limited to {MAX_REPEAT}"));
        }
        Ok(value)
    }
}

struct Compiler {
    program: Vec<Inst>,
    classes: Vec<Class>,
}

impl Compiler {
    fn emit(&mut self, inst: Inst) -> Result<usize, String> {
        if self.program.len() >= MAX_PROGRAM {
            return Err("Pattern is too large; use smaller repetition bounds".into());
        }
        self.program.push(inst);
        Ok(self.program.len() - 1)
    }

    fn node(&mut self, node: &Node) -> Result<(), String> {
        match node {
            Node::Empty => {}
            Node::Char(c) => {
                self.emit(Inst::Char(*c))?;
            }
            Node::Any => {
                self.emit(Inst::Any)?;
            }
            Node::Class(class) => {
                self.classes.push(class.clone());
                self.emit(Inst::Class(self.classes.len() - 1))?;
            }
            Node::LineStart => {
                self.emit(Inst::LineStart)?;
            }
            Node::LineEnd => {
                self.emit(Inst::LineEnd)?;
            }
            Node::Boundary(b) => {
                self.emit(Inst::Boundary(*b))?;
            }
            Node::Group(inner, None) => self.node(inner)?,
            Node::Group(inner, Some(index)) => {
                self.emit(Inst::Save(2 * index))?;
                self.node(inner)?;
                self.emit(Inst::Save(2 * index + 1))?;
            }
            Node::Concat(items) => {
                for item in items {
                    self.node(item)?;
                }
            }
            Node::Alternate(branches) => {
                let mut jumps = Vec::new();
                for (i, branch) in branches.iter().enumerate() {
                    if i + 1 < branches.len() {
                        let split = self.emit(Inst::Split(0, 0))?;
                        self.node(branch)?;
                        jumps.push(self.emit(Inst::Jump(0))?);
                        let next = self.program.len();
                        self.program[split] = Inst::Split(split + 1, next);
                    } else {
                        self.node(branch)?;
                    }
                }
                let end = self.program.len();
                for jump in jumps {
                    self.program[jump] = Inst::Jump(end);
                }
            }
            Node::Repeat {
                node,
                min,
                max,
                greedy,
            } => {
                for _ in 0..*min {
                    self.node(node)?;
                }
                let split = |this: usize, other: usize| {
                    if *greedy {
                        Inst::Split(this, other)
                    } else {
                        Inst::Split(other, this)
                    }
                };
                match max {
                    None => {
                        let start = self.emit(Inst::Split(0, 0))?;
                        self.node(node)?;
                        self.emit(Inst::Jump(start))?;
                        let end = self.program.len();
                        self.program[start] = split(start + 1, end);
                    }
                    Some(max) => {
                        let mut splits = Vec::new();
                        for _ in *min..*max {
                            splits.push(self.emit(Inst::Split(0, 0))?);
                            self.node(node)?;
                        }
                        let end = self.program.len();
                        for s in splits {
                            self.program[s] = split(s + 1, end);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Threads at one text position, in priority order. Every instruction is
/// visited at most once per position, which also ends empty loops.
struct Threads {
    pcs: Vec<usize>,
    captures: Vec<Vec<Option<usize>>>,
    seen: Vec<bool>,
    visited: Vec<usize>,
}
impl Threads {
    fn new(size: usize) -> Self {
        Self {
            pcs: Vec::new(),
            captures: Vec::new(),
            seen: vec![false; size],
            visited: Vec::new(),
        }
    }
    fn clear(&mut self) {
        for &pc in &self.visited {
            self.seen[pc] = false;
        }
        self.visited.clear();
        self.pcs.clear();
        self.captures.clear();
    }
}

impl Pattern {
    pub(crate) fn new(source: &str) -> Result<Self, String> {
        if source.is_empty() {
            return Err("Enter a pattern".into());
        }
        if source.chars().count() > MAX_PATTERN_CHARS {
            return Err(format!(
                "Patterns are limited to {MAX_PATTERN_CHARS} characters"
            ));
        }
        if source.chars().any(char::is_control) {
            return Err("Patterns cannot contain control characters".into());
        }
        let (fold, body) = match source.strip_prefix("(?i)") {
            Some(rest) => (true, rest),
            None => (false, source),
        };
        let mut parser = Parser {
            chars: body.chars().peekable(),
            groups: 0,
        };
        let tree = parser.alternation(0)?;
        if parser.chars.next().is_some() {
            return Err("Unmatched )".into());
        }
        let mut compiler = Compiler {
            program: Vec::new(),
            classes: Vec::new(),
        };
        compiler.emit(Inst::Save(0))?;
        compiler.node(&tree)?;
        compiler.emit(Inst::Save(1))?;
        compiler.emit(Inst::Match)?;
        if fold {
            for inst in &mut compiler.program {
                if let Inst::Char(c) = inst {
                    *c = lower(*c);
                }
            }
        }
        Ok(Self {
            program: compiler.program,
            classes: compiler.classes,
            groups: parser.groups,
            fold,
        })
    }

    pub(crate) fn groups(&self) -> usize {
        self.groups
    }

    /// Non-empty, non-overlapping leftmost-first matches, as byte ranges into
    /// `text`. `budget` bounds the total VM steps; when it runs out, the matches
    /// found so far are returned.
    pub(crate) fn find_all(&self, text: &str, mut budget: usize) -> Vec<Captures> {
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        let mut out = Vec::new();
        let mut start = 0;
        while start <= chars.len() {
            let Some(found) = self.run(text.len(), &chars, start, &mut budget) else {
                break;
            };
            let (whole_start, whole_end) = found[0].unwrap();
            let end_index = chars.partition_point(|&(b, _)| b < whole_end);
            if whole_end > whole_start {
                out.push(found);
                start = end_index;
            } else {
                start = end_index + 1;
            }
        }
        out
    }

    fn run(
        &self,
        length: usize,
        chars: &[(usize, char)],
        from: usize,
        budget: &mut usize,
    ) -> Option<Captures> {
        let slots = 2 * (self.groups + 1);
        let size = self.program.len();
        let mut current = Threads::new(size);
        let mut next = Threads::new(size);
        let mut matched: Option<Vec<Option<usize>>> = None;
        for i in from..=chars.len() {
            if matched.is_none() {
                self.add(&mut current, 0, vec![None; slots], i, chars, length, budget);
            }
            if current.pcs.is_empty() {
                if matched.is_some() || *budget == 0 {
                    break;
                }
                // An anchor failed here; a new attempt starts at the next position.
                current.clear();
                continue;
            }
            let c = chars.get(i).map(|&(_, c)| c);
            for t in 0..current.pcs.len() {
                if *budget == 0 {
                    return matched.map(|m| self.captures(&m));
                }
                *budget -= 1;
                let pc = current.pcs[t];
                let advance = match (&self.program[pc], c) {
                    (Inst::Match, _) => {
                        matched = Some(current.captures[t].clone());
                        break; // Lower-priority threads cannot win.
                    }
                    (Inst::Char(want), Some(c)) => *want == c || (self.fold && *want == lower(c)),
                    (Inst::Any, Some(c)) => c != '\n',
                    (Inst::Class(k), Some(c)) => self.classes[*k].matches(c, self.fold),
                    _ => false,
                };
                if advance {
                    let captures = current.captures[t].clone();
                    self.add(&mut next, pc + 1, captures, i + 1, chars, length, budget);
                }
            }
            std::mem::swap(&mut current, &mut next);
            next.clear();
            if matched.is_some() && current.pcs.is_empty() {
                break;
            }
        }
        matched.map(|m| self.captures(&m))
    }

    fn captures(&self, slots: &[Option<usize>]) -> Captures {
        slots
            .chunks(2)
            .map(|pair| match (pair[0], pair[1]) {
                (Some(a), Some(b)) if a <= b => Some((a, b)),
                _ => None,
            })
            .collect()
    }

    /// Follow empty transitions from `pc` at char index `i`, in priority order.
    #[allow(clippy::too_many_arguments)]
    fn add(
        &self,
        list: &mut Threads,
        pc: usize,
        captures: Vec<Option<usize>>,
        i: usize,
        chars: &[(usize, char)],
        length: usize,
        budget: &mut usize,
    ) {
        let mut stack = vec![(pc, captures)];
        while let Some((pc, mut captures)) = stack.pop() {
            if list.seen[pc] || *budget == 0 {
                continue;
            }
            *budget -= 1;
            list.seen[pc] = true;
            list.visited.push(pc);
            let before = i.checked_sub(1).and_then(|p| chars.get(p)).map(|&(_, c)| c);
            let at = chars.get(i).map(|&(_, c)| c);
            match self.program[pc] {
                Inst::Jump(to) => stack.push((to, captures)),
                Inst::Split(first, second) => {
                    // The stack pops the preferred branch first.
                    stack.push((second, captures.clone()));
                    stack.push((first, captures));
                }
                Inst::Save(slot) => {
                    captures[slot] = Some(chars.get(i).map_or(length, |&(b, _)| b));
                    stack.push((pc + 1, captures));
                }
                Inst::LineStart | Inst::LineEnd | Inst::Boundary(_) => {
                    let holds = match self.program[pc] {
                        Inst::LineStart => before.is_none_or(|c| c == '\n'),
                        Inst::LineEnd => at.is_none_or(|c| c == '\n'),
                        Inst::Boundary(want) => {
                            (before.is_some_and(word) != at.is_some_and(word)) == want
                        }
                        _ => unreachable!(),
                    };
                    if holds {
                        stack.push((pc + 1, captures));
                    }
                }
                _ => {
                    list.pcs.push(pc);
                    list.captures.push(captures);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn all(pattern: &str, text: &str) -> Vec<String> {
        Pattern::new(pattern)
            .unwrap()
            .find_all(text, 1_000_000)
            .into_iter()
            .map(|c| {
                let (a, b) = c[0].unwrap();
                text[a..b].to_owned()
            })
            .collect()
    }

    #[test]
    fn issue_patterns_find_every_id() {
        let jira = r"\b[A-Z][A-Z0-9_]+-\d+\b";
        assert_eq!(
            all(jira, "PROJ-12: fix OPS-7 and ABC-1x, not proj-3 or X-1"),
            ["PROJ-12", "OPS-7"]
        );
        assert_eq!(all(r"#(\d+)", "Refs #12, #345;#x"), ["#12", "#345"]);
        assert_eq!(
            all(r"(?i)\bgh-\d+", "GH-1 gh-2 Gh-3"),
            ["GH-1", "gh-2", "Gh-3"]
        );
        assert_eq!(
            all(r"^Fixes: \w+$", "Text\nFixes: A1\nFixes: B 2"),
            ["Fixes: A1"]
        );
        assert_eq!(
            all(r"修复#\d{2,3}", "修复#1 修复#12 修复#1234"),
            ["修复#12", "修复#123"]
        );
    }

    #[test]
    fn groups_alternation_and_laziness() {
        let p = Pattern::new(r"(PROJ|OPS)-(\d+)").unwrap();
        assert_eq!(p.groups(), 2);
        let text = "see OPS-42";
        let c = &p.find_all(text, 100_000)[0];
        assert_eq!(c[1], Some((4, 7)));
        assert_eq!(&text[c[2].unwrap().0..c[2].unwrap().1], "42");
        assert_eq!(all(r"a.*?b", "a1b a2b"), ["a1b", "a2b"]);
        assert_eq!(all(r"a.*b", "a1b a2b"), ["a1b a2b"]);
        assert_eq!(all(r"(?:ab)+", "ababa"), ["abab"]);
        assert_eq!(all(r"x{2}|y?z", "xxx z yz"), ["xx", "z", "yz"]);
        assert_eq!(all(r"[^\s,]+-\d", "a-1,b-2 c-3"), ["a-1", "b-2", "c-3"]);
        assert_eq!(all(r"[\w.]+@x", "a.b@x"), ["a.b@x"]);
        assert_eq!(all(r"\{1\}|{x", "{1} {x"), ["{1}", "{x"]);
    }

    #[test]
    fn empty_matches_are_skipped_and_nested_stars_terminate() {
        assert!(all(r"x*", "abc").is_empty());
        assert_eq!(all(r"(a*)*b", "aab b"), ["aab", "b"]);
        assert_eq!(all(r"(a|)+c", "aac"), ["aac"]);
    }

    #[test]
    fn hostile_patterns_stay_linear_and_bounded() {
        let text = "a".repeat(16 * 1024);
        let started = std::time::Instant::now();
        assert!(all(r"(a*)*b", &text).is_empty());
        assert!(all(r"(a|aa)+$x", &text).is_empty());
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        // A tiny budget returns what it found without running away.
        let p = Pattern::new(r"a").unwrap();
        assert!(p.find_all(&text, 50).len() < 50);
    }

    #[test]
    fn unsupported_syntax_is_refused_with_reasons() {
        for (pattern, reason) in [
            ("", "Enter"),
            ("(?=x)", "Lookaround"),
            (r"(a)\1", "Backreferences"),
            ("a(?i)b", "only at the start"),
            ("[z-a]", "out of order"),
            ("a{3,1}", "maximum"),
            ("a{1000}", "limited"),
            ("*a", "Nothing to repeat"),
            ("(a", "Missing )"),
            ("a)", "Unmatched"),
            ("[ab", "Missing ]"),
            (r"\p{L}", "Unsupported escape"),
            ("a++", "Possessive"),
            ("^*", "anchor"),
        ] {
            let error = Pattern::new(pattern).unwrap_err();
            assert!(error.contains(reason), "{pattern}: {error}");
        }
        assert!(Pattern::new(&"a".repeat(MAX_PATTERN_CHARS + 1)).is_err());
        assert!(
            Pattern::new("(a{100}){100}")
                .unwrap_err()
                .contains("too large")
        );
    }
}
