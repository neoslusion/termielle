//! Bounded per-render-thread glyph coverage cache; position and ink are not raster inputs.
use std::{cell::RefCell, collections::VecDeque, rc::Rc};

#[derive(Clone, PartialEq, Eq)]
pub(super) struct TextKey {
    pub text: String,
    pub width: u32,
    pub height: u32,
    pub font_size: i32,
    pub bold: bool,
    pub centered: bool,
}

pub(super) type Coverage = Vec<(u32, u32, u8)>;
type Entries = VecDeque<(TextKey, Rc<Coverage>)>;
thread_local! {
    static CACHE: RefCell<Entries> = const { RefCell::new(VecDeque::new()) };
}

pub(super) fn get(key: &TextKey) -> Option<Rc<Coverage>> {
    CACHE.with(|cache| {
        cache
            .borrow()
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, mask)| mask.clone())
    })
}

pub(super) fn insert(key: TextKey, mask: Coverage) -> Rc<Coverage> {
    let mask = Rc::new(mask);
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() == 128 {
            cache.pop_back();
        }
        cache.push_front((key, mask.clone()));
    });
    mask
}
