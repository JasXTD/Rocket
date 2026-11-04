use super::*;

fn data(count: usize) -> RepositoryMap {
    RepositoryMap {
        nodes: (0..count)
            .map(|i| MapNode {
                key: i.to_string().into_bytes(),
                label: format!("file {i}").into_bytes(),
                detail: Vec::new(),
                kind: MapNodeKind::File,
                path: Some(format!("src/file-{i}").into_bytes()),
                commit: None,
                count: 0,
                files: 1,
                evidence: Vec::new(),
                refs: Vec::new(),
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
fn explicit_fit_contains_every_node_and_long_titles_use_two_rows() {
    let ctx = Context::default();
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
            vec2(214., 66.) * s.zoom,
        );
        assert!(rect.contains_rect(node));
    }
    let mut output = ctx.run_ui(RawInput::default(), |ui| {
        let galley = ui
            .painter()
            .layout_job(drawing::title_job("界🚀".repeat(4000), 1., 196.));
        assert!(galley.rows.len() <= 2);
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
