# 项目快照

> 最后更新：2026-06-04（M21 遗留项完成：DELETE mark_deleted + 惰性 set_all_visible + benchmark）

## 文档体系变更

**2026-06-03 完成规则整合升级 v2.0**：

## 文档体系变更

**2026-06-03 完成规则整合升级 v2.0**：

| 状态 | 内容 |
|------|------|
| ✅ 已完成 | 废弃 `openspec/specs/rules/`（266 行规则内容已迁移到 `CLAUDE.md`） |
| ✅ 已完成 | `CLAUDE.md` 升级为"文档索引 + 规则唯一事实来源"双角色（356 行） |
| ✅ 已完成 | OpenSpec 验证通过：4 个 spec 全部 PASS |
| 📋 变更 | 走 OpenSpec change 流程：`consolidate-rules-into-claude-md` |

**2026-06-02 完成 OpenSpec 文档体系迁移**（v1.0）：

| 状态 | 内容 |
|------|------|
| ✅ 已完成 | OpenSpec v1.4.0 初始化，5 个 spec 全部通过验证 |
| ✅ 已完成 | 旧 `.claude/docs/{architecture,rules,learned,references,optimization}.md` 内容迁移到 `openspec/specs/`，旧文件已删除 |
| ✅ 已完成 | `CLAUDE.md` 更新为索引入口，指向 `openspec/specs/` + `.claude/docs/` 状态文档 |
| 📋 保留 | `snapshot.md` / `tasks.md` / `archive.md` / `superpowers/` 不迁移 |

**新文档结构**（v2.0，2026-06-03）：
- `openspec/specs/{architecture,learned,references,optimization}/spec.md` — 规范文档（4 个 + tx-id-allocation-benchmark 新增）
- `openspec/changes/` — 变更提案（含 active + archive）
- `.claude/docs/snapshot.md` — 项目快照（本文件）
- `.claude/docs/tasks.md` — 任务追踪
- `.claude/docs/archive.md` — 历史归档
- `CLAUDE.md` — **索引入口 + 规则唯一事实来源**（356 行）

**2026-06-03 M41 完成 + rustfmt 重格式化**：

| 状态 | 内容 |
|------|------|
| ✅ 已完成 | M41 事务 ID AtomicU64 无锁分配：4 任务全勾，commit `634764d` + `ee9ceee` |
| ✅ 已完成 | 微基准 `benches/tx_id_bench.rs`：4 场景，criterion 0.5 |
| ✅ 已完成 | 实测数据：单线程 5.1 ns/op (2.1x)、10 线程 18.6 ns/op (4.6x)、100 线程 22.5 ns/op (4.5x) |
| ✅ 已完成 | rustfmt 重格式化：25 文件 196/139 行变更，commit `e644a19` |
| ✅ 已完成 | 文档同步：ADR-009 + O001 完成 + L017 实测 + tasks.md M41 状态 |
| 📋 变更 | 走 OpenSpec change：`consolidate-m41-tx-id-atomic`（已归档） |

**2026-06-03 M38 网络 BufWriter + TCP_NODELAY 完成**：

| 状态 | 内容 |
|------|------|
| ✅ 已完成 | T2 TCP_NODELAY：`server.rs` accept 后 `set_nodelay(true)` |
| ✅ 已完成 | T1+T3 写缓冲：`PgProtocol` 新增 `write_buf`（8KB），所有写路径单次 `write_all`+`flush` |
| ✅ 已完成 | T4 测试：`pg_protocol_test.rs` 新增 2 测试（100 行批写 + 缓冲复用），11 tests 全通过 |
| ✅ 已完成 | 全量回归 0 失败 |
| 📋 Phase 2 | 可启动 M20（零拷贝 SlottedPageRef）或 M19（DataScan 路径） |

**当前阶段**：Phase 2 进展中（M20 ✅ → M36 ✅ → M19 ✅，待启动 M21 页面级 MVCC）

**2026-06-04 M21 页面级 MVCC 完成**：

| 状态 | 内容 |
|------|------|
| ✅ 已完成 | T1 `PageVisibilityInfo` 结构体 + `DashMap` 集成：`src/storage/page_visibility.rs` (新) + BufferPool 4 公开方法 + 4 单元测试 |
| ✅ 已完成 | T2 扫描快速路径：`find_visible_version` + `DataScanExecutor` 闭包外查询 visibility_map（`all_visible` / `all_invisible_for`）|
| ✅ 已完成 | T3 写路径更新：INSERT/DELETE/UPDATE/COMMIT 四路径均调用 `clear_all_visible`（含 Plan Agent 发现的 COMMIT 缺口）|
| ⏸️ 延后 | T4 benchmark + T2.3 惰性 `set_all_visible`（Plan Agent 建议先保正确性，避免竞态条件）|
| ✅ 已完成 | 全量回归：129 lib + 全量集成测试 0 failures；clippy 仅 2 pre-existing warnings |
| ✅ 已完成 | 走 OpenSpec change：`m21-page-visibility-map`（已归档为 `2026-06-04-m21-page-visibility-map`）|
| ✅ 已完成 | 文档同步：ADR-011 + L028 记忆 + tasks.md M21 状态 + snapshot.md |
| 📋 下一步 | M21 惰性设置（T2.3）+ 基准测试（T4），然后 M37（clone 消除）或 M31（BufferPool DashMap）|

**2026-06-04 M19 DataScan 数据页直接遍历完成**：

| 状态 | 内容 |
|------|------|
| ✅ 已完成 | T1 `DataScanExecutor` 纯顺序扫描：流式 `next()`，无 `Vec<Vec<Value>>` 预加载 |
| ✅ 已完成 | T2 MVCC 可见性：`with_page_data` 闭包内解析 VersionHeader + 不可见时 `find_visible_in_chain` 异步跨页查链 |
| ✅ 已完成 | T3 Planner 无 WHERE 路由：新增 `PhysicalPlan::DataScan` 变体 + `DataScanNode` + `pipeline.rs` dispatch + `correlated.rs` + `get_subquery_first_column` + `aggregate input_schema` 全部支持 |
| ✅ 已完成 | T4 Planner 非 PK WHERE 路由：新增 `has_pk_equality` 递归检查 AND 组合 → `Filter(DataScan)`，PK 等值路径（IndexScan / `Filter(Scan)`）保持不变 |
| ✅ 已完成 | T5 criterion bench：`benches/data_scan_bench.rs` + Cargo.toml 入口，**1K 1.81x / 10K 2.44x 提速**（达到预期 ~2x 目标） |
| ✅ 已完成 | 全量回归 464/464 测试通过（8 M19 测试 + 原有 456） |
| ✅ 已完成 | 走 OpenSpec change：`m19-datascan-path`（已归档为 `2026-06-04-m19-datascan-path`）|
| ✅ 已完成 | 文档同步：增量 spec → `openspec/specs/data-scan-path/spec.md` + L026 实测教训 + tasks.md M19 状态 |
| 📋 下一步 | Phase 2 启动 M21（页面级 MVCC）|

**2026-06-03 M20 零拷贝 SlottedPageRef 完成**：

| 状态 | 内容 |
|------|------|
| ✅ 已完成 | 闭包 API：`with_page_data` / 改造 `read_tuple_from_data_page` / `find_visible_version` |
| ✅ 已完成 | 删除编译不过的 `get_page_ref`（L022 记录 3 次失败） |
| ✅ 已完成 | 引入 `VisibilityResult<R>` 私有枚举 + `Option<F> + take()` 模式 |
| ✅ 已完成 | 3 个 Scan 执行器 + UpdateExecutor 闭包调用 |
| ✅ 已完成 | 修订 design.md 决策 3：`F: FnOnce(&[u8]) -> Result<R>`（原 `-> R` 会嵌套 Result） |
| ✅ 已完成 | 全量测试 0 失败（110 lib + 集成测试）+ cargo fmt 12 文件 + M20 范围内 clippy 0 warning |
| ✅ 已完成 | 性能对比（before-m20 baseline + after）：read 路径 -2.46% 到 -8.33%，write 路径 +3.99%（< 5% 阈值） |
| ⚠️ 部分 | ≥ 15% 提速目标**未达**（实际 4 项 read 路径改进幅度未到 15%），原因：micro_bench 行数小 + 分配器优。详见 learned/spec.md L024 |
| 📋 变更 | 走 OpenSpec change：`m20-zero-copy-slotted-page-ref`（T10 归档中） |

**2026-06-03 M36 零拷贝 ValueRef 完成**：

| 状态 | 内容 |
|------|------|
| ✅ 已完成 | 新增 `ValueRef<'a>` 零拷贝枚举（含 9 个方法 + 10 个单元测试）|
| ✅ 已完成 | 新增 `deserialize_value_refs` 借用 `&'a [u8]`（5 tag bytes 同 deserialize_tuple）|
| ✅ 已完成 | `Expression` trait 加 `evaluate_ref<'a>` 抽象方法；`evaluate` 改 trait 默认方法内部转调 |
| ✅ 已完成 | 3 个 Expression 实现补 `evaluate_ref`（Column/Constant/Parameter，Parameter 的 String 路径走 Null + M37 TODO）|
| ✅ 已完成 | 3 个 Scan 执行器闭包改造（Scan/IndexScan/IndexScanAll 各 2 路径）|
| ✅ 已完成 | 全量测试 0 失败（29 executor_test 含 2 个 M36 集成测试）+ clippy 0 M36 warning + fmt 4 文件 |
| ✅ 已完成 | Commits：ed81610/4f9a8e8/03c2deb/3ce2672/9bc8d28/75199d6/b75d307/95bb3f9/73076ac (9 个) |
| ✅ 性能验证 | after-m36 baseline 10 场景已存 `target/criterion/before-m36`（详见 learned/spec.md L025） |
| ⚠️ 限制 | 30万→0 分配 + ≥ 5% 速度未直接验证：micro_bench 用 Int 列（无 String 分配） + 无 before-m36 baseline（M36 已实施后才存）；L025 已标注 |
| ✅ 已归档 | OpenSpec change `m36-zero-copy-value-ref` → `archive/2026-06-03-m36-zero-copy-value-ref/`；增量 spec 同步到 `specs/zero-copy-value-ref/spec.md` |
| 📋 推送 | 10 commits pushed to origin/master（commit 73076ac） |

---**2026-06-03 M30 连接并发上限完成**：

| 状态 | 内容 |
|------|------|
| ✅ 已完成 | M30 连接并发 Semaphore：`Server::new(addr, db, max_connections)` + `Arc<Semaphore>` in spawn |
| ✅ 已完成 | 3 个并发压测全部通过：within-limit / over-limit queued / permit-release |
| ✅ 已完成 | 改动 4 文件 + 新增 `connection_limit_test.rs`（201 行）|
| ✅ 已完成 | 文档同步：O002 完成 + L018/L019 新增 + tasks.md M30 状态 + snapshot.md 更新 |

## 当前阶段

**Phase 1 基础设施 — 全部完成！** M41 ✅ → M30 ✅ → M38 ✅

| 里程碑 | 优化项 | 预期收益 | 状态 |
|--------|--------|---------|------|
| M41 | 事务 ID AtomicU64 | 分配延迟 100ns→10ns | ✅ 完成 (5.1 ns/op) |
| M30 | 连接并发 Semaphore | 防连接风暴 | ✅ 完成 (3 压测通过) |
| M38 | 网络 BufWriter + TCP_NODELAY | write 调用 -99% | ✅ 完成 (N→2 syscalls) |

**Phase 2 进展**：M20 ✅ (2026-06-03) → M36 ✅ (2026-06-03) → M19 ✅ (2026-06-04) → **M21 ✅ (2026-06-04, 页面级 MVCC)**；待 M21 惰性设置 + 基准测试；下一步 M37 或 M31

## 历史里程碑

**M1-M18 核心开发完成**（2026-05-24 归档）：
- ~430 tests pass, Clippy 0 warnings
- INSERT 332x faster, PK lookup 5.6x faster than SQLite
- 完整 SQL + WAL + Group Commit + 崩溃恢复 + B-Tree Split & Merge

**M41 性能优化**（2026-06-03 完成）：
- 实测分配延迟 5.1 ns/op（路线图"100ns→10ns"达成）
- 10 线程争用 4.6x 加速，100 线程 4.5x 加速
- 微基准 `benches/tx_id_bench.rs`（criterion 0.5，4 场景）
- 详见：`architecture/spec.md` ADR-009 + `optimization/spec.md` O001 + `learned/spec.md` L017

## 已知限制

- 全表扫描性能已通过 M19 DataScan 优化至 1.8-2.4x 提速
- 文件大小 ~6.5x SQLite（固定 Key + 两层索引）
- TableManager 纯内存：表定义不持久化
- BufferPool::mark_tx_aborted 是 stub
- M21 可见性摘要惰性设置待实现（`set_all_visible` 零调用者，快速路径暂不触发）

## Git 状态

- **当前分支**: master
- **最新 tag**: v0.1.0（M18 完成）
- **最近 commits**（2026-06-04 待推送）：
  - M19 T1-T5 多 commit（feat(executor): DataScan + planner routing + bench）
  - docs(openspec): archive m19-datascan-path
  - docs(snapshot/tasks): M19 状态同步
  - 提交前最近 3 commits（2026-06-03）：
    - `e644a19` style: apply rustfmt --all to clean up workspace formatting（25 文件，196/139）
    - `ee9ceee` chore(openspec): archive consolidate-m41-tx-id-atomic（归档 + 增量 spec 同步）
    - `634764d` feat(m41): add tx_id allocation micro-benchmark（M41 主变更）

## 待办与清理

- ⚠️ `git stash list` 有 `stash@{0}: Pre-merge stash: local docs updates`，是已过时的 OpenSpec 迁移前文档 stash（已被覆盖）。可手动 `git stash drop` 清理。
- 📋 Phase 1 下一步：M38（网络 BufWriter）
- 📋 Phase 2 下一步：M19（DataScan 路径）或 M20（零拷贝 SlottedPageRef）