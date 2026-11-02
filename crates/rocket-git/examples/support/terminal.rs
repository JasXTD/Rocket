//! All human-readable inspect output uses this renderer, including Error::Display.
pub fn safe(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for c in text.chars() {
        if (c.is_control() && !matches!(c, '\n' | '\t'))
            || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        {
            output.extend(c.escape_default());
        } else {
            output.push(c);
        }
    }
    output
}

// Lexical scope covers inspect and its clone submodule. A new print call cannot
// accidentally skip escaping an error, path, subject, stdout or stderr.
macro_rules! print { ($($arg:tt)*) => { std::print!("{}", crate::terminal::safe(&format!($($arg)*))) }; }
macro_rules! println { () => { std::println!() }; ($($arg:tt)*) => { std::println!("{}", crate::terminal::safe(&format!($($arg)*))) }; }
macro_rules! eprint { ($($arg:tt)*) => { std::eprint!("{}", crate::terminal::safe(&format!($($arg)*))) }; }
macro_rules! eprintln { () => { std::eprintln!() }; ($($arg:tt)*) => { std::eprintln!("{}", crate::terminal::safe(&format!($($arg)*))) }; }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ansi_osc_carriage_return_and_bidi_are_printable_text() {
        let raw = "safe 世界\n\t\x1b[2J\x1b]52;c;payload\x07\r\u{009b}\u{202e}hidden";
        let escaped = safe(raw);
        assert!(escaped.starts_with("safe 世界\n\t"));
        assert!(
            !escaped
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
        );
        assert!(!escaped.contains('\u{202e}'));
        assert!(escaped.contains("\\u{1b}"));
    }
}
