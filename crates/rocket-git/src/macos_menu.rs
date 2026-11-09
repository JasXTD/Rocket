//! Safe main-thread adapter for Cocoa menus; commands are queued for UI dispatch.
use std::{
    cell::RefCell,
    ffi::{CString, c_char},
    sync::{Arc, Mutex},
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: u32,
    pub group: u32,
    /// Zero for a top-level row; otherwise an earlier row in the same group.
    pub parent: u32,
    pub title: String,
    pub key: String,
    pub modifiers: u32,
    pub enabled: bool,
    pub checked: bool,
}
#[repr(C)]
struct NativeItem {
    id: u32,
    group: u32,
    parent: u32,
    title: *const c_char,
    key: *const c_char,
    modifiers: u32,
    enabled: bool,
    checked: bool,
}
unsafe extern "C" {
    fn rocket_menu_sync(items: *const NativeItem, count: usize, repaint: extern "C" fn()) -> i32;
    fn rocket_menu_pop() -> u32;
    fn rocket_menu_invoke(id: u32) -> bool;
}
type Wake = Arc<dyn Fn() + Send + Sync>;
static WAKE: Mutex<Option<Wake>> = Mutex::new(None);
thread_local! { static CURRENT: RefCell<Vec<Item>> = const { RefCell::new(Vec::new()) }; }
extern "C" fn repaint() {
    let wake = WAKE.lock().ok().and_then(|w| w.clone());
    if let Some(wake) = wake {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| wake()));
    }
}
pub fn synchronize(items: &[Item], wake: impl Fn() + Send + Sync + 'static) -> bool {
    if items.is_empty()
        || items.len() > 256
        || items
            .iter()
            .any(|i| i.id == 0 || i.group > 7 || i.title.len() > 512 || i.key.len() > 16)
    {
        return false;
    }
    let mut seen = std::collections::BTreeMap::new();
    for item in items {
        if seen.contains_key(&item.id)
            || (item.parent != 0 && seen.get(&item.parent) != Some(&item.group))
        {
            return false;
        }
        seen.insert(item.id, item.group);
    }
    if let Ok(mut w) = WAKE.lock() {
        *w = Some(Arc::new(wake));
    }
    CURRENT.with(|current| {
        if current.borrow().as_slice() == items {
            return true;
        }
        let text: Result<Vec<_>, _> = items
            .iter()
            .map(|i| {
                Ok((
                    CString::new(i.title.as_str())?,
                    CString::new(i.key.as_str())?,
                ))
            })
            .collect::<Result<_, std::ffi::NulError>>();
        let Ok(text) = text else { return false };
        let native: Vec<_> = items
            .iter()
            .zip(&text)
            .map(|(i, (title, key))| NativeItem {
                id: i.id,
                group: i.group,
                parent: i.parent,
                title: title.as_ptr(),
                key: key.as_ptr(),
                modifiers: i.modifiers,
                enabled: i.enabled,
                checked: i.checked,
            })
            .collect();
        // Cocoa copies all strings and retains only its own menu objects.
        if unsafe { rocket_menu_sync(native.as_ptr(), native.len(), repaint) } != 0 {
            return false;
        }
        *current.borrow_mut() = items.to_vec();
        true
    })
}
pub fn next_action() -> Option<u32> {
    match unsafe { rocket_menu_pop() } {
        0 => None,
        id => Some(id),
    }
}
/// Uses the actual NSMenu item's enabled state; never runs application code itself.
pub fn invoke(id: u32) -> bool {
    unsafe { rocket_menu_invoke(id) }
}
/// The last successfully installed menu, not a second set of synthetic controls.
pub fn snapshot() -> Vec<Item> {
    CURRENT.with(|current| current.borrow().clone())
}
