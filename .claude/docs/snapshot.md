# 项目快照

> 最后更新：2026-05-20

## 当前状态

- **阶段**: 初始化（未开始开发）
- **状态**: 正常
- **当前里程碑**: M0 准备开始

## 项目结构

```
RTsql/
├── .claude/
│   └── docs/
│       ├── architecture.md    - 架构决策记录
│       ├── learned.md         - 学习记忆
│       ├── optimization.md    - 优化方向与技术债务
│       ├── references.md      - 外部参考资料
│       ├── rules.md           - 编码规范与行为约束
│       ├── snapshot.md        - 项目状态快照
│       └── tasks.md           - 任务清单
└── CLAUDE.md                   - 文档入口
```

**注**: 项目尚未初始化 Cargo 项目结构，下一步将创建 Rust 项目骨架。

## 技术栈

| 类别 | 技术 | 版本 |
|------|------|------|
| 语言 | Rust | 最新稳定版（建议 1.75+） |
| 构建工具 | Cargo | Rust 内置 |
| 异步运行时 | Tokio | 最新稳定版 |
| SQL 解析 | sqlparser-rs | 最新稳定版 |
| 测试框架 | sqllogictest + proptest | 最新稳定版 |
| 代码格式化 | rustfmt | Rust 内置 |
| Lint | clippy | Rust 内置 |

## Git 状态

- **当前分支**: （未初始化 git repo）
- **最近提交**: （无）
- **未提交更改**: （无）

**注**: 项目尚未初始化 git 仓库，下一步将创建 git repo。

## 关键文件

| 文件 | 作用 | 状态 |
|------|------|------|
| Cargo.toml | Rust 项目配置 | 待创建 |
| src/main.rs | 程序入口 | 待创建 |
| src/lib.rs | 库入口 | 待创建 |
| src/storage/ | 存储引擎模块 | 待创建 |
| src/executor/ | 执行引擎模块 | 待创建 |
| src/transaction/ | 事务管理模块 | 待创建 |
| src/parser/ | SQL 解析模块 | 待创建 |
| src/network/ | 网络层模块 | 待创建 |

## 最近修改

| 时间 | 文件 | 改动类型 |
|------|------|----------|
| 2026-05-20 | .claude/docs/* | 初始化文档体系 |
| 2026-05-20 | CLAUDE.md | 创建文档入口 |

## 下一步行动

1. 初始化 Rust 项目（`cargo init`）
2. 初始化 git 仓库（`git init`）
3. 开始 M0 里程碑：项目骨架，引入 Tokio