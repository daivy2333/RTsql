# 项目快照

> 最后更新：2026-05-20

## 当前状态

- **阶段**: M0 完成（项目骨架已建立）
- **状态**: 正常
- **当前里程碑**: M1 准备开始

## 项目结构

```
RTsql/
├── Cargo.toml              # Rust 项目配置，含 Tokio 依赖
├── Cargo.lock              # 依赖锁定文件
├── .gitignore              # Git 忽略配置
├── CLAUDE.md               # 文档入口
├── src/
│   ├── main.rs             # 数据库服务器入口（#[tokio::main]）
│   ├── lib.rs              # 库入口，导出模块公共接口
│   ├── storage/
│   │   └── mod.rs          # 存储引擎模块（占位符）
│   ├── executor/
│   │   └── mod.rs          # 执行引擎模块（占位符）
│   ├── transaction/
│   │   └── mod.rs          # 事务管理模块（占位符）
│   ├── parser/
│   │   └── mod.rs          # SQL 解析模块（占位符）
│   └── network/
│   │   └── mod.rs          # 网络层模块（占位符）
├── tests/
│   └── runtime_test.rs     # 运行时功能验证测试（3 个测试）
└── .claude/
    └── docs/
        ├── architecture.md    - 架构决策记录
        ├── learned.md         - 学习记忆
        ├── optimization.md    - 优化方向与技术债务
        ├── references.md      - 外部参考资料
        ├── rules.md           - 编码规范与行为约束
        ├── snapshot.md        - 项目状态快照
        ├── tasks.md           - 任务清单
        └── superpowers/
            ├── specs/         - 设计规范
            └── plans/         - 实现计划
```

**注**: M0 骨架已完成，各模块为占位符，后续里程碑逐步填充实现。

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

- **当前分支**: master
- **最近提交**:
  - c8239c3 feat: Initialize RTsql project skeleton with Tokio runtime
  - 0030555 docs: Add M0 implementation plan
  - 3921ca1 docs: Add M0 design specification
- **未提交更改**: 无（working tree clean）

**注**: 项目已初始化 git 仓库，M0 骨架代码已提交。

## 关键文件

| 文件 | 作用 | 状态 |
|------|------|------|
| Cargo.toml | Rust 项目配置，Tokio 依赖 | ✅ 完成 |
| src/main.rs | 数据库服务器入口 | ✅ 完成（占位符） |
| src/lib.rs | 库入口，模块导出 | ✅ 完成 |
| src/storage/mod.rs | 存储引擎模块 | ✅ 占位符 |
| src/executor/mod.rs | 执行引擎模块 | ✅ 占位符 |
| src/transaction/mod.rs | 事务管理模块 | ✅ 占位符 |
| src/parser/mod.rs | SQL 解析模块 | ✅ 占位符 |
| src/network/mod.rs | 网络层模块 | ✅ 占位符 |
| tests/runtime_test.rs | 运行时验证测试 | ✅ 3 测试通过 |

## 最近修改

| 时间 | 文件 | 改动类型 |
|------|------|----------|
| 2026-05-20 | src/*, tests/* | M0 骨架实现 |
| 2026-05-20 | Cargo.toml | Tokio 依赖配置 |
| 2026-05-20 | .claude/docs/superpowers/* | 设计规范和实现计划 |
| 2026-05-20 | .claude/docs/* | 初始化文档体系 |
| 2026-05-20 | CLAUDE.md | 创建文档入口 |

## 下一步行动

1. 开始 M1 里程碑：文件/缓存层
2. 实现 `AsyncStorage` trait
3. 使用 `spawn_blocking` 读页
4. 实现异步 Buffer Pool