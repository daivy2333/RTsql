//! change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
//! Iteration 003（T10，ISS25）：`ALTER TABLE … DROP COLUMN` 的「提交完成 →
//! 目录应用」窗口的**受控暂停点**（测试支撑）。
//!
//! ISS25 的错读窗口只存在于这一个窗口里：提交之后数据面已经是窄化后的行，
//! 而目录与活布局仍是旧布局 ⇒ 按位置解码会静默错读。窗口在真实执行路径上
//! 不可观测，因此本模块提供一处**路径限定 + 一次消费 + 默认不生效**的注入
//! 点：布防后，该库下一次 `DROP COLUMN` 语句在提交点之后、目录窄化之前停住，
//! 直到测试放行。
//!
//! 路径限定让并行测试二进制互不抢对方的注入点（先例：
//! `src/storage/free_page_rebuild.rs`）；一次消费让注入只影响一条语句；未布防
//! 时本模块对执行路径零影响（`take` 命中为空即直接返回）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// 已布防的暂停点，按数据库路径索引。
static ARMED: Mutex<Option<HashMap<PathBuf, Arc<CommitWindowGate>>>> = Mutex::new(None);

/// 窗口暂停点的句柄——测试用它观察「窗口已被到达」并放行语句。
///
/// 句柄本身不携带任何运行身份（无 run-id / 指纹 / revision）：它只是一个
/// 放行信号与一个可观察的布尔锚点。
pub struct CommitWindowGate {
    /// 语句是否已停在窗口内（可观察锚点）。
    reached: AtomicBool,
    /// 放行信号。`notify_one` 在无等待者时保留一个许可 ⇒ 放行先于语句到达
    /// 也不会把语句永久卡在窗口里。
    release: Notify,
}

impl CommitWindowGate {
    /// 语句是否已停在「提交完成 → 目录应用」窗口内。
    pub fn reached(&self) -> bool {
        self.reached.load(Ordering::SeqCst)
    }

    /// 放行停在窗口内的语句。
    pub fn release(&self) {
        self.release.notify_one();
    }

    /// 停在窗口内直到被放行。
    async fn park(&self) {
        self.reached.store(true, Ordering::SeqCst);
        self.release.notified().await;
    }
}

/// 布防一次窗口暂停（路径限定 + 一次消费 + 默认不生效）。
///
/// 返回的句柄供测试观察窗口是否被到达并放行语句；布防本身不改变任何产品
/// 行为，只有该库下一次 `DROP COLUMN` 语句会消费它。
pub fn arm_ddl_commit_window(path: &Path) -> Arc<CommitWindowGate> {
    let gate = Arc::new(CommitWindowGate {
        reached: AtomicBool::new(false),
        release: Notify::new(),
    });
    let mut guard = ARMED.lock().unwrap();
    guard
        .get_or_insert_with(HashMap::new)
        .insert(path.to_path_buf(), gate.clone());
    gate
}

/// 消费该库的暂停点（一次消费）——有则停在窗口内直到被放行；无则立即返回。
pub(crate) async fn pause_at_ddl_commit_window(path: &Path) {
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
