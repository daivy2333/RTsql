# MS18 one-shot 路径性能收口 — Design

> 设计依据：proposal（用户批准 2026-09-26）；Plan 调查 Current-State Evidence 见 Iteration 000 Plan Context。I062 原方案（WAL 文件长度探针）经调查修正为双条件谓词（D1），DDL 无 WAL、schema 持久化依赖 checkpoint 刷脏页是修正依据。

## D1: 跳过判定谓词与守卫落点

**决策**：守卫放在 `CheckpointManager::checkpoint`（`src/wal/checkpoint.rs:131`）方法顶部、步骤 1 之前，单点覆盖全部触发路径（`Database::close()` → `checkpoint()`、公开 `checkpoint()`、库用户直调）。谓词：

```text
skip  ⟺  wal_writer.get_write_count() == 0  &&  !buffer_pool.has_dirty_pages()
```

- **WAL 侧信号用 `write_count`（内存计数器），不用文件长度**。调查确认：九步 checkpoint 的步骤 6 追加一条 `Checkpoint` 记录、步骤 7 重写截断保留该后缀——**checkpoint 后 WAL 恒含恰一条 Checkpoint 标记记录**（非空文件），「文件长度 == 0」在稳态下永假，文件长度与位点差值也无法区分「尾部标记」与「未决数据记录」。`write_count` 语义 = 自上次 `reset_write_count`（步骤 9）以来的 WAL 追加记录数：`WalWriter::write_record` +1（writer.rs:68）、`write_batch` +n（writer.rs，WALBuffer `do_flush` 唯一批量入口）、checkpoint 步骤 6 +1 后步骤 9 清零。`get_write_count()`（writer.rs:152）为同步 AtomicU64 读，零 I/O。
- **池侧信号用新增只读探测 `BufferPool::has_dirty_pages() -> bool`**（见 D3）。
- **跳过返回 `Ok(0)`**：既有返回值 = 捕获 LSN（信息性）；消费者清点——`Database::checkpoint` 丢弃（map 到 `()`）、全部测试 `.unwrap()` 丢弃、仅 `tests/checkpoint_test.rs:53` 绑定且其夹具先写 5 条记录（count>0 → 全量路径，断言不受影响）。`Ok(0)` 附 doc 说明「0 = 零工作跳过，未写位点」。
- **`Database::close()` doc 注释**同步补零工作跳过语义（I062）。

**替代方案与拒绝理由**：
- WAL 文件长度探针（I062 原文）——拒绝：稳态恒含 Checkpoint 标记记录，长度==0 永假；长度==标记记录大小需解析序列化定长，脆弱。
- 位点 lsn 与文件长度差值——拒绝：同一稳态问题（位点=0、文件=标记记录大小）；且跨进程 legacy 位点形态（步骤 5/8 之间崩溃）语义混杂。
- 「低于阈值跳过重写」——拒绝（用户裁定 2026-09-26）：close 后 WAL 残留未决记录 → 下次打开 `redo_count > 0` → 全量索引重建（I031 量化 40k 行 8.86s）+ 违背 M18「checkpoint-clean 关闭磁盘树可信」。
- 仅 WAL 条件单探针——拒绝（用户裁定）：DDL 不写 WAL（`src/pipeline.rs` DDL 直包执行器、`create_table.rs`/`drop_table.rs` 零 WAL 引用），schema 持久化全靠 checkpoint 刷脏页——单条件会把纯 DDL 会话的建表丢失。
- 守卫放 `close()` 而非 CheckpointManager——拒绝：显式 `checkpoint()` 与 close 语义应一致（无未决工作时两者都无工作可做），两处守卫制造分叉；单点守卫使 `open` 恢复后路径（若未来接线）自动一致。

## D2: 跳过安全性论证（不变量）

**跳过 ⟹ 磁盘状态恰为 checkpoint-clean 等价态**，按「未决工作」来源穷举：

| 未决工作来源 | 谓词捕获 | 论证 |
|---|---|---|
| 本会话 DML（begin/data/commit 经 WALBuffer） | `write_count > 0` | commit 路径 `append_commit_and_wait` 等待 `do_flush` → `write_batch` 推进计数（buffer.rs:82-104/147-182）；已提交数据必先落 WAL |
| 本会话 DDL（不写 WAL） | 池脏页 | CreateTable/DropTable 经池 `modify_page` 置脏（catalog/table_manager 写路径）；checkpoint 刷脏页是 schema 持久化唯一通道 |
| 本会话 SELECT 只读 | 两条件皆空 → 跳过 | 查询不包事务（pipeline.rs:235-245 `statement_snapshot(None)` 直构执行器，零 WAL 记录）；`set_all_visible` 等可见性惰性设置只写内存 vis_map（buffer_pool.rs:379 sync fn，不碰页字节）；读路径不置脏 |
| 本会话回滚 | 池脏页 | `abort_cleanup_versions` 版本清理/墓碑中性化经池写页；即便 AbortTxn 记录仍在 WALBuffer 缓冲未刷（append 不等待），池脏页已触发全量 |
| 前会话崩溃 + 已提交数据 | 池脏页 | recovery 重放 `redo_record` 经池写页（recovery.rs:536）→ 置脏；`redo_count` 只数已提交数据记录，Checkpoint 标记不计（recovery.rs:531-536） |
| 前会话崩溃 + 未提交数据 | 池脏页 | `mark_uncommitted_aborted` 经池写页；无数据记录的悬挂 Begin 为惰性（recovery 容忍，语义中性） |
| 前会话 pre-site 记录（legacy 位点 >0 形态） | 谓词皆空 → 跳过 | 位点前缀记录已被此前刷盘覆盖，非未决工作；跳过保持位点与文件原样，重开按既有位点过滤语义处理 |
| 新库 bootstrap | 两条件皆空 → 跳过 | `Catalog::bootstrap` 经池 init 后**自行 `flush_all()`**（catalog.rs:136，脏位清零）；格式头在 FileStorage::open 直写（先于池存在）；bootstrap 态已完整在盘——跳过不产生位点文件，重开走「从未 checkpoint」既有路径（位点 None + 空 WAL → 干净打开） |

- **WALBuffer 缓冲尾部**：守卫只看 `write_count`（已刷盘量）。缓冲中未刷记录的归属——已提交（count>0，见上）、已回滚（池脏）、悬挂 begin-only（惰性：与今日行为一致地可能丢于进程退出，recovery 对无数据记录的事务无操作）。跳过不引入新的丢失面。
- **并发窗口**：守卫判定与返回之间如有并发写入抵达（同进程多 task 共享 `Database`），与今日 checkpoint 的 flush→truncate 窗口同类（既有设计假定 close 期间无并发写者；文件锁保证跨进程单写者）。守卫不扩大既有窗口。
- **加密库**：WAL/位点保持明文（MS17-T01），长度与计数探测无差别，无特判。

## D3: `BufferPool::has_dirty_pages()`

新增 pub 只读探测：遍历 `pages: DashMap<PageId, Arc<Mutex<PageFrame>>>`，任一 frame `dirty == true` 即返回 true。同步 fn（无 await），遵守 M17 分片读锁纪律（与 `flush_all` Phase 1 同型迭代、但不置位不清位、可提前返回）。容量 ≤ 池上限（默认 100 页），开销为内存扫描，远低于其守卫的全量 checkpoint（多次 fsync）。不触碰热路径（仅在 checkpoint 入口调用）。

## D4: T1 测试见证设计

- **文件级字节对比是主见证**（无 API 扩面）：位点文件 24B 内容含 timestamp——全量 checkpoint 必改字节（新 timestamp），跳过必字节不变；新库场景「位点文件存在与否」直接二值可见。`.wal` 内容同理（全量重写后仍为单标记记录、字节不变，故位点文件是区分面）。
- **RED 见证**：新库零工作 close 场景在现状下必创建位点文件（全量无条件执行）——先观察 RED 再实现 GREEN。
- **新增 `tests/checkpoint_skip_test.rs`**（矩阵见 RTM）：零工作 close 跳过（位点/WAL 字节不变 + 重开等价）、新库零工作 close 不产生位点、DML 会话全量保持、DDL-only 会话全量保持且重开 schema 在（反「丢表」回归）、crash 脏库（INSERT 后 drop 不 close → 重开恢复 → SELECT → close）全量保持且位点推进、显式 `checkpoint()` 零工作无操作。
- **`src/wal/checkpoint.rs` 单测**：零工作 manager 直调 `checkpoint` 早退（tempdir 无位点文件产生 + 返回 0）。
- **既有套件零修改**：`checkpoint_test`（夹具有 WAL 记录 → 全量路径）、`checkpoint_redo_reduction_test`（会话有写 → 全量路径）、`schema_persistence_test`（含 DDL → 全量路径）——今日基线 3+9+8 全绿，实现后零校准通过即证零回归。

## D5: T2 CLI runtime flavor 判据与测量口径

**决策**：两 flavor 各跑三组实测（R26 runbook 口径），判据（用户批准 2026-09-26）：**current_thread 相对 multi_thread 峰值 RSS 下降 ≥15% 且 one-shot 时延与负载吞吐回归 ≤5% → 切换 `#[tokio::main(flavor = "current_thread")]`；否则保持 multi_thread**。数据（两组三值）与结论写入 design.md 本节（实施后由 Act 回填）与 Act Response。

- 测量口径（R26）：① one-shot 时延——对预建 2000 行库 50 次 `rtsql <db> "SELECT 1"` 循环取均值；② 峰值 RSS——`/usr/bin/time -v` 分别测 one-shot（`SELECT 1`）与 2000 行加载负载；③ 负载吞吐——2000 行加载负载 Elapsed（两 flavor 同负载同库起跑条件）。每组连续 3 次取稳定值（首跑弃用），判定用中位。
- current_thread 行为分析（调查结论，实施时验证）：CLI 生产路径 = open → 语句 → close（`src/cli/mod.rs:347`）；WALBuffer 后台 flush loop 为 `tokio::spawn` 任务（current_thread 上同线程协作调度，功能不变）；WAL/页 I/O 经 `spawn_blocking`（独立阻塞池，与 flavor 无关）；DataScan 预取 CLI 默认关闭；Server 面不经 `main.rs`；信号 future（tokio::signal）current_thread 支持。切换形态 = `src/main.rs` 单行 attribute 变更。
- 风险：单线程 runtime 下解析+执行+I/O 串行化可能劣化多语句/大负载吞吐——③ 直接覆盖该面；若判据不满足即保持现状（先量化再决定，MS08 纪律）。
- `cargo test` 不经 `main.rs`（测试自建 runtime/直用 lib），切换不影响测试基建与全量回归判定。

**实测回填（Act 2026-09-26，同机背靠背，每组 ×3 首跑弃用、判定值取后两次中位；2000 行预建库 `/tmp/r2k.db`，release 构建）**：

| 组 | multi_thread（现状） | current_thread | 判据 | 结论 |
|---|---|---|---|---|
| ① one-shot 时延（50 循环） | 6.54*/6.56/6.34 ms → **6.45** | 2.02*/2.07/1.99 ms → **2.03** | 回归 ≤5% | 改善 -68%，满足 |
| ② one-shot 峰值 RSS | 7080*/6560/6584 KB → **6572** | 6584*/6584/6840 KB → **6712** | 降 ≥15% | **+2.1%，不满足** |
| ③ 负载 Elapsed | 5.00*/3.61/4.01 s → **3.81** | 6.43*/4.35/3.39 s → **3.87** | 回归 ≤5% | +1.6%，满足 |
| （信息性）负载 RSS | 16824*/16804/16816 KB → **16810** | 16312*/16248/16548 KB → **16398** | 不参与判定 | -2.5% |

**决策：保持 multi_thread**（用户批准判据为合取，② RSS 臂不满足——两 flavor one-shot RSS 几乎相同，I063「num_cpus worker 线程是 RSS 大头」假设被实测证伪：32 worker 的栈未触碰不进 RSS，one-shot ~6.5 MB 主体是二进制/分配器/引擎结构）。current_thread 的时延收益（one-shot 6.45→2.03 ms，-68%；负载 +1.6% 在噪声带内）已留档，若将来以「时延」为主判据可重启评估——属 improvements/I063 后续，不在本 change 范围。`src/main.rs` attribute 还原，`git diff src/main.rs` 为空。

## D6: README 双语对比板块刷新口径

- 时机：T1+T2 全部落地、flavor 决策执行后，按 R26 重测受影响数字：one-shot 时延（50 次循环）、加载峰值 RSS、（若 flavor 切换）加载 Elapsed。引擎级 criterion 数字（INSERT/点查/全扫）不受本 change 影响——保持原值不动。
- 更新 `README.md` 与 `README.zh-CN.md`「性能与资源对比」板块对应行，保留口径说明与「单机观察」限定，追加测量日期（2026-09-26）与「close 零工作跳过后」语境（若 T2 切换 flavor 一并注明）。
- 双语数字与口径逐项一致；既有对比结构（表格行）不变，仅刷数值与注记。
