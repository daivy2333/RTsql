# 索引写侧原子性的确定性复现配方

- Status: active
- Last validated: 2026-10-25
- Environment: Linux x86_64（WSL2 6.18.40.1）、cargo 1.99.1 / rustc 1.99.1、RTsql `tests/index_write_atomicity_test.rs` 集成测试、tokio 多线程 test runtime、单用例自带 `tempfile::tempdir()` 现场
- Source: change `2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog` Iteration 000 / Cycle 000 Act Response 与 Plan Review（`openspec/changes/archive/2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog/iterations/000-index-write-side-atomicity/000-initial.md`，2026-10-24）；change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 004 / Cycle 000 Act Response 与 Plan Review（`.../iterations/004-release-durability-precondition/000-initial.md`，2026-10-25，**锚点写持久化判定的取样点**结论）

## 适用范围

- 需要**确定性**复现「取消 / 回滚落在索引写单元中间」的缺陷形态时使用：半写状态（结构变更已生效、根页 id 未发布）、键残留（同键再插报 `DuplicateKey`）、结构分叉（并发写者丢条目）。
- 需要验证「索引写侧修复是否真的把半写窗口钉成可重复命中的状态」时使用——本配方的判据不依赖两写者交错的时序。
- 不适用：真实断电 / 进程崩溃语义（用 `.agents/runbooks/in-process-crash-simulation.md`）；DDL 目录面事务化（用 `.agents/runbooks/ddl-catalog-commit-point.md`）；索引查询性能回归；全量回归的分批执行（用 `.agents/runbooks/m19-batched-full-regression.md`）。

## 前置条件

- 夹具入口可用：`IndexManager::arm_write_pause(position) -> WritePauseHandle`（`src/storage/btree/index_manager.rs`）。
  - 语义位置**唯一**：`WritePausePosition::AfterStructureChangeBeforeRootPublish` = 「结构变更已生效、根页 id 尚未发布」，**修复前后不变**（夹具位置随修复漂移会让 RED 失去意义）。
  - handle 提供三个动作：`arrivals()`（到达计数，可轮询）、`release()`（放行当前到达）、`disarm()`（卸载夹具）。
- 夹具槽位是**每个 `IndexManager` 一份**（非全局静态）⇒ 不跨用例污染；`disarm()` 只清自己的槽位项。
- 未装载暂停点时**零行为差异**：写路径只多做一次未命中检查，其余逐条相同（由 `cargo test --lib --bins` 420 passed = 基线见证）。
- 夹具不携带任何运行身份（无 run-id / 指纹 / 时间戳）。

## 操作步骤

1. **夹具几何自检**（先跑，确认暂停点可达的前提成立）：建单列主键表 → 插 92 行（`LEAF_FULL_ROWS = 92`，单叶恰好装满）→ 记录 `index_manager.root_page_id()` → 插第 93 个键 → 断言根页 id 变化（根页 id 是索引结构的公开可观测量）。用例：`leaf_fills_at_92_rows_and_the_93rd_key_splits_the_root`。
2. **装载暂停点**：取目标表主键索引的 `IndexManager`，调用 `arm_write_pause(WritePausePosition::AfterStructureChangeBeforeRootPublish)` 取得 handle。
3. **填满单叶**：插入 92 行（与步骤 1 同一夹具几何）。
4. **后台插入第 93 个键触发换根**：把 `db.execute_sql("INSERT INTO t VALUES (93)")` 的 future 放进后台任务——写单元到达暂停点后会停住。
5. **轮询到达**：`await_arrivals(pause, 1)`——用 `tokio::task::yield_now()` 轮询 `pause.arrivals() >= n`；不要用固定 sleep 判定。
6. **丢弃语句 future（取消）**：`drop(statement)`——写单元停在「结构变更已生效、根页 id 尚未发布」。夹具用 `tokio::select!` + `biased` 保证「语句先跑完」被当作夹具不成立直接 panic，而不是把「跑完」当成被取消。
7. **放行并卸载**：`pause.release()` 放行写单元继续，`pause.disarm()` 卸载夹具——不卸载会让后续写（回滚 / 回收触发的 `delete`、断言用的 `INSERT`）再次被停住而挂死。
8. **下一条写语句触发回滚 / 回收**：执行一条普通写语句（如 `INSERT INTO t2 VALUES (1)`）——隐式事务回收先于本语句的隐式事务发生。
9. **断言四类可观察量**：
   - 键可达性：键 1..92 全部可查（`index_manager.search` 与 SQL 入口 `SELECT id FROM t WHERE id = N` 各查一遍）。
   - 重复键：键 93 可再次插入且不报 `DuplicateKey`。
   - 条目数：被取消语句写入的键全部释放（隐式路径用例断言索引里只剩重新插入的键）。
   - 完整性：`check_integrity` 的 error 级发现为 0。

## 验证

- 成功判据：`cargo test --test index_write_atomicity_test -- --test-threads=1` 退出码 0，输出 `test result: ok. 5 passed; 0 failed`（2026-10-24 实测，连续 5 次 GREEN）。
- **修复前 RED 形态**（2026-10-24 实测，3 次连续稳定命中产品断言，`test result: FAILED. 1 passed; 4 failed`）：
  - ① `key 47 must stay reachable in the index after the cancelled write`（换根丢失，键 47..92 不可达）；
  - ② 显式事务用例 `INSERT INTO t VALUES (2) must succeed with affected rows, got Error { message: "Execution error: Duplicate key" }`；
  - ③ 隐式路径用例同一 `Duplicate key`；
  - ④ 并发用例 `the index must hold exactly one entry per row written by the concurrent writers`（实测 574 / 551 / 554 条，均 < 600）。
  - 几何自检用例 3 次均通过 ⇒ 夹具本身不是失败源。
- 判据全部落在**可观察状态**上（暂停点到达计数、索引查找与条目数、SQL 执行结果、完整性报告）；判定层在 `cargo test` 内，无项目测试框架之外的判定脚本。
- 失控兜底：`ANCHOR_TIMEOUT` 10 秒 timer，触发即 `expect` 失败——**是兜底不是行为判据**。

## 锚点写「是否已持久」的取样点（2026-10-25 补充）

- **来源与适用范围**：该结论在**释放持久化面**（`2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 004）实测定形，但**同族可复用**于本配方：两者都是「锚点写的持久化判定」。本节记录方法论，不改本配方的判据。
- **取样点选择**：判定「锚点写是否已被持久化」时，**取样点必须选在能观察到『该页仍为脏』的位置**——此刻「仍脏」等价于「其写尚未被任何刷盘写出」。若改在 checkpoint **自己的刷盘之后**再判定，「该页仍为脏 ⇒ 未写出」这条前提就**失去可观察的落点**（页可能已被写盘却仍带脏标记，判据不可分辨）。
- **纯脏标记判据会与已验收行为互斥**（实测）：只用「该页仍为脏 ⇒ 不提升/不判定」这一条判据，会拦掉**已经验收过的**释放后复用形态（`released_page_allocatable_after_release_durable` / `same_session_release_reuses_pages_after_release_durable`，二者都是「DROP → checkpoint ⇒ 被释放的页在同一会话内被复用」）——Act 临时把「什么都不提升」代入后实测 `10 passed; 2 failed`，失败的正是这两条。⇒ 两条流程在取样点上的状态**完全相同**，唯一可区分的信号是**失败历史**，不是脏标记本身。
- **可复用的判定形态**：把判据对**失败写**敏感，而不是对**脏标记**敏感——即「在该锚点写之后**发生过**针对该页的写盘失败」才改变结论。⇒ 既保住了已验收形态，又让失败情形有确定结果。
- **对本配方的含义**：若本配方将来要判定「被取消的写是否已落盘」，取同样的落点原则——**在脏标记仍可观察的位置取样**，并预期「脏标记本身不足以区分已写盘与未写盘」这一歧义。

## 失败处理

- **到达计数一直为 0（暂停点未命中）**：确认装载的是目标索引的 `IndexManager`（不是别的表），且第 93 个键确实触发换根——先单独跑几何自检用例确认夹具几何。
- **语句先跑完（`the statement finished before write-pause arrival N was reached`）**：夹具不成立，不是产品行为——按提示调小/调整写入规模使暂停点可达，不要放宽断言把它当成「被取消」。
- **10 秒 timer 触发**：夹具失控（写单元未放行，或放行后仍被停住）——检查是否调用了 `disarm()`；未卸载会让后续写被停住。
- **测试侧键解码错误**（`range end index 8 out of range for slice of length 7`）：不要用 `i64::to_be_bytes` 自行解码索引条目——产品 `Key::deserialize` 按尾部零扫描推断长度，256 / 1024 等值的键长为 7；改用产品的 `search` 逐键断言 + 条目总数断言。
- **并发用例的失败计数随运行波动**（574 / 551 / 554，均 < 600）：属并发固有的丢失比例差异，不影响判据（断言是「恰好 600 条目且 600 个键全部可查」）；确认失败稳定复现而非偶发即可。
- **回收与写单元尾部交错导致回滚仍报 `Duplicate key`**：该形态由「反向映射读与写单元串行」消除；若复现，说明该串行化被回退（修复见 `.agents/issues/` 中 ISS20 的关闭记录）。

## 回滚

不适用（测试配方，无产品状态变更）。夹具只影响装载它的那个 `IndexManager`；`disarm()` 后无残留，误用产生的只有 `tempdir()` 残留，随测试结束清理。

## 证据

- `openspec/changes/archive/2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog/iterations/000-index-write-side-atomicity/000-initial.md` —— Act Response 的 T1 / T2 / T3 实现说明与 Verification Evidence 表（RED 3/3、GREEN 5/5）；Plan Review 的 F6（夹具零行为差异、无身份机制）与 F8（四条 requirement 的判别力核对）。
- `openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/004-release-durability-precondition/000-initial.md` —— Act Response 的「Fix 2 的判据形态」段（取样点判定与 checkpoint 刷盘后判定的两次实测、取样点判定破坏 MS35 两条已验收用例的实测 `10 passed; 2 failed`）与 Plan Review 的 Deviation (a)（独立复核：取样点上锚点页必为脏、MS35 已验收的提升发生在同一取样点 ⇒ 纯脏标记判据不可实现，唯一可区分的信号是失败历史）。
- 见证代码：`tests/index_write_atomicity_test.rs`（5 用例）；夹具实现：`src/storage/btree/index_manager.rs`（`WritePausePosition` / `WritePauseHandle` / `arm_write_pause`）。
- 行为规格：`openspec/specs/index-write-side-atomicity/spec.md`（4 Requirement / 5 Scenario）。
- 相关配方：`.agents/runbooks/in-process-crash-simulation.md`（进程内崩溃模拟）、`.agents/runbooks/ddl-catalog-commit-point.md`（DDL 目录提交点）、`.agents/runbooks/m19-batched-full-regression.md`（分批全量）。
