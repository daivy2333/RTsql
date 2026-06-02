# CLAUDE.md

> 项目文档入口 | 上次更新：2026-06-02（OpenSpec 初始化）

## 项目简介

异步协程驱动的高性能嵌入式关系型数据库 - 以 Tokio 无栈协程为调度核心，实现轻量、便捷、高效的现代数据库系统。

## 技术栈

- **语言**: Rust (最新稳定版)
- **构建工具**: Cargo
- **异步运行时**: Tokio (多线程 scheduler)
- **SQL 解析**: sqlparser-rs
- **测试框架**: criterion.rs + tempfile + rusqlite
- **基准测试**: criterion.rs (6 套: micro/concurrent/scale/sqlite_compare/single/precise_compare)
- **代码格式化**: rustfmt
- **Lint**: clippy

## 文档体系

### OpenSpec（需求规范管理）

| 目录 | 用途 | 查询方式 |
|------|------|----------|
| `openspec/specs/architecture/` | 架构决策记录（ADR） | `grep "关键词" openspec/specs/architecture/spec.md` |
| `openspec/specs/rules/` | 编码规范（三大规则唯一来源） | `grep "关键词" openspec/specs/rules/spec.md` |
| `openspec/specs/learned/` | 学习记忆与踩坑档案 | `grep "关键词" openspec/specs/learned/spec.md` |
| `openspec/specs/references/` | 外部参考与依赖文档 | `grep "关键词" openspec/specs/references/spec.md` |
| `openspec/specs/optimization/` | 优化方向与技术债 | `grep "关键词" openspec/specs/optimization/spec.md` |
| `openspec/changes/` | 变更提案 | `openspec list` |

### 项目状态（日常维护）

| 文档 | 用途 | 查询方式 |
|------|------|----------|
| `.claude/docs/snapshot.md` | 项目状态快照 | `grep "关键词" .claude/docs/snapshot.md` |
| `.claude/docs/tasks.md` | 任务追踪与里程碑规划 | `grep "关键词" .claude/docs/tasks.md` |
| `.claude/docs/archive.md` | 历史归档 | `grep "关键词" .claude/docs/archive.md` |

## 读取顺序

| 场景 | 读取 | 写入 |
|------|------|------|
| 开始新会话 | CLAUDE.md → snapshot.md → tasks.md | — |
| 写新功能 | specs/rules/ + specs/architecture/ + specs/learned/ | tasks.md, specs/learned/ |
| 修复 Bug | specs/rules/ + snapshot.md + specs/learned/ | tasks.md, specs/learned/ |
| 重构 | specs/architecture/ + specs/optimization/ | specs/architecture/ |
| 记录决策 | specs/architecture/ | specs/architecture/ |
| 创建变更 | /opsx:explore 或 /opsx:propose | openspec/changes/ |

## OpenSpec 命令

| 命令 | 用途 | 何时用 |
|------|------|--------|
| `/opsx:propose` | 一步创建修改+所有规划产物 | 快速默认路径 |
| `/opsx:explore` | 探索想法，不创建产物 | 需求不明确时 |
| `/opsx:apply` | 按任务清单实施 | 准备写代码 |
| `/opsx:archive` | 归档完成的修改 | 全部工作完成 |

## 快速开始

- **开始编码前**: 阅读 `openspec/specs/rules/spec.md`
- **接手任务时**: 阅读 `.claude/docs/tasks.md` + `.claude/docs/snapshot.md`
- **回忆项目知识**: 阅读 `openspec/specs/learned/spec.md`（API路径、技巧、踩坑）
- **做技术决策后**: 更新 `openspec/specs/architecture/spec.md`
- **发现可优化点**: 记录到 `openspec/specs/optimization/spec.md`
- **探索发现新知识**: 记录到 `openspec/specs/learned/spec.md`
- **任务完成/受阻**: 更新 `.claude/docs/tasks.md` 和 `.claude/docs/snapshot.md`

## 检查清单

每次提交前确认：

- [ ] 命名清晰，揭示意图
- [ ] 函数 < 20 行，单一职责
- [ ] 无重复代码
- [ ] 无魔法数字/字符串
- [ ] 依赖显式注入
- [ ] 核心逻辑有测试覆盖
- [ ] 注释解释"为什么"
- [ ] 已运行格式化和静态分析（`cargo fmt` + `cargo clippy` + `cargo test`）
- [ ] 代码比来时更干净
- [ ] 只改必须改的代码
- [ ] 不添加未要求的功能

## Red Flags

```
❌ 假设不明确 → STOP，问
❌ 过度复杂 → 简化
❌ 改动超出请求 → 回滚
❌ 无测试变更代码 → Iron Law 违规
❌ 顺手添加功能 → Karpathy 违规
❌ Gate BLOCK 不记录 → Workflow 违规
❌ 用"实现简单"偷换"需求满足" → Requirements Integrity 违规
❌ 需求裁剪未经用户 approval → Requirements Integrity 违规
❌ 继续第 4 次相同修复尝试 → 3-Failure 违规
❌ 跳过 Verify 直接声明完成 → Verification 违规
❌ 使用"应该/大概/似乎" → Verification 违规
```

## 核心特性

- **轻量**: 单库静态链接，无外部服务依赖，运行时仅需少量线程
- **便捷**: API 简洁（`open`, `execute`, `query`），支持内存模式与持久化单文件
- **高效**: 基于协程的异步 I/O、MVCC 无锁读、零拷贝页访问、两阶段锁缓冲池
