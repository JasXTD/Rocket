#![cfg(windows)]
#[allow(dead_code)]
mod common;
use common::Fixture;
use rocket_git::*;
use std::{fs, path::Path};

/// New repository's check says what creating would do, without writing anything.
#[test]
fn new_repository_check_says_what_creating_would_do() {
    let f = Fixture::new();
    // The fixture path has `..` components, which creation refuses; use its real spelling.
    let root = f.root.canonicalize().unwrap();
    let parent = root.join("projects");
    fs::create_dir(&parent).unwrap();
    assert_eq!(check_init_destination(&parent.join("fresh")).unwrap(), InitDestination::New);
    let site = parent.join("site");
    fs::create_dir(&site).unwrap();
    fs::write(site.join("index.html"), "x").unwrap();
    fs::create_dir(site.join("assets")).unwrap();
    assert_eq!(
        check_init_destination(&site).unwrap(),
        InitDestination::Existing { entries: 2 }
    );
    let repository = f.repository.canonicalize().unwrap();
    for folder in [repository.clone(), repository.join("src")] {
        assert_eq!(
            check_init_destination(&folder).unwrap(),
            InitDestination::Repository(repository.clone()),
            "{}",
            folder.display()
        );
    }
    assert_eq!(
        check_init_destination(&parent.join("missing").join("app")).unwrap(),
        InitDestination::MissingParent
    );
    assert!(check_init_destination(Path::new("relative")).is_err());
    assert!(!parent.join("fresh").exists(), "checking created nothing");
}

/// Clone's check accepts only a new folder in an existing parent.
#[test]
fn clone_check_accepts_only_a_new_folder_in_an_existing_parent() {
    let f = Fixture::new();
    let parent = f.root.canonicalize().unwrap().join("clones");
    fs::create_dir(&parent).unwrap();
    let upstream = parent.join("upstream");
    assert_eq!(
        check_clone_destination(&upstream).unwrap(),
        CloneDestination::New(upstream.clone())
    );
    assert!(!upstream.exists(), "checking created nothing");
    fs::create_dir(&upstream).unwrap();
    assert_eq!(check_clone_destination(&upstream).unwrap(), CloneDestination::Exists);
    assert_eq!(
        check_clone_destination(&parent.join("missing").join("upstream")).unwrap(),
        CloneDestination::MissingParent
    );
    assert!(check_clone_destination(&parent.join("CON")).is_err(), "reserved names are refused");
}
