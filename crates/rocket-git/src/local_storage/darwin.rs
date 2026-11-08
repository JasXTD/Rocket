//! Read-only private-storage ACL checks, shared with the MCP token store.
use std::{fs, io, path::Path};
use crate::darwin_acl as acl;
pub(super) fn check_acl(path: &Path, expected: &fs::Metadata) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    crate::local_files::no_follow(&mut options);
    let file = options.open(path)?;
    acl::check(&file, path, expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;
    use std::{
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            // Only the unique fixture created below; never normalize user ACLs.
            let _ = Command::new("/bin/chmod").arg("-N").arg(&self.0).status();
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn chmod(path: &Path, args: &[&str]) {
        assert!(
            Command::new("/bin/chmod")
                .args(args)
                .arg(path)
                .status()
                .unwrap()
                .success()
        );
    }
    #[test]
    fn entry_updates_get_a_fresh_acl_but_permission_identity_and_file_changes_fail() {
        let root = std::env::temp_dir().join(format!(
            "rocket-mac-acl-change-{}-{}", std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos(),
        ));
        crate::local_storage::create_private_new(&root).unwrap();
        let fixture = Fixture(root);
        let directory = fixture.0.join("directory");
        crate::local_storage::create_private_new(&directory).unwrap();
        let before = fs::symlink_metadata(&directory).unwrap();
        fs::write(directory.join("draft"), b"preserve").unwrap();
        let changed = fs::symlink_metadata(&directory).unwrap();
        assert_ne!((before.ctime(), before.ctime_nsec()), (changed.ctime(), changed.ctime_nsec()));
        check_acl(&directory, &before).unwrap();

        chmod(&directory, &["+a", "everyone allow read,search"]);
        assert_eq!(check_acl(&directory, &before).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        chmod(&directory, &["-N"]);
        chmod(&directory, &["750"]);
        assert_eq!(check_acl(&directory, &before).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        chmod(&directory, &["700"]);

        let draft = directory.join("draft");
        let file_before = fs::symlink_metadata(&draft).unwrap();
        fs::write(&draft, b"edited").unwrap();
        assert_eq!(check_acl(&draft, &file_before).unwrap_err().kind(), io::ErrorKind::PermissionDenied);

        let preserved = fixture.0.join("preserved");
        fs::rename(&directory, &preserved).unwrap();
        crate::local_storage::create_private_new(&directory).unwrap();
        assert_eq!(check_acl(&directory, &before).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read(preserved.join("draft")).unwrap(), b"edited");
    }

    #[test]
    fn private_storage_accepts_deny_acls_and_refuses_allow_acls_without_repair() {
        let root = std::env::temp_dir().join(format!(
            "rocket-mac-acl-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        crate::local_storage::create_private_new(&root).unwrap();
        let fixture = Fixture(root);
        chmod(&fixture.0, &["+a", "everyone deny delete"]);
        crate::local_storage::check_private(&fixture.0).unwrap();
        chmod(&fixture.0, &["-N"]);
        chmod(&fixture.0, &["+a", "everyone allow read,search"]);
        let observed = || {
            Command::new("/bin/ls")
                .arg("-lde")
                .arg(&fixture.0)
                .output()
                .unwrap()
                .stdout
        };
        let before = observed();
        assert_eq!(fs::metadata(&fixture.0).unwrap().mode() & 0o777, 0o700);
        assert_eq!(
            crate::local_storage::check_private(&fixture.0)
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(crate::local_storage::create_private(&fixture.0.join("child")).is_err());
        assert!(!fixture.0.join("child").exists());
        assert_eq!(observed(), before, "existing ACL must stay unchanged");
    }
}
