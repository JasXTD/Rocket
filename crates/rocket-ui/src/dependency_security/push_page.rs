//! Presentation of an existing, single-attempt push decision. No scan policy lives here.
use super::{
    push::{Phase, TipReport},
    scan::Finding,
};
use crate::{
    app::{Rocket, UiControl},
    pages,
    style::*,
};
use eframe::egui::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};

type PackageKey<'a> = (&'a str, &'a str, &'a str);
type PackageGroups<'a> = BTreeMap<PackageKey<'a>, Vec<&'a Finding>>;

fn groups(findings: &[Finding]) -> PackageGroups<'_> {
    let mut groups = BTreeMap::new();
    for finding in findings {
        groups
            .entry((
                finding.ecosystem.as_str(),
                finding.package.as_str(),
                finding.version.as_str(),
            ))
            .or_insert_with(Vec::new)
            .push(finding);
    }
    groups
}
fn severity(value: &str) -> (u8, &'static str, Color32) {
    match value.to_ascii_lowercase().as_str() {
        "critical" => (5, "Critical", RED()),
        "high" => (4, "High", RED()),
        "medium" | "moderate" => (3, "Moderate", AMBER()),
        "low" => (2, "Low", MUTED()),
        _ => (1, "Unknown", MUTED()),
    }
}
fn fix_summary(findings: &[&Finding]) -> String {
    let Some(first) = findings.first() else {
        return String::new();
    };
    if findings.iter().all(|f| f.fixed.is_empty()) {
        "No fixed version reported".into()
    } else if findings.iter().any(|f| f.fixed.is_empty()) {
        "Some findings have no reported fix · Expand details".into()
    } else if findings.iter().all(|f| f.fixed == first.fixed) {
        format!("Reported fix: {}", first.fixed.join(", "))
    } else {
        "Reported fixes vary by advisory · Expand details".into()
    }
}
fn help(ui: &mut Ui, text: impl Into<String>) {
    ui.add(Label::new(RichText::new(text.into()).size(12.0).color(MUTED())).wrap());
}
fn badge(ui: &mut Ui, text: &str, color: Color32) {
    ui.label(
        RichText::new(text)
            .font(semibold(11.0))
            .color(color)
            .background_color(color.gamma_multiply(0.12)),
    );
}
fn panel(ui: &mut Ui, content: impl FnOnce(&mut Ui)) {
    Frame::new()
        .fill(CARD())
        .stroke(Stroke::new(1.0, CARD_STROKE()))
        .corner_radius(8)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
}
fn step(ui: &mut Ui, title: &str, state: &str, description: &str, tint: Color32, active: bool) {
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
        ui.painter()
            .circle_filled(rect.center(), 14.0, tint.gamma_multiply(0.14));
        if active {
            ui.put(rect.shrink(5.0), Spinner::new().color(tint));
        } else {
            icon(
                ui.painter(),
                rect.shrink(6.0),
                if state == "Passed" {
                    Icon::Check
                } else {
                    Icon::Push
                },
                tint,
            );
        }
        let width = ui.available_width();
        ui.vertical(|ui| {
            ui.set_width(width);
            let body = |ui: &mut Ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(title).font(semibold(13.0)));
                    badge(ui, state, tint);
                });
                help(ui, description);
                if active {
                    ui.add_space(6.0);
                    ui.separator();
                    help(
                        ui,
                        format!(
                            "OSV-Scanner {} · Local advisory data",
                            rocket_git::OSV_VERSION
                        ),
                    );
                }
            };
            if active {
                panel(ui, body);
            } else {
                body(ui);
            }
        });
    });
}
fn scanning(ui: &mut Ui, secrets: bool, ai_keys: bool) {
    step(
        ui,
        "Secret protection",
        if secrets { "Passed" } else { "Off" },
        if secrets {
            "No secrets detected by the enabled checks."
        } else {
            "Secret checks are disabled in Preferences."
        },
        if secrets { GREEN() } else { AMBER() },
        false,
    );
    ui.add_space(12.0);
    step(ui, "Guard AI API keys", if ai_keys { "Passed" } else { "Off" },
        if ai_keys { "No supported AI API-key formats detected." } else { "AI key format checks are disabled in Preferences." },
        if ai_keys { GREEN() } else { AMBER() }, false);
    ui.add_space(12.0);
    step(
        ui,
        "Dependency scan",
        "In progress",
        "Comparing committed package versions with local advisories.",
        BLUE(),
        true,
    );
    ui.add_space(12.0);
    step(
        ui,
        "Push to repository",
        "Waiting",
        "The push has not started.",
        MUTED(),
        false,
    );
    ui.add_space(16.0);
    help(ui, "Clear scan · The push continues automatically.");
    help(ui, "Warnings found · Review them before deciding to push.");
}
fn findings(ui: &mut Ui, reports: &[TipReport], widgets: &mut HashMap<String, UiControl>) {
    let count: usize = reports.iter().map(|r| r.report.findings.len()).sum();
    let packages: BTreeSet<_> = reports
        .iter()
        .flat_map(|r| r.report.findings.iter())
        .map(|f| (&f.ecosystem, &f.package, &f.version))
        .collect();
    ui.label(
        RichText::new(if count == 0 {
            "Dependency coverage needs review".into()
        } else {
            format!(
                "{count} {} in {} package {}",
                if count == 1 { "finding" } else { "findings" },
                packages.len(),
                if packages.len() == 1 {
                    "version"
                } else {
                    "versions"
                }
            )
        })
        .font(semibold(14.0)),
    );
    if count > 0 {
        let mut counts = BTreeMap::new();
        for finding in reports.iter().flat_map(|r| r.report.findings.iter()) {
            *counts
                .entry(severity(&finding.severity).0)
                .or_insert(0usize) += 1;
        }
        ui.horizontal_wrapped(|ui| {
            for (rank, count) in counts.iter().rev() {
                let (_, name, tint) = severity(match rank {
                    5 => "critical",
                    4 => "high",
                    3 => "moderate",
                    2 => "low",
                    _ => "unknown",
                });
                badge(ui, &format!("{count} {}", name.to_lowercase()), tint);
            }
        });
    }
    for (tip_index, tip) in reports.iter().enumerate() {
        ui.add_space(10.0);
        if reports.len() > 1 {
            help(ui, format!("Pushed tip {}", &tip.source.as_str()[..12]));
        }
        for note in &tip.report.notes {
            pages::callout(ui, AMBER(), note);
        }
        for (group_index, ((ecosystem, package, version), entries)) in
            groups(&tip.report.findings).into_iter().enumerate()
        {
            panel(ui, |ui| {
                let (_, name, tint) = entries
                    .iter()
                    .map(|f| severity(&f.severity))
                    .max_by_key(|s| s.0)
                    .unwrap();
                badge(ui, name, tint);
                // CollapsingHeader forces Extend even when the UI wraps. A custom
                // wrapping label keeps untrusted long package names inside the dialog.
                let id = ui.make_persistent_id(("dependency-push-package", tip_index, group_index));
                let mut clicked = false;
                let mut header =
                    collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
                        .show_header(ui, |ui| {
                            let label = ui.add(
                                Label::new(
                                    RichText::new(format!("{package} {version}"))
                                        .font(semibold(13.0)),
                                )
                                .wrap()
                                .sense(Sense::click()),
                            );
                            clicked = label.clicked();
                            label
                        });
                if clicked {
                    header.toggle();
                }
                let (_, header, _) = header.body(|ui| {
                    for (index, finding) in entries.iter().enumerate() {
                        if index > 0 {
                            ui.separator();
                        }
                        let (_, name, tint) = severity(&finding.severity);
                        badge(ui, name, tint);
                        ui.add(Label::new(&finding.summary).wrap());
                        help(ui, format!("{ecosystem} · {}", finding.file));
                        help(
                            ui,
                            if finding.fixed.is_empty() {
                                "No fixed version reported".into()
                            } else {
                                format!("Reported fix: {}", finding.fixed.join(", "))
                            },
                        );
                        ui.horizontal_wrapped(|ui| {
                            for id in &finding.ids {
                                ui.hyperlink_to(id, format!("https://osv.dev/vulnerability/{id}"));
                            }
                        });
                    }
                });
                pages::control(
                    widgets,
                    format!("dependency-push-package-{tip_index}-{group_index}"),
                    &header.inner,
                    ui.clip_rect(),
                );
                help(
                    ui,
                    format!(
                        "{} {} · {ecosystem}",
                        entries.len(),
                        if entries.len() == 1 {
                            "finding"
                        } else {
                            "findings"
                        }
                    ),
                );
                help(ui, fix_summary(&entries));
            });
            ui.add_space(8.0);
        }
    }
    let detail = CollapsingHeader::new("Scan details").id_salt("dependency-push-scope").show(ui, |ui| {
        for tip in reports { help(ui, format!("Tip {} · {} packages · {} files", &tip.source.as_str()[..12], tip.report.packages, tip.report.files)); }
        help(ui, format!("OSV-Scanner {} · Local advisory data", rocket_git::OSV_VERSION));
        help(ui, "Working-copy edits are excluded. Advisory matches do not establish whether vulnerable code is reachable; historical dependency versions are not checked.");
    });
    pages::control(
        widgets,
        "dependency-push-details",
        &detail.header_response,
        ui.clip_rect(),
    );
}

impl Rocket {
    pub(crate) fn dependency_push_review_ui(&mut self, ctx: &Context) {
        let Some(active) = &self.dependencies.push_review.active else {
            return;
        };
        let mut cancel = false;
        let mut approve = false;
        let mut setup = false;
        let reviewing = matches!(active.phase, Phase::Review { .. });
        let failed = matches!(active.phase, Phase::Failed(_));
        let (phase, title, subtitle, tint) = match &active.phase {
            Phase::Scanning => (
                "scanning",
                "Checking dependencies before push",
                "Rocket is checking the dependencies in your committed files.",
                BLUE(),
            ),
            Phase::Continuing => (
                "continuing",
                "Continuing the reviewed push",
                "Finishing checks for this push attempt.",
                BLUE(),
            ),
            Phase::Review { .. } => (
                "review",
                "Review before pushing",
                "Dependency warnings need your attention. Your push is paused.",
                AMBER(),
            ),
            Phase::Failed(_) => (
                "failed",
                "Push stopped",
                "The dependency scan could not finish.",
                RED(),
            ),
        };
        // Use the owning operation, never the selected tab or a guessed branch.
        let operation = self.write.as_ref().and_then(|w| {
            w.owner
                .and_then(|owner| self.tabs.iter().find(|t| t.id == owner))
                .filter(|t| t.path == active.repository)
                .map(|_| {
                    w.title.strip_prefix("Pushing to ").map_or_else(
                        || crate::operation_names::done(&w.label),
                        |remote| format!("Push to {remote}"),
                    )
                })
        });
        let secrets = self.secret_protection;
        let ai_keys = self.guard_ai_api_keys;
        let editable = self.dependencies.editable;
        let widgets = &mut self.frame_widgets;
        // Phase-specific sizing avoids inheriting loading bounds in a warning view.
        let response = Modal::new(Id::new(("dependency-push-review", active.id, phase)))
            .frame(Frame::new().fill(PANEL()).stroke(Stroke::new(1.0, CARD_STROKE())).corner_radius(12).inner_margin(22))
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 100.0).clamp(280.0, 610.0));
                ui.style_mut().wrap_mode = Some(TextWrapMode::Wrap);
                ui.spacing_mut().item_spacing = vec2(10.0, 6.0);
                let top = ui.cursor().top();
                ui.horizontal_top(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
                    ui.painter().rect_filled(rect, 9, tint.gamma_multiply(0.12));
                    icon(ui.painter(), rect.shrink(9.0), if failed { Icon::Warning } else { Icon::Shield }, tint);
                    let width = (ui.available_width() - 36.0).max(100.0);
                    ui.vertical(|ui| { ui.set_width(width); ui.add(Label::new(RichText::new(title).font(semibold(20.0))).wrap()); help(ui, subtitle); });
                    let close = ui.add_sized(vec2(26.0, 28.0), Button::new("×").frame(false)).on_hover_text(if failed { "Close" } else { "Cancel push" });
                    pages::control(widgets, "dependency-push-close", &close, ui.clip_rect()); cancel = close.clicked();
                });
                ui.add_space(12.0);
                panel(ui, |ui| { ui.horizontal_wrapped(|ui| {
                    ui.add(Label::new(RichText::new(active.repository.file_name().unwrap_or(active.repository.as_os_str()).to_string_lossy()).font(semibold(12.0))).wrap());
                    if let Some(operation) = &operation { help(ui, operation); }
                }); });
                ui.add_space(14.0);
                // Reserve the footer before laying out arbitrarily long advisory text.
                let height = (ctx.content_rect().height() - (ui.cursor().top() - top) - 154.0).clamp(90.0, 410.0);
                let scroll = ScrollArea::vertical().id_salt(("dependency-push-results", active.id, phase)).max_height(height).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    match &active.phase {
                        Phase::Scanning => scanning(ui, secrets, ai_keys),
                        Phase::Continuing => { ui.horizontal(|ui| { ui.spinner(); help(ui, "Continuing the reviewed push…"); }); }
                        Phase::Failed(error) => { pages::callout(ui, RED(), error); ui.add_space(10.0); help(ui, "No push was sent by this attempt. Check scanner setup, then retry Push. Your local commits are retained."); }
                        Phase::Review { reports, .. } => { ui.push_id(active.id, |ui| findings(ui, reports, widgets)); }
                    }
                });
                pages::region(widgets, "dependency-push-scroll", scroll.inner_rect, true);
                ui.add_space(12.0); ui.separator(); ui.add_space(6.0);
                help(ui, if reviewing { "Push anyway keeps these warnings and continues this push only." } else if failed { "Fix the setup and retry. This error cannot be bypassed here." } else { "Cancelling stops this attempt and keeps your local commits." });
                ui.add_space(5.0);
                ui.allocate_ui_with_layout(vec2(ui.available_width(), 36.0), Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().button_padding = vec2(13.0, 8.0);
                    if failed { let button = pages::primary(ui, editable, "Open scanner setup"); pages::control(widgets, "dependency-push-setup", &button, ui.clip_rect()); setup = button.clicked(); }
                    let button = if reviewing { pages::primary(ui, true, "Cancel push") } else { ui.button(if failed { "Close" } else { "Cancel push" }) };
                    pages::control(widgets, "dependency-push-cancel", &button, ui.clip_rect());
                    let focus = Id::new(("dependency-push-focus", active.id, phase));
                    if !ui.ctx().data(|d| d.get_temp::<bool>(focus).unwrap_or(false)) { button.request_focus(); ui.ctx().data_mut(|d| d.insert_temp(focus, true)); }
                    cancel |= button.clicked();
                    if reviewing { let button = ui.add_enabled(editable, Button::new(RichText::new("Push anyway").color(AMBER()))); pages::control(widgets, "dependency-push-continue", &button, ui.clip_rect()); approve = button.clicked(); }
                });
            });
        cancel |= response.should_close();
        if cancel || setup {
            self.dependencies.push_review.cancel();
        } else if approve
            && let Some(active) = &mut self.dependencies.push_review.active
            && let Phase::Review { decision, .. } = &active.phase
        {
            if decision.try_send(()).is_err() {
                active.cancel.cancel();
            }
            active.phase = Phase::Continuing;
        }
        if setup {
            self.show_preferences(crate::preferences::Page::Security);
            self.dependencies.setup_page = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn grouping_keeps_ecosystems_versions_and_per_advisory_fixes_distinct() {
        let first = Finding {
            package: "example".into(),
            version: "1.0".into(),
            ecosystem: "npm".into(),
            file: "a/package-lock.json".into(),
            ids: vec!["DEMO-1".into()],
            severity: "High".into(),
            summary: "Fixture".into(),
            fixed: vec!["2.0".into()],
        };
        let mut second = first.clone();
        second.file = "b/package-lock.json".into();
        second.fixed = vec!["3.0".into()];
        let mut other = first.clone();
        other.ecosystem = "crates.io".into();
        let findings = vec![first, second, other];
        let grouped = groups(&findings);
        assert_eq!(grouped.len(), 2);
        let npm = &grouped[&("npm", "example", "1.0")];
        assert_eq!(npm.len(), 2);
        assert!(fix_summary(npm).contains("vary by advisory"));
        assert_eq!(fix_summary(&[&findings[0]]), "Reported fix: 2.0");
        let mut no_fix = findings[0].clone();
        no_fix.fixed.clear();
        assert!(fix_summary(&[&no_fix]).starts_with("No fixed"));
        assert!(fix_summary(&[&no_fix, &findings[0]]).starts_with("Some findings"));
        assert_eq!(severity("MODERATE").0, 3);
        assert_eq!(severity("unknown").0, 1);
    }
}
