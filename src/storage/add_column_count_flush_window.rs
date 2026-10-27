//! `ALTER TABLE … ADD COLUMN` 在列数刷盘返回成功之后、决定提交之前的暂停点。
//!
//! 这个窗口在普通执行里不可观察：刷盘已经成功，活布局还没换入。本模块按库路径
//! 提供一次消费、默认不生效的注入点。未布防时直接返回，不改变加列、失败补偿
//! 或删列。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// 已布防的暂停点，按数据库路径索引。
static ARMED: Mutex<Option<HashMap<PathBuf, Arc<AddColumnCountFlushGate>>>> = Mutex::new(None);

/// 列数刷盘之后的暂停句柄。测试用它观察语句是否已停住，并放行后续决定。
///
/// 句柄只有放行信号和可观察锚点，不携带运行身份。
pub struct AddColumnCountFlushGate {
    reached: AtomicBool,
    release: Notify,
}

impl AddColumnCountFlushGate {
    /// 语句是否已停在列数刷盘成功之后、提交决定之前。
    pub fn reached(&self) -> bool {
        self.reached.load(Ordering::SeqCst)
    }

    /// 放行停在窗口内的语句。
    pub fn release(&self) {
        self.release.notify_one();
    }

    async fn park(&self) {
        self.reached.store(true, Ordering::SeqCst);
        self.release.notified().await;
    }
}

/// 布防一次暂停（按路径、一次消费、默认不生效）。
///
/// 返回的句柄供测试观察和放行。只有该库下一次加列会在列数刷盘成功后消费它。
pub fn arm_add_column_count_flush_window(path: &Path) -> Arc<AddColumnCountFlushGate> {
    let gate = Arc::new(AddColumnCountFlushGate {
        reached: AtomicBool::new(false),
        release: Notify::new(),
    });
    let mut guard = ARMED.lock().unwrap();
    guard
        .get_or_insert_with(HashMap::new)
        .insert(path.to_path_buf(), gate.clone());
    gate
}

/// 消费该库的暂停点。有则停到被放行；无则立即返回。
pub(crate) async fn pause_at_add_column_count_flush_window(path: &Path) {
    let gate = {
        let mut guard = ARMED.lock().unwrap();
        match guard.as_mut() {
            Some(armed) => armed.remove(path),
            None => None,
        }
    };
    if let Some(gate) = gate {
        gate.park().await;
    }
}
