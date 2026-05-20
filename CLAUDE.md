# CLAUDE.md

> 项目文档入口 | 上次更新：2026-05-20

## 项目简介

异步协程驱动的高性能嵌入式关系型数据库 - 以 Tokio 无栈协程为调度核心，实现轻量、便捷、高效的现代数据库系统。

## 技术栈

- **语言**: Rust (最新稳定版)
- **构建工具**: Cargo
- **异步运行时**: Tokio (多线程 scheduler)
- **SQL 解析**: sqlparser-rs
- **测试框架**: sqllogictest + proptest
- **代码格式化**: rustfmt
- **Lint**: clippy

## 文档体系

本项目使用 `.claude/docs/` 目录下的单一职责文档管理开发知识与约束。

| 文档 | 用途 | 何时读取 |
|------|------|----------|
| [rules.md](.claude/docs/rules.md) | 编码规范与行为约束 | 编码前、修改代码前 |
| [architecture.md](.claude/docs/architecture.md) | 架构决策记录 | 设计新功能、重构时 |
| [snapshot.md](.claude/docs/snapshot.md) | 项目当前状态快照 | 恢复上下文、开始新任务时 |
| [tasks.md](.claude/docs/tasks.md) | 当前任务与待办 | 需要知道"接下来做什么"时 |
| [learned.md](.claude/docs/learned.md) | 学习记忆与探索发现 | 需要回忆API路径、技巧、踩坑经验时 |
| [references.md](.claude/docs/references.md) | 外部参考资料 | 需要查阅技术细节时 |
| [optimization.md](.claude/docs/optimization.md) | 优化方向与技术债 | 优化迭代或重构前 |

## 快速开始

- **开始编码前**: 阅读 `rules.md`
- **接手任务时**: 阅读 `tasks.md` + `snapshot.md`
- **回忆项目知识**: 阅读 `learned.md`（API路径、技巧、踩坑）
- **做技术决策后**: 更新 `architecture.md`
- **发现可优化点**: 记录到 `optimization.md`
- **探索发现新知识**: 记录到 `learned.md`
- **任务完成/受阻**: 更新 `tasks.md` 和 `snapshot.md`

## 核心特性

- **轻量**: 单库静态链接，无外部服务依赖，运行时仅需少量线程
- **便捷**: API 简洁（`open`, `execute`, `query`），支持内存模式与持久化单文件
- **高效**: 基于协程的异步 I/O、MVCC 无锁读、紧凑存储格式，实现高并发与低延迟