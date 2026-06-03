# 项目快照

> 最后更新：2026-06-03（规则整合升级 v2.0）

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
- `openspec/specs/{architecture,learned,references,optimization}/spec.md` — 规范文档（4 个）
- `openspec/changes/` — 变更提案（含 active + archive）
- `.claude/docs/snapshot.md` — 项目快照（本文件）
- `.claude/docs/tasks.md` — 任务追踪
- `.claude/docs/archive.md` — 历史归档
- `CLAUDE.md` — **索引入口 + 规则唯一事实来源**（356 行）

## 当前阶段

**全维度性能优化（M19-M23）**

核心短板：Full Scan 4x slower than SQLite，文件大小 6.5x larger。

| 里程碑 | 优化项 | 预期收益 | 状态 |
|--------|--------|---------|------|
| M19 | DataScan 路径 | ~2x 扫描提速 | 待开始 |
| M20 | 零拷贝读取 | ~20-30% I/O 提速 | 待开始 |
| M21 | 页面级 MVCC | ~10-15% 提速 | 待开始 |
| M22 | 预取 Prefetch | 大表 ~15-25% 提速 | 待开始 |
| M23 | Varint Key 编码 | 索引空间 ~70% 缩减 | 待开始 |

## 历史里程碑

M1-M18 核心开发完成（2026-05-24 归档）：
- ~430 tests pass, Clippy 0 warnings
- INSERT 332x faster, PK lookup 5.6x faster than SQLite
- 完整 SQL + WAL + Group Commit + 崩溃恢复 + B-Tree Split & Merge

## 已知限制

- 全表扫描性能落后 SQLite ~4x
- 文件大小 ~6.5x SQLite（固定 Key + 两层索引）
- TableManager 纯内存：表定义不持久化
- BufferPool::mark_tx_aborted 是 stub

## Git 状态

- **当前分支**: master
- **最新 tag**: v0.1.0（M18 完成）