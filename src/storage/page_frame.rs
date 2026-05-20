use std::sync::{Arc, Mutex};

use crate::storage::Page;

/// 缓存中的页帧，包含元数据
pub struct PageFrame {
    pub page: Page,
    pub dirty: bool,
    pub ref_count: u32,
    pub clock_bit: bool,
}

impl PageFrame {
    pub fn new(page: Page) -> Self {
        Self {
            page,
            dirty: false,
            ref_count: 0,
            clock_bit: true,
        }
    }
}

/// 页访问守卫
pub struct PageGuard {
    frame: Arc<Mutex<PageFrame>>,
}

impl PageGuard {
    pub fn new(frame: Arc<Mutex<PageFrame>>) -> Self {
        {
            let mut f = frame.lock().unwrap();
            f.ref_count += 1;
            f.clock_bit = true;
        }
        Self { frame }
    }

    pub fn mark_dirty(&self) {
        self.frame.lock().unwrap().dirty = true;
    }

    pub fn ref_count(&self) -> u32 {
        self.frame.lock().unwrap().ref_count
    }

    pub fn page(&self) -> Page {
        self.frame.lock().unwrap().page.clone()
    }
}

impl Drop for PageGuard {
    fn drop(&mut self) {
        self.frame.lock().unwrap().ref_count -= 1;
    }
}
