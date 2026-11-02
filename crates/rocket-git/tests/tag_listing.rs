mod common;
use common::{Fixture, isolated};
use rocket_git::*;

fn token() -> CancellationToken {
    CancellationToken::new()
}
fn head(f: &Fixture) -> ObjectId {
    f.repo().resolve_commit("HEAD", &token()).unwrap()
}
/// `git tag`, with the tag (and so its tagger date) made at `date`.
fn tag_at(f: &Fixture, date: &str, arguments: &[&str]) {
    let mut all = vec!["tag"];
    all.extend(arguments);
    let output = f
        .command(&f.repository, &all)
        .env("GIT_COMMITTER_DATE", date)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn listed_tags_carry_their_commit_subject_date_and_target_in_either_order() {
    isolated(
        "ROCKET_TAG_LISTING_CASE",
        "listed_tags_carry_their_commit_subject_date_and_target_in_either_order",
        listing,
    );
}
fn listing() {
    for format in ["sha1", "sha256"] {
        let f = Fixture::with_format(format);
        f.write("a", "first\n");
        f.commit("First 中文 subject\n\nBody line");
        let first = head(&f);
        f.write("a", "second\n");
        f.commit("Second 🚀");
        let second = head(&f);
        // Lightweight tags take their commit's date (the fixture's 2026-01-01).
        f.run(&["tag", "light", first.as_str()]);
        tag_at(&f, "2026-03-01T00:00:00+00:00", &["-a", "-m", "Release 文\n\nNotes", "zeta/新"]);
        tag_at(&f, "2026-02-01T00:00:00+00:00", &["-a", "-m", "Older", "alpha", first.as_str()]);
        let by_name = f.repo().tags(10, &token()).unwrap();
        let names = |page: &TagPage| {
            page.entries
                .iter()
                .map(|e| String::from_utf8(e.name.clone()).unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&by_name), ["alpha", "light", "zeta/新"]);
        assert_eq!(by_name.summaries.len(), by_name.entries.len());
        let newest = f.repo().tags_ordered(10, TagOrder::Newest, &token()).unwrap();
        assert_eq!(names(&newest), ["zeta/新", "alpha", "light"]);
        let summary = |page: &TagPage, name: &str| {
            let i = page.entries.iter().position(|e| e.name == name.as_bytes()).unwrap();
            page.summaries[i].clone()
        };
        // Annotated: the peeled commit, its subject (not the tag's) and the tagger date.
        assert_eq!(
            summary(&newest, "zeta/新"),
            TagSummary {
                target: Some(second.clone()),
                target_type: "commit".into(),
                subject: "Second 🚀".as_bytes().to_vec(),
                date: Some(1_772_323_200),
            }
        );
        assert_eq!(summary(&newest, "alpha").subject, "First 中文 subject".as_bytes());
        // Lightweight: the commit itself, dated by its committer.
        assert_eq!(
            summary(&newest, "light"),
            TagSummary {
                target: Some(first.clone()),
                target_type: "commit".into(),
                subject: "First 中文 subject".as_bytes().to_vec(),
                date: Some(1_767_196_800),
            }
        );
        // The entries stay exact identities for reviews.
        let entry = newest.entries.iter().find(|e| e.name == b"zeta/\xe6\x96\xb0").unwrap();
        assert!(entry.annotated());
        f.repo().preview_delete_tag(entry, &token()).unwrap();
        // The limit applies after ordering.
        let page = f.repo().tags_ordered(1, TagOrder::Newest, &token()).unwrap();
        assert!(page.more);
        assert_eq!(names(&page), ["zeta/新"]);
        assert_eq!(page.summaries.len(), 1);
    }
}

#[test]
fn annotation_parts_separate_tagger_message_and_signature_block() {
    let object = "object 0123456789abcdef0123456789abcdef01234567\ntype commit\ntag v1\n\
tagger Rocket Test <rocket@example.invalid> 1767196800 +0800\n\nRelease 文\n\nNotes\n"
        .as_bytes();
    let parts = TagAnnotation::parse(object);
    let tagger = parts.tagger.unwrap();
    assert_eq!(tagger.name, b"Rocket Test");
    assert_eq!((tagger.timestamp, tagger.offset_minutes), (1_767_196_800, 480));
    assert_eq!(parts.message, "Release 文\n\nNotes\n".as_bytes());
    assert!(!parts.signed);
    for block in [
        "-----BEGIN PGP SIGNATURE-----\n\niQEz\n-----END PGP SIGNATURE-----\n",
        "-----BEGIN SSH SIGNATURE-----\nU1NI\n-----END SSH SIGNATURE-----\n",
        "-----BEGIN SIGNED MESSAGE-----\nMIIB\n-----END SIGNED MESSAGE-----\n",
    ] {
        let mut signed = object.to_vec();
        signed.extend_from_slice(block.as_bytes());
        let parts = TagAnnotation::parse(&signed);
        assert!(parts.signed, "{block}");
        assert_eq!(parts.message, "Release 文\n\nNotes\n".as_bytes());
    }
    // A marker inside a line is message text, not a block.
    let mut quoted = object.to_vec();
    quoted.extend_from_slice(b"see -----BEGIN PGP SIGNATURE----- here\n");
    assert!(!TagAnnotation::parse(&quoted).signed);
    // No tagger (very old tags) and no message.
    let bare = TagAnnotation::parse(b"object 01\ntype commit\ntag v0\n");
    assert_eq!(bare, TagAnnotation::default());
}
