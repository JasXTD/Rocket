//! Static web assets in the pinned Git tree. A target needs only its tree entry;
//! images, fonts and other asset bodies are never opened for this analysis.
mod css;
mod html;
mod named_entities;

use super::{CancellationToken, LABEL_BYTES, Result, cargo, packages};
use std::collections::HashSet;
use std::ops::Range;
use super::references::SourceLocation;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Format {
    Html,
    Css,
}

pub(super) fn format(path: &[u8]) -> Option<Format> {
    let ext = packages::extension(path);
    if ext.eq_ignore_ascii_case(b"html") || ext.eq_ignore_ascii_case(b"htm") {
        Some(Format::Html)
    } else if ext.eq_ignore_ascii_case(b"css") {
        Some(Format::Css)
    } else {
        None
    }
}

/// Strip URL components before decoding, keeping an encoded '#' inside a filename.
/// Encoded separators never introduce a new filesystem component.
fn local_path(base: &[u8], raw: &str, root_site: bool) -> Option<(Vec<u8>, bool)> {
    let raw = raw.trim_matches(|c: char| c.is_ascii_whitespace());
    let path = raw.split(['?', '#']).next()?;
    if path.is_empty()
        || path.len() > LABEL_BYTES
        || path.starts_with("//")
        || path.contains(['\\', '{', '}', '<', '>', '`'])
        || path.contains("\u{fffd}")
    {
        return None;
    }
    let (base, path) = if let Some(relative) = path.strip_prefix('/') {
        if !root_site {
            return None;
        }
        (b"".as_slice(), relative)
    } else {
        (base, path)
    };
    let mut bytes = path.bytes();
    let mut decoded = Vec::with_capacity(path.len());
    while let Some(mut b) = bytes.next() {
        if b == b'%' {
            let hi = (bytes.next()? as char).to_digit(16)?;
            let lo = (bytes.next()? as char).to_digit(16)?;
            b = ((hi << 4) | lo) as u8;
            if matches!(b, b'/' | b'\\') {
                return None;
            }
        }
        if b.is_ascii_control() || matches!(b, b':' | b'\\') {
            return None;
        }
        decoded.push(b);
    }
    let directory = matches!(decoded.rsplit(|&b| b == b'/').next(), Some(b"" | b"." | b".."));
    let joined = if decoded.is_empty() {
        Vec::new()
    } else {
        cargo::relative(base, &decoded)?
    };
    (joined.len() <= LABEL_BYTES).then_some((joined, directory))
}

fn page_base(path: &[u8], href: &str, root_site: bool) -> Option<Vec<u8>> {
    let folder = packages::parent(path);
    if href.trim().split(['?', '#']).next()?.is_empty() {
        return Some(folder.to_vec());
    }
    let (target, directory) = local_path(folder, href, root_site)?;
    Some(if directory {
        target
    } else {
        packages::parent(&target).to_vec()
    })
}

/// Byte position of a slice borrowed from this parser input.
fn offset(input: &str, slice: &str) -> usize {
    slice.as_ptr() as usize - input.as_ptr() as usize
}

pub(super) fn read(
    path: &[u8],
    source: &str,
    known: &HashSet<&[u8]>,
    remaining: &mut usize,
    cancel: &CancellationToken,
    mut found: impl FnMut(&'static str, Vec<u8>, SourceLocation),
) -> Result<bool> {
    cancel.check()?;
    let Some(format) = format(path) else { return Ok(true) };
    let root_site = known.contains(b"index.html".as_slice()) || known.contains(b"index.htm".as_slice());
    let base = match format {
        Format::Html => match html::base_directory(source, path, root_site, cancel)? {
            Some(folder) => folder,
            None => return Ok(true),
        },
        Format::Css => packages::parent(path).to_vec(),
    };
    let newlines = std::cell::OnceCell::new();
    let mut emit = |kind, url: &str, span: Range<usize>| {
        if *remaining == 0 {
            return false;
        }
        *remaining -= 1;
        if let Some((target, false)) = local_path(&base, url, root_site)
            && known.contains(target.as_slice())
        {
            let breaks = newlines.get_or_init(|| source.match_indices('\n').map(|(i, _)| i).collect::<Vec<_>>());
            let row = breaks.partition_point(|&i| i < span.start);
            let start = row.checked_sub(1).map_or(0, |i| breaks[i] + 1);
            let end = breaks.get(row).copied().unwrap_or(source.len()).min(span.end);
            found(kind, target, SourceLocation {
                line: row as u32 + 1,
                columns: ((span.start - start) as u32, (end - start) as u32),
            });
        }
        true
    };
    match format {
        Format::Html => html::read(source, cancel, &mut emit),
        Format::Css => css::read(source, cancel, &mut emit),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links(path: &str, source: &str, files: &[&str]) -> Vec<String> {
        let known = files.iter().map(|s| s.as_bytes()).collect();
        let mut result = Vec::new();
        assert!(
            read(
                path.as_bytes(),
                source,
                &known,
                &mut 1000,
                &CancellationToken::new(),
                |kind, p, _| {
                    result.push(format!("{kind} {}", String::from_utf8(p).unwrap()));
                }
            )
            .unwrap()
        );
        result
    }

    #[test]
    fn html_attributes_raw_text_and_inline_styles() {
        assert_eq!(
            links("p.htm", "<!-- a > b ---><img src=real>", &["real"]),
            ["image real"]
        );
        let html = r##"<!-- <script src="fake"></script> ---><!--><!----!>
            <title><img src=fake></title><textarea><img src=fake></textarea>
            <template><template><img src=fake></template><base href="https://example.invalid/"></template>
            <noscript><img src=fake></noscript><script>let html = "<img src=fake>";</script>
            <LINK title=">" rel="alternate StyleSheet" href="a&amp;b.css?rev=1#main">
            <link rel=next href=fake><script SRC=one.js src=fake></script>
            <img src='&#x732b;.png'><input type=image src="input.png">
            <style>body{background:url(inside.png)}</style>
            <div style="background:url(&quot;two words.png&quot;)"></div>
            <link rel=preload as=image imagesrcset="two.png 2x">
            <img data-src=fake><source srcset="fake 1x">
            <img src="https://example.invalid/fake"><img src="//example.invalid/fake"><img src="#fake">
            <img src=fake title='unterminated >"##;
        assert_eq!(
            links(
                "doc.HTML",
                html,
                &[
                    "a&b.css",
                    "one.js",
                    "猫.png",
                    "input.png",
                    "inside.png",
                    "two words.png",
                    "two.png",
                    "fake"
                ]
            ),
            [
                "stylesheet a&b.css",
                "script one.js",
                "image 猫.png",
                "image input.png",
                "url() inside.png",
                "url() two words.png",
                "srcset two.png"
            ]
        );
    }

    #[test]
    fn named_html_entities_resolve_the_same_assets_as_numeric_references() {
        let html = r#"<base href="caf&eacute;/"><img src="caf&eacute;.png">
            <img src="caf&#233;.png"><img src="x&NotEqualTilde;.png">
            <img srcset="&copy;.png 1x"><div style="background:url(&quot;&pound;.png&quot;)"></div>"#;
        let files = ["café/café.png", "café/x≂̸.png", "café/©.png", "café/£.png"];
        let known = files.iter().map(|p| p.as_bytes()).collect();
        let mut found = Vec::new();
        assert!(read(b"index.html", html, &known, &mut 10, &CancellationToken::new(), |kind, target, site| {
            let line = html.lines().nth(site.line as usize - 1).unwrap();
            let (start, end) = site.columns;
            found.push((kind, String::from_utf8(target).unwrap(), &line[start as usize..end as usize]));
        }).unwrap());
        let found: Vec<_> = found.iter().map(|(kind, target, raw)| (*kind, target.as_str(), *raw)).collect();
        assert_eq!(found, [
            ("image", "café/café.png", "caf&eacute;.png"), ("image", "café/café.png", "caf&#233;.png"),
            ("image", "café/x≂̸.png", "x&NotEqualTilde;.png"), ("srcset", "café/©.png", "&copy;.png"),
            ("url()", "café/£.png", "&pound;.png"),
        ]);
        assert_eq!(links("index.html", "<img src='caf&eacute.png'><img src='&copy=x'><img src='&fake;.png'>",
            &["café.png", "&copy=x", "&fake;.png"]), ["image café.png", "image &copy=x", "image &fake;.png"]);
    }

    #[test]
    fn physical_lines_survive_entities_multiline_attributes_and_raw_styles() {
        let html = concat!(
            "<img src='\r\n a.png' srcset='missing&#32;1x,&#10;\r\n", // lines 1 and 2
            " a.png 2x'>\n",                                           // 3
            "<div style='color:red&#59; background:url(&quot;a.png&quot;);\n", // 4
            " mask:url(\n a.png)'></div>\n",                         // 5 and 6
            "<style>p { background: url(\n a.png) }</style>"            // 7 and 8
        );
        let known = HashSet::from([b"a.png".as_slice()]);
        let mut sites = Vec::new();
        let mut budget = 10;
        assert!(read(b"index.html", html, &known, &mut budget, &CancellationToken::new(),
            |kind, _, site| sites.push((kind, site.line))).unwrap());
        assert_eq!(sites, [("image", 2), ("srcset", 3), ("url()", 4), ("url()", 6), ("url()", 8)]);
    }

    #[test]
    fn srcset_candidates_keep_embedded_commas_and_validate_descriptors() {
        let html = r#"<picture><source srcset="data:image/png;base64,AAAA 1x, comma,ok.png 2x, bad 2x 3x, wide.png 320w 200h, bad 10h">
            <img srcset="plain.png,  bad unknown(a,b), unique.png .5x"></picture>"#;
        assert_eq!(
            links(
                "index.htm",
                html,
                &["AAAA", "comma,ok.png", "bad", "wide.png", "plain.png", "unique.png"]
            ),
            [
                "srcset comma,ok.png",
                "srcset wide.png",
                "srcset plain.png",
                "srcset unique.png"
            ]
        );
    }

    #[test]
    fn first_base_applies_to_the_entire_page_and_external_bases_do_not_guess() {
        let docs = ["pages/a.css", "assets/a.css", "a.css", "index.html"];
        let tag = "<link rel=stylesheet href='a.css'>";
        for (base, want) in [
            ("../assets/", "assets/a.css"),
            ("../assets/index.htm?q=x", "assets/a.css"),
            ("../assets/.", "assets/a.css"),
            ("?x", "pages/a.css"),
            ("/", "a.css"),
        ] {
            assert_eq!(
                links(
                    "pages/p.htm",
                    &format!("{tag}<base href='{base}'><base href='ignored/'>"),
                    &docs
                ),
                [format!("stylesheet {want}")]
            );
        }
        for base in [
            "https://outside.invalid/",
            "//outside.invalid",
            "data:html,x",
            "../../",
            "{{asset_root}}",
        ] {
            assert!(links("pages/p.htm", &format!("{tag}<base href='{base}'>"), &docs).is_empty());
        }
        assert!(links("pages/p.htm", "<img src=/logo.png>", &["logo.png"]).is_empty());
        assert_eq!(
            links("pages/p.htm", "<img src=/logo.png>", &["logo.png", "index.html"]),
            ["image logo.png"]
        );
    }

    #[test]
    fn css_imports_urls_escapes_and_non_references() {
        let css = r##"/* @import 'fake'; */ @im\70ort "sub/theme.css?mode" layer(theme);
            @import URL(../reset.css#base) print;
            @font-face { src: local("example"), url('../fonts/f\6f nt.woff2?v=1') format("woff2"); }
            p::before {content:"url(fake)";background:url(../img/green\ leaf.png)}
            a{background-image:image-set(URL('../img/猫.png') 1x, url(../img/large.png) 2x)}
            x{a:my-url(fake);b:url(var(--x));c:url('fake' bad);d:url(#self);e:url(https://example.invalid/fake)}
            #url(fake) {color:black}"##;
        assert_eq!(
            links(
                "styles/site.CSS",
                css,
                &[
                    "styles/sub/theme.css",
                    "reset.css",
                    "fonts/font.woff2",
                    "img/green leaf.png",
                    "img/猫.png",
                    "img/large.png",
                    "styles/fake"
                ]
            ),
            [
                "@import styles/sub/theme.css",
                "@import reset.css",
                "url() fonts/font.woff2",
                "url() img/green leaf.png",
                "url() img/猫.png",
                "url() img/large.png"
            ]
        );
    }

    #[test]
    fn uri_components_and_escape_boundaries_are_checked_before_matching() {
        let css = r##"a{a:url(img/%E7%8C%AB%23face.png?v=1#crop); b:url(%2e%2e/leak);
            c:url(img%2fcat.png);d:url(img%5ccat.png);e:url(bad%00);f:url(bad%ff/);
            g:url(bad%xx);h:url(file:///leak);i:url(?inline);j:url(${asset})}"##;
        assert_eq!(
            links("page.css", css, &["img/猫#face.png", "leak", "img/cat.png", "bad%xx"]),
            ["url() img/猫#face.png"]
        );
    }

    #[test]
    fn cancellation_during_both_readers_and_a_shared_reference_budget() {
        let known: HashSet<&[u8]> = [b"x".as_slice()].into_iter().collect();
        for (file, source) in [
            (b"a.htm".as_slice(), "<img src=x>".repeat(500)),
            (b"a.css", "a{src:url(x)}".repeat(500)),
        ] {
            let cancel = CancellationToken::new();
            let mut budget = 12000;
            assert!(matches!(
                read(file, &source, &known, &mut budget, &cancel, |_, _, _| cancel.cancel()),
                Err(crate::Error::Cancelled)
            ));
        }
        let mut budget = 2;
        let mut n = 0;
        let t = CancellationToken::new();
        assert!(
            read(
                b"a.htm",
                "<img src=missing>",
                &known,
                &mut budget,
                &t,
                |_, _, _| unreachable!()
            )
            .unwrap()
        );
        assert!(!read(b"a.css", "a{a:url(x);b:url(x)}", &known, &mut budget, &t, |_, _, _| n += 1).unwrap());
        assert_eq!((n, budget), (1, 0));
    }
}
