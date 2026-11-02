use super::*;

impl State {
    pub(super) fn composite(
        &mut self,
        ui: &mut Ui,
        selection: &ImageComparison,
        widgets: &mut HashMap<String, UiControl>,
    ) {
        ui.add(
            Label::new(format!(
                "Before: {}  ·  After: {}",
                selection.before_label(),
                selection.after_label()
            ))
            .truncate(),
        );
        let dimensions = |i: Option<&Info>| {
            i.map_or_else(
                || "absent".into(),
                |i| format!("{} × {}", i.width, i.height),
            )
        };
        ui.small(format!(
            "{} → {} · aligned at top left",
            dimensions(self.before.as_ref()),
            dimensions(self.after.as_ref())
        ));
        if matches!(self.mode, Mode::Overlay | Mode::Swipe) {
            ui.horizontal_wrapped(|ui| {
                let r = ui
                    .button("−")
                    .on_hover_text("Decrease after opacity / move divider left");
                widgets.insert("image-mix-less".into(), (&r).into());
                if r.clicked() {
                    self.mix = (self.mix - 0.1).max(0.0);
                }
                let r = ui.add(
                    Slider::new(&mut self.mix, 0.0..=1.0)
                        .text(if self.mode == Mode::Overlay {
                            "After opacity"
                        } else {
                            "Before fraction"
                        })
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                );
                widgets.insert("image-mix".into(), (&r).into());
                let r = ui
                    .button("+")
                    .on_hover_text("Increase after opacity / move divider right");
                widgets.insert("image-mix-more".into(), (&r).into());
                if r.clicked() {
                    self.mix = (self.mix + 0.1).min(1.0);
                }
            });
        } else {
            if let Some(e) = &self.difference_error {
                ui.colored_label(AMBER(), e);
                return;
            }
            if let Some(s) = &self.stats {
                ui.small(format!(
                    "{} / {} pixels changed ({:.2}%) · magenta = RGBA differs",
                    s.changed,
                    s.compared,
                    100.0 * s.changed as f64 / s.compared.max(1) as f64
                ));
                if let Some([x, y, w, h]) = s.bounds {
                    ui.small(format!("Changed bounds: x {x}, y {y}, {w} × {h}"));
                }
            }
        }
        let width = self
            .before
            .iter()
            .chain(self.after.iter())
            .map(|i| i.width)
            .max()
            .unwrap_or(1) as f32;
        let height = self
            .before
            .iter()
            .chain(self.after.iter())
            .map(|i| i.height)
            .max()
            .unwrap_or(1) as f32;
        let available = vec2(
            ui.available_width().max(1.0),
            (ui.available_height() - 20.0).max(24.0),
        );
        let scale = if self.fit {
            (available.x / width).min(available.y / height).min(1.0)
        } else {
            self.zoom
        };
        let size = vec2(width, height) * scale;
        let scroll = ScrollArea::both()
            .id_salt(("image-composite", self.key))
            .max_height(available.y)
            .min_scrolled_height(0.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let canvas = available.max(size);
                let (rect, _) = ui.allocate_exact_size(canvas, Sense::hover());
                let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
                painter.rect_filled(rect, 0, IMAGE_BG());
                let image_rect = Rect::from_center_size(rect.center(), size);
                if self.checker {
                    checkerboard(&painter, image_rect);
                }
                let textures = self.textures();
                let draw = |side: usize, painter: &Painter, alpha: u8| {
                    if let Some(t) = &textures[side] {
                        let s = t.size_vec2() * scale;
                        painter.image(
                            t.id(),
                            Rect::from_min_size(image_rect.min, s),
                            Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                            Color32::from_white_alpha(alpha),
                        );
                    }
                };
                match self.mode {
                    Mode::Overlay => {
                        draw(0, &painter, 255);
                        draw(1, &painter, (self.mix * 255.0).round() as u8);
                    }
                    Mode::Swipe => {
                        let x = image_rect.left() + image_rect.width() * self.mix;
                        let left = Rect::from_min_max(image_rect.min, pos2(x, image_rect.bottom()));
                        let right = Rect::from_min_max(pos2(x, image_rect.top()), image_rect.max);
                        draw(
                            0,
                            &painter.with_clip_rect(painter.clip_rect().intersect(left)),
                            255,
                        );
                        draw(
                            1,
                            &painter.with_clip_rect(painter.clip_rect().intersect(right)),
                            255,
                        );
                        painter.line_segment(
                            [pos2(x, image_rect.top()), pos2(x, image_rect.bottom())],
                            Stroke::new(2.0, BLUE()),
                        );
                    }
                    Mode::Pixels => draw(2, &painter, 255),
                    Mode::Side => (),
                }
            });
        let rect = scroll.inner_rect.intersect(ui.clip_rect());
        if rect.is_positive() {
            widgets.insert(
                "image-composite".into(),
                UiControl {
                    rect,
                    enabled: true,
                },
            );
        }
    }
}
