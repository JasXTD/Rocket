use super::*;

/// Folders only: the folder tree shows no file cards (files count towards their folder).
fn data(count: usize) -> RepositoryMap {
    RepositoryMap {
        nodes: (0..count)
            .map(|i| MapNode {
                key: i.to_string().into_bytes(),
                label: format!("file {i}").into_bytes(),
                detail: Vec::new(),
                kind: MapNodeKind::Directory,
                path: Some(format!("src/file-{i}").into_bytes()),
                commit: None,
                count: 0,
                files: 1,
                evidence: Vec::new(),
                refs: Vec::new(),
                time: 0,
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn history_layout_preserves_first_parent_lanes_and_filter_positions() {
    let mut d = data(6);
    d.kind = MapKind::History;
    for n in &mut d.nodes {
        n.kind = MapNodeKind::Commit;
    }
    d.links = [(1, 0), (2, 0), (3, 1), (3, 2), (4, 2), (5, 3), (5, 4)]
        .into_iter()
        .map(|(from, to)| MapLink {
            from,
            to,
            kind: MapLinkKind::Parent,
            weight: 1,
            evidence: vec![],
        })
        .collect();
    // Non-parent edges must not reserve lanes or affect first-parent ordering.
    d.links.insert(
        0,
        MapLink {
            from: 0,
            to: 2,
            kind: MapLinkKind::Changes,
            weight: 1,
            evidence: vec![],
        },
    );
    let mut s = MapPage {
        data: Some(Arc::new(d)),
        ..Default::default()
    };
    s.rebuild();
    let expected = vec![
        pos2(0., 0.),
        pos2(0., 94.),
        pos2(260., 188.),
        pos2(0., 282.),
        pos2(260., 376.),
        pos2(0., 470.),
    ];
    assert_eq!(s.positions, expected);
    s.query = "file 3".into();
    s.rebuild();
    assert!(s.visible.len() < 6 && s.visible.contains(&3));
    assert_eq!(s.positions, expected);
}

#[test]
fn history_layout_handles_the_full_node_and_link_budgets() {
    let mut d = data(MAP_HISTORY_MAX);
    d.kind = MapKind::History;
    for n in &mut d.nodes {
        n.kind = MapNodeKind::Commit;
    }
    for child in 0..MAP_HISTORY_MAX {
        for parent in child + 1..(child + 7).min(MAP_HISTORY_MAX) {
            d.links.push(MapLink {
                from: parent,
                to: child,
                kind: MapLinkKind::Parent,
                weight: 1,
                evidence: vec![],
            });
        }
    }
    for parent in 7..28 {
        d.links.push(MapLink {
            from: parent,
            to: 0,
            kind: MapLinkKind::Parent,
            weight: 1,
            evidence: vec![],
        });
    }
    assert_eq!(d.links.len(), 12_000);
    let mut s = MapPage {
        data: Some(Arc::new(d)),
        ..Default::default()
    };
    let start = std::time::Instant::now();
    s.rebuild();
    println!(
        "2,000-node / 12,000-link history rebuild: {:?}",
        start.elapsed()
    );
    assert_eq!(s.positions.len(), MAP_HISTORY_MAX);
    assert!(s.positions.iter().all(|p| p.is_finite()));
    assert_eq!(s.projected_links.len(), 12_000);
    assert_eq!(s.visible.len(), DISPLAY_NODES);
    assert_eq!(s.view_total, MAP_HISTORY_MAX);
    let positions = s.positions.clone();
    s.query = "file 1999".into();
    s.rebuild();
    assert!(s.visible.contains(&1999));
    assert_eq!(s.positions, positions);
}

#[test]
fn stop_wins_completed_result_race_and_errors_preserve_previous_snapshot() {
    let previous = Arc::new(data(1));
    let mut s = MapPage {
        data: Some(previous.clone()),
        cancelling: true,
        ..Default::default()
    };
    s.finish(Ok(data(2)));
    assert!(Arc::ptr_eq(s.data.as_ref().unwrap(), &previous));
    assert!(s.notice.as_ref().unwrap().contains("stopped"));
    assert!(!s.cancelling);
    s.finish(Err(Error::Unsupported("missing objects".into())));
    assert!(s.error.as_ref().unwrap().contains("missing objects"));
    assert!(Arc::ptr_eq(s.data.as_ref().unwrap(), &previous));
    s.error = None;
    s.neighbor_anchor = Some(b"removed".to_vec());
    s.collapsed.insert(b"removed".to_vec());
    s.finish(Ok(data(3)));
    assert_eq!(s.data.as_ref().unwrap().nodes.len(), 3);
    assert!(s.notice.is_none());
    assert!(s.neighbor_anchor.is_none() && s.collapsed.is_empty());
}

#[test]
fn unicode_search_keeps_direct_match_ahead_of_bounded_neighbor_context() {
    let mut d = data(400);
    d.nodes[399].label = "結果 🚀 é".as_bytes().to_vec();
    d.links = (0..399)
        .map(|i| MapLink {
            from: i,
            to: 399,
            kind: MapLinkKind::Contains,
            weight: 1,
            evidence: vec![],
        })
        .collect();
    let mut s = MapPage {
        data: Some(Arc::new(d)),
        query: "結果 🚀 é".into(),
        ..Default::default()
    };
    s.rebuild();
    assert!(s.visible.contains(&399));
    assert_eq!(s.visible.len(), DISPLAY_NODES);
    assert!(s.positions.iter().all(|p| p.is_finite()));
    s.query = "no such node".into();
    s.rebuild();
    assert!(s.visible.is_empty());
    assert!(s.selected.is_none());
}

#[test]
fn hidden_nodes_are_not_exposed_as_clickable_controls_at_compact_sizes() {
    let ctx = Context::default();
    // Card titles use the semibold face the app installs.
    crate::style::install(&ctx);
    let mut s = MapPage {
        data: Some(Arc::new(data(500))),
        ..Default::default()
    };
    s.rebuild();
    for size in [vec2(480., 270.), vec2(1000., 600.)] {
        let rect = Rect::from_min_size(pos2(0., 0.), size);
        for covered in [false, true] {
            let mut controls = HashMap::new();
            let mut output = ctx.run_ui(
                RawInput {
                    screen_rect: Some(rect),
                    ..Default::default()
                },
                |ui| {
                    drawing::canvas(ui, &mut s, rect, covered, &mut controls);
                },
            );
            output.textures_delta.clear();
            for (id, c) in &controls {
                assert!(rect.contains_rect(c.rect), "{id}: {:?}", c.rect);
            }
            if covered {
                assert!(controls.is_empty());
            } else {
                assert!(controls.contains_key("map-fit"));
                assert!(controls.len() < 25);
            }
        }
    }
}

#[test]
fn explicit_fit_contains_every_node_and_long_titles_stay_on_one_row() {
    let ctx = Context::default();
    // Card titles use the semibold face the app installs.
    crate::style::install(&ctx);
    let mut s = MapPage {
        data: Some(Arc::new(data(500))),
        ..Default::default()
    };
    s.rebuild();
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(480., 270.));
    drawing::fit(&mut s, rect);
    assert!(
        s.zoom < 0.08,
        "A long file list must fit below the former zoom floor"
    );
    for &i in &s.visible {
        let node = Rect::from_min_size(
            rect.min + s.pan + s.positions[i].to_vec2() * s.zoom,
            drawing::CARD * s.zoom,
        );
        assert!(rect.contains_rect(node));
    }
    let mut output = ctx.run_ui(RawInput::default(), |ui| {
        let galley = ui
            .painter()
            .layout_job(drawing::title_job("界🚀".repeat(4000), 1., 196.));
        assert_eq!(galley.rows.len(), 1);
    });
    output.textures_delta.clear();
    s.finish(Ok(data(1)));
    assert!(s.visible.is_empty());
    assert!(s.positions.is_empty());
    s.rebuild();
    assert_eq!(s.visible, [0]);
}

#[test]
fn collapsed_groups_keep_cross_group_edges_and_search_can_find_hidden_unicode() {
    let mut d = data(6);
    for (i, name) in [(0, "ui"), (3, "core")] {
        d.nodes[i].kind = MapNodeKind::Directory;
        d.nodes[i].key = name.as_bytes().to_vec();
        d.nodes[i].label = name.as_bytes().to_vec();
    }
    d.nodes[2].label = "結果 🚀 é".as_bytes().to_vec();
    // The Cargo reader promotes a package directory from Directory to Crate.
    d.nodes[0].kind = MapNodeKind::Crate;
    d.links = [
        (0, 1, MapLinkKind::Contains),
        (0, 2, MapLinkKind::Contains),
        (3, 4, MapLinkKind::Contains),
        (3, 5, MapLinkKind::Contains),
        (2, 4, MapLinkKind::DependsOn),
    ]
    .into_iter()
    .map(|(from, to, kind)| MapLink {
        from,
        to,
        kind,
        weight: 1,
        evidence: vec![],
    })
    .collect();
    let mut s = MapPage {
        data: Some(Arc::new(d)),
        selected: Some(2),
        ..Default::default()
    };
    s.toggle_group(0);
    s.toggle_group(3);
    s.rebuild();
    assert_eq!(s.visible, [0, 3]);
    assert_eq!(s.selected, Some(0));
    assert!(
        s.projected_links
            .iter()
            .any(|e| e.from == 0 && e.to == 3 && e.kind == MapLinkKind::DependsOn)
    );
    s.query = "結果 🚀 é".into();
    s.rebuild();
    assert_eq!(s.visible, [0, 3]);
    s.toggle_group(0);
    s.rebuild();
    assert!(s.visible.contains(&2));
    assert!(!s.visible.contains(&1));
}

#[test]
fn neighbor_focus_excludes_unrelated_nodes_and_filtering_keeps_positions() {
    let mut d = data(5);
    d.links = [(0, 1), (1, 2), (3, 4)]
        .into_iter()
        .map(|(from, to)| MapLink {
            from,
            to,
            kind: MapLinkKind::DependsOn,
            weight: 1,
            evidence: vec![],
        })
        .collect();
    let mut s = MapPage {
        data: Some(Arc::new(d)),
        selected: Some(1),
        ..Default::default()
    };
    s.rebuild();
    let positions = s.positions.clone();
    s.neighbors();
    s.rebuild();
    assert_eq!(s.visible, [0, 1, 2]);
    assert_eq!(s.positions, positions);
    s.neighbors();
    s.rebuild();
    assert_eq!(s.visible.len(), 5);
    s.query = "file 4".into();
    s.rebuild();
    assert_eq!(s.visible, [3, 4]);
    assert_eq!(s.positions, positions);
}

#[test]
fn loaded_nodes_can_be_paged_without_reloading_and_search_resets_the_page() {
    let mut s = MapPage {
        data: Some(Arc::new(data(605))),
        ..Default::default()
    };
    s.rebuild();
    assert_eq!(s.view_total, 605);
    assert_eq!(s.visible[0], 0);
    s.display_offset = 480;
    s.filter_key = None;
    s.rebuild();
    assert_eq!(s.visible.first(), Some(&480));
    assert_eq!(s.visible.last(), Some(&604));
    s.query = "file 4".into();
    s.rebuild();
    assert_eq!(s.display_offset, 0);
    assert!(s.visible.contains(&4));
}

#[test]
fn search_keeps_keyboard_focus_when_loaded_history_adds_paging_controls() {
    let ctx = Context::default();
    crate::style::install(&ctx);
    let mut tab = Tab::new(std::path::PathBuf::from("synthetic"), None);
    tab.map.kind = MapKind::History;
    let mut first = data(10);
    first.kind = MapKind::History;
    tab.map.page_mut().data = Some(Arc::new(first));
    tab.map.page_mut().query = "file".into();
    let mut controls = HashMap::new();
    let mut frame = |tab: &mut Tab, events| {
        controls.clear();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200., 800.))),
                events,
                ..Default::default()
            },
            |ui| view(ui, tab, &mut Vec::new(), &mut controls),
        );
        output.textures_delta.clear();
        controls["map-search"].rect.center()
    };
    let at = frame(&mut tab, vec![]);
    frame(
        &mut tab,
        vec![
            Event::PointerMoved(at),
            Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
    );
    frame(
        &mut tab,
        vec![Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    let mut more = data(605);
    more.kind = MapKind::History;
    tab.map.page_mut().finish(Ok(more));
    frame(&mut tab, vec![]);
    frame(&mut tab, vec![Event::Text("x".into())]);
    assert!(
        tab.map.page().query.contains('x'),
        "Search lost focus when the paging row appeared"
    );
}

/// History whose commit `i` has `parents[i]`, first parent first.
fn history(parents: &[&[usize]]) -> RepositoryMap {
    let mut d = data(parents.len());
    d.kind = MapKind::History;
    for (i, n) in d.nodes.iter_mut().enumerate() {
        n.kind = MapNodeKind::Commit;
        n.label = format!("commit {i}").into_bytes();
        n.count = parents[i].len();
    }
    for (child, list) in parents.iter().enumerate() {
        for &parent in list.iter() {
            d.links.push(MapLink {
                from: parent,
                to: child,
                kind: MapLinkKind::Parent,
                weight: 1,
                evidence: vec![],
            });
        }
    }
    d
}

#[test]
fn branches_and_merges_fold_plain_commits_and_list_merged_and_unmerged_work() {
    // 0 merges 3 and 4 (which started at 2); 2 is tagged; topic is one commit ahead of 5.
    let mut d = history(&[&[1, 3], &[2], &[5], &[4], &[2], &[], &[5]]);
    d.nodes[2].refs = vec![b"refs/tags/v1".to_vec()];
    d.nodes[6].refs = vec![b"refs/heads/topic".to_vec()];
    let t = lists::build(&d);
    let lines: Vec<(bool, Vec<usize>)> = t
        .lines
        .iter()
        .map(|l| match l {
            lists::Line::Commit { node, merged } => {
                (true, std::iter::once(*node).chain(merged.iter().copied()).collect())
            }
            lists::Line::Run { nodes } => (false, nodes.clone()),
        })
        .collect();
    assert_eq!(
        lines,
        [(true, vec![0, 3, 4]), (false, vec![1]), (true, vec![2]), (true, vec![5])]
    );
    assert_eq!(t.tips, [(6, vec![6])]);
}

#[test]
fn merge_lines_name_the_branch_and_its_authors() {
    assert_eq!(lists::merge_name("Merge branch 'feature/x' into main"), "feature/x");
    assert_eq!(lists::merge_name("Merge pull request #12 from user/fix-thing"), "user/fix-thing");
    assert_eq!(lists::merge_name("Merge remote-tracking branch 'origin/dev'"), "origin/dev");
    assert_eq!(lists::merge_name("Merge topic"), "topic");
    assert_eq!(lists::merge_name("Fix the build"), "Fix the build");
    let mut d = history(&[&[], &[], &[], &[]]);
    for (i, name) in ["Ada", "Ben", "Ada", "Cy"].iter().enumerate() {
        d.nodes[i].detail = name.as_bytes().to_vec();
    }
    assert_eq!(lists::authors(&d, &[0, 2]), "Ada");
    assert_eq!(lists::authors(&d, &[0, 1]), "Ada and Ben");
    assert_eq!(lists::authors(&d, &[0, 1, 2, 3]), "Ada, Ben and 1 more");
}

#[test]
fn people_cells_keep_files_inside_their_folder() {
    use details::in_area;
    assert!(in_area(b"docs/wiki/a.md", b"docs"));
    assert!(!in_area(b"docs2/a.md", b"docs"));
    assert!(!in_area(b"docs", b"docs"));
    assert!(in_area(b"README.md", b"/"));
    assert!(!in_area(b"src/main.rs", b"/"));
    assert!(in_area(b"crates/rocket-ui/src/app.rs", b"crates/rocket-ui"));
}

#[test]
fn translated_folder_captions_change_without_reloading_or_relaying_out() {
    use crate::i18n::{Language, scoped};
    let mut snapshot = data(4);
    for n in &mut snapshot.nodes { n.kind = MapNodeKind::File; }
    let mut page = MapPage { data: Some(Arc::new(snapshot)), ..Default::default() };
    scoped(Language::English, || page.rebuild());
    let data = page.data.as_ref().unwrap();
    let group = &page.file_groups[0];
    let position = group.at;
    let original = scoped(Language::English, || drawing::group_about(group, data));
    assert_eq!(original, "in the top folder");
    for language in Language::ALL.into_iter().skip(1) {
        let shown = scoped(language, || drawing::group_about(group, data));
        assert_ne!(shown, original, "{language:?}");
        assert_eq!(group.at, position);
        assert_eq!(group.files, 4);
    }
    let few = FileGroup { folder: None, files: 1, about: "資料 🚀.rs".into(), at: Pos2::ZERO };
    for language in Language::ALL {
        assert_eq!(scoped(language, || drawing::group_about(&few, data)), "資料 🚀.rs");
    }
}

#[test]
fn folder_tree_counts_files_in_their_folder_and_opens_a_path() {
    let mut d = data(5);
    let entries = [
        ("crates", MapNodeKind::Directory),
        ("crates/a", MapNodeKind::Directory),
        ("crates/a/x.rs", MapNodeKind::File),
        ("crates/b.rs", MapNodeKind::File),
        ("README.md", MapNodeKind::File),
    ];
    for (i, (path, kind)) in entries.into_iter().enumerate() {
        d.nodes[i].kind = kind;
        d.nodes[i].key = path.as_bytes().to_vec();
        d.nodes[i].path = Some(path.as_bytes().to_vec());
        d.nodes[i].label = path.rsplit('/').next().unwrap().as_bytes().to_vec();
    }
    d.links = [(0, 1), (1, 2), (0, 3)]
        .into_iter()
        .map(|(from, to)| MapLink {
            from,
            to,
            kind: MapLinkKind::Contains,
            weight: 1,
            evidence: vec![],
        })
        .collect();
    let mut s = MapPage {
        data: Some(Arc::new(d)),
        selected: Some(2),
        ..Default::default()
    };
    s.rebuild();
    // Files get no card of their own: a selected file selects its folder, and each
    // folder's own files are summed up beside it.
    assert_eq!(s.visible, [0, 1]);
    assert_eq!(s.selected, Some(1));
    let mut groups: Vec<(Option<usize>, usize, &str)> =
        s.file_groups.iter().map(|g| (g.folder, g.files, g.about.as_str())).collect();
    groups.sort();
    // A few files are named on their card.
    assert_eq!(groups, [(None, 1, "README.md"), (Some(0), 1, "b.rs"), (Some(1), 1, "x.rs")]);
    s.toggle_group(0);
    s.rebuild();
    assert_eq!(s.visible, [0]);
    assert_eq!(s.selected, Some(0));
    let groups: Vec<(Option<usize>, usize)> = s.file_groups.iter().map(|g| (g.folder, g.files)).collect();
    assert_eq!(groups, [(None, 1)]);
    // Show folders / a changed path opens every folder on the way.
    s.open_path(b"crates/a/x.rs");
    s.rebuild();
    assert!(s.collapsed.is_empty());
    assert_eq!(s.selected, Some(1));
}

#[test]
fn a_narrow_toolbar_wraps_inside_the_window_and_leaves_room_for_the_map() {
    // History with every chip showing: the toolbar has to take a second row. It once
    // stacked its revision field over the map, and later ran off the right edge.
    let ctx = Context::default();
    crate::style::install(&ctx);
    let mut tab = Tab::new(std::path::PathBuf::from("synthetic"), None);
    tab.map.kind = MapKind::History;
    let p = tab.map.page_mut();
    p.data = Some(Arc::new(history(&[&[1], &[2], &[]])));
    p.condensed = false;
    p.all_branches = true;
    p.query = "commit".into();
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(560., 480.));
    let mut controls = HashMap::new();
    for _ in 0..3 {
        controls.clear();
        let mut output = ctx.run_ui(
            RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ui| view(ui, &mut tab, &mut Vec::new(), &mut controls),
        );
        output.textures_delta.clear();
    }
    assert!(controls.contains_key("map-canvas"), "the map keeps its body");
    let mut rects: Vec<_> = controls.iter().map(|(k, c)| (k.clone(), c.rect)).collect();
    rects.sort_by(|a, b| a.1.top().total_cmp(&b.1.top()));
    assert!(controls.contains_key("map-revision-pick"), "{rects:?}");
    for (id, c) in &controls {
        assert!(c.rect.right() <= screen.right() + 1., "{id} runs off the window: {:?}", c.rect);
    }
    let search = controls["map-search"].rect;
    let revision = controls["map-revision-pick"].rect;
    assert!((revision.top() - search.top()).abs() < 2., "the revision stays on the first row");
    assert!(controls["map-neighbors"].rect.top() > search.bottom(), "the last chip wraps to a new row");
    // One height per row: the search and the picker are a control high, and the
    // segmented options are 22 points, so their framed group is one too.
    assert!((revision.height() - search.height()).abs() < 1., "{revision:?} {search:?}");
    let option = controls["map-mode-commits"].rect;
    assert!((option.height() - 22.).abs() < 0.6, "{option:?}");
    assert!((option.height() + 6. - search.height()).abs() < 0.6, "{option:?} {search:?}");
}

#[test]
fn a_wide_toolbar_keeps_one_centre_line() {
    // With room for one row, every control sits on the search box's centre line. A
    // segmented group one row taller than its options once pushed it 3 points down.
    let ctx = Context::default();
    crate::style::install(&ctx);
    let mut tab = Tab::new(std::path::PathBuf::from("synthetic"), None);
    tab.map.kind = MapKind::History;
    let p = tab.map.page_mut();
    p.data = Some(Arc::new(history(&[&[1], &[2], &[]])));
    p.condensed = false;
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1480., 900.));
    let mut controls = HashMap::new();
    for _ in 0..3 {
        controls.clear();
        let mut output = ctx.run_ui(
            RawInput { screen_rect: Some(screen), ..Default::default() },
            |ui| view(ui, &mut tab, &mut Vec::new(), &mut controls),
        );
        output.textures_delta.clear();
    }
    let search = controls["map-search"].rect;
    for id in ["map-mode-topology", "map-mode-commits", "map-all-branches", "map-neighbors", "map-revision-pick"] {
        let r = controls[id].rect;
        assert!((r.center().y - search.center().y).abs() < 1., "{id} {r:?} is off the line of {search:?}");
    }
}

#[test]
fn translated_map_zoom_controls_keep_their_complete_labels() {
    for language in crate::i18n::Language::ALL {
        crate::i18n::scoped(language, || {
            let ctx = Context::default();
            crate::style::install(&ctx);
            ctx.set_fonts(crate::fonts::definitions());
            let mut tab = Tab::new(std::path::PathBuf::from("synthetic"), None);
            tab.map.page_mut().data = Some(Arc::new(data(3)));
            tab.map.page_mut().selected = Some(0);
            for width in [560., 780., 1280.] {
                let screen = Rect::from_min_size(Pos2::ZERO, vec2(width, 620.));
                let mut controls = HashMap::new();
                let mut minimum = HashMap::new();
                for _ in 0..3 {
                    controls.clear();
                    ctx.run_ui(RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
                        for (id, label) in [("map-fit", tr("Fit")), ("map-focus", tr("Focus"))] {
                            let text = WidgetText::from(label).into_galley(ui, Some(TextWrapMode::Extend), f32::INFINITY, TextStyle::Button);
                            minimum.insert(id, text.size().x + 2. * ui.spacing().button_padding.x);
                        }
                        view(ui, &mut tab, &mut Vec::new(), &mut controls);
                    }).drop_without_applying_deltas();
                }
                for (id, width_needed) in &minimum {
                    let control = &controls[*id];
                    assert!(control.enabled, "{language:?} {id}");
                    assert!(control.rect.width() + 1. >= *width_needed,
                        "{language:?} at {width}: {id} clips its label: {:?}, needs {width_needed}", control.rect);
                    assert!(screen.contains_rect(control.rect), "{language:?} {id} leaves the viewport");
                }
            }
        });
    }
}
