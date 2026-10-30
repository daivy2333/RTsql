//! WAL 内存缓冲 + Group Commit 策略
//!
//! 将 WAL 记录先写入内存缓冲，满足以下条件之一时批量刷盘：
//! - 缓冲区满（capacity 条记录）
//! - commit 请求（append_commit_and_wait 触发）
//! - 定时器到期（flush_interval_ms）；`quiesce_for_checkpoint` 期间定时器不写未达容量的记录
//! - shutdown 时强制刷盘
//!
//! 同一时刻只有一个刷盘执行 `write_batch`。`notified().await` 和暂停等待都不持有刷盘闸门。

use super::record::{WalError, WalRecord};
use super::writer::WalWriter;
use crate::diagnostics::{Category, WaitSlot};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use tokio::time;

/// 等待 flush 确认的登记项：通知句柄 + 诊断归属。
///
/// 诊断部分只在采集打开时存在：登记发生在语句任务内，句柄与阶段 id 随
/// 等待者进入等待表，刷盘任务只回写 Io 与 Schedule（不解释语义）。
struct CommitWaiter {
    /// 提交被刷盘释放后唤醒本等待者。
    notify: Arc<Notify>,
    /// 本等待者所属语句的等待槽（采集关闭时为空）。
    slot: Option<WaitSlot>,
    /// 登记时最内层 open 阶段的 id（`None` = 语句根）。
    stage_id: Option<u64>,
    /// 通知刷盘的时刻（Schedule 的起点）。
    notified_at: Instant,
    /// 通知时刷盘循环是否在空等（正忙时 Schedule 记 0）。
    flush_was_idle: bool,
}

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
    /// tx_id -> 等待者（通知句柄 + 诊断归属）
    commit_waiters: Mutex<HashMap<u64, CommitWaiter>>,
    /// 关闭标志。为真后后台循环退出，且不再 spawn。
    shutdown: AtomicBool,
    /// 暂停标志。为真时后台循环不刷盘，直到恢复或停止。
    paused: AtomicBool,
    /// 把「取走缓冲、write_batch、唤醒已取走的等待者」串成一次刷盘。
    flush_gate: Mutex<()>,
    /// 会话闸门。本模块的刷盘、暂停、恢复、停止和追加都不获取它。
    /// 提交路径尚未持有它，非测试构建没有读取点。
    #[allow(dead_code)]
    session_gate: Mutex<()>,
    /// 暂停、恢复或停止时唤醒后台循环和容量刷盘等待。
    state_notify: Notify,
    /// 刷盘循环是否正停在 select 上空等（正忙时通知不产生调度等待）。
    flush_loop_idle: AtomicBool,
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
            paused: AtomicBool::new(false),
            flush_gate: Mutex::new(()),
            session_gate: Mutex::new(()),
            state_notify: Notify::new(),
            flush_loop_idle: AtomicBool::new(false),
            capacity,
            flush_interval_ms,
            flush_handle: std::sync::Mutex::new(None),
        }
    }

    /// 会话闸门的共享引用。调用方在外部持有它时，本模块的刷盘路径不能再获取它。
    /// 提交路径尚未持有它，非测试构建没有调用点。
    #[allow(dead_code)]
    pub(crate) fn session_gate(&self) -> &Mutex<()> {
        &self.session_gate
    }

    /// 追加 WAL 记录到内存缓冲，返回分配的 LSN
    ///
    /// 如果缓冲区满（达到 capacity），在暂停结束后刷盘。暂停期间不持有刷盘闸门。
    pub async fn append(&self, record: WalRecord) -> u64 {
        let lsn = self.current_lsn.fetch_add(1, Ordering::SeqCst);

        let should_flush = {
            let mut buf = self.buffer.lock().await;
            buf.push((lsn, record));
            buf.len() >= self.capacity
        };

        if should_flush {
            self.flush_after_pause().await;
        }

        lsn
    }

    /// 注册 commit 等待 + 通知后台刷盘
    ///
    /// 调用者应先 append CommitTxn/Commit 记录，再调用此方法等待持久化确认。
    /// 多个并发事务的 commit 会合并到同一次 fsync（Group Commit）。
    ///
    /// MS44 Iter002（T3）：刷盘闸门的等待记为 Lock；`notified().await` 的
    /// 剩余部分记为 Queue（减去本次刷盘的 Io 与 Schedule）。登记时把当前
    /// 语句的等待槽与阶段 id 一起放进等待表——刷盘任务只对被它释放的等待者
    /// 回写 Io 与 Schedule。
    pub async fn append_commit_and_wait(&self, tx_id: u64) -> Result<(), WalError> {
        // 采集关闭时为 `None`——不建立归属，也不计时。
        let (slot, stage_id) = match crate::diagnostics::current_wait_slot() {
            Some((slot, stage_id)) => (Some(slot), stage_id),
            None => (None, None),
        };
        let gate_start = crate::diagnostics::wait_timing_start();
        let waiter = {
            let _gate = self.flush_gate.lock().await;
            if let Some(start) = gate_start {
                crate::diagnostics::record_wait(Category::Lock, start.elapsed());
            }
            let notify = Arc::new(Notify::new());
            // 通知时刻与刷盘循环的空闲状态：Schedule 的起点与「是否正忙」。
            let notified_at = Instant::now();
            let flush_was_idle = self.flush_loop_idle.load(Ordering::SeqCst);
            {
                let mut waiters = self.commit_waiters.lock().await;
                waiters.insert(
                    tx_id,
                    CommitWaiter {
                        notify: notify.clone(),
                        slot: slot.clone(),
                        stage_id,
                        notified_at,
                        flush_was_idle,
                    },
                );
            }
            {
                let mut pending = self.pending_commits.lock().await;
                pending.push(tx_id);
            }
            if self.shutdown.load(Ordering::SeqCst) {
                self.flush_locked().await;
            }
            notify
        };

        if !self.shutdown.load(Ordering::SeqCst) {
            self.flush_notify.notify_one();
        }

        // Queue：`notified().await` 的实测耗时减去本次刷盘已记入该语句的
        // Io 与 Schedule（刷盘任务先回写、后唤醒，这里读到的一定是本次的）。
        let queue_start = crate::diagnostics::wait_timing_start();
        waiter.notified().await;
        if let Some(start) = queue_start {
            let elapsed = start.elapsed();
            let (io, schedule) = slot
                .as_ref()
                .and_then(|slot| slot.last_flush(stage_id))
                .map(|flush| (flush.io, flush.schedule))
                .unwrap_or((Duration::ZERO, Duration::ZERO));
            let queue = elapsed.saturating_sub(io).saturating_sub(schedule);
            crate::diagnostics::record_wait(Category::Queue, queue);
        }
        Ok(())
    }

    /// 启动后台刷盘 task
    pub fn start_flush_loop(self: &Arc<Self>) {
        let mut guard = self.flush_handle.lock().unwrap();
        if self.shutdown.load(Ordering::SeqCst) {
            return;
        }
        let this_for_spawn = self.clone();
        let handle = tokio::spawn(async move {
            this_for_spawn.flush_loop().await;
        });
        *guard = Some(handle);
    }

    /// 后台刷盘循环
    async fn flush_loop(&self) {
        let interval = time::Duration::from_millis(self.flush_interval_ms);

        loop {
            if self.shutdown.load(Ordering::SeqCst) {
                break;
            }
            if self.paused.load(Ordering::SeqCst) {
                self.wait_until_flush_allowed().await;
                continue;
            }

            self.flush_loop_idle.store(true, Ordering::SeqCst);
            tokio::select! {
                _ = self.flush_notify.notified() => {}
                _ = self.state_notify.notified() => {}
                _ = time::sleep(interval) => {}
            }
            self.flush_loop_idle.store(false, Ordering::SeqCst);

            if self.shutdown.load(Ordering::SeqCst) {
                break;
            }

            let _gate = self.flush_gate.lock().await;
            if self.shutdown.load(Ordering::SeqCst) {
                break;
            }
            if self.paused.load(Ordering::SeqCst) {
                continue;
            }
            self.flush_locked().await;
        }
    }

    /// 执行一次刷盘：
    /// 1. take 缓冲区所有记录
    /// 2. 计算基于文件偏移量的 LSN → write_batch + fsync 到磁盘
    /// 3. 通知等待中的 commit 事务
    pub async fn do_flush(&self) {
        let _gate = self.flush_gate.lock().await;
        self.flush_locked().await;
    }

    /// 先刷出当前缓冲，再让定时器停在可被恢复或停止唤醒的位置。
    ///
    /// 已经停止时直接返回，不启动新任务。checkpoint 尚未调用。
    #[allow(dead_code)]
    pub(crate) async fn quiesce_for_checkpoint(&self) {
        if self.shutdown.load(Ordering::SeqCst) {
            return;
        }
        self.paused.store(true, Ordering::SeqCst);
        self.state_notify.notify_waiters();

        let _gate = self.flush_gate.lock().await;
        self.flush_locked().await;
    }

    /// 恢复定时器刷盘。已经停止时直接返回，不启动新任务。checkpoint 尚未调用。
    #[allow(dead_code)]
    pub(crate) async fn resume_after_checkpoint(&self) {
        if self.shutdown.load(Ordering::SeqCst) {
            return;
        }
        self.paused.store(false, Ordering::SeqCst);
        self.state_notify.notify_waiters();
    }

    /// 关闭：刷盘所有缓冲 + 停止后台 task
    pub async fn shutdown(&self) {
        {
            let _gate = self.flush_gate.lock().await;
            self.shutdown.store(true, Ordering::SeqCst);
            self.flush_locked().await;
        }

        self.state_notify.notify_waiters();
        self.flush_notify.notify_one();

        let handle = {
            let mut guard = self.flush_handle.lock().unwrap();
            guard.take()
        };

        if let Some(handle) = handle {
            let _ = handle.await;
        }
    }

    /// 暂停且尚未停止时等待恢复或停止。等待期间不持有刷盘闸门。
    ///
    /// `state_notify` 使用 `notify_waiters`，`Notified` 在创建后就能收到这次唤醒。
    async fn wait_until_flush_allowed(&self) {
        loop {
            let notified = self.state_notify.notified();
            if self.shutdown.load(Ordering::SeqCst) || !self.paused.load(Ordering::SeqCst) {
                return;
            }
            notified.await;
        }
    }

    /// 容量触发的刷盘：暂停时先放开闸门等待，恢复或停止后再写。
    async fn flush_after_pause(&self) {
        loop {
            self.wait_until_flush_allowed().await;
            let _gate = self.flush_gate.lock().await;
            if !self.shutdown.load(Ordering::SeqCst) && self.paused.load(Ordering::SeqCst) {
                continue;
            }
            self.flush_locked().await;
            break;
        }
    }

    /// 调用方必须已经持有 `flush_gate`。
    ///
    /// 取走缓冲和 pending，尝试 `write_batch`，然后唤醒这次取走的等待者。
    /// `write_batch` 失败时仍然唤醒，避免等待者停在 `notified()`。
    ///
    /// MS44 Iter002（T3）：本次刷盘的 Io（`write_batch` 的整段耗时）与
    /// Schedule（通知到开始处理这批的间隔）只回写给被这次刷盘释放、且带
    /// 诊断槽的等待者——先回写、后唤醒，保证语句侧读到的是本次的值。
    async fn flush_locked(&self) {
        let records: Vec<(u64, WalRecord)>;
        let committed_tx_ids: Vec<u64>;
        // 本批开始处理的时刻（Schedule 的终点）。
        let batch_start = Instant::now();

        {
            let mut buf = self.buffer.lock().await;
            records = std::mem::take(&mut *buf);

            let mut pending = self.pending_commits.lock().await;
            committed_tx_ids = std::mem::take(&mut *pending);
        }

        // 本批等待者（含诊断归属）：先取走，写盘后再回写并唤醒。
        let waiters: Vec<CommitWaiter> = {
            let mut waiters = self.commit_waiters.lock().await;
            let mut taken = Vec::with_capacity(committed_tx_ids.len());
            for tx_id in &committed_tx_ids {
                if let Some(waiter) = waiters.remove(tx_id) {
                    taken.push(waiter);
                }
            }
            taken
        };

        // 有诊断归属的等待者才需要给写盘计时（采集全关时零成本）。
        let tracing = waiters.iter().any(|waiter| waiter.slot.is_some());
        let mut io = Duration::ZERO;

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

            // Io = 这次 `write_batch` 的整段耗时（失败也照记）。
            let io_start = if tracing { Some(Instant::now()) } else { None };
            if self
                .wal_writer
                .write_batch(file_offset_lsn_records)
                .await
                .is_err()
            {
                // Log error but don't panic - WAL write failures are critical
                // but we still need to notify waiters to avoid hanging
            }
            io = io_start
                .map(|start| start.elapsed())
                .unwrap_or(Duration::ZERO);
        }

        // 回写 Io 与 Schedule（先回写，后唤醒）。
        for waiter in &waiters {
            let Some(slot) = &waiter.slot else {
                continue;
            };
            // 通知到达时刷盘循环不在空等（正忙）时，调度等待记 0。
            let schedule = if waiter.flush_was_idle {
                batch_start
                    .checked_duration_since(waiter.notified_at)
                    .unwrap_or(Duration::ZERO)
            } else {
                Duration::ZERO
            };
            slot.record_flush(waiter.stage_id, io, schedule);
        }

        for waiter in waiters {
            waiter.notify.notify_one();
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

#[cfg(test)]
mod tests {
    use super::super::reader::WalReader;
    use super::super::record::WalRecord;
    use super::super::writer::WalWriter;
    use super::WALBuffer;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::NamedTempFile;

    fn open_buffer(capacity: usize, flush_interval_ms: u64) -> (NamedTempFile, Arc<WALBuffer>) {
        let file = NamedTempFile::new().unwrap();
        let writer = Arc::new(WalWriter::open(file.path()).unwrap());
        let buffer = Arc::new(WALBuffer::new(writer, capacity, flush_interval_ms));
        (file, buffer)
    }

    fn read_records(db_path: &std::path::Path) -> Vec<WalRecord> {
        let wal_path = db_path.with_extension("wal");
        let mut reader = WalReader::open(&wal_path).unwrap();
        reader.read_all().unwrap()
    }

    fn contains_begin(records: &[WalRecord], tx_id: u64) -> bool {
        records
            .iter()
            .any(|record| matches!(record, WalRecord::BeginTxn { tx_id: id } if *id == tx_id))
    }

    #[tokio::test]
    async fn quiesce_flushes_then_pauses_timer() {
        let (file, buffer) = open_buffer(100, 50);
        buffer.start_flush_loop();

        buffer.append(WalRecord::BeginTxn { tx_id: 1 }).await;
        buffer.quiesce_for_checkpoint().await;

        let after_quiesce = read_records(file.path());
        assert!(
            contains_begin(&after_quiesce, 1),
            "first record should be in the WAL after quiesce, got {after_quiesce:?}"
        );

        buffer.append(WalRecord::BeginTxn { tx_id: 2 }).await;
        tokio::time::sleep(Duration::from_secs(1)).await;

        let while_paused = read_records(file.path());
        assert!(
            !contains_begin(&while_paused, 2),
            "second record must stay out of the WAL while the timer is paused, got {while_paused:?}"
        );

        buffer.shutdown().await;
    }

    #[tokio::test]
    async fn quiesce_does_not_take_session_gate() {
        let (_file, buffer) = open_buffer(100, 10_000);
        buffer.start_flush_loop();

        let held = buffer.session_gate().lock().await;
        let quiesced =
            tokio::time::timeout(Duration::from_secs(2), buffer.quiesce_for_checkpoint()).await;
        assert!(
            quiesced.is_ok(),
            "quiesce_for_checkpoint must return within 2 seconds while the caller holds session_gate"
        );
        drop(held);

        buffer.shutdown().await;
    }

    #[tokio::test]
    async fn resume_lets_timer_flush() {
        let (file, buffer) = open_buffer(100, 50);
        buffer.start_flush_loop();
        buffer.quiesce_for_checkpoint().await;
        buffer.append(WalRecord::BeginTxn { tx_id: 11 }).await;
        buffer.resume_after_checkpoint().await;

        tokio::time::sleep(Duration::from_secs(1)).await;
        let records = read_records(file.path());
        assert!(
            contains_begin(&records, 11),
            "timer should write the buffered record within 1 second after resume, got {records:?}"
        );

        buffer.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_after_quiesce_flushes_paused_records() {
        let (file, buffer) = open_buffer(100, 10_000);
        buffer.start_flush_loop();
        buffer.quiesce_for_checkpoint().await;
        buffer.append(WalRecord::BeginTxn { tx_id: 21 }).await;

        let stopped = tokio::time::timeout(Duration::from_secs(2), buffer.shutdown()).await;
        assert!(
            stopped.is_ok(),
            "shutdown after quiesce should finish within 2 seconds"
        );

        let records = read_records(file.path());
        assert!(
            contains_begin(&records, 21),
            "shutdown should flush the record appended while paused, got {records:?}"
        );
    }

    #[tokio::test]
    async fn resume_after_shutdown_does_not_flush_by_timer() {
        let (file, buffer) = open_buffer(100, 50);
        buffer.start_flush_loop();
        buffer.shutdown().await;
        buffer.resume_after_checkpoint().await;
        buffer.append(WalRecord::BeginTxn { tx_id: 31 }).await;

        tokio::time::sleep(Duration::from_secs(1)).await;
        let records = read_records(file.path());
        assert!(
            !contains_begin(&records, 31),
            "resume after shutdown must not start a timer that writes the record, got {records:?}"
        );
    }

    #[tokio::test]
    async fn second_shutdown_returns() {
        let (_file, buffer) = open_buffer(100, 10_000);
        buffer.start_flush_loop();
        buffer.shutdown().await;
        buffer.shutdown().await;
    }

    #[tokio::test]
    async fn overlapping_do_flush_preserves_record_count() {
        let (file, buffer) = open_buffer(100, 10_000);
        let count = 4u64;
        for tx_id in 0..count {
            buffer.append(WalRecord::BeginTxn { tx_id }).await;
        }

        let first = buffer.clone();
        let second = buffer.clone();
        let left = tokio::spawn(async move {
            first.do_flush().await;
        });
        let right = tokio::spawn(async move {
            second.do_flush().await;
        });
        left.await.unwrap();
        right.await.unwrap();

        let records = read_records(file.path());
        assert_eq!(
            records.len(),
            count as usize,
            "concurrent do_flush should leave each record readable once, got {records:?}"
        );
    }
}
