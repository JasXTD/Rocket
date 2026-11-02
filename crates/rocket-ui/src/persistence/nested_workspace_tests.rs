use super::*;

fn sample() -> SavedState {
    let mut s = SavedState::default();
    let path = std::env::temp_dir().join("nested-workspaces-資料 🚀");
    let a = s
        .library
        .save_repository(path.clone(), "資料 🚀".into(), None)
        .unwrap();
    let first = s
        .library
        .save_workspace(None, "Work".into(), vec![a], Some(a))
        .unwrap();
    let second = s
        .library
        .save_workspace(None, "Personal".into(), vec![], None)
        .unwrap();
    s.library
        .save_workspace_in(None, "Backend".into(), vec![a], Some(a), Some(first))
        .unwrap();
    s.library
        .save_workspace_in(None, "Backend".into(), vec![], None, Some(second))
        .unwrap();
    s.recent.push(path.clone());
    s.drafts
        .insert(path, "PRIVATE_DRAFT 文 🚀 e\u{301}\nSecond line".into());
    s.preferences.sidebar = false;
    s.preferences.theme.choice = crate::theme::Choice::Light;
    s.preferences
        .shortcuts
        .set(
            crate::shortcuts::model::Action::Sidebar,
            vec![crate::shortcuts::model::Binding::parse("MouseBack").unwrap()],
        )
        .unwrap();
    s
}

#[test]
fn nested_workspace_roundtrip_preserves_duplicate_child_names_and_drafts() {
    let state = sample();
    let bytes = encode(&state).unwrap();
    assert!(bytes.starts_with(WORKSPACES_MAGIC));
    let restored = decode(&bytes).unwrap();
    assert_eq!(restored, state);
    assert_eq!(
        restored
            .library
            .workspace_path(restored.library.workspaces[2].id)
            .as_deref(),
        Some("Work / Backend")
    );
    // An older header cannot parse the new data as an old format with lost parents.
    let mut downgrade = bytes;
    downgrade[..THEME_MAGIC.len()].copy_from_slice(THEME_MAGIC);
    assert!(decode(&downgrade).is_err());
}

#[test]
fn nested_workspace_migration_keeps_flat_formats_and_preferences() {
    let mut flat = sample();
    flat.library.workspaces.truncate(2);
    let legacy = encode(&flat).unwrap();
    assert!(!legacy.starts_with(WORKSPACES_MAGIC));
    let mut migrated = decode(&legacy).unwrap();
    assert_eq!(migrated, flat);
    assert!(
        migrated
            .library
            .workspaces
            .iter()
            .all(|w| w.parent.is_none())
    );
    let parent = migrated.library.workspaces[0].id;
    let child = migrated.library.workspaces[1].id;
    migrated
        .library
        .save_workspace_in(Some(child), "Personal".into(), vec![], None, Some(parent))
        .unwrap();
    assert_eq!(decode(&encode(&migrated).unwrap()).unwrap(), migrated);
    assert_eq!(migrated.drafts, flat.drafts);
    assert_eq!(migrated.preferences, flat.preferences);
    assert_eq!(migrated.library.repositories, flat.library.repositories);
}

#[test]
fn nested_workspace_malformed_parent_records_are_rejected() {
    let state = sample();
    let bytes = encode(&state).unwrap();
    let start = bytes.len() - 4 - state.library.workspaces.len() * 16;
    let reject = |position: usize, value: &[u8]| {
        let mut bad = bytes.clone();
        bad[position..position + value.len()].copy_from_slice(value);
        assert!(decode(&bad).is_err());
    };
    reject(start, &129u32.to_le_bytes());
    reject(start + 4, &999u64.to_le_bytes());
    reject(
        start + 4 + 16,
        &state.library.workspaces[0].id.to_le_bytes(),
    );
    reject(start + 12, &state.library.workspaces[0].id.to_le_bytes());
    reject(start + 12, &state.library.workspaces[2].id.to_le_bytes());
    reject(start + 12 + 16 * 2, &999u64.to_le_bytes());
    for length in [start, start + 4, bytes.len() - 1] {
        assert!(decode(&bytes[..length]).is_err());
    }
    let mut bad = bytes;
    bad.push(1);
    assert!(decode(&bad).is_err());
}

#[test]
fn nested_workspace_store_retains_streamed_drafts_and_refuses_invalid_save() {
    let dir = std::env::temp_dir().join(format!(
        "rocket-nested-state-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let state = sample();
    let (store, _) = Store::open(&dir).unwrap();
    store.save(&state).unwrap();
    let before = fs::read(dir.join("state.bin")).unwrap();
    let archive = store.archive.lock().unwrap().clone().unwrap();
    let draft_bytes = fs::read(dir.join(&archive)).unwrap();
    let mut invalid = state.clone();
    invalid.library.workspaces[0].parent = Some(invalid.library.workspaces[2].id);
    assert!(store.save(&invalid).is_err());
    assert_eq!(fs::read(dir.join("state.bin")).unwrap(), before);
    assert_eq!(fs::read(dir.join(&archive)).unwrap(), draft_bytes);
    drop(store);
    let (store, restored) = Store::open(&dir).unwrap();
    assert_eq!(restored, state);
    drop(store);
    // Unsupported/corrupt files remain byte-identical when reopening is refused.
    let mut broken = before;
    broken[0] = b'?';
    fs::write(dir.join("state.bin"), &broken).unwrap();
    assert!(Store::open(&dir).is_err());
    assert_eq!(fs::read(dir.join("state.bin")).unwrap(), broken);
    assert_eq!(fs::read(dir.join(&archive)).unwrap(), draft_bytes);
    fs::remove_dir_all(dir).unwrap();
}
