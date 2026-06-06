# 项目快照

> 最后更新：2026-06-04（M21 遗留项完成：DELETE mark_deleted + 惰性 set_all_visible + benchmark）

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

**2026-06-03 M41 + rustfmt**：M41 事务 ID AtomicU64（5.1 ns/op, 2-4.6x 加速，详见 L017）+ 25 文件重格式化（commit `e644a19`）。

**2026-06-03 M38 网络 BufWriter + TCP_NODELAY**：N+1 syscalls→2 syscalls（write+flush），11 pg_protocol tests 全过。

**当前阶段**：Phase 2 全部完成（M20 ✅ → M36 ✅ → M19 ✅ → M21 ✅），下一步 M37 / M31。

**2026-06-04 M21 页面级 MVCC**：DashMap visibility_map + 4 写路径清标志 + 延后项已完成（DELETE mark_deleted + 惰性 set_all_visible + bench）。详见 ADR-011 + L028/L030。

**2026-06-04 M19 DataScan**：数据页链表遍历，1K 1.81x / 10K 2.44x 提速（464/464 tests pass）。详见 L026。

**2026-06-03 M20 零拷贝 SlottedPageRef**：with_page_data 闭包 API，read 路径 -2.46%~-8.33%（≥15% 目标未达，详见 L024）。

**2026-06-03 M36 零拷贝 ValueRef**：ValueRef<'a> + deserialize_value_refs 借用 0 String 分配（10 commits pushed，5% 目标未直接验证，详见 L025）。

**2026-06-03 M30 连接并发 Semaphore**：3 压测通过（within-limit / over-limit / permit-release）。详见 L018/L019。

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