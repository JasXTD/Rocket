//! About and Release notes: dialogs with nothing to decide. The × (or Esc) closes
//! them. Release notes opens from Help; keyboard shortcuts are a Preferences page.
use crate::{
    app::*,
    pages::{self, control},
    style::*,
};
use eframe::egui::*;
use std::collections::HashMap;

impl Rocket {
    /// About and Release notes.
    pub(crate) fn app_dialog(&mut self, ctx: &Context) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        let mut next = None;
        let response = match dialog.kind {
            DialogKind::About => about(ctx, &mut self.frame_widgets, &mut next),
            DialogKind::Legal => crate::legal::dialog(ctx, &mut self.frame_widgets),
            DialogKind::Changelog => release_notes(ctx, &mut self.frame_widgets),
            _ => {
                self.dialog = Some(dialog);
                return;
            }
        };
        // Automation closed these with their Done button.
        if let Some(close) = self.frame_widgets.get("dialog-close").copied() {
            self.frame_widgets.insert("dialog-submit".into(), close);
        }
        if let Some(kind) = next {
            self.show_dialog(kind);
        } else if !response.should_close() {
            self.dialog = Some(dialog);
        }
    }
}

/// About: what this is, its creator, version and license, and the legal notices.
fn about(ctx: &Context, widgets: &mut HashMap<String, UiControl>, next: &mut Option<DialogKind>) -> pages::ReviewResponse {
    pages::plain(ctx, "rocket-about", 460.0, widgets, |ui, widgets| {
        let version = env!("CARGO_PKG_VERSION");
        ui.vertical_centered(|ui| {
            let (tile, _) = ui.allocate_exact_size(vec2(60.0, 60.0), Sense::hover());
            ui.painter().rect_filled(tile, 14.0, Palette::NAVY.deep);
            crate::app_icon::paint_mark(ui.painter(), tile, None);
            ui.add_space(4.0);
            ui.label(RichText::new("Rocket").font(semibold(20.0)));
            ui.label(RichText::new(crate::i18n::tr("Created by Xetro")).color(TEXT_SOFT()));
            ui.label(RichText::new(crate::i18n::trf!("Version {version} · a native Rust Git workspace", version)).color(MUTED()));
            ui.add_space(8.0);
            ui.label(
                RichText::new(crate::i18n::trf!(
                    "Free and open-source software under the {license} license.",
                    license = crate::legal::license()
                ))
                .color(TEXT_SOFT()),
            );
            ui.label(RichText::new(format!("© {}", crate::legal::copyright())).size(12.0).color(HINT()));
            ui.add_space(10.0);
            let r = ui.add(Button::new(crate::i18n::tr("Legal")).min_size(vec2(84.0, 30.0)));
            control(widgets, "dialog-about-legal", &r, ui.clip_rect());
            if r.clicked() {
                crate::legal::reset(ui.ctx());
                *next = Some(DialogKind::Legal);
            }
        });
    })
}

/// One version of the release notes.
struct Version {
    number: String,
    title: String,
    /// (heading, paragraphs); bullets keep their reflowed text.
    sections: Vec<(Option<String>, Vec<String>)>,
}

/// The release notes as versions, newest first. Wrapped bullet lines are joined so
/// the dialog can wrap them to its width.
fn versions() -> Vec<Version> {
    parse_versions(include_str!("../../../CHANGELOG.md"))
}

fn parse_versions(changelog: &str) -> Vec<Version> {
    let mut versions: Vec<Version> = Vec::new();
    let mut open = false;
    let mut released = false;
    for line in changelog.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            released = heading.starts_with("0.");
            open = false;
            if !released { continue; }
            let (number, title) = heading.split_once(" — ").unwrap_or((heading, ""));
            versions.push(Version {
                number: number.trim().to_owned(),
                title: title.trim().to_owned(),
                sections: vec![(None, Vec::new())],
            });
            continue;
        }
        if !released { continue; }
        let Some(version) = versions.last_mut() else {
            continue;
        };
        if let Some(heading) = line.strip_prefix("### ") {
            version.sections.push((Some(heading.trim().to_owned()), Vec::new()));
            open = false;
        } else if let Some(text) = line.strip_prefix("- ") {
            if let Some((_, items)) = version.sections.last_mut() {
                items.push(text.trim().to_owned());
                open = true;
            }
        } else if line.trim().is_empty() {
            open = false;
        } else if let Some((_, items)) = version.sections.last_mut() {
            match items.last_mut() {
                Some(last) if open => {
                    last.push(' ');
                    last.push_str(line.trim());
                }
                _ => {
                    items.push(line.trim().to_owned());
                    open = true;
                }
            }
        }
    }
    versions
}

/// Release notes: the versions on the left, the chosen one's notes on the right.
fn release_notes(ctx: &Context, widgets: &mut HashMap<String, UiControl>) -> pages::ReviewResponse {
    let head = pages::ReviewHeader {
        image: Icon::File,
        title: crate::i18n::tr("Release notes"),
        subtitle: crate::i18n::tr("What changed in each Rocket build."),
        danger: false,
    };
    let versions = versions();
    let current = env!("CARGO_PKG_VERSION");
    let height = (ctx.content_rect().height() - 300.0).clamp(220.0, 520.0);
    pages::wide_info(ctx, "rocket-dialog", head, widgets, "dialog-scroll", |ui, part, widgets| {
        if part != pages::Part::Body || versions.is_empty() {
            return;
        }
        let chosen_id = Id::unique("release-notes-version");
        let mut chosen = ui.data(|d| d.get_temp::<usize>(chosen_id)).unwrap_or(0).min(versions.len() - 1);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            ui.vertical(|ui| {
                ui.set_width(210.0);
                let list = ScrollArea::vertical()
                    .id_salt("release-notes-list")
                    .max_height(height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for (i, v) in versions.iter().enumerate() {
                            let r = version_row(ui, v, i == chosen, v.number == current);
                            control(widgets, format!("dialog-release-notes-{i}"), &r, ui.clip_rect());
                            if r.clicked() {
                                chosen = i;
                            }
                        }
                    });
                pages::region(widgets, "dialog-release-notes-list-scroll", list.inner_rect.intersect(ui.clip_rect()), true);
            });
            let (line, _) = ui.allocate_exact_size(vec2(1.0, height), Sense::hover());
            ui.painter().vline(line.center().x, line.y_range(), Stroke::new(1.0, HAIRLINE()));
            ui.vertical(|ui| {
                let v = &versions[chosen];
                let notes = ScrollArea::vertical()
                    .id_salt(("release-notes", chosen))
                    .max_height(height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            RichText::new(if v.number == current { crate::i18n::trf!("{version} · this build", version = v.number) } else { v.number.clone() })
                                .size(12.0)
                                .color(HINT()),
                        );
                        ui.add(Label::new(RichText::new(&v.title).font(semibold(18.0))).wrap());
                        ui.add_space(6.0);
                        for (heading, items) in &v.sections {
                            if let Some(heading) = heading {
                                ui.add_space(4.0);
                                ui.label(RichText::new(heading).size(12.0).color(MUTED()).strong());
                            }
                            for item in items {
                                ui.horizontal_top(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    let (dot, _) = ui.allocate_exact_size(vec2(6.0, 18.0), Sense::hover());
                                    ui.painter().circle_filled(pos2(dot.center().x, dot.top() + 9.0), 2.5, HINT());
                                    ui.add(Label::new(RichText::new(item).color(TEXT_ITEM())).wrap());
                                });
                                ui.add_space(3.0);
                            }
                        }
                    });
                pages::region(widgets, "dialog-release-notes-scroll", notes.inner_rect.intersect(ui.clip_rect()), true);
            });
        });
        ui.data_mut(|d| d.insert_temp(chosen_id, chosen));
    })
}

/// A version in the release-notes list: its number (and "This build"), then its title.
fn version_row(ui: &mut Ui, v: &Version, selected: bool, current: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 7.0, SELECTED());
    } else if response.hovered() || response.has_focus() {
        painter.rect_filled(rect, 7.0, HOVER());
    }
    let number = painter.layout_no_wrap(v.number.clone(), semibold(13.0), TEXT());
    let number_width = number.size().x;
    painter.galley(pos2(rect.left() + 10.0, rect.top() + 6.0), number, TEXT());
    if current {
        let pill = painter.layout_no_wrap(crate::i18n::tr("This build").to_owned(), FontId::proportional(11.0), BLUE());
        let pill_rect = Rect::from_min_size(
            pos2(rect.left() + 18.0 + number_width, rect.top() + 6.0),
            vec2(pill.size().x + 12.0, 17.0),
        );
        painter.rect_filled(pill_rect, 8.5, BLUE().gamma_multiply(0.14));
        painter.galley(pos2(pill_rect.left() + 6.0, pill_rect.center().y - pill.size().y / 2.0), pill, BLUE());
    }
    let mut job = text::LayoutJob::simple_singleline(v.title.clone(), FontId::proportional(12.0), HINT());
    job.wrap = text::TextWrapping::truncate_at_width(rect.width() - 20.0);
    let title = painter.layout_job(job);
    painter.galley(pos2(rect.left() + 10.0, rect.bottom() - 6.0 - title.size().y), title, HINT());
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, &v.number));
    response
}

#[cfg(test)]
mod tests {
    #[test]
    fn unreleased_sections_between_versions_do_not_become_release_records() {
        let versions = super::parse_versions("## Unreleased — Future\n- Future change\n\n## 0.9.0 — Current\n- A wrapped\n  release bullet\n\n## Unreleased — Another task\n- Not a release\n\n## 0.8.0 — Previous\n- Previous change\n");
        assert_eq!(versions.iter().map(|v| v.number.as_str()).collect::<Vec<_>>(), ["0.9.0", "0.8.0"]);
        assert_eq!(versions[0].sections[0].1, ["A wrapped release bullet"]);
        assert_eq!(versions[1].sections[0].1, ["Previous change"]);
    }

    #[test]
    fn release_notes_join_wrapped_lines_into_whole_bullets() {
        let versions = super::versions();
        assert!(versions.len() > 10);
        for v in &versions {
            assert!(v.number.starts_with("0."), "{}", v.number);
            assert!(!v.title.is_empty(), "{} has a title", v.number);
            for (_, items) in &v.sections {
                for item in items {
                    assert!(!item.contains('\n') && !item.starts_with(' '), "{item}");
                }
            }
        }
        // The newest version's first bullet continues past its first source line.
        let first = &versions[0].sections[0].1[0];
        assert!(first.len() > 90, "{first}");
    }
}
