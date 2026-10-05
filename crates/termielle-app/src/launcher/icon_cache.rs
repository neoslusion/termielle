use crate::animation::FrameBuffer;
use std::collections::VecDeque;
use std::sync::Arc;

pub(super) const MAX_CACHED_ICONS: usize = 32;

#[derive(Default)]
pub(super) struct IconCache {
    entries: VecDeque<(Vec<u8>, Option<Arc<FrameBuffer>>)>,
}

impl IconCache {
    pub fn get(
        &mut self,
        target: &[u8],
        load: impl FnOnce() -> Option<FrameBuffer>,
    ) -> Option<Arc<FrameBuffer>> {
        if let Some(index) = self.entries.iter().position(|(key, _)| key == target) {
            let entry = self.entries.remove(index).unwrap();
            let icon = entry.1.clone();
            self.entries.push_back(entry);
            return icon;
        }
        let icon = load().map(Arc::new);
        self.entries.push_back((target.to_vec(), icon.clone()));
        if self.entries.len() > MAX_CACHED_ICONS {
            self.entries.pop_front();
        }
        icon
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> FrameBuffer {
        FrameBuffer {
            width: 1,
            height: 1,
            pixels_pbgra: vec![255; 4],
            delay_ms: 0,
            loop_index: 0,
            scale: 1.0,
        }
    }

    #[test]
    fn hits_share_the_bitmap_and_negative_results_are_cached() {
        let mut cache = IconCache::default();
        let first = cache.get(&[1], || Some(frame())).unwrap();
        let second = cache
            .get(&[1], || panic!("cached artwork must not be decoded again"))
            .unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert!(cache.get(&[2], || None).is_none());
        assert!(
            cache
                .get(&[2], || panic!("failed extraction must not spin"))
                .is_none()
        );
    }

    #[test]
    fn cache_evicts_least_recently_used_icons_and_releases_them_on_refresh() {
        let mut cache = IconCache::default();
        let first = cache.get(&[0], || Some(frame())).unwrap();
        let weak = Arc::downgrade(&first);
        drop(first);
        for index in 1..MAX_CACHED_ICONS {
            cache.get(&[index as u8], || None);
        }
        cache.get(&[0], || panic!("the recent entry is still cached"));
        cache.get(&[MAX_CACHED_ICONS as u8], || None);
        assert_eq!(cache.entries.len(), MAX_CACHED_ICONS);
        assert!(!cache.entries.iter().any(|(target, _)| target == &[1]));
        assert!(weak.upgrade().is_some());
        cache.clear();
        assert!(weak.upgrade().is_none());
    }
}
