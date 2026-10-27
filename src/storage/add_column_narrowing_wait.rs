//! 同表 `ADD COLUMN` 在写目录之前的暂停点。
//!
//! 在途窄化窗口里，加列任务会停在这里，直到测试放行。按库路径索引，默认不生效。
//! 第一次暂停不从 map 里摘掉；测试用 `disarm_add_column_narrowing_wait` 摘掉。
//! 未布防时 `pause_at_add_column_narrowing_wait` 立即返回 `false`，不加次数。
//! 句柄只有放行信号和进入次数，不携带运行身份。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// 已布防的暂停点，按数据库路径索引。
static ARMED: Mutex<Option<HashMap<PathBuf, Arc<AddColumnNarrowingWaitGate>>>> = Mutex::new(None);

/// 加列等待暂停点的句柄。测试用它观察进入次数并放行。
pub struct AddColumnNarrowingWaitGate {
    /// 进入等待的次数。在开始等 `release` 之前加一。
    reached: AtomicUsize,
    /// 放行信号。没有等待者时保留一个许可，避免 `release` 早于 `await` 把任务停死。
    release: Notify,
}

impl AddColumnNarrowingWaitGate {
    /// 已经进入等待的次数。
    pub fn reached_count(&self) -> usize {
        self.reached.load(Ordering::SeqCst)
    }

    /// 放行当前停在这个句柄上的一次等待。
    pub fn release(&self) {
        self.release.notify_one();
    }

    async fn park(&self) -> bool {
        self.reached.fetch_add(1, Ordering::SeqCst);
        self.release.notified().await;
        true
    }
}

/// 布防加列等待暂停点。默认不改变产品行为，直到加列路径调用暂停函数。
pub fn arm_add_column_narrowing_wait(path: &Path) -> Arc<AddColumnNarrowingWaitGate> {
    let gate = Arc::new(AddColumnNarrowingWaitGate {
        reached: AtomicUsize::new(0),
        release: Notify::new(),
    });
    let mut guard = ARMED.lock().unwrap();
    guard
        .get_or_insert_with(HashMap::new)
        .insert(path.to_path_buf(), gate.clone());
    gate
}

/// 把该路径从 map 里摘掉。之后的暂停立即返回，不再加次数。
pub fn disarm_add_column_narrowing_wait(path: &Path) {
    let mut guard = ARMED.lock().unwrap();
    if let Some(armed) = guard.as_mut() {
        armed.remove(path);
    }
}

/// 该路径已布防且尚未摘掉时停到被放行，并返回 `true`。否则立即返回 `false`。
pub(crate) async fn pause_at_add_column_narrowing_wait(path: &Path) -> bool {
    let gate = {
        let guard = ARMED.lock().unwrap();
        guard.as_ref().and_then(|armed| armed.get(path).cloned())
    };
    match gate {
        Some(gate) => gate.park().await,
        None => false,
    }
}
