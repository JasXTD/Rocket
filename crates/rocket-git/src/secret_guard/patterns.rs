//! Local, bounded detection. No provider request is made to validate a candidate.
use super::SecretKind;
use crate::Result;

#[path = "syntax.rs"]
mod syntax;
use syntax::{SCAN_WINDOW, literal};
#[path = "ai_keys.rs"]
mod ai_keys;

#[derive(Clone, Copy)]
pub(super) struct Policy {
    pub secrets: bool,
    pub ai_keys: bool,
}
impl Policy {
    pub const ALL: Self = Self {
        secrets: true,
        ai_keys: true,
    };
}

// A check covers cancellation and the scan's shared deadline. Inspection must
// remain interruptible even for a binary blob or a single very long text line.
fn checkpoint(at: usize, check: &mut dyn FnMut() -> Result<()>) -> Result<()> {
    if at & (64 * 1024 - 1) == 0 {
        check()?;
    }
    Ok(())
}

// Skip chunks that cannot start a candidate using the standard byte search.
// Inspect against the original slice so prefixes spanning a chunk boundary
// are still complete. Each chunk remains a cancellation/deadline checkpoint.
fn find_candidate<T>(
    bytes: &[u8],
    first: u8,
    check: &mut dyn FnMut() -> Result<()>,
    mut inspect: impl FnMut(usize) -> Option<T>,
) -> Result<Option<T>> {
    const CHUNK: usize = 64 * 1024;
    for (index, chunk) in bytes.chunks(CHUNK).enumerate() {
        check()?;
        if !chunk.contains(&first) {
            continue;
        }
        for (offset, &byte) in chunk.iter().enumerate() {
            if byte == first
                && let Some(found) = inspect(index * CHUNK + offset)
            {
                return Ok(Some(found));
            }
        }
    }
    Ok(None)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Context {
    Content,
    Message,
}

pub(super) fn environment_file(name: &[u8]) -> bool {
    let lower = name.to_ascii_lowercase();
    if matches!(
        lower.as_slice(),
        b".env.example" | b".env.sample" | b".env.template"
    ) {
        return false;
    }
    lower == b".env" || lower.starts_with(b".env.") || lower.ends_with(b".env")
}

fn reference_name(value: &[u8]) -> bool {
    value
        .first()
        .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        && value
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

fn reference_path(value: &[u8]) -> bool {
    let value = value.trim_ascii();
    // Helm starts paths with a dot; JS and Actions use dotted identifiers.
    let value = value.strip_prefix(b".").unwrap_or(value);
    value.split(|b| *b == b'.').all(reference_name)
}

fn placeholder(value: &[u8], quoted: bool) -> bool {
    let v = value.to_ascii_lowercase();
    v.is_empty()
        // A quoted "$..." or "%..." can be a real password. Exempt complete
        // references, not a prefix or a reference followed by literal material.
        || v.strip_prefix(b"${")
            .and_then(|body| body.strip_suffix(b"}"))
            .is_some_and(|body| reference_path(body.strip_suffix(b":-").unwrap_or(body)))
        || v.strip_prefix(b"$env:").is_some_and(reference_name)
        || (!quoted && v.strip_prefix(b"$").is_some_and(reference_name))
        || v.strip_prefix(b"%")
            .and_then(|body| body.strip_suffix(b"%"))
            .is_some_and(reference_name)
        || v.strip_prefix(b"<")
            .and_then(|body| body.strip_suffix(b">"))
            .is_some_and(|body| body.split(|b| *b == b'-').all(reference_name))
        || [b"{{".as_slice(), b"${{"].iter().any(|prefix| {
            v.strip_prefix(*prefix)
                .and_then(|body| body.strip_suffix(b"}}"))
                .is_some_and(reference_path)
        })
        || [b"#/definitions/".as_slice(), b"#/$defs/", b"#/components/schemas/"]
            .iter()
            .any(|prefix| v.strip_prefix(*prefix).is_some_and(|body| {
                body.split(|b| *b == b'/').all(reference_name)
            }))
        // Source expressions are references only outside a quoted literal.
        // Arbitrary text after a familiar marker is still credential material.
        || (!quoted && v.strip_prefix(b"process.env.").is_some_and(reference_name))
        || matches!(v.as_slice(),
            b"example" | b"example-password" | b"example_password" | b"placeholder"
            | b"changeme" | b"replace_me" | b"redacted" | b"fixture" | b"dummy"
            | b"test-only" | b"your_password" | b"your_token" | b"your_api_key"
            | b"your_secret" | b"none" | b"null" | b"true" | b"false")
}

fn github(bytes: &[u8], check: &mut dyn FnMut() -> Result<()>) -> Result<bool> {
    Ok(find_candidate(bytes, b'g', check, |i| {
        if !matches!(bytes.get(i + 1), Some(b'h' | b'i')) {
            return None;
        }
        for prefix in [
            b"ghp_".as_slice(),
            b"gho_",
            b"ghu_",
            b"ghs_",
            b"ghr_",
            b"github_pat_",
        ] {
            if bytes[i..].starts_with(prefix) {
                let n = bytes[i + prefix.len()..]
                    .iter()
                    .take_while(|b| {
                        b.is_ascii_alphanumeric()
                            || (prefix == b"github_pat_" && **b == b'_')
                            // GitHub's stateless installation tokens also use
                            // underscores, hyphens and JWT dots (2026 format).
                            || (prefix == b"ghs_" && b"._-".contains(b))
                    })
                    .take(30)
                    .count();
                // Recognize current GitHub families without requiring a valid checksum
                // or contacting GitHub. Short explanatory placeholders are not tokens.
                if n >= 30 {
                    return Some(());
                }
            }
        }
        None
    })?
    .is_some())
}

const CREDENTIAL_ENTROPY_THRESHOLD: f64 = 3.5;
// A compound identifier draws letters from several different words. Do not use
// a single credential's diversity cutoff for that aggregate. Each word still
// uses the stricter 3.5 bound; this separate bound rejects opaque split alphabets
// after full case/separator/length matching. Credential detection is unchanged.
const COMPOUND_IDENTIFIER_ENTROPY_THRESHOLD: f64 = 4.0;

// Callers validate ASCII first. Sharing this calculation keeps the ordinary
// credential heuristic and the structured-identifier check on the same scale.
fn ascii_entropy(bytes: impl Iterator<Item = u8>) -> f64 {
    let mut counts = [0usize; 128];
    let mut length = 0usize;
    for byte in bytes {
        counts[usize::from(byte)] += 1;
        length += 1;
    }
    counts
        .iter()
        .filter(|n| **n > 0)
        .map(|n| {
            let p = *n as f64 / length as f64;
            -p * p.log2()
        })
        .sum()
}

fn identifier_words(value: &[u8], separators: &[u8], upper: bool, minimum: usize) -> bool {
    let mut words = 0;
    for word in value.split(|b| separators.contains(b)) {
        // Long opaque payloads and mixed-case/digit-bearing components are not
        // ordinary label words. Check each word and the aggregate so separators
        // cannot disguise a random-looking payload as many small components.
        if word.is_empty()
            || word.len() > 24
            || !word.iter().all(|b| {
                if upper {
                    b.is_ascii_uppercase()
                } else {
                    b.is_ascii_lowercase()
                }
            })
            || ascii_entropy(word.iter().copied()) >= CREDENTIAL_ENTROPY_THRESHOLD
        {
            return false;
        }
        words += 1;
    }
    words >= minimum
        && ascii_entropy(value.iter().copied().filter(u8::is_ascii_alphabetic))
            < COMPOUND_IDENTIFIER_ENTROPY_THRESHOLD
}

fn protocol_identifier(value: &[u8]) -> bool {
    // A complete ENUM_STYLE_TAG[_Vn]:lowercase-label carries structural case,
    // separators and version digits that inflate whole-value entropy. Classify
    // its word components instead; no application, event or provider names are
    // recognized here. The caller restricts this to the ambiguous `token` key.
    // Line-oriented protocols may include one escaped LF or CRLF inside the
    // literal. Remove only that terminal sequence for structural classification,
    // not arbitrary escapes/whitespace or repeated endings. Failed recognition
    // still checks the original value, and the caller validates its closing quote.
    let value = value
        .strip_suffix(b"\\r\\n")
        .or_else(|| value.strip_suffix(b"\\n"))
        .unwrap_or(value);
    let mut parts = value.split(|b| *b == b':');
    let (Some(mut tag), Some(subject), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    if let Some(at) = tag.iter().rposition(|b| *b == b'_') {
        let suffix = &tag[at + 1..];
        if suffix.first() == Some(&b'V') && suffix.get(1).is_some_and(u8::is_ascii_digit) {
            // Version numbers may have one to four digits, not arbitrary nonce
            // data. All other tag components must be uppercase words.
            if suffix.len() > 5 || !suffix[1..].iter().all(u8::is_ascii_digit) {
                return false;
            }
            tag = &tag[..at];
        }
    }
    identifier_words(tag, b"_", true, 2) && identifier_words(subject, b"_-.", false, 1)
}

fn short_marker_identifier(value: &[u8]) -> bool {
    // Diagnostic sentinels can combine uppercase label words with a short hex
    // ID. Whole-value entropy mistakes that case/digit structure for a secret.
    // Match the complete grammar, not any project's labels or test filenames.
    // Callers additionally require an explicit constant and an ambiguous key.
    let Some(at) = value.iter().rposition(|b| matches!(b, b'-' | b'_')) else {
        return false;
    };
    let separator = value[at];
    let (label, suffix) = (&value[..at], &value[at + 1..]);
    (4..=8).contains(&suffix.len())
        && suffix
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        && suffix.iter().any(u8::is_ascii_digit)
        && suffix.iter().any(u8::is_ascii_lowercase)
        && label.split(|b| *b == separator).all(|word| word.len() >= 2)
        && identifier_words(label, &[separator], true, 2)
}

fn word_label_identifier(value: &[u8]) -> bool {
    // Source constants can surround an uppercase label with lowercase namespace
    // or annotation words. Case transitions inflate whole-value entropy even
    // when each component is an ordinary word. Require one uniform separator,
    // one contiguous uppercase label, and bounded component/aggregate
    // diversity. No names, file paths or marker prefixes grant an exemption.
    let Some(&separator) = value.iter().find(|b| matches!(b, b'-' | b'_')) else {
        return false;
    };
    let mut upper_words = 0;
    let mut lower_words = 0;
    let mut label_ended = false;
    for word in value.split(|b| *b == separator) {
        let upper = word.first().is_some_and(u8::is_ascii_uppercase);
        if word.len() < 2 || !identifier_words(word, &[], upper, 1) {
            return false;
        }
        if upper {
            if label_ended {
                return false;
            }
            upper_words += 1;
        } else {
            lower_words += 1;
            label_ended |= upper_words > 0;
        }
    }
    upper_words >= 2
        && lower_words >= 1
        && ascii_entropy(
            value
                .iter()
                .copied()
                .filter(u8::is_ascii_alphabetic)
                .map(|b| b.to_ascii_lowercase()),
        ) < COMPOUND_IDENTIFIER_ENTROPY_THRESHOLD
}

fn camel_label(value: &[u8]) -> bool {
    if !value.iter().all(u8::is_ascii_alphabetic) {
        return false;
    }
    let mut start = 0;
    let mut words = 0;
    for end in (1..=value.len()).filter(|&at| at == value.len() || value[at].is_ascii_uppercase()) {
        let word = &value[start..end];
        if !(3..=24).contains(&word.len())
            || !word[1..].iter().all(u8::is_ascii_lowercase)
            || ascii_entropy(word.iter().map(u8::to_ascii_lowercase))
                >= CREDENTIAL_ENTROPY_THRESHOLD
        {
            return false;
        }
        words += 1;
        start = end;
    }
    words >= 3
}

fn counter_identifier(value: &[u8]) -> bool {
    // Complete namespace + descriptive camel label + decimal cycle. A full
    // ascending/descending walk through ten digits adds character diversity but
    // no random payload. Never generalize this to arbitrary numeric suffixes,
    // placeholder words, provider prefixes or filenames. Constant context and
    // complete literal boundaries are enforced by the caller; provider checks
    // remain independent and take precedence over this generic classification.
    let Some(&separator) = value.iter().find(|b| matches!(b, b'_' | b'-')) else {
        return false;
    };
    let mut parts = value.split(|b| *b == separator);
    let (Some(namespace), Some(label), Some(cycle), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    if cycle.len() != 10
        || !cycle.iter().all(u8::is_ascii_digit)
        || ![1, 9].iter().any(|step| {
            cycle
                .windows(2)
                .all(|pair| (pair[0] - b'0' + step) % 10 == pair[1] - b'0')
        })
    {
        return false;
    }
    namespace.len() >= 2
        && identifier_words(namespace, &[], namespace[0].is_ascii_uppercase(), 1)
        && camel_label(label)
        && ascii_entropy(namespace.iter().chain(label).map(u8::to_ascii_lowercase))
            < COMPOUND_IDENTIFIER_ENTROPY_THRESHOLD
}

fn credential_like(value: &[u8], quoted: bool) -> bool {
    // A name such as `token` is not evidence by itself. Reject source syntax,
    // prose/UI labels and references before applying a bounded entropy check.
    // Specific GitHub/PEM rules are independent of this generic heuristic.
    if value.len() < 8
        || placeholder(value, quoted)
        || !value.iter().all(|b| {
            if quoted {
                // Inside a literal, punctuation (including ':' and escaped
                // quotes) is password content rather than source syntax.
                b.is_ascii_graphic()
            } else {
                b.is_ascii_alphanumeric() || b"_-./+=!@#$%^&*?~".contains(b)
            }
        })
    {
        return false;
    }
    let letters = value.iter().filter(|b| b.is_ascii_alphabetic()).count();
    let digits = value.iter().filter(|b| b.is_ascii_digit()).count();
    let symbols = value
        .iter()
        .filter(|b| !b.is_ascii_alphanumeric() && !b"_-".contains(b))
        .count();
    let capitals = value.iter().filter(|b| b.is_ascii_uppercase()).count();
    if letters == 0 {
        return false;
    }
    let shape = if quoted {
        (digits > 0 && (value.len() >= 12 || symbols > 0))
            || (value.len() >= 16 && symbols >= 2)
            || (value.len() >= 24 && capitals >= 3 && letters > capitals)
    } else {
        // Plain identifiers/types are ambiguous. Bare config values need
        // stronger evidence; quoted literals are handled above.
        (value.len() >= 20 && digits >= 2) || (value.len() >= 12 && digits * 4 >= value.len())
    };
    if !shape {
        return false;
    }
    ascii_entropy(value.iter().copied()) >= CREDENTIAL_ENTROPY_THRESHOLD
}

fn looks_like_type_reference(value: &[u8]) -> bool {
    if !value.first().is_some_and(u8::is_ascii_uppercase)
        || !value.iter().all(u8::is_ascii_alphanumeric)
    {
        return false;
    }
    // PascalCase references may contain acronyms and version numbers. Require
    // multiple word-sized components; mixed random letters/digits do not get a
    // general identifier exemption. This is used only for plain `token:` prose.
    let mut at = 0;
    let mut words = 0;
    while at < value.len() {
        if value[at].is_ascii_digit() {
            at += 1;
            continue;
        }
        if !value[at].is_ascii_uppercase() {
            return false;
        }
        at += 1;
        let start = at;
        while value.get(at).is_some_and(u8::is_ascii_lowercase) {
            at += 1;
        }
        let lower = at - start;
        if lower == 1 {
            return false;
        }
        words += usize::from(lower >= 3);
    }
    words >= 2
}

fn message_reference(value: &[u8], quoted: bool, prose_prefix: bool) -> bool {
    let object_id =
        !quoted && matches!(value.len(), 40 | 64) && value.iter().all(u8::is_ascii_hexdigit);
    object_id || ((!quoted || prose_prefix) && looks_like_type_reference(value))
}

fn assignment(
    line: &[u8],
    context: Context,
    policy: Policy,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<Option<SecretKind>> {
    // Resolve prefix context once. Repeated `token:` clauses must not rescan
    // the whole prefix for every candidate in a long message line.
    let (first_structure, first_word) = if context == Context::Message {
        (
            line.iter().position(|b| matches!(b, b'`' | b'{' | b'[')),
            line.iter().position(u8::is_ascii_alphabetic),
        )
    } else {
        (None, None)
    };
    for (i, &b) in line.iter().enumerate() {
        checkpoint(i, check)?;
        if b != b'=' && b != b':' {
            continue;
        }
        if (b == b'=' && matches!(line.get(i + 1), Some(b'=' | b'>')))
            || (b == b':' && line.get(i + 1) == Some(&b':'))
        {
            continue; // Comparisons and arrows are not assignments.
        }
        let original_left = line[..i].trim_ascii_end();
        let quoted_key = matches!(original_left.last(), Some(b'\'' | b'"'));
        let left = original_left
            .strip_suffix(b"\"")
            .or_else(|| original_left.strip_suffix(b"'"))
            .unwrap_or(original_left);
        let direct_key = left
            .rsplit(|b| !b.is_ascii_alphanumeric() && *b != b'_' && *b != b'-')
            .next()
            .unwrap_or_default();
        let go_binding = (b == b'=' && !quoted_key)
            .then(|| syntax::go_binding(left))
            .flatten();
        let (raw_key, key_start) =
            go_binding.unwrap_or((direct_key, left.len() - direct_key.len()));
        let mut key = Vec::with_capacity(24);
        for (at, &byte) in raw_key.iter().enumerate() {
            checkpoint(at, check)?;
            if !matches!(byte, b'_' | b'-') {
                key.push(byte.to_ascii_lowercase());
                // All recognized keys are shorter. Do not allocate a copy of
                // an arbitrarily long binary/source identifier.
                if key.len() > 24 {
                    break;
                }
            }
        }
        let general_key = matches!(
            key.as_slice(),
            b"token"
                | b"githubtoken"
                | b"ghtoken"
                | b"githubpat"
                | b"password"
                | b"passwd"
                | b"secret"
                | b"apikey"
                | b"apisecret"
                | b"accesstoken"
                | b"authtoken"
                | b"clientsecret"
                | b"awssecretaccesskey"
                | b"databasepassword"
                | b"dbpassword"
        );
        let ai_key = policy.ai_keys && ai_keys::context_name(&key);
        if !(policy.secrets && general_key || ai_key) {
            continue;
        }
        let shorthand = b == b':' && line.get(i + 1) == Some(&b'=');
        let mut source_binding = shorthand || go_binding.is_some();
        let mut full_right = line[i + 1 + usize::from(shorthand)..].trim_ascii_start();
        let mut colon_value = b == b':' && !shorthand;
        if colon_value
            && !quoted_key
            && let Some(value) = syntax::annotated_value(full_right)
        {
            full_right = value;
            source_binding = true;
            colon_value = false;
        }
        let right = &full_right[..full_right.len().min(SCAN_WINDOW)];
        let Some(literal) = literal(right) else {
            continue;
        };
        let (value, quoted) = (literal.value, literal.quoted);
        if ai_key && ai_keys::context_value(value) {
            return Ok(Some(SecretKind::AiApiKey));
        }
        if !policy.secrets || !general_key {
            continue;
        }
        // Generic token/secret names can describe structured data. Stronger
        // credential fields stay strict even when their value has this shape.
        let protocol = key == b"token" && protocol_identifier(value);
        let marker = matches!(key.as_slice(), b"secret" | b"token")
            && !quoted_key
            && !colon_value
            && syntax::constant_binding(&line[..key_start])
            && (short_marker_identifier(value)
                || word_label_identifier(value)
                || counter_identifier(value));
        if quoted
            && full_right.len() == right.len()
            && let Some(end) = literal.end
            && (protocol || marker)
        {
            // Require a complete literal boundary. Concatenation, an unclosed
            // quote or an expression beyond the bounded prefix still blocks.
            let tail = right[end..].trim_ascii_start();
            if tail.is_empty()
                || tail.starts_with(b";")
                || tail.starts_with(b"//")
                || (protocol && tail.starts_with(b"#"))
                || (protocol && colon_value && matches!(tail.first(), Some(b',' | b'}')))
            {
                continue; // Other assignments/provider tokens/keys still scan.
            }
        }
        if context == Context::Message
            && colon_value
            && key == b"token"
            && !quoted_key
            // Inline code/object snippets retain content matching too.
            && !first_structure.is_some_and(|at| at < key_start)
            && message_reference(value, quoted, first_word.is_some_and(|at| at < key_start))
        {
            continue;
        }
        if !quoted {
            if source_binding {
                continue; // A typed/shorthand RHS is a source expression, not bare config.
            }
            let mut words = line.trim_ascii_start().split(|b| b.is_ascii_whitespace());
            let mut first = words.next().unwrap_or_default();
            if first == b"export" {
                first = words.next().unwrap_or_default();
            }
            if first.starts_with(b"pub(")
                || matches!(
                    first,
                    b"let"
                        | b"const"
                        | b"var"
                        | b"type"
                        | b"pub"
                        | b"readonly"
                        | b"private"
                        | b"protected"
                        | b"public"
                        | b"final"
                )
            {
                continue; // A bare source binding references a value/type.
            }
        }
        // Unquoted source expressions are not credential literals. Quoted values
        // and environment/config-style bare tokens are checked conservatively.
        if !quoted
            && value
                .iter()
                .any(|b| matches!(b, b'(' | b')' | b'[' | b']' | b'{' | b'}' | b':' | b'.'))
        {
            continue;
        }
        if credential_like(value, quoted) {
            return Ok(Some(SecretKind::Credential));
        }
    }
    Ok(None)
}

fn ascii_scan(
    bytes: &[u8],
    context: Context,
    policy: Policy,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<Option<(SecretKind, usize)>> {
    let mut fence: Option<(u8, usize)> = None;
    for (number, line) in bytes.split(|b| *b == b'\n').enumerate() {
        // CRLF's final carriage return is a line boundary, not part of an
        // unfinished literal. Keep LF and CRLF refusal behavior identical.
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        check()?;
        if policy.ai_keys && ai_keys::contains_key(line, check)? {
            return Ok(Some((SecretKind::AiApiKey, number + 1)));
        }
        if policy.secrets && github(line, check)? {
            return Ok(Some((SecretKind::GithubToken, number + 1)));
        }
        let mut line_context = context;
        if context == Context::Message {
            let trimmed = line.trim_ascii_start();
            let marker = trimmed.first().copied();
            let length = trimmed.iter().take_while(|b| Some(**b) == marker).count();
            if matches!(marker, Some(b'`' | b'~')) && length >= 3 {
                if let Some((opened, minimum)) = fence {
                    if marker == Some(opened)
                        && length >= minimum
                        && trimmed[length..].trim_ascii().is_empty()
                    {
                        fence = None;
                    }
                } else {
                    fence = marker.map(|marker| (marker, length));
                }
                line_context = Context::Content;
            } else if fence.is_some() || line.starts_with(b"    ") || line.starts_with(b"\t") {
                line_context = Context::Content;
            }
        }
        if (policy.secrets || policy.ai_keys)
            && let Some(kind) = assignment(line, line_context, policy, check)?
        {
            return Ok(Some((kind, number + 1)));
        }
    }
    Ok(None)
}

pub(super) fn scan(bytes: &[u8]) -> Option<(SecretKind, usize)> {
    scan_with_check(bytes, &mut || Ok(())).expect("an unconditional check cannot fail")
}

#[cfg(test)]
pub(super) fn scan_message(bytes: &[u8]) -> Option<(SecretKind, usize)> {
    scan_message_with_check(bytes, &mut || Ok(())).expect("an unconditional check cannot fail")
}

pub(super) fn scan_with_check(
    bytes: &[u8],
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<Option<(SecretKind, usize)>> {
    scan_context(bytes, Context::Content, Policy::ALL, check)
}

#[cfg(test)]
pub(super) fn scan_message_with_check(
    bytes: &[u8],
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<Option<(SecretKind, usize)>> {
    scan_context(bytes, Context::Message, Policy::ALL, check)
}

pub(super) fn scan_with_policy(
    bytes: &[u8],
    message: bool,
    policy: Policy,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<Option<(SecretKind, usize)>> {
    scan_context(
        bytes,
        if message {
            Context::Message
        } else {
            Context::Content
        },
        policy,
        check,
    )
}

fn scan_context(
    bytes: &[u8],
    context: Context,
    policy: Policy,
    check: &mut dyn FnMut() -> Result<()>,
) -> Result<Option<(SecretKind, usize)>> {
    check()?;
    // A UTF-8 BOM is metadata, not part of the first source keyword. Otherwise
    // e.g. a BOM-prefixed `readonly` binding loses its reference exemption.
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    // Candidate prefixes can overlap inside armor comments. Bound the total
    // lookahead as well as each candidate, so a crafted header swarm cannot
    // repeatedly rescan the object. Exhaustion is a refusal, not a clean result.
    let mut armor_budget: usize = 2 * 1024 * 1024;
    for header in [
        b"-----BEGIN PRIVATE KEY-----".as_slice(),
        b"-----BEGIN ENCRYPTED PRIVATE KEY-----",
        b"-----BEGIN RSA PRIVATE KEY-----",
        b"-----BEGIN EC PRIVATE KEY-----",
        b"-----BEGIN OPENSSH PRIVATE KEY-----",
        b"-----BEGIN PGP PRIVATE KEY BLOCK-----",
    ]
    .into_iter()
    .filter(|_| policy.secrets)
    {
        if let Some(found) = find_candidate(bytes, b'-', check, |at| {
            if !bytes[at..].starts_with(header) {
                return None;
            }
            let (has_body, inspected) = private_key_body(&bytes[at + header.len()..]);
            let Some(remaining) = armor_budget.checked_sub(inspected) else {
                return Some((
                    SecretKind::Incomplete,
                    bytes[..at].iter().filter(|b| **b == b'\n').count() + 1,
                ));
            };
            armor_budget = remaining;
            if has_body {
                return Some((
                    SecretKind::PrivateKey,
                    bytes[..at].iter().filter(|b| **b == b'\n').count() + 1,
                ));
            }
            None
        })? {
            return Ok(Some(found));
        }
    }
    if let Some(found) = ascii_scan(bytes, context, policy, check)? {
        return Ok(Some(found));
    }
    // Windows-authored configuration files often use UTF-16. Never execute or
    // decode archives; this only normalizes BOM-marked UTF-16 text locally.
    let little = if bytes.starts_with(&[0xff, 0xfe]) {
        true
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        false
    } else {
        check()?;
        return Ok(None);
    };
    let words = bytes[2..].as_chunks::<2>().0.iter().map(|b| {
        if little {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        }
    });
    let mut text = String::new();
    for (at, character) in char::decode_utf16(words).enumerate() {
        checkpoint(at, check)?;
        text.push(character.unwrap_or(char::REPLACEMENT_CHARACTER));
    }
    scan_context(text.as_bytes(), context, policy, check)
}

fn private_key_body(rest: &[u8]) -> (bool, usize) {
    let Some(newline) = [b"\\r\\n".as_slice(), b"\\n", b"\r\n", b"\n"]
        .into_iter()
        .find(|newline| rest.starts_with(newline))
    else {
        return (false, 0);
    };
    // Armor may have a blank separator and metadata (OpenPGP or traditional
    // encrypted PEM). Bound each candidate so repeated misleading headers cannot
    // make us repeatedly walk the entire object. This is a shape detector, not a
    // complete key parser; excessively long metadata remains outside its scope.
    let rest = &rest[..rest.len().min(16 * 1024)];
    let mut body = &rest[newline.len()..];
    let mut inspected = newline.len();
    loop {
        let end = body.windows(newline.len()).position(|w| w == newline);
        inspected += end.map_or(body.len(), |at| at + newline.len());
        let line = &body[..end.unwrap_or(body.len())];
        let line = line.trim_ascii();
        let metadata = [
            b"Version:".as_slice(),
            b"Comment:",
            b"MessageID:",
            b"Hash:",
            b"Charset:",
            b"Proc-Type:",
            b"DEK-Info:",
        ]
        .iter()
        .any(|prefix| line.starts_with(prefix));
        // Preserve detection of truncated/embedded key material: the bytes after
        // the base64 prefix may be a quote, a binary delimiter or a missing tail.
        if !metadata
            && line
                .iter()
                .take_while(|c| c.is_ascii_alphanumeric() || b"+/=".contains(c))
                .count()
                >= 16
        {
            return (true, inspected);
        }
        if !line.is_empty() && !metadata {
            return (false, inspected);
        }
        let Some(end) = end else {
            return (false, inspected);
        };
        body = &body[end + newline.len()..];
    }
}

pub(super) fn safe_path(bytes: &[u8]) -> String {
    // Display-only sanitization must not repeat a huge uncancellable content
    // scan on a malformed tree filename. The guarded path scan still runs first.
    if bytes.len() > 4096 {
        return "[long filename hidden]".into();
    }
    if scan(bytes).is_some() {
        return "[sensitive filename hidden]".into();
    }
    String::from_utf8_lossy(bytes)
        .chars()
        .take(240)
        .flat_map(|c| c.escape_default())
        .collect()
}

#[cfg(test)]
#[path = "pattern_tests.rs"]
mod extended_tests;

#[cfg(test)]
#[path = "ai_key_tests.rs"]
mod ai_key_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_references_labels_and_templates_are_not_credentials() {
        for text in [
            "pub token: CancellationToken,",
            "pub token: Option<String>,",
            "pub password: SecretString,",
            "let token = cancellation_token;",
            "type Token = CancellationToken;",
            "token: cancellation_token,",
            "token: &cancel_token,",
            "let password = r#\"Enter your password\"#;",
            "const token = existingToken;",
            "token: CancellationToken | undefined;",
            "const token = await getToken();",
            "token: suppliedToken ?? fallbackToken,",
            "if (password === expectedPassword) {}",
            "var password = inputPassword;",
            "token = existing_token",
            "token := requestToken",
            "Token: requestToken,",
            "TOKEN=$GITHUB_TOKEN",
            "$token = $env:GITHUB_TOKEN",
            "{\"password\": \"Enter your password\"}",
            "{\"password\": \"Mot de passe\"}",
            "{\"token\": \"Authentication\"}",
            "<input type=\"password\" autocomplete=\"current-password\">",
            "password: *database_password",
            "password: \"{{ .Values.database.password }}\"",
            "token: \"{{ github_token }}\"",
            "token: '${{ secrets.GITHUB_TOKEN }}'",
            "password: ${DATABASE_PASSWORD:-}",
            "token = configuration.github.token",
            "token = std::env::var(\"GITHUB_TOKEN\")",
            "token: process.env.GITHUB_TOKEN",
            "password = \"example-password\"",
            "token = \"replace_me\"",
            "token = \"\"",
            "token = \"<github-token>\"",
            "token = \"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\"",
            "Use -----BEGIN PRIVATE KEY----- to identify PEM material.",
            "GitHub tokens use ghp_ and github_pat_ prefixes.",
            "fn ghp_authentication_configuration_helpers() {}",
            "Fix token: CancellationToken ownership and lifetime",
            concat!("pub token: ", "OAuth2AuthenticationTokenV3,"),
            concat!("const token = ", "authStateVersion12Token;"),
            concat!("readonly token: ", "AccessTokenVersion123;"),
        ] {
            assert_eq!(
                scan(text.as_bytes()),
                None,
                "ordinary source/template: {text}"
            );
        }
    }

    #[test]
    fn literal_credentials_still_match_without_treating_names_as_secrets() {
        let value = "n6F0dL8pR2vW4sT9aQ3x";
        for text in [
            format!("const token = \"{value}\";"),
            format!("export const token = '{value}';"),
            format!("TOKEN={value}"),
            format!("{{\"label\":\"safe\",\"token\":\"{value}\"}}"),
            format!("token=\"\"; client_secret=\"{value}\""),
            format!("password = \"{}\"", "B8m!q2W#t7N$p4Z@"),
            format!("password = \"{}\"", "CorrectHorseBatteryStaple"),
            format!("api_key = \"{}\"", value.repeat(30)),
        ] {
            assert_eq!(scan(text.as_bytes()), Some((SecretKind::Credential, 1)));
        }
        // No broad test-file, comment or template allowlist may hide a provider
        // token or key just because the generic value would be ignored.
        let github = format!("ghp_{}", "Q3".repeat(18));
        for text in [
            format!("// token: {github}"),
            format!("token=\"${{{github}}}\""),
            format!("token=\"{{{{ {github} }}}}\""),
            format!("token=\"example-{github}\""),
        ] {
            assert_eq!(scan(text.as_bytes()), Some((SecretKind::GithubToken, 1)));
        }
    }

    #[test]
    fn github_alphabets_preserve_classic_and_stateless_formats() {
        for prefix in ["ghp_", "gho_", "ghu_", "ghr_"] {
            assert_eq!(
                scan(format!("{prefix}authentication_configuration_helpers").as_bytes()),
                None
            );
            assert_eq!(
                scan(format!("{prefix}{}", "A7".repeat(18)).as_bytes()),
                Some((SecretKind::GithubToken, 1))
            );
        }
        let installation = format!("ghs_1_eyJhbGciOiJIUzI1NiJ9.e30.{}", "Q3-_".repeat(30));
        assert_eq!(
            scan(installation.as_bytes()),
            Some((SecretKind::GithubToken, 1))
        );
        assert_eq!(scan(b"Use ghs_APPID_JWT"), None);
        let fine_grained = format!("github_pat_{}_{}", "A7".repeat(11), "z5Q".repeat(19) + "a7");
        assert_eq!(
            scan(fine_grained.as_bytes()),
            Some((SecretKind::GithubToken, 1))
        );
    }

    #[test]
    fn template_references_do_not_hide_private_key_material() {
        let private = ["-----BEGIN ", "PRIVATE KEY-----\n", &"A7".repeat(40)].concat();
        let text = format!("token=\"${{value}}\"\n{private}");
        assert_eq!(scan(text.as_bytes()), Some((SecretKind::PrivateKey, 2)));
    }

    #[test]
    fn detects_github_families_private_keys_and_literal_credentials() {
        for prefix in ["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_"] {
            let token = format!("{prefix}{}", "A7".repeat(40));
            assert_eq!(scan(token.as_bytes()), Some((SecretKind::GithubToken, 1)));
            assert_eq!(safe_path(token.as_bytes()), "[sensitive filename hidden]");
        }
        let private = ["-----BEGIN ", "OPENSSH PRIVATE KEY-----\n", &"A".repeat(40)].concat();
        assert_eq!(scan(private.as_bytes()), Some((SecretKind::PrivateKey, 1)));
        assert_eq!(
            scan(b"hello\nCLIENT_SECRET=abc123456789\n"),
            Some((SecretKind::Credential, 2))
        );
        assert_eq!(
            scan(
                concat!(
                    "{\"github_token\"",
                    ": \"0123456789abcdef0123456789abcdef01234567\"}"
                )
                .as_bytes()
            ),
            Some((SecretKind::Credential, 1))
        );
    }
    #[test]
    fn permits_templates_references_and_short_examples_but_scans_template_content() {
        for text in [
            "TOKEN=",
            "TOKEN=${GITHUB_TOKEN}",
            "token = process.env.GITHUB_TOKEN",
            "password = \"YOUR_PASSWORD\"",
            "client_secret: null",
            "token: \"fixture-test-token\"",
            "example ghp_example",
        ] {
            assert_eq!(scan(text.as_bytes()), None, "{text}");
        }
        for name in [
            b".env".as_slice(),
            b".ENV",
            b".env.local",
            b"production.env",
        ] {
            assert!(environment_file(name));
        }
        for name in [
            b".env.example".as_slice(),
            b".env.sample",
            b".env.template",
            b"environment.rs",
        ] {
            assert!(!environment_file(name));
        }
    }
    #[test]
    fn recognizes_utf16_and_embedded_binary_tokens_without_printing_them() {
        let secret = format!("prefix\nghp_{}", "Q3".repeat(18));
        let mut encoded = vec![0xff, 0xfe];
        encoded.extend(secret.encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(scan(&encoded), Some((SecretKind::GithubToken, 2)));
        let bytes = [b"\0\xff".as_slice(), secret.as_bytes(), b"\0"].concat();
        assert_eq!(scan(&bytes), Some((SecretKind::GithubToken, 2)));
        let private = ["-----BEGIN ", "PRIVATE KEY-----\n", &"A".repeat(40)].concat();
        for little in [true, false] {
            let mut encoded = if little {
                vec![0xff, 0xfe]
            } else {
                vec![0xfe, 0xff]
            };
            encoded.extend(private.encode_utf16().flat_map(|word| {
                if little {
                    word.to_le_bytes()
                } else {
                    word.to_be_bytes()
                }
            }));
            assert_eq!(scan(&encoded), Some((SecretKind::PrivateKey, 1)));
        }
    }
}
