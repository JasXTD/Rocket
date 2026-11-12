//! One texture budget per egui context, shared by tabs and native review windows.
//! Weak entries never keep a closed preview alive. Eviction drops texture owners;
//! an evicted view offers Reload instead of repeatedly decoding against the budget.
use super::*;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicU64, Ordering},
};

const BUDGET_BYTES: usize = 192 * 1024 * 1024;
static TOUCH: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
pub(super) struct Cached {
    pub handles: [Option<TextureHandle>; 3],
    used: u64,
}
impl Cached {
    fn bytes(&self) -> usize {
        self.handles
            .iter()
            .flatten()
            .map(|t| t.size()[0] * t.size()[1] * 4)
            .sum()
    }
    pub fn touch(&mut self) {
        self.used = TOUCH.fetch_add(1, Ordering::Relaxed);
    }
}
struct Budget {
    limit: usize,
    entries: Vec<Weak<Mutex<Cached>>>,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            limit: BUDGET_BYTES,
            entries: Vec::new(),
        }
    }
}
impl Budget {
    fn admit(&mut self, target: &Arc<Mutex<Cached>>, bytes: usize) -> bool {
        if bytes > self.limit {
            return false;
        }
        self.entries.retain(|entry| entry.strong_count() > 0);
        let mut others = Vec::new();
        for entry in self.entries.iter().filter_map(Weak::upgrade) {
            if !Arc::ptr_eq(&entry, target) {
                let data = entry.lock().unwrap();
                others.push((data.used, data.bytes(), entry.clone()));
            }
        }
        others.sort_by_key(|(used, _, _)| *used);
        let mut total = bytes + others.iter().map(|(_, bytes, _)| bytes).sum::<usize>();
        for (_, size, entry) in others {
            if total <= self.limit {
                break;
            }
            entry.lock().unwrap().handles = Default::default();
            total -= size;
        }
        if !self
            .entries
            .iter()
            .any(|e| e.ptr_eq(&Arc::downgrade(target)))
        {
            self.entries.push(Arc::downgrade(target));
        }
        target.lock().unwrap().touch();
        true
    }
}
pub(super) fn admit(ctx: &Context, target: &Arc<Mutex<Cached>>, bytes: usize) -> bool {
    let budget = ctx.data_mut(|data| {
        let id = Id::unique("rocket-image-texture-budget");
        if let Some(budget) = data.get_temp::<Arc<Mutex<Budget>>>(id) {
            budget
        } else {
            let budget = Arc::new(Mutex::new(Budget::default()));
            data.insert_temp(id, budget.clone());
            budget
        }
    });
    budget.lock().unwrap().admit(target, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn evicts_least_recent_texture_and_closed_views_release_ownership() {
        let ctx = Context::default();
        let mut budget = Budget {
            limit: 32,
            entries: vec![],
        };
        let make = |budget: &mut Budget| {
            let entry = Arc::new(Mutex::new(Cached::default()));
            assert!(budget.admit(&entry, 16));
            entry.lock().unwrap().handles[0] = Some(ctx.load_texture(
                "fixture",
                ColorImage::filled([2, 2], Color32::WHITE),
                TextureOptions::NEAREST,
            ));
            entry
        };
        let first = make(&mut budget);
        let second = make(&mut budget);
        let second_id = second.lock().unwrap().handles[0].as_ref().unwrap().id();
        first.lock().unwrap().touch();
        let third = make(&mut budget);
        assert!(first.lock().unwrap().handles[0].is_some());
        assert!(second.lock().unwrap().handles[0].is_none());
        assert!(ctx.tex_manager().read().meta(second_id).is_none());
        let third_id = third.lock().unwrap().handles[0].as_ref().unwrap().id();
        drop(third);
        assert!(ctx.tex_manager().read().meta(third_id).is_none());
        assert!(!budget.admit(&second, 33));
        assert!(first.lock().unwrap().handles[0].is_some());
    }
}
