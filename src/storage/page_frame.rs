use std::ops::Deref;
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

/// Zero-copy page data accessor. Holds the MutexGuard so the borrow
/// stays valid while the caller reads the data.
pub struct PageDataGuard<'a> {
    _guard: std::sync::MutexGuard<'a, PageFrame>,
}

impl Deref for PageDataGuard<'_> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self._guard.page.data[..]
    }
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

    /// Clone the entire page (4KB allocation). Prefer `page_data()` for reads.
    pub fn page(&self) -> Page {
        self.frame.lock().unwrap().page.clone()
    }

    /// Zero-copy access to page data bytes.
    ///
    /// Returns a guard that derefs to `&[u8]` (4096 bytes).
    /// The MutexGuard is held for the lifetime of the returned guard,
    /// so no other thread can modify the page while reading.
    ///
    /// SAFETY: This uses std::sync::Mutex which must NOT be held across .await.
    /// The returned PageDataGuard is not Send and cannot be held across await points.
    pub fn page_data(&self) -> PageDataGuard<'_> {
        PageDataGuard {
            _guard: self.frame.lock().unwrap(),
        }
    }

    /// Get mutable access to page data and automatically mark dirty
    /// The closure receives a mutable reference to the page
    pub fn modify_page<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Page) -> R,
    {
        let mut frame = self.frame.lock().unwrap();
        frame.dirty = true;
        f(&mut frame.page)
    }
}

impl Drop for PageGuard {
    fn drop(&mut self) {
        self.frame.lock().unwrap().ref_count -= 1;
    }
}
