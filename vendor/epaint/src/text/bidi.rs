//! Bidirectional text levels.
//!
//! Download Engine patch to epaint 0.36.2 (see `patch.md`). Upstream epaint
//! shapes each font run in one direction, so Latin words and digits inside Arabic
//! text were drawn reversed. This module resolves embedding levels with the parts of
//! the Unicode Bidirectional Algorithm (UAX #9) that plain interface text needs:
//! weak types (W1–W7), paired brackets (N0), neutrals (N1–N2), implicit levels
//! (I1–I2) and the separator part of L1. `text_layout` reorders each line (L1–L2).
//! Explicit embeddings, overrides and isolates (U+202A–U+202E, U+2066–U+2069) are not
//! supported; like before, they are invisible and do not change the levels.

use icu_properties::{
    CodePointMapData,
    props::{BidiClass, BidiMirroringGlyph, BidiPairedBracketType},
};

/// Bidi character types used by the resolution rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    L,
    R,
    AL,
    EN,
    ES,
    ET,
    AN,
    CS,
    NSM,
    /// Boundary neutrals, and the unsupported explicit formatting characters.
    BN,
    B,
    S,
    WS,
    ON,
}

fn class(c: char) -> Class {
    let class = CodePointMapData::<BidiClass>::new().get(c);
    if class == BidiClass::LeftToRight {
        Class::L
    } else if class == BidiClass::RightToLeft {
        Class::R
    } else if class == BidiClass::ArabicLetter {
        Class::AL
    } else if class == BidiClass::EuropeanNumber {
        Class::EN
    } else if class == BidiClass::EuropeanSeparator {
        Class::ES
    } else if class == BidiClass::EuropeanTerminator {
        Class::ET
    } else if class == BidiClass::ArabicNumber {
        Class::AN
    } else if class == BidiClass::CommonSeparator {
        Class::CS
    } else if class == BidiClass::NonspacingMark {
        Class::NSM
    } else if class == BidiClass::ParagraphSeparator {
        Class::B
    } else if class == BidiClass::SegmentSeparator {
        Class::S
    } else if class == BidiClass::WhiteSpace {
        Class::WS
    } else if class == BidiClass::OtherNeutral {
        Class::ON
    } else {
        // BN, and LRE, RLE, LRO, RLO, PDF, LRI, RLI, FSI, PDI (ignored, see above).
        Class::BN
    }
}

/// Whether `c` belongs to a right-to-left script block or is an Arabic digit.
///
/// Text without such characters keeps upstream epaint's layout path unchanged.
/// The ranges are deliberately wide: inside them every run is shaped with an explicit
/// direction instead of harfrust's script guess, which reversed mixed runs.
#[inline]
pub(super) fn needs_bidi(c: char) -> bool {
    matches!(
        c,
        '\u{0590}'..='\u{08FF}'
            | '\u{FB1D}'..='\u{FDFF}'
            | '\u{FE70}'..='\u{FEFE}'
            | '\u{10800}'..='\u{10FFF}'
            | '\u{1E800}'..='\u{1EFFF}'
    )
}

/// Whether `c` is a strong right-to-left character (bidi class R or AL).
///
/// A paragraph that contains one is laid out right to left. The native interface is
/// not mirrored and egui has no paragraph direction, so this rule (rather than the
/// first strong character) keeps translated Arabic text that starts with a Latin word
/// or a number, such as "SHA-256 المتوقع", in Arabic reading order.
#[inline]
pub(super) fn is_rtl_letter(c: char) -> bool {
    needs_bidi(c) && matches!(class(c), Class::R | Class::AL)
}

/// Resolves the embedding level of every byte of `text`, one paragraph without
/// newlines, with `base_level` as the paragraph level (0 or 1).
pub(super) fn resolve_levels(text: &str, base_level: u8, levels: &mut Vec<u8>) {
    levels.clear();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let original: Vec<Class> = chars.iter().map(|&(_, c)| class(c)).collect();
    let embedding = if base_level % 2 == 0 {
        Class::L
    } else {
        Class::R
    };

    // X9: boundary neutrals do not take part in the weak and neutral rules.
    let kept: Vec<usize> = (0..chars.len())
        .filter(|&i| original[i] != Class::BN)
        .collect();
    let mut types: Vec<Class> = kept.iter().map(|&i| original[i]).collect();

    resolve_weak_types(&mut types, embedding);
    resolve_paired_brackets(&mut types, &kept, &chars, &original, embedding);
    resolve_neutral_types(&mut types, embedding);

    // I1, I2.
    let mut char_levels = vec![base_level; chars.len()];
    for (&i, &class) in kept.iter().zip(&types) {
        char_levels[i] = match (base_level % 2, class) {
            (0, Class::R) => base_level + 1,
            (0, Class::AN | Class::EN) => base_level + 2,
            (1, Class::L | Class::AN | Class::EN) => base_level + 1,
            _ => base_level,
        };
    }
    // Removed boundary neutrals take the level of the preceding character.
    for i in 0..chars.len() {
        if original[i] == Class::BN {
            char_levels[i] = if i == 0 {
                base_level
            } else {
                char_levels[i - 1]
            };
        }
    }
    // L1: separators, and whitespace before them or at the end, use the paragraph level.
    let mut trailing = true;
    for i in (0..chars.len()).rev() {
        match original[i] {
            Class::S | Class::B => {
                char_levels[i] = base_level;
                trailing = true;
            }
            Class::WS | Class::BN if trailing => char_levels[i] = base_level,
            _ => trailing = false,
        }
    }

    levels.resize(text.len(), base_level);
    for (index, &(start, c)) in chars.iter().enumerate() {
        levels[start..start + c.len_utf8()].fill(char_levels[index]);
    }
}

/// W1–W7.
fn resolve_weak_types(types: &mut [Class], embedding: Class) {
    // W1: a nonspacing mark takes the type of the previous character.
    let mut previous = embedding;
    for class in types.iter_mut() {
        if *class == Class::NSM {
            *class = previous;
        }
        previous = *class;
    }
    // W2: European numbers after an Arabic letter are Arabic numbers. W3: AL is R.
    let mut last_strong = embedding;
    for class in types.iter_mut() {
        match *class {
            Class::L | Class::R => last_strong = *class,
            Class::AL => {
                last_strong = Class::AL;
                *class = Class::R;
            }
            Class::EN if last_strong == Class::AL => *class = Class::AN,
            _ => {}
        }
    }
    // W4: a single separator between two numbers of the same kind joins them.
    for i in 1..types.len().saturating_sub(1) {
        let (before, after) = (types[i - 1], types[i + 1]);
        types[i] = match (before, types[i], after) {
            (Class::EN, Class::ES | Class::CS, Class::EN) => Class::EN,
            (Class::AN, Class::CS, Class::AN) => Class::AN,
            (_, class, _) => class,
        };
    }
    // W5: terminators next to European numbers, such as % or a currency sign.
    let mut i = 0;
    while i < types.len() {
        if types[i] != Class::ET {
            i += 1;
            continue;
        }
        let start = i;
        while i < types.len() && types[i] == Class::ET {
            i += 1;
        }
        let touches_number = (start > 0 && types[start - 1] == Class::EN)
            || (i < types.len() && types[i] == Class::EN);
        if touches_number {
            types[start..i].fill(Class::EN);
        }
    }
    // W6: remaining separators and terminators are neutral.
    for class in types.iter_mut() {
        if matches!(*class, Class::ES | Class::ET | Class::CS) {
            *class = Class::ON;
        }
    }
    // W7: European numbers after left-to-right text are left-to-right.
    let mut last_strong = embedding;
    for class in types.iter_mut() {
        match *class {
            Class::L | Class::R => last_strong = *class,
            Class::EN if last_strong == Class::L => *class = Class::L,
            _ => {}
        }
    }
}

/// The direction a resolved type gives to neutrals: numbers count as right-to-left.
fn strong_direction(class: Class) -> Option<Class> {
    match class {
        Class::L => Some(Class::L),
        Class::R | Class::EN | Class::AN => Some(Class::R),
        _ => None,
    }
}

/// BD16 and N0: a bracket pair takes the direction of the text it encloses, so
/// "Windows (x64)" inside Arabic text keeps both parentheses with the Latin text.
fn resolve_paired_brackets(
    types: &mut [Class],
    kept: &[usize],
    chars: &[(usize, char)],
    original: &[Class],
    embedding: Class,
) {
    // Canonically equivalent angle brackets pair with each other.
    let canonical = |c: char| match c {
        '\u{2329}' => '\u{3008}',
        '\u{232A}' => '\u{3009}',
        c => c,
    };
    let mirroring = CodePointMapData::<BidiMirroringGlyph>::new();
    let mut openers: Vec<(char, usize)> = Vec::new();
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for (k, &i) in kept.iter().enumerate() {
        if types[k] != Class::ON {
            continue;
        }
        let c = chars[i].1;
        let glyph = mirroring.get(c);
        match glyph.paired_bracket_type {
            BidiPairedBracketType::Open => {
                if openers.len() == 63 {
                    break;
                }
                if let Some(closing) = glyph.mirroring_glyph {
                    openers.push((canonical(closing), k));
                }
            }
            BidiPairedBracketType::Close => {
                if let Some(position) = openers
                    .iter()
                    .rposition(|&(closing, _)| closing == canonical(c))
                {
                    pairs.push((openers[position].1, k));
                    openers.truncate(position);
                }
            }
            _ => {}
        }
    }
    pairs.sort_unstable();

    for (open, close) in pairs {
        let mut found_embedding = false;
        let mut found_opposite = false;
        for &class in &types[open + 1..close] {
            match strong_direction(class) {
                Some(direction) if direction == embedding => {
                    found_embedding = true;
                    break;
                }
                Some(_) => found_opposite = true,
                None => {}
            }
        }
        let direction = if found_embedding {
            embedding
        } else if found_opposite {
            // N0 c: follow the context before the opening bracket.
            types[..open]
                .iter()
                .rev()
                .find_map(|&class| strong_direction(class))
                .unwrap_or(embedding)
        } else {
            continue;
        };
        for bracket in [open, close] {
            types[bracket] = direction;
            // Marks that followed the bracket before W1 change with it.
            let mut k = bracket + 1;
            while k < types.len() && original[kept[k]] == Class::NSM {
                types[k] = direction;
                k += 1;
            }
        }
    }
}

/// N1, N2: neutrals between text of one direction take that direction, other
/// neutrals take the paragraph direction.
fn resolve_neutral_types(types: &mut [Class], embedding: Class) {
    let is_neutral = |class: Class| matches!(class, Class::B | Class::S | Class::WS | Class::ON);
    let mut i = 0;
    while i < types.len() {
        if !is_neutral(types[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < types.len() && is_neutral(types[i]) {
            i += 1;
        }
        let before = if start == 0 {
            embedding
        } else {
            strong_direction(types[start - 1]).unwrap_or(embedding)
        };
        let after = if i == types.len() {
            embedding
        } else {
            strong_direction(types[i]).unwrap_or(embedding)
        };
        let direction = if before == after { before } else { embedding };
        types[start..i].fill(direction);
    }
}
