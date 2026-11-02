//! Read completions wake the viewport that started them. Periodic polling stays
//! in place for disconnected workers and animations.
use eframe::egui::Context;
use rocket_git::{CancellationToken, ReadTask, Result};

pub(crate) fn spawn<T: Send + 'static>(
    ctx: &Context,
    read: impl FnOnce(CancellationToken) -> Result<T> + Send + 'static,
) -> Result<ReadTask<T>> {
    let viewport = ctx.viewport_id();
    let context = ctx.clone();
    ReadTask::spawn_with_wake(read, move || context.request_repaint_of(viewport))
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{RawInput, Ui, ViewportId, ViewportInfo};
    use std::{sync::mpsc, time::Duration};

    fn frame(ctx: &Context, input: RawInput, ui: impl FnMut(&mut Ui)) {
        let mut output = ctx.run_ui(input, ui);
        output.textures_delta.clear();
    }

    #[test]
    fn completed_read_requests_a_repaint_with_the_result_ready() {
        let ctx = Context::default();
        for _ in 0..3 {
            frame(&ctx, RawInput::default(), |_| {});
        }
        let (repaint, notification) = mpsc::channel();
        ctx.set_request_repaint_callback(move |info| {
            let _ = repaint.send(info.viewport_id);
        });
        let mut task = spawn(&ctx, |_| Ok(42)).unwrap();
        assert_eq!(
            notification.recv_timeout(Duration::from_secs(5)).unwrap(),
            ViewportId::ROOT
        );
        assert_eq!(task.try_finish().unwrap().unwrap(), 42);
        assert!(task.try_finish().is_none());
    }

    #[test]
    fn completion_wakes_its_original_viewport_after_another_viewport_renders() {
        let ctx = Context::default();
        let auxiliary = ViewportId::from_hash_of("loading-test");
        let mut auxiliary_input = RawInput {
            viewport_id: auxiliary,
            ..Default::default()
        };
        auxiliary_input.viewports.insert(
            auxiliary,
            ViewportInfo {
                parent: Some(ViewportId::ROOT),
                ..Default::default()
            },
        );
        for _ in 0..3 {
            frame(&ctx, auxiliary_input.clone(), |_| {});
        }
        let (release, gate) = mpsc::channel();
        let mut task = None;
        let mut gate = Some(gate);
        frame(&ctx, auxiliary_input.clone(), |ui| {
            if let Some(gate) = gate.take() {
                task = Some(
                    spawn(ui.ctx(), move |_| {
                        gate.recv().unwrap();
                        Ok(7)
                    })
                    .unwrap(),
                );
            }
        });
        let root_input = RawInput {
            viewport_id: ViewportId::ROOT,
            ..auxiliary_input
        };
        for _ in 0..3 {
            frame(&ctx, root_input.clone(), |_| {});
        }
        let (repaint, notification) = mpsc::channel();
        ctx.set_request_repaint_callback(move |info| {
            let _ = repaint.send(info.viewport_id);
        });
        release.send(()).unwrap();
        assert_eq!(
            notification.recv_timeout(Duration::from_secs(5)).unwrap(),
            auxiliary
        );
        assert_eq!(task.unwrap().try_finish().unwrap().unwrap(), 7);
    }
}
