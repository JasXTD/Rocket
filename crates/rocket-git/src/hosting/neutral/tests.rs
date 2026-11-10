use super::*;

#[test]
fn credential_lookup_refuses_ambiguous_ceiling_paths_before_starting_a_helper() {
    let client = crate::GitClient::discover().unwrap();
    let cancel = crate::CancellationToken::new();
    let ambiguous = std::env::temp_dir()
        .join(if cfg!(windows) {
            "ambiguous;parent"
        } else {
            "ambiguous:parent"
        })
        .join("cwd");
    for cwd in [Path::new("relative"), &ambiguous] {
        assert!(matches!(
            client.runner.credential(cwd, false, Vec::new(), &cancel),
            Err(Error::InvalidInput(_))
        ));
        assert!(matches!(
            client
                .runner
                .browser_credential(cwd, crate::ForgeProvider::GitHub, &cancel),
            Err(Error::InvalidInput(_))
        ));
    }
}

#[test]
fn concurrent_sessions_have_distinct_private_folders_and_share_lifetime() {
    let threads: Vec<_> = (0..16)
        .map(|_| std::thread::spawn(|| neutral_directory().unwrap()))
        .collect();
    let directories: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    let paths: std::collections::HashSet<_> = directories.iter().map(|d| d.to_path_buf()).collect();
    assert_eq!(paths.len(), directories.len());
    for directory in &directories {
        assert_eq!(fs::read_dir(&**directory).unwrap().count(), 0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            local_storage::check_private(directory).unwrap();
            assert_eq!(
                fs::symlink_metadata(&**directory).unwrap().mode() & 0o777,
                0o700
            );
        }
    }
    let kept = directories[0].clone();
    drop(directories);
    assert!(kept.is_dir());
    assert_eq!(paths.iter().filter(|p| p.exists()).count(), 1);
    drop(kept);
    assert!(paths.iter().all(|p| !p.exists()));
}

#[test]
fn existing_directories_and_files_are_refused_and_preserved() {
    let parent = neutral_directory().unwrap();
    let directory = parent.join("already there 文");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("keep"), b"existing data").unwrap();
    let file = parent.join("file");
    fs::write(&file, b"existing file").unwrap();
    for path in [&directory, &file] {
        let result = NeutralDirectory::create_new(path.clone());
        assert!(matches!(result, Err(e) if e.kind() == io::ErrorKind::AlreadyExists));
    }
    assert_eq!(fs::read(directory.join("keep")).unwrap(), b"existing data");
    assert_eq!(fs::read(&file).unwrap(), b"existing file");
    fs::remove_file(directory.join("keep")).unwrap();
    fs::remove_dir(directory).unwrap();
    fs::remove_file(file).unwrap();
}

#[test]
fn cleanup_preserves_unexpected_helper_files() {
    let directory = neutral_directory().unwrap();
    let path = directory.to_path_buf();
    fs::write(path.join("keep"), b"helper output").unwrap();
    drop(directory);
    assert_eq!(fs::read(path.join("keep")).unwrap(), b"helper output");
    fs::remove_file(path.join("keep")).unwrap();
    fs::remove_dir(path).unwrap();
}

#[cfg(unix)]
#[test]
fn symlinks_and_unsafe_ancestors_are_refused_without_modification() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let parent = neutral_directory().unwrap();
    let target = parent.join("target");
    fs::create_dir(&target).unwrap();
    let link = parent.join("alias");
    symlink(&target, &link).unwrap();
    assert!(
        matches!(NeutralDirectory::create_new(link.clone()), Err(e) if e.kind() == io::ErrorKind::AlreadyExists)
    );
    assert!(
        matches!(NeutralDirectory::create_new(link.join("new")), Err(e) if e.kind() == io::ErrorKind::PermissionDenied)
    );
    fs::set_permissions(&target, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(
        matches!(NeutralDirectory::create_new(target.join("new")), Err(e) if e.kind() == io::ErrorKind::PermissionDenied)
    );
    assert!(!target.join("new").exists());
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o777
    );
    assert_eq!(fs::read_link(&link).unwrap(), target);
    fs::remove_file(link).unwrap();
    fs::remove_dir(target).unwrap();
}
