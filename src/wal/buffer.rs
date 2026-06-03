//! WAL 内存缓冲 + Group Commit 策略
//!
//! 将 WAL 记录先写入内存缓冲，满足以下条件之一时批量刷盘：
//! - 缓冲区满（capacity 条记录）
//! - commit 请求（append_commit_and_wait 触发）
//! - 定时器到期（flush_interval_ms）
//! - shutdown 时强制刷盘

use super::record::{WalError, WalRecord};
use super::writer::WalWriter;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use tokio::time;

/// WAL 内存缓冲 + Group Commit
pub struct WALBuffer {
    /// (lsn, record) 缓冲队列
    buffer: Mutex<Vec<(u64, WalRecord)>>,
    /// LSN 分配器（单调递增）
    current_lsn: AtomicU64,
    /// 持久化写入器
    wal_writer: Arc<WalWriter>,
    /// 通知后台 task 刷盘
    flush_notify: Notify,
    /// 等待 flush 确认的 tx_id 列表
    pending_commits: Mutex<Vec<u64>>,
    /// tx_id -> 等待通知（commit 完成后唤醒）
    commit_waiters: Mutex<HashMap<u64, Arc<Notify>>>,
    /// 关闭标志
    shutdown: AtomicBool,
    /// 缓冲区容量
    capacity: usize,
    /// 定时刷盘间隔（毫秒）
    flush_interval_ms: u64,
    /// 后台 task handle（使用 std::sync::Mutex，因为 start_flush_loop 在 tokio runtime 内调用）
    flush_handle: std::sync::Mutex<Option<JoinHandle<()>>>,
}

impl WALBuffer {
    /// 创建 WALBuffer
    pub fn new(wal_writer: Arc<WalWriter>, capacity: usize, flush_interval_ms: u64) -> Self {
        Self {
            buffer: Mutex::new(Vec::new()),
            current_lsn: AtomicU64::new(1),
            wal_writer,
            flush_notify: Notify::new(),
            pending_commits: Mutex::new(Vec::new()),
            commit_waiters: Mutex::new(HashMap::new()),
            shutdown: AtomicBool::new(false),
            capacity,
            flush_interval_ms,
            flush_handle: std::sync::Mutex::new(None),
        }
    }

    /// 追加 WAL 记录到内存缓冲，返回分配的 LSN
    ///
    /// 如果缓冲区满（达到 capacity），立即触发 do_flush
    pub async fn append(&self, record: WalRecord) -> u64 {
        let lsn = self.current_lsn.fetch_add(1, Ordering::SeqCst);

        let should_flush = {
            let mut buf = self.buffer.lock().await;
            buf.push((lsn, record));
            buf.len() >= self.capacity
        };

        if should_flush {
            self.do_flush().await;
        }

        lsn
    }

    /// 注册 commit 等待 + 通知后台刷盘
    ///
    /// 调用者应先 append CommitTxn/Commit 记录，再调用此方法等待持久化确认。
    /// 多个并发事务的 commit 会合并到同一次 fsync（Group Commit）。
    pub async fn append_commit_and_wait(&self, tx_id: u64) -> Result<(), WalError> {
        // 1. Register a waiter for this tx_id
        let waiter = {
            let mut waiters = self.commit_waiters.lock().await;
            let notify = Arc::new(Notify::new());
            waiters.insert(tx_id, notify.clone());
            notify
        };

        // 2. Add to pending commits list
        {
            let mut pending = self.pending_commits.lock().await;
            pending.push(tx_id);
        }

        // 3. Signal the flush loop to wake up and flush
        self.flush_notify.notify_one();

        // 4. Wait until do_flush notifies us
        waiter.notified().await;

        Ok(())
    }

    /// 启动后台刷盘 task
    pub fn start_flush_loop(self: &Arc<Self>) {
        let this_for_spawn = self.clone();
        let handle = tokio::spawn(async move {
            this_for_spawn.flush_loop().await;
        });
        // Store the handle using std::sync::Mutex (safe because lock is held briefly)
        {
            let mut guard = self.flush_handle.lock().unwrap();
            *guard = Some(handle);
        }
    }

    /// 后台刷盘循环
    async fn flush_loop(&self) {
        let interval = time::Duration::from_millis(self.flush_interval_ms);

        loop {
            tokio::select! {
                // Wait for flush notification (commit or capacity trigger)
                _ = self.flush_notify.notified() => {
                    if self.shutdown.load(Ordering::SeqCst) {
                        break;
                    }
                    self.do_flush().await;
                }
                // Periodic timer flush
                _ = time::sleep(interval) => {
                    if self.shutdown.load(Ordering::SeqCst) {
                        break;
                    }
                    self.do_flush().await;
                }
            }
        }
    }

    /// 执行一次刷盘：
    /// 1. take 缓冲区所有记录
    /// 2. 计算基于文件偏移量的 LSN → write_batch + fsync 到磁盘
    /// 3. 通知等待中的 commit 事务
    pub async fn do_flush(&self) {
        // 1. Take all records from the buffer
        let records: Vec<(u64, WalRecord)>;
        let committed_tx_ids: Vec<u64>;

        {
            let mut buf = self.buffer.lock().await;
            records = std::mem::take(&mut *buf);

            let mut pending = self.pending_commits.lock().await;
            committed_tx_ids = std::mem::take(&mut *pending);
        }

        // 2. Write to disk if there are records
        if !records.is_empty() {
            // Compute file-offset-based LSN for each record so WalReader can parse them
            let base_offset = self.wal_writer.get_current_lsn().await.unwrap_or(0);

            let mut file_offset_lsn_records = Vec::with_capacity(records.len());
            let mut offset = base_offset;
            for (_logical_lsn, record) in records {
                let serialized_len = record.serialize_with_lsn(offset).len() as u64;
                file_offset_lsn_records.push((offset, record));
                offset += serialized_len;
            }

            if self
                .wal_writer
                .write_batch(file_offset_lsn_records)
                .await
                .is_err()
            {
                // Log error but don't panic - WAL write failures are critical
                // but we still need to notify waiters to avoid hanging
            }
        }

        // 3. Notify all waiting commit transactions
        if !committed_tx_ids.is_empty() {
            let mut waiters = self.commit_waiters.lock().await;
            for tx_id in &committed_tx_ids {
                if let Some(notify) = waiters.remove(tx_id) {
                    notify.notify_one();
                }
            }
        }
    }

    /// 关闭：刷盘所有缓冲 + 停止后台 task
    pub async fn shutdown(&self) {
        // 1. Set shutdown flag
        self.shutdown.store(true, Ordering::SeqCst);

        // 2. Flush any remaining buffered records
        self.do_flush().await;

        // 3. Notify the flush loop to exit
        self.flush_notify.notify_one();

        // 4. Wait for the background task to finish
        let handle = {
            let mut guard = self.flush_handle.lock().unwrap();
            guard.take()
        };

        if let Some(handle) = handle {
            let _ = handle.await;
        }
    }
}

impl std::fmt::Debug for WALBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WALBuffer")
            .field("capacity", &self.capacity)
            .field("flush_interval_ms", &self.flush_interval_ms)
            .field("shutdown", &self.shutdown.load(Ordering::SeqCst))
            .finish()
    }
}
