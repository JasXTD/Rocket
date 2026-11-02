//! `commit.template` reads: unset, relative and absolute paths, comment strings,
//! and refusal of missing, oversized or non-UTF-8 templates.
#[allow(dead_code)]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;

fn token() -> CancellationToken {
    CancellationToken::new()
}

#[test]
fn commit_template_is_read_from_configuration_and_split_into_message_and_guidance() {
    // Global configuration must not supply a template or comment character.
    isolated(
        "ROCKET_COMMIT_MESSAGE_CASE",
        "commit_template_is_read_from_configuration_and_split_into_message_and_guidance",
        read_commit_template,
    );
}

fn read_commit_template() {
    let f = Fixture::new();
    let repo = f.repo();
    assert_eq!(repo.commit_template(&token()).unwrap(), None);

    // Relative paths resolve from the work tree.
    f.write(
        ".gitmessage 文",
        "feat: \n\n# Why is this change needed?\n# 说明\nRefs: \n",
    );
    f.run(&["config", "commit.template", ".gitmessage 文"]);
    let template = repo.commit_template(&token()).unwrap().unwrap();
    assert_eq!(template.message, "feat:\n\nRefs:");
    assert_eq!(template.guidance, "Why is this change needed?\n说明");
    assert!(template.path.ends_with(".gitmessage 文"));

    // A configured comment character changes what counts as guidance.
    f.run(&["config", "core.commentChar", ";"]);
    let template = repo.commit_template(&token()).unwrap().unwrap();
    assert_eq!(
        template.message,
        "feat:\n\n# Why is this change needed?\n# 说明\nRefs:"
    );
    assert_eq!(template.guidance, "");
    f.run(&["config", "core.commentChar", "auto"]);
    let template = repo.commit_template(&token()).unwrap().unwrap();
    assert_eq!(template.guidance, "Why is this change needed?\n说明");

    // Absolute paths outside the repository are allowed.
    let outside = f.root.join("outside template.txt");
    std::fs::write(&outside, "# guidance only\n").unwrap();
    f.run(&["config", "commit.template", outside.to_str().unwrap()]);
    let template = repo.commit_template(&token()).unwrap().unwrap();
    assert_eq!(
        (template.message.as_str(), template.guidance.as_str()),
        ("", "guidance only")
    );

    // A missing, oversized or non-UTF-8 template is an error, not an empty message.
    f.run(&["config", "commit.template", "missing.txt"]);
    assert!(matches!(
        repo.commit_template(&token()),
        Err(Error::Io { .. })
    ));
    f.write("big.txt", vec![b'x'; 64 * 1024 + 1]);
    f.run(&["config", "commit.template", "big.txt"]);
    assert!(matches!(
        repo.commit_template(&token()),
        Err(Error::Unsupported(_))
    ));
    f.write("binary.txt", [0xff, 0xfe, 0x00]);
    f.run(&["config", "commit.template", "binary.txt"]);
    assert!(matches!(
        repo.commit_template(&token()),
        Err(Error::Unsupported(_))
    ));
}
