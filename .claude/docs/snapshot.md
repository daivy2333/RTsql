# 项目快照

> 最后更新：2026-06-03（M38 完成 — Phase 1 全部完成）

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

**当前阶段**：Phase 1 全部完成！进入 Phase 2 存储引擎核心优化。

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
| ✅ 已完成 | Commits：ed81610/4f9a8e8/03c2deb/3ce2672/9bc8d28/75199d6/b75d307 (7 个) |
| ⏳ 性能验证 | 详见 learned/spec.md L025（待 T9 跑完填数据） |
| 📋 变更 | 走 OpenSpec change：`m36-zero-copy-value-ref`（T10 归档中） |

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

**Phase 2 待开始**：M19 DataScan 路径 → M20 零拷贝 SlottedPageRef → M21 页面级 MVCC → M36 零拷贝 ValueRef

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

- 全表扫描性能落后 SQLite ~4x（M19 准备优化）
- 文件大小 ~6.5x SQLite（固定 Key + 两层索引）
- TableManager 纯内存：表定义不持久化
- BufferPool::mark_tx_aborted 是 stub

## Git 状态

- **当前分支**: master
- **最新 tag**: v0.1.0（M18 完成）
- **最近 3 commits**（2026-06-03）：
  - `e644a19` style: apply rustfmt --all to clean up workspace formatting（25 文件，196/139）
  - `ee9ceee` chore(openspec): archive consolidate-m41-tx-id-atomic（归档 + 增量 spec 同步）
  - `634764d` feat(m41): add tx_id allocation micro-benchmark（M41 主变更）

## 待办与清理

- ⚠️ `git stash list` 有 `stash@{0}: Pre-merge stash: local docs updates`，是已过时的 OpenSpec 迁移前文档 stash（已被覆盖）。可手动 `git stash drop` 清理。
- 📋 Phase 1 下一步：M38（网络 BufWriter）
- 📋 Phase 2 下一步：M19（DataScan 路径）或 M20（零拷贝 SlottedPageRef）