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

/// The same cards as packages: Packages draws and pages them as one graph.
fn crates(count: usize) -> RepositoryMap {
    let mut d = data(count);
    for n in &mut d.nodes {
        n.kind = MapNodeKind::Crate;
    }
    d
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
    let mut d = crates(400);
    d.nodes[399].label = "結果 🚀 é".as_bytes().to_vec();
    d.links = (0..399)
        .map(|i| MapLink {
            from: i,
            to: 399,
            kind: MapLinkKind::DependsOn,
            weight: 1,
            evidence: vec![],
        })
        .collect();
    let mut s = MapPage {
        data: Some(Arc::new(d)),
        query: "結果 🚀 é".into(),
        alternate: true,
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
        data: Some(Arc::new(crates(500))),
        alternate: true,
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
        data: Some(Arc::new(crates(500))),
        alternate: true,
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
    s.finish(Ok(crates(1)));
    assert!(s.visible.is_empty());
    assert!(s.positions.is_empty());
    s.rebuild();
    assert_eq!(s.visible, [0]);
}

#[test]
fn neighbor_focus_excludes_unrelated_nodes_and_filtering_keeps_positions() {
    let mut d = crates(5);
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
        alternate: true,
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
        data: Some(Arc::new(crates(605))),
        alternate: true,
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

#[test]
fn package_details_buttons_keep_their_labels_on_one_line() {
    // Show folders, Show modules and Commits here share one row in the details card.
    // The third once got only what the first two left of the row and broke its label
    // into "Comm / its / here" instead of moving to the next row.
    let mut d = data(3);
    d.nodes[0].kind = MapNodeKind::Crate;
    let d = Arc::new(d);
    for language in crate::i18n::Language::ALL {
        crate::i18n::scoped(language, || {
            let ctx = Context::default();
            crate::style::install(&ctx);
            ctx.set_fonts(crate::fonts::definitions());
            let mut s = MapPage { data: Some(d.clone()), selected: Some(0), alternate: true, ..Default::default() };
            s.rebuild();
            // The card beside the map, and a narrow window's drawer.
            for width in [DETAILS_WIDTH, 200.] {
                let panel = Rect::from_min_size(Pos2::ZERO, vec2(width, 600.));
                let mut controls = HashMap::new();
                let mut needed = HashMap::new();
                for _ in 0..3 {
                    controls.clear();
                    ctx.run_ui(RawInput { screen_rect: Some(panel), ..Default::default() }, |ui| {
                        for (id, label) in [
                            ("map-show-folders", crate::i18n::tr("Show folders")),
                            ("map-show-modules", tr("Show modules")),
                            ("map-history", tr("Commits here")),
                        ] {
                            let text = WidgetText::from(label).into_galley(ui, Some(TextWrapMode::Extend), f32::INFINITY, TextStyle::Button);
                            let padding = ui.spacing().button_padding;
                            let one_line = (text.size().y + 2. * padding.y).max(ui.spacing().interact_size.y);
                            needed.insert(id, (label, vec2(text.size().x + 2. * padding.x, one_line)));
                        }
                        details::card(ui, &mut s, MapKind::Code, panel, false, &mut Vec::new(), &mut controls);
                    })
                    .drop_without_applying_deltas();
                }
                for (id, (label, size)) in &needed {
                    let r = controls.get(*id).unwrap_or_else(|| panic!("{language:?} at {width}: {id} missing")).rect;
                    assert!(r.width() + 1. >= size.x, "{language:?} at {width}: {id} squeezes {label:?} into {r:?}, needs {size:?}");
                    assert!(r.height() <= size.y + 1., "{language:?} at {width}: {id} breaks {label:?} over lines: {r:?}, needs {size:?}");
                }
            }
        });
    }
}

/// A People map: people with these commit totals, then folders with these paths and
/// totals, and `weight(person, folder)` commits in each cell (0 for none). People are
/// nodes 0.., the folders follow them in the order given.
fn people_map(people: &[usize], folders: &[(&str, usize)], weight: impl Fn(usize, usize) -> usize) -> RepositoryMap {
    let node = |kind, label: String, path: Option<String>, count| MapNode {
        key: label.clone().into_bytes(),
        label: label.into_bytes(),
        detail: Vec::new(),
        kind,
        path: path.map(String::into_bytes),
        commit: None,
        count,
        files: 1,
        evidence: Vec::new(),
        refs: Vec::new(),
        time: 0,
    };
    let mut d = RepositoryMap { kind: MapKind::People, ..Default::default() };
    for (p, &count) in people.iter().enumerate() {
        d.nodes.push(node(MapNodeKind::Person, format!("Person {p}"), None, count));
    }
    for &(path, count) in folders {
        d.nodes.push(node(MapNodeKind::Area, path.to_owned(), Some(path.to_owned()), count));
    }
    for p in 0..people.len() {
        for a in 0..folders.len() {
            let weight = weight(p, a);
            if weight > 0 {
                d.links.push(MapLink { from: p, to: people.len() + a, kind: MapLinkKind::Changes, weight, evidence: vec![] });
            }
        }
    }
    d
}

/// Two people, the second with more commits, and three folders out of their commit
/// order: docs, crates/core and the top folder are nodes 2, 3 and 4.
fn two_people() -> MapPage {
    let weights = [[1, 3, 0], [2, 7, 2]];
    let mut s = MapPage {
        data: Some(Arc::new(people_map(&[4, 9], &[("docs", 3), ("crates/core", 10), ("/", 2)], |p, a| weights[p][a]))),
        show_details: false,
        ..Default::default()
    };
    s.rebuild();
    s
}

/// What one frame of the People table registered for MCP, how many shapes it painted
/// and the text it painted.
struct GridFrame {
    controls: HashMap<String, UiControl>,
    shapes: usize,
    texts: Vec<String>,
}
impl GridFrame {
    fn rect(&self, id: &str) -> Rect {
        self.controls.get(id).unwrap_or_else(|| panic!("{id} missing: {:?}", self.controls.keys())).rect
    }
    fn painted(&self, text: &str) -> bool {
        self.texts.iter().any(|t| t == text)
    }
}

fn grid_frame(ctx: &Context, s: &mut MapPage, rect: Rect, events: Vec<Event>, time: f64) -> GridFrame {
    grid_frame_covered(ctx, s, rect, false, events, time)
}

fn grid_frame_covered(ctx: &Context, s: &mut MapPage, rect: Rect, covered: bool, events: Vec<Event>, time: f64) -> GridFrame {
    fn texts(shape: &Shape, out: &mut Vec<String>) {
        match shape {
            Shape::Text(text) => out.push(text.galley.text().to_owned()),
            Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, out)),
            _ => {}
        }
    }
    let mut controls = HashMap::new();
    let mut output = ctx.run_ui(RawInput { screen_rect: Some(rect), events, time: Some(time), ..Default::default() }, |ui| {
        lists::grid(ui, s, rect, covered, &mut controls);
    });
    output.textures_delta.clear();
    let mut painted = Vec::new();
    for clipped in &output.shapes {
        texts(&clipped.shape, &mut painted);
    }
    GridFrame { controls, shapes: output.shapes.len(), texts: painted }
}

/// Clicks at `at`, as a pointer press and release in one frame, then draws one more.
fn click_events(at: Pos2) -> Vec<Event> {
    let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    vec![Event::PointerMoved(at), press(true), press(false)]
}

/// Scrolls with the wheel over `at` and lets egui's smoothing finish.
fn scroll_grid(ctx: &Context, s: &mut MapPage, rect: Rect, at: Pos2, delta: Vec2, time: &mut f64) -> GridFrame {
    let wheel = Event::MouseWheel { unit: MouseWheelUnit::Point, delta, phase: TouchPhase::Move, source: MouseWheelSource::Unknown, modifiers: Modifiers::NONE };
    *time += 0.2;
    let mut frame = grid_frame(ctx, s, rect, vec![Event::PointerMoved(at), wheel], *time);
    for _ in 0..30 {
        *time += 0.2;
        frame = grid_frame(ctx, s, rect, vec![], *time);
    }
    frame
}

#[test]
fn people_table_puts_folders_down_the_side_and_people_across_the_top() {
    // The grid once slanted folder names over the columns and listed people down the
    // side. Both run by commits here, which is not their order in the map.
    crate::i18n::scoped(crate::i18n::Language::English, || {
        let ctx = Context::default();
        crate::style::install(&ctx);
        let mut s = two_people();
        for width in [880., 756.] {
            let rect = Rect::from_min_size(Pos2::ZERO, vec2(width, 600.));
            grid_frame(&ctx, &mut s, rect, vec![], 0.);
            let frame = grid_frame(&ctx, &mut s, rect, vec![], 0.1);
            // Folders run down the side, most commits first, in a column wide enough
            // for a long path in a wide card.
            let folders = [frame.rect("map-node-3"), frame.rect("map-node-2"), frame.rect("map-node-4")];
            for pair in folders.windows(2) {
                assert_eq!(pair[0].left(), pair[1].left());
                assert!(pair[0].bottom() <= pair[1].top() + 0.5, "{folders:?}");
            }
            assert_eq!(folders[0].width(), if width >= 800. { 256. } else { 188. });
            // People run across the top, most commits first.
            let people = [frame.rect("map-node-1"), frame.rect("map-node-0")];
            assert_eq!(people[0].top(), people[1].top());
            assert!(people[0].right() <= people[1].left() + 0.5, "{people:?}");
            assert!(people[0].bottom() <= folders[0].top() && people[0].left() >= folders[0].right(), "{people:?} {folders:?}");
            // A cell sits under its person and beside its folder; an empty pair has none.
            for (id, person, folder) in [
                ("map-cell-1-3", people[0], folders[0]),
                ("map-cell-0-3", people[1], folders[0]),
                ("map-cell-1-2", people[0], folders[1]),
                ("map-cell-0-2", people[1], folders[1]),
                ("map-cell-1-4", people[0], folders[2]),
            ] {
                let cell = frame.rect(id);
                assert!(person.x_range().contains(cell.center().x) && folder.y_range().contains(cell.center().y), "{id}: {cell:?}");
            }
            assert!(!frame.controls.contains_key("map-cell-0-4"));
            for (id, c) in &frame.controls {
                assert!(rect.contains_rect(c.rect), "{id}: {:?}", c.rect);
            }
            // The captions, the counts, the key, the top folder's name and, with two
            // people, each folder's commits.
            for text in ["FOLDER", "COMMITS", "2 people · 13 commits", "Commits in a cell", "top folder", "10"] {
                assert!(frame.painted(text), "{text} is not painted: {:?}", frame.texts);
            }
            // Under the details drawer nothing in the table takes a click.
            let covered = grid_frame_covered(&ctx, &mut s, rect, true, vec![], 0.2);
            assert!(covered.controls.is_empty(), "{:?}", covered.controls.keys());
        }
        // With one person a commits column would repeat the cells, so there is none.
        let mut one = MapPage { data: Some(Arc::new(people_map(&[5], &[("docs", 5)], |_, _| 5))), ..Default::default() };
        one.rebuild();
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(880., 600.));
        grid_frame(&ctx, &mut one, rect, vec![], 0.3);
        let frame = grid_frame(&ctx, &mut one, rect, vec![], 0.4);
        assert!(frame.painted("1 person · 5 commits") && !frame.painted("COMMITS"), "{:?}", frame.texts);
    });
}

#[test]
fn people_table_clicks_pick_a_cell_a_folder_or_a_person() {
    let ctx = Context::default();
    crate::style::install(&ctx);
    let mut s = two_people();
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(880., 600.));
    grid_frame(&ctx, &mut s, rect, vec![], 0.);
    let frame = grid_frame(&ctx, &mut s, rect, vec![], 0.1);
    let mut time = 0.1;
    for (id, picked) in [
        // A cell picks its person and folder and opens Details; a folder or a person
        // picks just that.
        ("map-cell-0-3", (Some(0), Some((0, 3)))),
        ("map-node-2", (Some(2), None)),
        ("map-node-1", (Some(1), None)),
    ] {
        time += 0.5;
        grid_frame(&ctx, &mut s, rect, click_events(frame.rect(id).center()), time);
        time += 0.1;
        grid_frame(&ctx, &mut s, rect, vec![], time);
        assert_eq!((s.selected, s.cell), picked, "{id}");
        assert!(s.show_details, "{id}");
    }
}

#[test]
fn people_table_keeps_names_in_place_while_the_cells_scroll() {
    // Forty people and twenty folders, with commits everywhere: only what shows is
    // painted, and after scrolling right and down the people row and the folder column
    // stay where they were while the cells move under them.
    let ctx = Context::default();
    crate::style::install(&ctx);
    let people: Vec<usize> = (0..40).map(|p| 1000 - p).collect();
    let paths: Vec<String> = (0..20).map(|a| format!("folder-{a}")).collect();
    let folders: Vec<(&str, usize)> = paths.iter().enumerate().map(|(a, p)| (p.as_str(), 1000 - a)).collect();
    let mut s = MapPage { data: Some(Arc::new(people_map(&people, &folders, |p, a| 1 + (p + a) % 5))), ..Default::default() };
    s.rebuild();
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(700., 420.));
    grid_frame(&ctx, &mut s, rect, vec![], 0.);
    let before = grid_frame(&ctx, &mut s, rect, vec![], 0.1);
    // About 7 columns by 9 rows show; painting all 800 cells would take more than
    // 1,600 shapes.
    assert!(before.shapes < 600, "{} shapes painted", before.shapes);
    let person = before.rect("map-node-0");
    let folder = before.rect("map-node-40");
    let mut time = 0.1;
    let after = scroll_grid(&ctx, &mut s, rect, pos2(500., 300.), vec2(-400., -260.), &mut time);
    assert!(!after.controls.contains_key("map-node-0") && !after.controls.contains_key("map-node-40"), "the first person and folder scrolled away");
    let mut shown = (0, 0);
    for (id, c) in &after.controls {
        let Some(n) = id.strip_prefix("map-node-").and_then(|n| n.parse::<usize>().ok()) else { continue };
        if n < 40 {
            shown.0 += 1;
            assert_eq!(c.rect.top(), person.top(), "{id} left the people row");
        } else {
            shown.1 += 1;
            assert_eq!(c.rect.left(), folder.left(), "{id} left the folder column");
        }
    }
    assert!(shown.0 > 0 && shown.1 > 0, "{shown:?}");
    for (id, c) in after.controls.iter().filter(|(id, _)| id.starts_with("map-cell-")) {
        assert!(c.rect.top() >= person.bottom() && c.rect.left() >= folder.right(), "{id} is under a fixed edge: {:?}", c.rect);
        assert!(rect.contains_rect(c.rect), "{id}: {:?}", c.rect);
    }
}

#[test]
fn people_table_in_a_short_card_scrolls_its_people_row_away_to_reach_every_folder() {
    // A card too short for two folder rows under a fixed people row: the people row
    // scrolls with the rows, so the last folder can still be brought fully into view.
    let ctx = Context::default();
    crate::style::install(&ctx);
    let paths: Vec<String> = (0..10).map(|a| format!("folder-{a}")).collect();
    let folders: Vec<(&str, usize)> = paths.iter().enumerate().map(|(a, p)| (p.as_str(), 100 - a)).collect();
    let mut s = MapPage { data: Some(Arc::new(people_map(&[9, 8], &folders, |_, _| 1))), ..Default::default() };
    s.rebuild();
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(880., 150.));
    grid_frame(&ctx, &mut s, rect, vec![], 0.);
    let before = grid_frame(&ctx, &mut s, rect, vec![], 0.1);
    assert!(before.controls.contains_key("map-node-0"), "{:?}", before.controls.keys());
    let mut time = 0.1;
    let after = scroll_grid(&ctx, &mut s, rect, pos2(500., 120.), vec2(0., -400.), &mut time);
    assert!(!after.controls.contains_key("map-node-0"), "the people row scrolled away with the rows");
    assert_eq!(after.rect("map-node-11").height(), 36., "the last folder shows in full");
}

#[test]
fn people_table_key_takes_its_own_line_in_a_narrow_card() {
    crate::i18n::scoped(crate::i18n::Language::English, || {
        let ctx = Context::default();
        crate::style::install(&ctx);
        let mut s = two_people();
        let mut first_row = Vec::new();
        for width in [880., 480.] {
            let rect = Rect::from_min_size(Pos2::ZERO, vec2(width, 600.));
            grid_frame(&ctx, &mut s, rect, vec![], 0.);
            let frame = grid_frame(&ctx, &mut s, rect, vec![], 0.1);
            assert!(frame.painted("Commits in a cell"), "{:?}", frame.texts);
            first_row.push(frame.rect("map-node-3").top());
        }
        // Title, counts and key fit on one line at 880 points; at 480 the key moves to
        // a line of its own and the table starts one line lower.
        assert_eq!(first_row[1] - first_row[0], 22., "{first_row:?}");
    });
}

#[test]
fn a_people_cell_is_dropped_when_another_node_is_picked() {
    // Search picking another person, or a click in Shared files, once left the cell
    // in place, and Details went on showing that cell.
    let mut s = two_people();
    s.selected = Some(0);
    s.cell = Some((0, 3));
    s.query = "Person 1".into();
    s.rebuild();
    assert_eq!((s.selected, s.cell), (Some(1), None));
    // While its person stays picked, the cell stays.
    s.cell = Some((1, 3));
    s.query = "Person".into();
    s.rebuild();
    assert_eq!((s.selected, s.cell), (Some(1), Some((1, 3))));

    let ctx = Context::default();
    crate::style::install(&ctx);
    let mut map = people_map(&[4, 9], &[("docs", 3)], |_, _| 1);
    map.links.push(MapLink { from: 0, to: 1, kind: MapLinkKind::SharedFiles, weight: 1, evidence: vec![] });
    let mut s = MapPage { data: Some(Arc::new(map)), alternate: true, fit: true, ..Default::default() };
    s.rebuild();
    s.selected = Some(0);
    s.cell = Some((0, 2));
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(880., 600.));
    let frame = |s: &mut MapPage, events: Vec<Event>, time: f64| {
        let mut controls = HashMap::new();
        ctx.run_ui(RawInput { screen_rect: Some(rect), events, time: Some(time), ..Default::default() }, |ui| {
            drawing::canvas(ui, s, rect, false, &mut controls);
        })
        .drop_without_applying_deltas();
        controls
    };
    frame(&mut s, vec![], 0.);
    let controls = frame(&mut s, vec![], 0.1);
    let at = controls.get("map-node-1").unwrap_or_else(|| panic!("{:?}", controls.keys())).rect.center();
    frame(&mut s, click_events(at), 0.6);
    frame(&mut s, vec![], 0.7);
    assert_eq!((s.selected, s.cell), (Some(1), None));
}

#[test]
fn people_counts_say_person_or_people() {
    crate::i18n::scoped(crate::i18n::Language::English, || {
        assert_eq!(lists::people_counts(1, 91), "1 person · 91 commits");
        assert_eq!(lists::people_counts(1, 1), "1 person · 1 commit");
        assert_eq!(lists::people_counts(2, 1422), "2 people · 1,422 commits");
    });
}

// ---- Code → Modules -----------------------------------------------------------

/// A Modules read: a package, its crate root and modules (path, parent, lines), and
/// connections between modules' files with the names they use.
fn modules_map(modules: &[(&str, Option<usize>, usize)], uses: &[(usize, usize, &[&str])]) -> RepositoryMap {
    let mut nodes = vec![MapNode {
        key: b"package:crates/app".to_vec(),
        label: b"app".to_vec(),
        detail: b"Cargo package".to_vec(),
        kind: MapNodeKind::Crate,
        path: Some(b"crates/app".to_vec()),
        commit: None,
        count: modules.len(),
        files: modules.len() + 1,
        evidence: Vec::new(),
        refs: Vec::new(),
        time: 0,
    }];
    let mut links = Vec::new();
    for (k, (path, parent, lines)) in modules.iter().enumerate() {
        let label = if path.is_empty() { "lib.rs".to_owned() } else { path.rsplit("::").next().unwrap().to_owned() };
        let file = if path.is_empty() { "crates/app/src/lib.rs".to_owned() } else { format!("crates/app/src/{}.rs", path.replace("::", "/")) };
        nodes.push(MapNode {
            key: format!("module:{path}").into_bytes(),
            label: label.into_bytes(),
            detail: if path.is_empty() { b"Crate root".to_vec() } else { b"Rust module".to_vec() },
            kind: MapNodeKind::Module,
            path: Some(file.into_bytes()),
            commit: None,
            count: *lines,
            files: 1,
            evidence: Vec::new(),
            refs: Vec::new(),
            time: 0,
        });
        if let Some(p) = parent {
            links.push(MapLink { from: p + 1, to: k + 1, kind: MapLinkKind::Contains, weight: 1, evidence: vec![] });
        }
    }
    for (from, to, names) in uses {
        links.push(MapLink {
            from: from + 1,
            to: to + 1,
            kind: MapLinkKind::DependsOn,
            weight: names.len(),
            evidence: names.iter().map(|n| n.as_bytes().to_vec()).collect(),
        });
    }
    RepositoryMap {
        kind: MapKind::Modules,
        nodes,
        links,
        scope: MapOptions { kind: MapKind::Modules, package: Some(PackageRef { folder: b"crates/app".to_vec(), language: ModuleLanguage::Rust }), ..Default::default() },
        modules: Some(ModuleMap { package: 0, root: Some(1), ..Default::default() }),
        ..Default::default()
    }
}
/// The node of module `k` of `modules_map`.
fn m(k: usize) -> usize {
    k + 1
}
fn modules_page(data: RepositoryMap) -> MapPage {
    let mut s = MapPage::default();
    s.finish(Ok(data));
    s.rebuild();
    s
}
fn owned(list: &[(String, Option<usize>, usize)]) -> Vec<(&str, Option<usize>, usize)> {
    list.iter().map(|(p, a, l)| (p.as_str(), *a, *l)).collect()
}

/// lib.rs; chosen with chosen::inner inside it; ten users; four modules it uses, one of
/// them only from inside.
fn around_fixture() -> RepositoryMap {
    let mut modules: Vec<(String, Option<usize>, usize)> = vec![("".into(), None, 10), ("chosen".into(), Some(0), 100), ("chosen::inner".into(), Some(1), 50)];
    for k in 0..10 {
        modules.push((format!("user{k}"), Some(0), 5));
    }
    for name in ["style", "pages", "app", "x"] {
        modules.push((name.into(), Some(0), 5));
    }
    let mut uses: Vec<(usize, usize, &[&str])> = (0..10).map(|k| (3 + k, 1, &["Picker"][..])).collect();
    // user0 also uses a name from inside the chosen module.
    uses.push((3, 2, &["scope_check"]));
    uses.push((1, 13, &["icon", "Icon"]));
    uses.push((1, 14, &["pages"]));
    uses.push((1, 15, &["Rocket"]));
    uses.push((2, 16, &["helper"]));
    // The chosen module's own child is inside it, not beside it.
    uses.push((1, 2, &["check"]));
    modules_map(&owned(&modules), &uses)
}
/// A crate root with one more top-level module than can be drawn whole.
fn large_fixture(uses: &[(usize, usize, &[&str])]) -> RepositoryMap {
    let mut modules: Vec<(String, Option<usize>, usize)> = vec![("".into(), None, 1)];
    for k in 0..=modules::WHOLE {
        modules.push((format!("m{k:02}"), Some(0), 1));
    }
    modules_map(&owned(&modules), uses)
}

#[test]
fn modules_around_one_module_put_users_left_and_what_it_uses_right() {
    crate::i18n::scoped(crate::i18n::Language::English, || {
        let mut s = modules_page(around_fixture());
        // A package small enough to draw whole starts there.
        assert!(s.whole && s.selected.is_none());
        s.whole = false;
        s.selected = Some(m(1));
        s.rebuild();
        let users: Vec<usize> = (3..13).map(m).collect();
        let uses = [m(13), m(14), m(15), m(16)];
        // Eight users are cards; the other two are counted on one card.
        let shown_users: Vec<usize> = s.visible.iter().copied().filter(|i| users.contains(i)).collect();
        assert_eq!(shown_users.len(), modules::SIDE);
        assert!(shown_users.contains(&m(3)), "the user with the most names is shown");
        let more = s.file_groups.iter().find(|g| g.more == Some(modules::More::Users)).expect("a card for the rest");
        assert_eq!(more.files, 2);
        for &u in &uses {
            assert!(s.visible.contains(&u), "{u}");
        }
        assert!(!s.visible.contains(&m(2)), "a module inside the chosen one is not beside it");
        let x = |i: usize| s.positions[i].x;
        assert!(shown_users.iter().all(|&u| x(u) < x(m(1))));
        assert!(uses.iter().all(|&u| x(u) > x(m(1))));
        // Names from the module and from inside it come together on one connection.
        let first = s.projected_links.iter().find(|e| e.from == m(3) && e.to == m(1)).unwrap();
        assert_eq!(first.evidence, [b"Picker".to_vec(), b"scope_check".to_vec()]);
        let to_x = s.projected_links.iter().find(|e| e.from == m(1) && e.to == m(16)).unwrap();
        assert_eq!(to_x.evidence, [b"helper".to_vec()]);
        let captions: Vec<(modules::CaptionKind, usize)> = s.captions.iter().map(|c| (c.what, c.count)).collect();
        assert_eq!(captions, [(modules::CaptionKind::UsedBy, 10), (modules::CaptionKind::Uses, 4)]);
        // The list counts users the same way.
        let units = s.units().unwrap();
        assert_eq!(units.used_by[&m(1)], 10);
        assert_eq!(units.size(m(1)), (2, 150));
        let data = s.data.clone().unwrap();
        assert_eq!(modules::card_detail(&s, &data, m(3)), "Picker, scope_check");
        assert_eq!(modules::card_detail(&s, &data, m(1)), "2 files · 150 lines");
        assert!(modules::hover(&s, &data, m(13)).starts_with("chosen uses 2 names from style\n"));
    });
}

#[test]
fn modules_a_nested_module_sees_its_parent_and_siblings_by_their_paths() {
    let data = modules_map(
        &[("", None, 1), ("map", Some(0), 1), ("map::details", Some(1), 1), ("map::drawing", Some(1), 1), ("style", Some(0), 1), ("style::tokens", Some(4), 1)],
        &[(2, 1, &["MapPage"]), (2, 3, &["CARD"]), (2, 5, &["TEXT"]), (1, 2, &["card"])],
    );
    let mut s = modules_page(data);
    s.whole = false;
    s.selected = Some(m(2));
    s.rebuild();
    let data = s.data.clone().unwrap();
    let names: std::collections::BTreeSet<String> = s.visible.iter().map(|&i| modules::name(&data, i)).collect();
    // Its parent as itself, a sibling by its path, another module's file by that module.
    assert_eq!(names, ["map", "map::details", "map::drawing", "style"].into_iter().map(String::from).collect());
    let users: Vec<usize> = s.projected_links.iter().filter(|e| e.to == m(2)).map(|e| e.from).collect();
    assert_eq!(users, [m(1)]);
    // map uses details and details uses map: map is drawn once, on the users' side,
    // with a connection each way, so Export gets each card once.
    let mut seen = s.visible.clone();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), s.visible.len(), "{:?}", s.visible);
    assert!(s.positions[m(1)].x < s.positions[m(2)].x);
    assert!(s.projected_links.iter().any(|e| e.from == m(2) && e.to == m(1)));
    assert!(map_mermaid(&data, &s.visible, &s.projected_links, &CancellationToken::new()).is_ok());
}

#[test]
fn modules_whole_package_layers_users_above_what_they_use_and_sets_the_rest_apart() {
    // a uses b, b uses c, a uses c; b and e use each other; d has no connection.
    let data = modules_map(
        &[("", None, 1), ("a", Some(0), 1), ("b", Some(0), 1), ("c", Some(0), 1), ("d", Some(0), 1), ("e", Some(0), 1)],
        &[(1, 2, &["B"]), (2, 3, &["C"]), (1, 3, &["C"]), (2, 5, &["E"]), (5, 2, &["B"])],
    );
    let s = modules_page(data);
    assert!(s.whole);
    let y = |i: usize| s.positions[i].y;
    assert!(y(m(1)) < y(m(2)) && y(m(2)) < y(m(3)), "{:?}", s.positions);
    // A circle does not push cards down without end.
    assert!(y(m(5)) <= y(m(3)) + drawing::CARD.y + 80., "{:?}", s.positions);
    let bottom = [1, 2, 3, 5].iter().map(|&k| y(m(k))).fold(0f32, f32::max);
    assert!(y(m(4)) > bottom, "a module with no connection sits apart");
    assert_eq!(s.captions.iter().map(|c| (c.what, c.count)).collect::<Vec<_>>(), [(modules::CaptionKind::NotConnected, 1)]);
    assert_eq!(s.projected_links.len(), 5);
}

#[test]
fn modules_start_on_the_whole_package_or_the_most_used_module_and_keep_it_on_refresh() {
    let small = around_fixture();
    let mut s = modules_page(small.clone());
    assert!(s.whole);
    // A package too large to draw whole starts on its most used module.
    let large = large_fixture(&[(1, 7, &["X"]), (2, 7, &["X"]), (3, 9, &["Y"])]);
    let mut big = modules_page(large.clone());
    assert!(!big.whole);
    assert_eq!(big.selected, Some(m(7)));
    big.selected = Some(m(9));
    big.finish(Ok(large.clone()));
    assert_eq!(big.selected, Some(m(9)), "a refresh keeps the chosen module");
    // Another package starts again.
    let mut other = large;
    other.scope.package = Some(PackageRef { folder: b"crates/other".to_vec(), language: ModuleLanguage::Rust });
    big.finish(Ok(other));
    assert_eq!(big.selected, Some(m(7)));
    // The whole package stays chosen across a refresh of the same package.
    s.finish(Ok(small));
    assert!(s.whole);
}

#[test]
fn modules_list_rows_search_open_what_holds_a_match_and_order_by_name() {
    let data = modules_map(
        &[("", None, 1), ("map", Some(0), 1), ("map::details", Some(1), 1), ("style", Some(0), 1), ("app", Some(0), 1)],
        &[(1, 3, &["TEXT"]), (4, 3, &["TEXT"]), (4, 1, &["view"])],
    );
    let mut s = modules_page(data);
    let units = s.units().unwrap();
    let data = s.data.clone().unwrap();
    let names = |rows: &[modules::Row]| rows.iter().map(|r| (modules::name(&data, r.node), r.depth)).collect::<Vec<_>>();
    // Most used first: style (2), map (1), then app and the crate root (0), by name.
    let rows = modules::rows(&data, &units, &s, "");
    assert_eq!(names(&rows), [("style".into(), 0), ("map".into(), 0), ("app".into(), 0), ("lib.rs".into(), 0)]);
    assert!(rows[1].folder && !rows[1].open);
    assert!(!rows[3].folder, "the crate root holds every module, so it opens nothing");
    // A search opens a module to show the match inside it.
    let rows = modules::rows(&data, &units, &s, "details");
    assert_eq!(names(&rows), [("map".into(), 0), ("map::details".into(), 1)]);
    s.by_name = true;
    let rows = modules::rows(&data, &units, &s, "");
    assert_eq!(names(&rows)[0].0, "app");
    // Search moves the picture to the first match.
    s.whole = false;
    s.selected = Some(m(3));
    s.query = "app".into();
    s.rebuild();
    assert_eq!(s.selected, Some(m(4)));
}

/// Draws Modules' list and picture in `rect` and returns the controls.
fn modules_frame(ctx: &Context, s: &mut MapPage, rect: Rect, compact: bool, events: Vec<Event>, time: f64) -> HashMap<String, UiControl> {
    let mut controls = HashMap::new();
    let mut intents = Vec::new();
    let mut output = ctx.run_ui(RawInput { screen_rect: Some(rect), events, time: Some(time), ..Default::default() }, |ui| {
        s.rebuild();
        modules::body(ui, s, rect, false, compact, &mut intents, &mut controls);
    });
    output.textures_delta.clear();
    controls
}

#[test]
fn modules_list_and_picture_take_clicks_and_fold_into_a_button_when_narrow() {
    crate::i18n::scoped(crate::i18n::Language::English, || {
        let ctx = Context::default();
        crate::style::install(&ctx);
        let mut s = modules_page(around_fixture());
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(880., 632.));
        modules_frame(&ctx, &mut s, rect, false, vec![], 0.);
        let controls = modules_frame(&ctx, &mut s, rect, false, vec![], 0.1);
        for (id, c) in &controls {
            assert!(rect.contains_rect(c.rect), "{id}: {:?}", c.rect);
        }
        assert!(controls.contains_key("map-module-whole"));
        // The whole package: every module of the crate root drawn.
        assert!(controls.contains_key(&format!("map-node-{}", m(13))));
        // A row puts its module in the middle; the rest of its users go on one card.
        let row = controls[&format!("map-module-{}", m(1))].rect;
        modules_frame(&ctx, &mut s, rect, false, click_events(row.center()), 0.2);
        assert_eq!((s.whole, s.selected), (false, Some(m(1))));
        modules_frame(&ctx, &mut s, rect, false, vec![], 0.3);
        let controls = modules_frame(&ctx, &mut s, rect, false, vec![], 0.4);
        assert!(controls.contains_key("map-module-more-users"), "{:?}", controls.keys());
        // A card moves to the middle and the list follows it.
        let card = controls[&format!("map-node-{}", m(13))].rect;
        modules_frame(&ctx, &mut s, rect, false, click_events(card.center()), 0.5);
        assert_eq!(s.selected, Some(m(13)));
        modules_frame(&ctx, &mut s, rect, false, vec![], 0.6);
        assert!(s.visible.contains(&m(1)), "chosen is now among style's users");
        // A twisty opens a module without choosing it.
        let controls = modules_frame(&ctx, &mut s, rect, false, vec![], 0.7);
        let toggle = controls[&format!("map-module-toggle-{}", m(1))].rect;
        modules_frame(&ctx, &mut s, rect, false, click_events(toggle.center()), 0.8);
        assert!(s.expanded.contains(b"module:chosen".as_slice()));
        assert_eq!(s.selected, Some(m(13)));
        // Narrow: the list is a button over the picture.
        let narrow = Rect::from_min_size(Pos2::ZERO, vec2(700., 400.));
        modules_frame(&ctx, &mut s, narrow, true, vec![], 0.9);
        let controls = modules_frame(&ctx, &mut s, narrow, true, vec![], 1.0);
        assert!(controls.contains_key("map-module-switcher"));
        assert!(!controls.contains_key(&format!("map-module-{}", m(1))));
    });
}

#[test]
fn modules_without_a_package_say_so_and_a_large_whole_package_asks_for_one_module() {
    crate::i18n::scoped(crate::i18n::Language::English, || {
        let ctx = Context::default();
        crate::style::install(&ctx);
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(880., 600.));
        let mut none = modules_page(RepositoryMap { kind: MapKind::Modules, ..Default::default() });
        modules_frame(&ctx, &mut none, rect, false, vec![], 0.);
        let controls = modules_frame(&ctx, &mut none, rect, false, vec![], 0.1);
        assert!(controls.contains_key("map-modules-show-folders"));
        let mut big = modules_page(large_fixture(&[]));
        big.whole = true;
        big.selected = None;
        modules_frame(&ctx, &mut big, rect, false, vec![], 0.);
        let controls = modules_frame(&ctx, &mut big, rect, false, vec![], 0.1);
        assert!(controls.contains_key("map-module-whole"), "Whole package stays listed while chosen");
        assert!(!controls.keys().any(|k| k.starts_with("map-node-")));
        // Not chosen, it is still listed: a large package's facts are reached through it.
        big.whole = false;
        big.selected = Some(m(1));
        modules_frame(&ctx, &mut big, rect, false, vec![], 0.2);
        let controls = modules_frame(&ctx, &mut big, rect, false, vec![], 0.3);
        assert!(controls.contains_key("map-module-whole"));
    });
}

#[test]
fn modules_used_by_counts_match_what_details_lists() {
    // Counted in one pass for the list; Details walks the links for one module.
    let nested = modules_map(
        &[("", None, 1), ("map", Some(0), 1), ("map::details", Some(1), 1), ("map::drawing", Some(1), 1), ("style", Some(0), 1), ("style::tokens", Some(4), 1)],
        &[(2, 1, &["MapPage"]), (2, 3, &["CARD"]), (2, 5, &["TEXT"]), (1, 2, &["card"]), (4, 0, &["Rocket"]), (5, 2, &["x"])],
    );
    for data in [around_fixture(), nested] {
        let mut s = modules_page(data);
        let units = s.units().unwrap();
        let data = s.data.clone().unwrap();
        for i in (0..data.nodes.len()).filter(|&i| data.nodes[i].kind == MapNodeKind::Module) {
            assert_eq!(units.used_by[&i], units.neighbors(&data, i).0.len(), "{}", modules::name(&data, i));
        }
    }
}

#[test]
fn modules_a_module_chosen_during_a_search_stays_chosen() {
    let data = modules_map(
        &[("", None, 1), ("map", Some(0), 1), ("map::details", Some(1), 1), ("style", Some(0), 1)],
        &[(2, 1, &["MapPage"]), (1, 3, &["TEXT"])],
    );
    let mut s = modules_page(data);
    s.whole = false;
    s.selected = Some(m(3));
    s.query = "details".into();
    s.rebuild();
    assert_eq!(s.selected, Some(m(2)), "a new search picks its first match");
    // A neighbour card or a Details row picked while the search is on stays picked.
    s.selected = Some(m(1));
    s.rebuild();
    assert_eq!(s.selected, Some(m(1)));
    assert!(s.visible.contains(&m(1)));
}

#[test]
fn modules_with_no_connection_wrap_into_rows() {
    let mut modules: Vec<(String, Option<usize>, usize)> = vec![("".into(), None, 1), ("a".into(), Some(0), 1), ("b".into(), Some(0), 1)];
    for k in 0..9 {
        modules.push((format!("alone{k}"), Some(0), 1));
    }
    let s = modules_page(modules_map(&owned(&modules), &[(1, 2, &["B"])]));
    assert!(s.whole);
    let rows: std::collections::BTreeSet<i64> = (3..12).map(|k| s.positions[m(k)].y as i64).collect();
    assert_eq!(rows.len(), 3, "nine modules, four a row");
    assert!((3..12).all(|k| s.positions[m(k)].x < 4. * (drawing::CARD.x + 26.)));
}

#[test]
fn modules_list_keeps_the_picked_row_in_view() {
    // The list once ended up scrolled far below the module just picked from it.
    crate::i18n::scoped(crate::i18n::Language::English, || {
        let ctx = Context::default();
        crate::style::install(&ctx);
        let uses: Vec<(usize, usize, &[&str])> = (2..30).map(|k| (k, 1, &["X"][..])).collect();
        let mut s = modules_page(large_fixture(&uses));
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(880., 632.));
        let mut time = 0.;
        let mut frame = |s: &mut MapPage, events: Vec<Event>| {
            time += 0.1;
            modules_frame(&ctx, s, rect, false, events, time)
        };
        frame(&mut s, vec![]);
        frame(&mut s, vec![]);
        // Another package's read, then this one again, as the package button does.
        let mut other = large_fixture(&uses);
        other.scope.package = Some(PackageRef { folder: b"crates/other".to_vec(), language: ModuleLanguage::Rust });
        s.finish(Ok(other));
        frame(&mut s, vec![]);
        s.finish(Ok(large_fixture(&uses)));
        frame(&mut s, vec![]);
        let controls = frame(&mut s, vec![]);
        let pick = m(5);
        let row = controls.get(&format!("map-module-{pick}")).unwrap_or_else(|| panic!("row {pick} not in view: {:?}", controls.keys())).rect;
        frame(&mut s, click_events(row.center()));
        assert_eq!(s.selected, Some(pick));
        for _ in 0..3 {
            frame(&mut s, vec![]);
        }
        let controls = frame(&mut s, vec![]);
        assert!(controls.contains_key(&format!("map-module-{pick}")), "the picked row left the list's view");
        // Picked on the picture, a module far down the list is scrolled into view.
        let far = m(modules::WHOLE);
        s.selected = Some(far);
        s.list_jump = true;
        frame(&mut s, vec![]);
        let controls = frame(&mut s, vec![]);
        assert!(controls.contains_key(&format!("map-module-{far}")), "a module picked elsewhere is scrolled into the list's view");
    });
}

/// A unit of `language_map`: key, label, detail, Go folder, files, lines, parent.
type TestUnit<'a> = (&'a str, &'a str, &'a str, bool, usize, usize, Option<usize>);
/// A Modules read of another language than Rust: a package node, then its units, and
/// connections between them.
fn language_map(language: ModuleLanguage, units: &[TestUnit], uses: &[(usize, usize, &[&str])]) -> RepositoryMap {
    let node = |key: String, label: &str, detail: &str, kind, files, count| MapNode {
        key: key.into_bytes(),
        label: label.as_bytes().to_vec(),
        detail: detail.as_bytes().to_vec(),
        kind,
        path: Some(b"pkg".to_vec()),
        commit: None,
        count,
        files,
        evidence: Vec::new(),
        refs: Vec::new(),
        time: 0,
    };
    let mut nodes = vec![node("package-x:pkg".into(), "pkg", "Package", MapNodeKind::Crate, units.len(), units.len())];
    let mut links = Vec::new();
    for (k, (key, label, detail, folder, files, lines, parent)) in units.iter().enumerate() {
        let kind = if *folder { MapNodeKind::Directory } else { MapNodeKind::Module };
        nodes.push(node(format!("module:{key}"), label, detail, kind, *files, *lines));
        if let Some(p) = parent {
            links.push(MapLink { from: p + 1, to: k + 1, kind: MapLinkKind::Contains, weight: 1, evidence: vec![] });
        }
    }
    for (from, to, names) in uses {
        links.push(MapLink { from: from + 1, to: to + 1, kind: MapLinkKind::DependsOn, weight: names.len(), evidence: names.iter().map(|n| n.as_bytes().to_vec()).collect() });
    }
    let package = PackageRef { folder: b"pkg".to_vec(), language };
    RepositoryMap {
        kind: MapKind::Modules,
        nodes,
        links,
        scope: MapOptions { kind: MapKind::Modules, package: Some(package), ..Default::default() },
        modules: Some(ModuleMap { package: 0, language, ..Default::default() }),
        module_packages: vec![ModulePackage { folder: b"pkg".to_vec(), name: b"pkg".to_vec(), language, dialect: None, manifest: true, files: units.len(), node: Some(0) }],
        ..Default::default()
    }
}

#[test]
fn modules_go_packages_stand_alone_and_folders_only_hold_them_in_the_list() {
    crate::i18n::scoped(crate::i18n::Language::English, || {
        // net holds net/http in the list, but each is a package of its own.
        let data = language_map(
            ModuleLanguage::Go,
            &[
                ("archive", "archive", "Folder", true, 0, 0, None),
                ("archive/tar", "tar", "Go package", false, 2, 40, Some(0)),
                ("io", "io", "Go package", false, 3, 100, None),
                ("net", "net", "Go package", false, 5, 500, None),
                ("net/http", "http", "Go package", false, 32, 18_604, Some(3)),
            ],
            &[(1, 2, &["EOF"]), (4, 2, &["Copy", "ReadAll"]), (4, 3, &["Dial"])],
        );
        let mut s = modules_page(data);
        let units = s.units().unwrap();
        let data = s.data.clone().unwrap();
        // Every package counts at the top; the folder only in the list.
        let mut top: Vec<String> = units.top.iter().map(|&i| modules::name(&data, i)).collect();
        top.sort();
        assert_eq!(top, ["archive/tar", "io", "net", "net/http"]);
        // net/http's use of net is net's, not a use from inside it.
        assert_eq!(units.used_by[&m(2)], 2);
        assert_eq!(units.used_by[&m(3)], 1);
        assert_eq!(units.used_by[&m(4)], 0);
        assert_eq!(units.size(m(3)), (5, 500), "a package holds its own files only");
        let (users, uses) = units.neighbors(&data, m(4));
        assert!(users.is_empty());
        let uses: Vec<String> = uses.iter().map(|n| modules::name(&data, n.unit)).collect();
        assert_eq!(uses, ["io", "net"]);
        let rows = modules::rows(&data, &units, &s, "");
        let first: Vec<(String, bool)> = rows.iter().filter(|r| r.depth == 0).map(|r| (modules::name(&data, r.node), r.folder)).collect();
        assert_eq!(first, [("io".to_owned(), false), ("net".into(), true), ("archive".into(), true)], "most used first, the folder last");
        // A search never chooses a folder.
        s.whole = false;
        s.query = "arch".into();
        s.rebuild();
        assert_eq!(s.selected, Some(m(1)));
        let words = modules::Words::of(&s);
        assert_eq!((words.find(), words.legend(4, 3)), ("Find a package", "Packages 4 · links 3 · arrows point to what is used".to_owned()));
        assert_eq!(words.files(32), "32 Go files");
        // net/http chosen on the picture: the list opens net to show it.
        let ctx = Context::default();
        crate::style::install(&ctx);
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(880., 600.));
        s.query.clear();
        s.selected = Some(m(4));
        s.list_jump = true;
        s.filter_key = None;
        modules_frame(&ctx, &mut s, rect, false, vec![], 0.);
        let controls = modules_frame(&ctx, &mut s, rect, false, vec![], 0.1);
        assert!(s.expanded.contains(&data.nodes[m(3)].key), "net is opened");
        assert!(controls.contains_key(&format!("map-module-{}", m(4))), "net/http is a row in view");
    });
}

#[test]
fn modules_javascript_folders_group_their_files_and_cards_use_file_names() {
    crate::i18n::scoped(crate::i18n::Language::English, || {
        let data = language_map(
            ModuleLanguage::JavaScript,
            &[
                ("tests", "tests", "Folder", false, 0, 0, None),
                ("tests/git-relay.cjs", "git-relay.cjs", "JavaScript module", false, 1, 28, Some(0)),
                ("tests/features", "features", "Folder", false, 0, 0, Some(0)),
                ("tests/features/a.cjs", "a.cjs", "JavaScript module", false, 1, 70, Some(2)),
                ("tests/smoke.cjs", "smoke.cjs", "JavaScript module", false, 1, 10, Some(0)),
                ("mcp", "mcp", "Folder", false, 0, 0, None),
            ],
            &[(3, 1, &["gitRelay"]), (4, 1, &["gitRelay"]), (1, 5, &["client"])],
        );
        let mut s = modules_page(data);
        let units = s.units().unwrap();
        let data = s.data.clone().unwrap();
        let mut top: Vec<String> = units.top.iter().map(|&i| modules::name(&data, i)).collect();
        top.sort();
        assert_eq!(top, ["mcp", "tests"], "a project's top folders are its top level");
        // Without a root, a link joins the top-level folders that hold its two ends.
        assert_eq!(units.top_links, 1);
        // Inside an open folder the rows follow the list's order too: most used first.
        s.expanded.insert(data.nodes[m(0)].key.clone());
        let inside: Vec<String> = modules::rows(&data, &units, &s, "").iter().filter(|r| r.depth == 1).map(|r| modules::name(&data, r.node)).collect();
        assert_eq!(inside, ["git-relay.cjs", "features", "smoke.cjs"]);
        s.by_name = true;
        let inside: Vec<String> = modules::rows(&data, &units, &s, "").iter().filter(|r| r.depth == 1).map(|r| modules::name(&data, r.node)).collect();
        assert_eq!(inside, ["features", "git-relay.cjs", "smoke.cjs"]);
        s.by_name = false;
        assert_eq!(units.size(m(0)), (3, 108), "a folder sums the files inside it");
        assert_eq!(modules::name(&data, m(1)), "git-relay.cjs");
        // Beside git-relay.cjs the features folder stands for the file inside it.
        let (users, _) = units.neighbors(&data, m(1));
        let users: Vec<String> = users.iter().map(|n| modules::name(&data, n.unit)).collect();
        assert_eq!(users, ["features", "smoke.cjs"]);
        let words = modules::Words::of(&s);
        assert_eq!(words.find(), "Find a module");
        assert_eq!(words.files(226), "226 JavaScript files");
        assert_eq!(words.keyword(OutlineKind::Function), "function");
        assert_eq!(modules::Words { language: ModuleLanguage::Python, dialect: None }.keyword(OutlineKind::Function), "def");
        assert_eq!(modules::Words { language: ModuleLanguage::Go, dialect: None }.keyword(OutlineKind::Function), "func");
        assert_eq!(modules::Words { language: ModuleLanguage::JavaScript, dialect: Some(Dialect::TypeScript) }.package_kind(false), "TypeScript folder · modules");
    });
}

#[test]
fn modules_java_packages_stand_alone_and_each_language_names_its_units() {
    crate::i18n::scoped(crate::i18n::Language::English, || {
        // com.acme.app.web holds web.api in the list, as Go's folders do; each is a package.
        let data = language_map(
            ModuleLanguage::Java,
            &[
                ("com.acme.app", "app", "Java package", false, 1, 20, None),
                ("com.acme.app.store", "store", "Java package", false, 2, 60, None),
                ("com.acme.app.web", "web", "Folder", true, 0, 0, None),
                ("com.acme.app.web.api", "api", "Java package", false, 3, 90, Some(2)),
            ],
            &[(0, 1, &["Db"]), (3, 1, &["Db", "Cache"])],
        );
        let mut s = modules_page(data);
        let units = s.units().unwrap();
        let data = s.data.clone().unwrap();
        let mut top: Vec<String> = units.top.iter().map(|&i| modules::name(&data, i)).collect();
        top.sort();
        assert_eq!(top, ["api", "app", "store"], "every package counts at the top, the folder only in the list");
        assert_eq!(units.used_by[&m(1)], 2);
        assert_eq!(units.size(m(3)), (3, 90), "a package holds its own files only");
        let words = modules::Words::of(&s);
        assert_eq!((words.find(), words.legend(3, 2)), ("Find a package", "Packages 3 · links 2 · arrows point to what is used".to_owned()));
        assert_eq!(words.files(12), "12 Java files");
        assert_eq!(words.package_kind(true), "Java project · packages");
        assert_eq!(words.keyword(OutlineKind::Enum), "enum");
        assert_eq!(words.keyword(OutlineKind::Annotation), "@interface");
        assert_eq!(words.keyword(OutlineKind::Record), "record");
        // Kotlin's words in a project mostly in Kotlin.
        let kotlin = modules::Words { language: ModuleLanguage::Java, dialect: Some(Dialect::Kotlin) };
        assert_eq!((kotlin.files(1), kotlin.package_kind(false)), ("1 Kotlin file".to_owned(), "Kotlin folder · packages"));
        let keywords: Vec<&str> = [OutlineKind::Function, OutlineKind::Const, OutlineKind::Var, OutlineKind::Enum, OutlineKind::Object, OutlineKind::Type, OutlineKind::Annotation].iter().map(|k| kotlin.keyword(*k)).collect();
        assert_eq!(keywords, ["fun", "val", "var", "enum class", "object", "typealias", "annotation class"]);
        // C# has namespaces; C and C++ files.
        let csharp = modules::Words { language: ModuleLanguage::CSharp, dialect: None };
        assert_eq!((csharp.find(), csharp.legend(5, 4)), ("Find a namespace", "Namespaces 5 · links 4 · arrows point to what is used".to_owned()));
        assert_eq!((csharp.files(2), csharp.package_kind(true), csharp.keyword(OutlineKind::Type)), ("2 C# files".to_owned(), "C# project · namespaces", "delegate"));
        let c = modules::Words { language: ModuleLanguage::C, dialect: None };
        assert_eq!((c.find(), c.legend(4, 1)), ("Find a file", "Files 4 · links 1 · arrows point to what is used".to_owned()));
        assert_eq!((c.files(3), c.package_kind(false)), ("3 C files".to_owned(), "C folder · files"));
        for (language, name) in [(ModuleLanguage::PowerShell, "PowerShell"), (ModuleLanguage::Bash, "Bash"), (ModuleLanguage::Lua, "Lua")] {
            let scripts = modules::Words { language, dialect: None };
            assert_eq!(scripts.find(), "Find a file");
            assert_eq!(scripts.files(1), format!("1 {name} file"));
            assert_eq!(scripts.files(2), format!("2 {name} files"));
            assert_eq!(scripts.package_kind(false), format!("{name} scripts"));
            assert_eq!(modules::language_name(language, None), name);
        }
        let workflow = modules::Words { language: ModuleLanguage::GitHubActions, dialect: None };
        assert_eq!(workflow.find(), "Find a file");
        assert_eq!(workflow.files(2), "2 files");
        assert_eq!(workflow.package_kind(false), "GitHub Actions");
        assert_eq!(modules::language_name(workflow.language, None), "GitHub Actions");
        let data = language_map(workflow.language, &[
            ("ci.yml", "ci.yml", "GitHub Actions workflow", false, 1, 8, None),
            ("tool.sh", "tool.sh", "Referenced file", false, 1, 0, None),
        ], &[(0, 1, &["run"])]);
        let units = modules::Units::new(&data);
        assert_eq!(modules::size_text(&units, 1), "8 lines");
        assert!(modules::size_text(&units, 2).is_empty());
        let keywords: Vec<&str> = [OutlineKind::Function, OutlineKind::Macro, OutlineKind::Type, OutlineKind::Struct, OutlineKind::Var].iter().map(|k| c.keyword(*k)).collect();
        assert_eq!(keywords, ["function", "#define", "typedef", "struct", "extern"]);
        let cpp = modules::Words { language: ModuleLanguage::C, dialect: Some(Dialect::Cpp) };
        assert_eq!((cpp.files(9), cpp.package_kind(true)), ("9 C++ files".to_owned(), "C++ project · files"));
        for (dialect, name) in [(Dialect::ObjectiveC, "Objective-C"), (Dialect::ObjectiveCpp, "Objective-C++")] {
            let words = modules::Words { language: ModuleLanguage::C, dialect: Some(dialect) };
            assert_eq!(modules::language_name(words.language, words.dialect), name);
            assert_eq!(words.files(2), format!("2 {name} files"));
            assert_eq!(words.package_kind(false), format!("{name} folder · files"));
            assert_eq!(words.package_kind(true), format!("{name} project · files"));
        }
        let names: Vec<&str> = [(ModuleLanguage::Java, None), (ModuleLanguage::Java, Some(Dialect::Kotlin)), (ModuleLanguage::C, None), (ModuleLanguage::C, Some(Dialect::Cpp)), (ModuleLanguage::CSharp, None)]
            .iter()
            .map(|(l, d)| modules::language_name(*l, *d))
            .collect();
        assert_eq!(names, ["Java", "Kotlin", "C", "C++", "C#"]);
    });
}

// ---- Code → Folders as a code map (canvas section 47).

/// A Code read of these committed files: every folder on the way becomes a folder
/// node, joined to what it holds. Keys are paths, as the Code read makes them.
fn code_snapshot(files: &[&str]) -> RepositoryMap {
    let mut d = RepositoryMap::default();
    let mut index: HashMap<String, usize> = HashMap::new();
    for file in files {
        let parts: Vec<&str> = file.split('/').collect();
        let mut parent: Option<usize> = None;
        for k in 0..parts.len() {
            let path = parts[..=k].join("/");
            let folder = k + 1 < parts.len();
            let at = *index.entry(path.clone()).or_insert_with(|| {
                d.nodes.push(MapNode {
                    key: path.clone().into_bytes(),
                    label: parts[k].as_bytes().to_vec(),
                    detail: Vec::new(),
                    kind: if folder { MapNodeKind::Directory } else { MapNodeKind::File },
                    path: Some(path.clone().into_bytes()),
                    commit: None,
                    count: if folder { 0 } else { 100 },
                    files: 0,
                    evidence: Vec::new(),
                    refs: Vec::new(),
                    time: 0,
                });
                let at = d.nodes.len() - 1;
                if let Some(p) = parent {
                    d.links.push(MapLink { from: p, to: at, kind: MapLinkKind::Contains, weight: 1, evidence: vec![] });
                }
                at
            });
            d.nodes[at].files += 1;
            parent = Some(at);
        }
    }
    d
}
/// Links between ends: a path ending in `/` is a folder end.
fn code_links(links: &[(&str, &str)]) -> CodeLinks {
    let mut out = CodeLinks::default();
    let end = |p: &str, out: &mut CodeLinks| -> usize {
        let e = CodeEnd { path: p.trim_end_matches('/').as_bytes().to_vec(), folder: p.ends_with('/') };
        match out.ends.iter().position(|x| *x == e) {
            Some(i) => i,
            None => {
                out.ends.push(e);
                out.ends.len() - 1
            }
        }
    };
    for (a, b) in links {
        let (from, to) = (end(a, &mut out), end(b, &mut out));
        out.links.push(CodeLink { from, to, names: vec!["name".into()] });
    }
    out.packages.push(PackageRef { folder: Vec::new(), language: ModuleLanguage::Rust });
    out
}
/// A Folders page with every folder closed, as a first read leaves it.
fn code_page(files: &[&str], links: &[(&str, &str)]) -> MapPage {
    let d = code_snapshot(files);
    let mut s = MapPage { data: Some(Arc::new(d)), ..Default::default() };
    let data = s.data.clone().unwrap();
    s.collapsed = data.nodes.iter().filter(|n| n.kind == MapNodeKind::Directory).map(|n| n.key.clone()).collect();
    s.code.links = Some(Arc::new(code_links(links)));
    s
}
fn node(s: &MapPage, path: &str) -> usize {
    s.data.as_ref().unwrap().nodes.iter().position(|n| n.path.as_deref() == Some(path.as_bytes())).unwrap()
}
fn open(s: &mut MapPage, path: &str) {
    let key = path.as_bytes().to_vec();
    s.collapsed.remove(&key);
    s.filter_key = None;
}
fn picture(s: &MapPage) -> Arc<code_map::Picture> {
    s.code.layout.clone().expect("a laid out code map")
}
/// Arrows as `from -> to (files)`, by the name of their card or box.
fn arrows_of(s: &MapPage) -> Vec<String> {
    let p = picture(s);
    let data = s.data.as_ref().unwrap();
    let name = |i: usize| match p.placed[i].item {
        code_map::Item::Node(n) => String::from_utf8_lossy(data.nodes[n].path.as_deref().unwrap()).into_owned(),
        code_map::Item::Files(_) => "files".into(),
    };
    let mut out: Vec<String> = p.arrows.iter().map(|a| format!("{} -> {} ({})", name(a.from), name(a.to), a.files)).collect();
    out.sort();
    out
}
const CODE: [&str; 7] = [
    "crates/ui/src/lib.rs",
    "crates/ui/src/view.rs",
    "crates/core/src/lib.rs",
    "crates/core/src/store.rs",
    "tools/a.js",
    "tools/b.js",
    "README.md",
];
const USES: [(&str, &str); 4] = [
    ("crates/ui/src/lib.rs", "crates/core/"),
    ("crates/ui/src/view.rs", "crates/core/src/store.rs"),
    ("crates/core/src/store.rs", "crates/core/src/lib.rs"),
    ("tools/a.js", "crates/ui/src/lib.rs"),
];

#[test]
fn code_map_gathers_links_between_closed_folders_and_splits_them_when_opened() {
    let mut s = code_page(&CODE, &USES);
    s.rebuild();
    // Closed: one arrow between the top folders; the file nothing uses is apart.
    assert_eq!(arrows_of(&s), ["tools -> crates (1)"]);
    let p = picture(&s);
    assert_eq!(p.captions.len(), 1, "README.md sits under Not connected");
    let readme = p.index(code_map::Item::Node(node(&s, "README.md"))).unwrap();
    let tools = p.index(code_map::Item::Node(node(&s, "tools"))).unwrap();
    assert!(p.placed[readme].rect.top() > p.placed[tools].rect.bottom(), "unconnected cards come after");
    // What uses sits above what it uses.
    let crates = p.index(code_map::Item::Node(node(&s, "crates"))).unwrap();
    assert!(p.placed[tools].rect.bottom() < p.placed[crates].rect.top());
    // Opening crates splits its arrow; two files of ui use core.
    open(&mut s, "crates");
    s.rebuild();
    assert_eq!(arrows_of(&s), ["crates/ui -> crates/core (2)", "tools -> crates/ui (1)"]);
    let p = picture(&s);
    let boxed = p.index(code_map::Item::Node(node(&s, "crates"))).unwrap();
    assert!(p.placed[boxed].open);
    for child in ["crates/ui", "crates/core"] {
        let i = p.index(code_map::Item::Node(node(&s, child))).unwrap();
        assert_eq!(p.placed[i].parent, Some(boxed));
        assert!(p.placed[boxed].rect.contains_rect(p.placed[i].rect), "{child} sits inside its box");
    }
    // A folder holding one open folder shares its box: crates/ui / src.
    open(&mut s, "crates/ui");
    open(&mut s, "crates/ui/src");
    s.rebuild();
    let p = picture(&s);
    let ui = p.index(code_map::Item::Node(node(&s, "crates/ui/src"))).unwrap();
    assert_eq!(p.placed[ui].chain, [node(&s, "crates/ui")]);
    assert_eq!(p.index(code_map::Item::Node(node(&s, "crates/ui"))), Some(ui));
    assert!(arrows_of(&s).contains(&"crates/ui/src/view.rs -> crates/core (1)".to_string()));
    assert!(arrows_of(&s).contains(&"crates/ui/src/lib.rs -> crates/core (1)".to_string()));
    // The cards shown are what the rest of the Map works from.
    assert!(s.visible.contains(&node(&s, "crates/ui/src/view.rs")));
    assert!(s.projected_links.iter().any(|e| e.kind == MapLinkKind::DependsOn && e.weight == 1));
}

#[test]
fn code_map_marks_arrows_that_point_up_and_keeps_many_own_files_on_one_card() {
    let mut files: Vec<String> = (0..12).map(|i| format!("lib/f{i}.rs")).collect();
    files.push("app/main.rs".into());
    let paths: Vec<&str> = files.iter().map(String::as_str).collect();
    let mut s = code_page(&paths, &[("app/main.rs", "lib/f0.rs"), ("lib/f1.rs", "app/main.rs"), ("lib/f2.rs", "app/main.rs")]);
    s.rebuild();
    let p = picture(&s);
    let up: Vec<bool> = p.arrows.iter().map(|a| a.up).collect();
    assert_eq!(p.arrows.len(), 2);
    assert_eq!(up.iter().filter(|u| **u).count(), 1, "the two folders use each other: one arrow points up");
    // Twelve files of their own share one card in an open folder; it opens to show them.
    open(&mut s, "lib");
    s.rebuild();
    let lib = Some(node(&s, "lib"));
    assert!(picture(&s).index(code_map::Item::Files(lib)).is_some());
    assert!(!s.visible.contains(&node(&s, "lib/f0.rs")));
    s.code.files_open.insert(b"lib".to_vec());
    s.rebuild();
    assert!(picture(&s).index(code_map::Item::Files(lib)).is_none());
    assert!(s.visible.contains(&node(&s, "lib/f0.rs")));
}

#[test]
fn code_map_connected_only_keeps_the_selection_what_it_connects_to_and_their_boxes() {
    let mut s = code_page(&CODE, &USES);
    open(&mut s, "crates");
    s.selected = Some(node(&s, "crates/core"));
    s.rebuild();
    s.neighbors();
    s.rebuild();
    let mut shown: Vec<String> = picture(&s)
        .placed
        .iter()
        .map(|p| match p.item {
            code_map::Item::Node(n) => String::from_utf8_lossy(s.data.as_ref().unwrap().nodes[n].path.as_deref().unwrap()).into_owned(),
            code_map::Item::Files(_) => "files".into(),
        })
        .collect();
    shown.sort();
    assert_eq!(shown, ["crates", "crates/core", "crates/ui"]);
}

#[test]
fn code_map_colours_by_kind_activity_owner_and_changes() {
    let mut s = code_page(&CODE, &USES);
    open(&mut s, "crates");
    s.rebuild();
    let ui = node(&s, "crates/ui");
    let at = |s: &MapPage, n: usize| picture(s).index(code_map::Item::Node(n)).unwrap();
    // Kind: all code, so one band in the code colour.
    let paint = picture(&s).placed[at(&s, ui)].paint.clone();
    assert_eq!(paint.band.len(), 1);
    assert_eq!(paint.band[0].0, MARK_1());
    assert_eq!(picture(&s).legend.kinds, [FileKind::Code, FileKind::Docs]);
    // Activity: commits in the period, shaded against the busiest card.
    let id = |n: u8| ObjectId::parse(&format!("{n:040x}")).unwrap();
    let now = 4_000_000_000;
    s.code.history = Some(Arc::new(CodeHistory {
        tip: None,
        commits: vec![
            HistoryCommit { id: id(1), time: now, author: 0 },
            HistoryCommit { id: id(2), time: now, author: 1 },
            HistoryCommit { id: id(3), time: now, author: 1 },
        ],
        authors: vec![b"Ann".to_vec(), b"Bo".to_vec()],
        paths: [
            (b"crates/ui/src/lib.rs".to_vec(), vec![0, 1]),
            (b"crates/ui/src/view.rs".to_vec(), vec![1]),
            (b"tools/a.js".to_vec(), vec![2]),
        ]
        .into_iter()
        .collect(),
        limited: false,
    }));
    s.code.colour = code_map::Colour::Activity;
    s.code.period = Period::All;
    s.rebuild();
    let p = picture(&s);
    assert_eq!(p.placed[at(&s, ui)].paint.sub, "2 commits");
    assert_eq!(p.legend.most, 2);
    assert_eq!(p.placed[at(&s, node(&s, "crates/core"))].paint.sub, "No commits");
    // Owner without CODEOWNERS: the person with the most commits to each file.
    s.code.colour = code_map::Colour::Owner;
    s.rebuild();
    let p = picture(&s);
    assert_eq!(p.placed[at(&s, ui)].paint.sub, "Bo");
    assert!(p.legend.owners_from.is_none());
    // Owner with CODEOWNERS: the rules, and files no rule covers belong to nobody.
    let mut links = (**s.code.links.as_ref().unwrap()).clone();
    links.owners = Some(OwnerRules {
        file: b".github/CODEOWNERS".to_vec(),
        gitlab: false,
        rules: 1,
        skipped: 0,
        sets: vec![vec!["@core-team".into()]],
        files: [(b"crates/core/src/lib.rs".to_vec(), Some(0)), (b"crates/core/src/store.rs".to_vec(), Some(0))].into_iter().collect(),
    });
    s.code.links = Some(Arc::new(links));
    s.rebuild();
    let p = picture(&s);
    assert_eq!(p.placed[at(&s, node(&s, "crates/core"))].paint.sub, "@core-team");
    assert_eq!(p.placed[at(&s, ui)].paint.sub, "Nobody");
    assert!(p.placed[at(&s, ui)].paint.dashed);
    assert_eq!(p.legend.owners_from.as_deref(), Some(".github/CODEOWNERS"));
    // Changes: letters for changed files, hatching one step away.
    let changed = |path: &str, change| ComparedFile { path: RepoPath::new(path.as_bytes().to_vec()).unwrap(), original_path: None, change, similarity: None };
    s.code.changes = Some(Arc::new(CodeChanges {
        outcome: ChangesOutcome::Ready,
        base: "main".into(),
        base_id: None,
        split: None,
        splits: Vec::new(),
        commits: 1,
        files: vec![changed("crates/core/src/store.rs", Change::Modified), changed("tools/b.js", Change::Added)],
        lines: [(b"crates/core/src/store.rs".to_vec(), (3, 1))].into_iter().collect(),
    }));
    s.code.colour = code_map::Colour::Changes;
    s.rebuild();
    let p = picture(&s);
    // view.rs uses store.rs; lib.rs uses the core package, which holds it.
    assert_eq!((p.legend.added, p.legend.changed, p.legend.uses), (1, 1, 2));
    assert_eq!(p.placed[at(&s, node(&s, "crates/core"))].paint.sub, "1 changed");
    // Changed only: the changed files, those that use them, and the boxes on the way.
    s.code.changed_only = true;
    s.rebuild();
    let mut shown: Vec<String> = picture(&s)
        .placed
        .iter()
        .filter_map(|p| match p.item {
            code_map::Item::Node(n) if s.data.as_ref().unwrap().nodes[n].kind == MapNodeKind::File => {
                Some(String::from_utf8_lossy(s.data.as_ref().unwrap().nodes[n].path.as_deref().unwrap()).into_owned())
            }
            _ => None,
        })
        .collect();
    shown.sort();
    assert_eq!(shown, ["crates/core/src/store.rs", "crates/ui/src/lib.rs", "crates/ui/src/view.rs", "tools/b.js"]);
    let store = picture(&s).index(code_map::Item::Node(node(&s, "crates/core/src/store.rs"))).unwrap();
    assert_eq!(picture(&s).placed[store].paint.badge, Some('M'));
    assert_eq!(picture(&s).placed[store].paint.sub, "+3 \u{2212}1");
}

#[test]
fn code_map_captions_follow_the_language_without_moving_cards() {
    use crate::i18n::{Language, scoped};
    let files: Vec<String> = (0..10).map(|i| format!("f{i}.md")).collect();
    let paths: Vec<&str> = files.iter().map(String::as_str).collect();
    let mut s = code_page(&paths, &[]);
    scoped(Language::English, || s.rebuild());
    let top = picture(&s).index(code_map::Item::Files(None)).unwrap();
    let (rect, english) = (picture(&s).placed[top].rect, picture(&s).placed[top].paint.sub.clone());
    assert_eq!(english, "in the top folder");
    for language in Language::ALL.into_iter().skip(1) {
        scoped(language, || s.rebuild());
        let p = picture(&s);
        assert_ne!(p.placed[top].paint.sub, english, "{language:?}");
        assert_eq!(p.placed[top].rect, rect, "{language:?}");
    }
}

#[test]
fn code_map_canvas_registers_its_cards_inside_the_map_and_none_when_covered() {
    let ctx = Context::default();
    crate::style::install(&ctx);
    let mut s = code_page(&CODE, &USES);
    open(&mut s, "crates");
    s.rebuild();
    for size in [vec2(640., 420.), vec2(1100., 700.)] {
        let rect = Rect::from_min_size(Pos2::ZERO, size);
        for covered in [false, true] {
            let mut controls = HashMap::new();
            let mut intents = Vec::new();
            s.fit = true;
            let mut output = ctx.run_ui(RawInput { screen_rect: Some(rect), ..Default::default() }, |ui| {
                code_map::canvas(ui, &mut s, rect, covered, &mut intents, &mut controls);
            });
            output.textures_delta.clear();
            for (id, c) in &controls {
                assert!(rect.contains_rect(c.rect), "{id}: {:?}", c.rect);
            }
            if covered {
                assert!(controls.keys().all(|k| !k.starts_with("map-node-")), "{:?}", controls.keys());
            } else {
                let crates = node(&s, "crates");
                assert!(controls.contains_key(&format!("map-node-{crates}")), "{:?}", controls.keys());
                assert!(controls.contains_key(&format!("map-toggle-{crates}")));
                assert!(controls.contains_key("map-fit"));
            }
        }
    }
}

/// Polls the code map until the read it started has finished.
fn finish_code_read(s: &mut MapPage) {
    let started = std::time::Instant::now();
    while !s.code.poll() {
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "the read never finished");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
fn code_tip() -> ObjectId {
    ObjectId::parse(&format!("{:040x}", 7)).unwrap()
}

#[test]
fn code_map_closes_an_open_box_from_its_header() {
    // The box's node index is past the cards placed: its header arrow once took the node
    // index for a placed one, and panicked or closed another card.
    let ctx = Context::default();
    crate::style::install(&ctx);
    let files = ["a.md", "b.md", "c.md", "d.md", "e.md", "f.md", "g.md", "h.md", "i.md", "zone/src/lib.rs"];
    let mut s = code_page(&files, &[]);
    open(&mut s, "zone");
    s.rebuild();
    let zone = node(&s, "zone");
    let at = picture(&s).index(code_map::Item::Node(zone)).unwrap();
    assert!(picture(&s).placed[at].open);
    assert!(zone >= picture(&s).placed.len(), "the fixture keeps the two indices apart");
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(1100., 700.));
    let mut controls = HashMap::new();
    let mut frame = |s: &mut MapPage, events: Vec<Event>| -> Option<Pos2> {
        controls.clear();
        let mut output = ctx.run_ui(RawInput { screen_rect: Some(rect), events, ..Default::default() }, |ui| {
            code_map::canvas(ui, s, rect, false, &mut Vec::new(), &mut controls);
        });
        output.textures_delta.clear();
        controls.get(&format!("map-toggle-{zone}")).map(|c| c.rect.center())
    };
    s.fit = true;
    let chevron = frame(&mut s, vec![]).expect("the box header has its arrow");
    frame(&mut s, vec![Event::PointerMoved(chevron), Event::PointerButton { pos: chevron, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
    frame(&mut s, vec![Event::PointerButton { pos: chevron, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
    let key = s.data.as_ref().unwrap().nodes[zone].key.clone();
    assert!(s.collapsed.contains(&key), "the box closed");
    assert_eq!(s.selected, Some(zone));
}

#[test]
fn code_map_stop_holds_until_refresh_and_a_folder_read_shows_its_arrows() {
    let ctx = Context::default();
    let tip = code_tip();
    let mut s = code_page(&CODE, &USES);
    s.code.renew(Some(&tip));
    // Stop: the read ends without a result and is not asked for again until Refresh.
    assert!(matches!(code_map::wanted(&s, &tip), Some(code_map::CodeRead::Links(None))));
    s.code.links_read_for_test(&ctx, Vec::new(), Err(rocket_git::Error::Cancelled));
    finish_code_read(&mut s);
    assert!(s.code.stopped && s.code.links.is_none() && s.code.error().is_none());
    assert!(code_map::wanted(&s, &tip).is_none(), "Stop means stop");
    s.code.renew(Some(&tip));
    assert!(!s.code.stopped);
    assert!(matches!(code_map::wanted(&s, &tip), Some(code_map::CodeRead::Links(None))), "Refresh reads again");
    // A folder read past the first read's limit joins what was read, and its arrows show.
    let tools = PackageRef { folder: b"tools".to_vec(), language: ModuleLanguage::JavaScript };
    let mut first = code_links(&USES[..3]);
    first.unread.push(tools.clone());
    first.limited = true;
    s.code.links = Some(Arc::new(first));
    s.rebuild();
    assert!(arrows_of(&s).is_empty(), "{:?}", arrows_of(&s));
    let mut more = code_links(&[USES[3]]);
    more.packages = vec![tools];
    s.code.links_read_for_test(&ctx, vec![b"tools".to_vec()], Ok(more));
    finish_code_read(&mut s);
    s.rebuild();
    assert_eq!(arrows_of(&s), ["tools -> crates (1)"]);
    let links = s.code.links.clone().unwrap();
    assert!(links.unread.is_empty() && !links.limited);
    assert!(s.code.too_large.is_empty());
}

#[test]
fn code_map_reads_unread_packages_once_their_folder_is_open_and_never_again_when_too_large() {
    let ctx = Context::default();
    let tip = code_tip();
    let mut s = code_page(&CODE, &USES);
    s.code.renew(Some(&tip));
    let mut links = code_links(&USES);
    links.unread = vec![PackageRef { folder: b"tools".to_vec(), language: ModuleLanguage::JavaScript }];
    s.code.links = Some(Arc::new(links));
    s.rebuild();
    assert!(code_map::wanted(&s, &tip).is_none(), "a closed folder is not read");
    // However it opens (its arrow, Expand all, Show folder), an open folder's code is read.
    open(&mut s, "tools");
    s.rebuild();
    assert!(matches!(code_map::wanted(&s, &tip), Some(code_map::CodeRead::Links(Some(ref l))) if l == &[b"tools".to_vec()]));
    // Still unread after a read of its own: more code than one read takes, not asked again.
    s.code.links_read_for_test(&ctx, vec![b"tools".to_vec()], Ok(CodeLinks::default()));
    finish_code_read(&mut s);
    s.rebuild();
    assert!(s.code.too_large.contains(b"tools".as_slice()));
    assert!(code_map::wanted(&s, &tip).is_none(), "no read again and again");
}

#[test]
fn code_map_filters_keep_a_chosen_files_card_and_hide_deep_files_inside_open_boxes() {
    // Connected only from a card that uses a folder's "N files" card keeps that card and
    // its box.
    let mut files: Vec<String> = (0..12).map(|k| format!("lib/f{k}.rs")).collect();
    files.push("app/main.rs".into());
    let files: Vec<&str> = files.iter().map(String::as_str).collect();
    let mut s = code_page(&files, &[("app/main.rs", "lib/f0.rs")]);
    open(&mut s, "lib");
    s.rebuild();
    let (app, lib) = (node(&s, "app"), node(&s, "lib"));
    let group = code_map::Item::Files(Some(lib));
    assert!(picture(&s).index(group).is_some(), "twelve files share one card");
    assert_eq!(arrows_of(&s), ["app -> files (1)"]);
    s.selected = Some(app);
    s.neighbor_anchor = Some(s.data.as_ref().unwrap().nodes[app].key.clone());
    s.filter_key = None;
    s.rebuild();
    let p = picture(&s);
    assert!(p.index(group).is_some(), "the card app uses stays");
    assert!(p.index(code_map::Item::Node(lib)).is_some_and(|i| p.placed[i].open), "with its box");
    assert_eq!(arrows_of(&s), ["app -> files (1)"]);
    // Changed only: an unchanged file two folders down inside an open box shows nowhere,
    // so nothing points at the box for it.
    let mut s = code_page(&["web/src/deep/a.ts", "web/src/b.ts", "api/c.ts"], &[("api/c.ts", "web/src/deep/a.ts")]);
    let changed = |path: &str| ComparedFile { path: RepoPath::new(path.as_bytes().to_vec()).unwrap(), original_path: None, change: Change::Modified, similarity: None };
    s.code.changes = Some(Arc::new(CodeChanges {
        outcome: ChangesOutcome::Ready,
        base: "main".into(),
        base_id: None,
        split: None,
        splits: Vec::new(),
        commits: 1,
        files: vec![changed("web/src/b.ts"), changed("api/c.ts")],
        lines: HashMap::new(),
    }));
    s.code.colour = code_map::Colour::Changes;
    s.code.changed_only = true;
    s.rebuild();
    assert!(picture(&s).index(code_map::Item::Node(node(&s, "web/src/b.ts"))).is_some());
    assert!(arrows_of(&s).is_empty(), "{:?}", arrows_of(&s));
}

#[test]
fn code_map_shades_against_cards_dashes_unowned_and_hatches_go_packages() {
    let id = |n: u8| ObjectId::parse(&format!("{n:040x}")).unwrap();
    // Go packages use each other as wholes, so their ends are folders.
    let mut s = code_page(&["pkg/a/a.go", "pkg/a/a2.go", "pkg/b/b.go", "README.md"], &[("pkg/a/", "pkg/b/")]);
    open(&mut s, "pkg");
    s.rebuild();
    assert_eq!(arrows_of(&s), ["pkg/a -> pkg/b (1)"]);
    let a = node(&s, "pkg/a");
    let at = |s: &MapPage| picture(s).index(code_map::Item::Node(a)).unwrap();
    // Activity: the pkg box counts both commits, each card one; cards set the scale.
    s.code.history = Some(Arc::new(CodeHistory {
        tip: None,
        commits: vec![HistoryCommit { id: id(1), time: 4_000_000_000, author: 0 }, HistoryCommit { id: id(2), time: 4_000_000_000, author: 0 }],
        authors: vec![b"Ann".to_vec()],
        paths: [(b"pkg/a/a.go".to_vec(), vec![0]), (b"pkg/b/b.go".to_vec(), vec![1])].into_iter().collect(),
        limited: false,
    }));
    s.code.colour = code_map::Colour::Activity;
    s.code.period = Period::All;
    s.rebuild();
    assert_eq!(picture(&s).legend.most, 1);
    // Owner: files a rule leaves without owners are dashed, as are files nobody owns.
    let mut links = (**s.code.links.as_ref().unwrap()).clone();
    links.owners = Some(OwnerRules {
        file: b"CODEOWNERS".to_vec(),
        gitlab: false,
        rules: 1,
        skipped: 2,
        sets: Vec::new(),
        files: [(b"pkg/a/a.go".to_vec(), None), (b"pkg/a/a2.go".to_vec(), None)].into_iter().collect(),
    });
    s.code.links = Some(Arc::new(links));
    s.code.colour = code_map::Colour::Owner;
    s.rebuild();
    assert_eq!(picture(&s).placed[at(&s)].paint.sub, "Unowned");
    assert!(picture(&s).placed[at(&s)].paint.dashed);
    assert_eq!(picture(&s).legend.owners_skipped, 2);
    // Changes: a Go package that uses a changed one uses it with each of its files.
    s.code.changes = Some(Arc::new(CodeChanges {
        outcome: ChangesOutcome::Ready,
        base: "main".into(),
        base_id: None,
        split: None,
        splits: Vec::new(),
        commits: 1,
        files: vec![ComparedFile { path: RepoPath::new(b"pkg/b/b.go".to_vec()).unwrap(), original_path: None, change: Change::Modified, similarity: None }],
        lines: HashMap::new(),
    }));
    s.code.colour = code_map::Colour::Changes;
    s.rebuild();
    assert_eq!(picture(&s).legend.uses, 2);
    assert_eq!(picture(&s).placed[at(&s)].paint.sub, "2 use a change");
}

#[test]
fn code_map_java_and_csharp_packages_use_a_change_with_each_of_their_files() {
    // Java packages and C# namespaces use each other as wholes too: their ends are folders.
    let mut s = code_page(&["jv/a/A.java", "jv/a/Util.kt", "jv/b/B.java", "cs/m/Model.cs", "cs/w/Web.cs"], &[("jv/a/", "jv/b/"), ("cs/w/", "cs/m/")]);
    open(&mut s, "jv");
    open(&mut s, "cs");
    s.rebuild();
    let changed = |path: &[u8]| ComparedFile { path: RepoPath::new(path.to_vec()).unwrap(), original_path: None, change: Change::Modified, similarity: None };
    s.code.changes = Some(Arc::new(CodeChanges {
        outcome: ChangesOutcome::Ready,
        base: "main".into(),
        base_id: None,
        split: None,
        splits: Vec::new(),
        commits: 1,
        files: vec![changed(b"jv/b/B.java"), changed(b"cs/m/Model.cs")],
        lines: HashMap::new(),
    }));
    s.code.colour = code_map::Colour::Changes;
    s.rebuild();
    // A.java and Util.kt use B's package; Web.cs uses the Model namespace.
    assert_eq!(picture(&s).legend.uses, 3);
    let a = node(&s, "jv/a");
    let at = picture(&s).index(code_map::Item::Node(a)).unwrap();
    assert_eq!(picture(&s).placed[at].paint.sub, "2 use a change");
}

#[test]
fn code_map_search_and_show_folder_reach_files_inside_closed_folders() {
    let mut s = code_page(&["应用/src/結果 🚀 é.rs", "应用/src/main.rs", "other/x.rs", "Cargo.toml"], &[]);
    // Cargo reads a Cargo.toml at the top as its package; on the map it stays a file card.
    let manifest = node(&s, "Cargo.toml");
    let mut data = (**s.data.as_ref().unwrap()).clone();
    data.nodes[manifest].kind = MapNodeKind::Crate;
    data.nodes[manifest].label = b"root-lib".to_vec();
    s.data = Some(Arc::new(data));
    s.rebuild();
    let p = picture(&s);
    let at = p.index(code_map::Item::Node(manifest)).expect("a card for Cargo.toml");
    assert!(!p.placed[at].open);
    assert_eq!(code_map::title(s.data.as_ref().unwrap(), s.code.facts.as_ref().unwrap(), code_map::Item::Node(manifest)).2, "Cargo.toml");
    // Search finds a file inside a closed folder and chooses the card that shows it.
    s.query = "結果 🚀".into();
    s.rebuild();
    assert_eq!(s.selected, Some(node(&s, "应用")));
    // A file chosen without a card of its own (from Details) selects the card showing it.
    s.query.clear();
    s.selected = Some(node(&s, "other/x.rs"));
    s.filter_key = None;
    s.rebuild();
    assert_eq!(s.selected, Some(node(&s, "other")));
    // Show folder from elsewhere opens the folders on the way, and the folder.
    s.open_path("应用/src".as_bytes());
    s.rebuild();
    let p = picture(&s);
    assert!(p.index(code_map::Item::Node(node(&s, "应用"))).is_some_and(|i| p.placed[i].open));
    assert_eq!(s.selected, Some(node(&s, "应用/src")));
}

#[test]
fn export_is_a_worded_button_that_says_why_it_is_greyed_out() {
    /// One frame of the page: what it registered and the text it painted, with where.
    fn page(ctx: &Context, tab: &mut Tab, events: Vec<Event>, time: f64) -> (HashMap<String, UiControl>, Vec<(String, Rect)>) {
        fn texts(shape: &Shape, out: &mut Vec<(String, Rect)>) {
            match shape {
                Shape::Text(text) => out.push((text.galley.text().to_owned(), text.visual_bounding_rect())),
                Shape::Vec(shapes) => shapes.iter().for_each(|shape| texts(shape, out)),
                _ => {}
            }
        }
        let mut controls = HashMap::new();
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1480., 900.));
        let mut output = ctx.run_ui(RawInput { screen_rect: Some(screen), events, time: Some(time), ..Default::default() }, |ui| {
            view(ui, tab, &mut Vec::new(), &mut controls)
        });
        output.textures_delta.clear();
        let mut painted = Vec::new();
        for clipped in &output.shapes {
            texts(&clipped.shape, &mut painted);
        }
        (controls, painted)
    }
    let ctx = Context::default();
    crate::style::install(&ctx);
    ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.);
    // A map on screen: a button with its word, not only an icon.
    let mut tab = Tab::new(std::path::PathBuf::from("synthetic"), None);
    tab.map.kind = MapKind::History;
    tab.map.page_mut().data = Some(Arc::new(history(&[&[1], &[2], &[]])));
    let mut frame = page(&ctx, &mut tab, vec![], 0.);
    for time in [0.1, 0.2] {
        frame = page(&ctx, &mut tab, vec![], time);
    }
    let export = frame.0["map-export"];
    assert!(export.enabled);
    let word = tr("Export");
    assert!(frame.1.iter().any(|(text, rect)| text == word && export.rect.contains_rect(*rect)), "{word:?} is not on {:?}: {:?}", export.rect, frame.1);
    // Nothing read yet: greyed out, and pointing at it says why.
    let mut empty = Tab::new(std::path::PathBuf::from("synthetic"), None);
    let (controls, _) = page(&ctx, &mut empty, vec![], 1.);
    let export = controls["map-export"];
    assert!(!export.enabled);
    let mut painted = page(&ctx, &mut empty, vec![Event::PointerMoved(export.rect.center())], 1.1).1;
    for time in [1.6, 2.1, 2.6] {
        painted = page(&ctx, &mut empty, vec![], time).1;
    }
    let why = tr("Export as Mermaid, once the map shows something");
    assert!(painted.iter().any(|(text, _)| text == why), "no {why:?} in {painted:?}");
}
