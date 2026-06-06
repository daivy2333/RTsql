# CLAUDE.md

> 项目文档入口 + 规则唯一事实来源 | 最后更新：2026-06-06（M31 完成 + 文档全同步）
> 上次更新：2026-06-03（规则整合升级 v2.0）

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
| `openspec/specs/learned/` | 学习记忆与踩坑档案 | `grep "关键词" openspec/specs/learned/spec.md` |
| `openspec/specs/references/` | 外部参考与依赖文档 | `grep "关键词" openspec/specs/references/spec.md` |
| `openspec/specs/optimization/` | 优化方向与技术债 | `grep "关键词" openspec/specs/optimization/spec.md` |
| `openspec/specs/{delta}/` | 增量规范（按需 M19/M20/M21/M31/M36/M41） | `ls openspec/specs/` |
| `openspec/changes/` | 变更提案 | `openspec list` |

> 注：原 `openspec/specs/rules/` 已废弃（2026-06-03）。规则全文已整合到本文"规则（唯一事实来源）"章节。

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
| 写新功能 | 编码规范（本文 § 二）+ architecture + learned | tasks.md, learned |
| 修复 Bug | 编码规范（本文 § 二）+ snapshot.md + learned | tasks.md, learned（踩坑） |
| 重构 | architecture + optimization + 编码规范（本文 § 二） | architecture |
| 记录决策 | architecture | architecture |
| 创建变更 | /opsx:explore 或 /opsx:propose | openspec/changes/ |

## OpenSpec 命令

| 命令 | 用途 | 何时用 |
|------|------|--------|
| `/opsx:propose` | 一步创建修改+所有规划产物 | 快速默认路径 |
| `/opsx:explore` | 探索想法，不创建产物 | 需求不明确时 |
| `/opsx:apply` | 按任务清单实施 | 准备写代码 |
| `/opsx:archive` | 归档完成的修改 | 全部工作完成 |

## 快速开始

- **开始编码前**: 阅读本文"规则"章节（特别是一~四）
- **接手任务时**: 阅读 `.claude/docs/tasks.md` + `.claude/docs/snapshot.md`
- **回忆项目知识**: 阅读 `openspec/specs/learned/spec.md`（API路径、技巧、踩坑）
- **做技术决策后**: 更新 `openspec/specs/architecture/spec.md`
- **发现可优化点**: 记录到 `openspec/specs/optimization/spec.md`
- **探索发现新知识**: 记录到 `openspec/specs/learned/spec.md`
- **任务完成/受阻**: 更新 `.claude/docs/tasks.md` 和 `.claude/docs/snapshot.md`

## 核心特性

- **轻量**: 单库静态链接，无外部服务依赖，运行时仅需少量线程
- **便捷**: API 简洁（`open`, `execute`, `query`），支持内存模式与持久化单文件
- **高效**: 基于协程的异步 I/O、MVCC 无锁读、零拷贝页访问、两阶段锁缓冲池

---

# 规则（唯一事实来源）

> **本节是项目规则的唯一事实来源**（取代了已废弃的 `openspec/specs/rules/`）。所有 agent 必须遵守。

## 一、Karpathy Guidelines（行为约束）

### 1. Think Before Coding

**不假设。不隐藏困惑。暴露权衡。**

实现前：
- 明确陈述假设，不确定就问
- 多种解读存在时，全部呈现 - 不 silently 选择
- 更简单的方法存在时，说出来。必要时 push back
- 不清楚时，STOP。命名困惑点。问。

### 2. Simplicity First

**最小代码解决问题。无投机性功能。**

- 不添加未被要求的功能
- 单次使用代码不抽象
- 未要求的"灵活性"或"可配置性"不加
- 不可能场景的错误处理不加
- 200 行能减到 50 行，重写

问自己："资深工程师会说这过度复杂吗？" 是 → 简化

### 3. Surgical Changes

**只改必须改。只清理自己的烂摊子。**

编辑现有代码：
- 不"改进"相邻代码、注释、格式
- 不重构没坏的东西
- 匹配现有风格，即使你做法不同
- 注意到无关死代码，提及 - 不删除

改动创建孤儿时：
- 删除 YOUR 改动导致未用的 import/变量/函数
- 不删除先前存在的死代码（除非被要求）

测试：每行改动应直接追溯到用户请求

### 4. Goal-Driven Execution

**定义成功标准。循环直到验证。**

将任务转化为可验证目标：
- "添加验证" → "写无效输入测试，然后让它们通过"
- "修复 bug" → "写复现它的测试，然后让它通过"
- "重构 X" → "确保前后测试都通过"

多步任务，简述计划：
1. [步骤] → verify: [检查]
2. [步骤] → verify: [检查]
3. [步骤] → verify: [检查]

强成功标准 → 可独立循环。弱标准需要不断澄清。

### 5. Requirements Integrity

**不裁剪用户需求。未经 approval 不得放弃。**

- 用户明确要求的所有功能必须实现
- 简化实现 ≠ 裁剪功能
- 任何裁剪必须先报告，获 approval 后才执行
- 缺依赖、缺时间不是裁剪理由

---

## 二、务实编码原则（代码质量）

### 十大铁律

1. **命名即文档** — 精准、可读、可搜索的名称
2. **函数单一职责** — < 20行，只做一件事，无副作用
3. **DRY & 正交性** — 三次法则，模块独立
4. **显式胜于隐式** — 依赖注入，常量命名
5. **健壮边界** — 依赖抽象，核心与框架解耦
6. **可测试设计** — 纯函数优先，依赖可注入
7. **尽早重构** — 小步重构，每次提交更好
8. **务实破窗** — 看到问题立即修，不留给以后
9. **自动化检查** — 格式化、静态分析、测试覆盖
10. **注释解释意图** — 注释"为什么"，不注释"做什么"

### 项目特定规范

**命名规范**：
- 模块名：snake_case（`buffer_pool`、`slotted_page`）
- 类型名：PascalCase（`PageGuard`、`WalRecord`）
- 常量：SCREAMING_SNAKE_CASE（`MAX_RETRY_COUNT`）
- 布尔值：`is_`/`has_`/`can_`/`should_` 前缀
- 集合：复数形式（`pages`、`slots`）

**代码结构**：
- 源码目录：`src/`
- 测试目录：`tests/`（集成测试）+ 文件内 `#[cfg(test)]`（单元测试）
- 基准测试：`benches/`（criterion）
- 存储层：`src/storage/`（buffer_pool、btree、page_format、file_storage）
- 执行器：`src/executor/`（每个执行器独立文件）
- 解析器：`src/parser/`（planner、ast）

**测试规范**：
- 单元测试：`#[cfg(test)] mod tests` 在每个模块内
- 集成测试：`tests/` 目录，端到端验证
- 基准测试：criterion，6 套（micro/concurrent/scale/sqlite_compare/single/precise_compare）
- 测试命名：描述行为，非 `test1`、`test2`
- 测试覆盖：核心逻辑必须有测试

**提交规范**：
- 格式：`feat(scope): description` 或 `fix(scope): description`
- 提交前：`cargo fmt` + `cargo clippy` + `cargo test`
- 不在 git 提交中列 Claude 为共同创作者（禁 co-author）

---

## 三、Workflow Designer（流程框架）

### 核心概念

- **Phase** — 逻辑分组的工作容器（进入/退出条件明确）
- **Gate** — 检查点（PASS 或 BLOCK，BLOCK 必须记录原因）
- **Task** — 最小执行单元（可独立验证，完成必须展示证据）
- **Loop** — 重复处理（clarification / review-fix / iteration / retry）

### 执行铁律

```
1. Phase 进入前必须 Gate PASS
2. Task 开始前必须 Gate PASS
3. Task 完成必须展示证据
4. Loop 退出必须条件 PASS
5. Gate BLOCK 必须记录原因
6. 声明完成必须验证证据
```

### 工具映射

| 概念 | 工具 |
|------|------|
| Phase/Task 状态 | TaskCreate / TaskUpdate |
| Gate 检查 | AskUserQuestion + 逐项验证 |
| Loop 控制 | 条件判断 |
| 并行执行 | Agent (subagent) |
| 验证证据 | Bash + 输出展示 |

---

## 四、核心执行约束（8 条铁律）

```
1. 不探索清楚不实现（Gate 1 / BDD）
2. 不计划清楚不实现（Gate 2）
3. 不完整覆盖需求不实现（Gate 2 / Requirements Integrity）
4. 不测试通过不提交（Gate 5）
5. 不验证成功不声明（Gate 5）
6. 三次失败必须反思（Gate 6）
7. 不见证据不变更（TDD Iron Law / Gate 3）
8. 不见场景缺口不进设计（BDD 智能缺口 / Gate 1）
```

---

## 五、技能执行规则（强制）

> **本节规则专门防止"步骤可跳过"和"完成无证据"类失误。**

### 1. 强制任务化（TaskCreate）

调用任何 skill 时，**第一步必须是 TaskCreate 任务化所有 Phase 步骤**：

```
1. 读取 skill 文档所有 Phase/Step 标题
2. 每个 Phase/Step 创建一条 TaskCreate
3. 开始 Phase X 前 → TaskUpdate mark in_progress
4. 完成 Phase X 后 → TaskUpdate mark completed（带证据）
5. 跳过任何步骤 → TaskUpdate 状态 "SKIPPED: {原因}"（不允许静默跳过）
6. 最终报告前 → TaskList 检查所有任务有 completed 或 SKIPPED 状态
```

### 2. 显式记录跳过（无静默跳过）

```
❌ 禁止：跳过步骤不在报告里说明
✅ 必须：跳过任何 step 必须在最终报告里显式列出（含 N/A 原因）
✅ 必须：用户询问"完成了？"时，主动列出未做的步骤
```

### 3. 完成后自审（5 问）

完成所有 Phase 后，**声明完成前必须回答 5 问**：

```
1. 我执行了技能里的每一步吗？
2. 跳过的步骤有显式记录原因吗？
3. 关键 Gate/Loop 都通过了吗？
4. 有输出证据（命令、文件、片段）吗？
5. 报告前我读过 TaskList 确认状态吗？

任一为 NO → 不允许声明完成
```

### 4. 禁止强假设推断

```
❌ 看到部分证据就推断整体完成（如：看到目录存在就推断整个初始化做完）
✅ 必须逐项打勾，每条 step 有输出或显式记录 "N/A（原因）"
✅ 不确定时 STOP 问，不确定时倾向于"漏做"而非"早收尾"
```

### 5. CodeGraph 优先

```
CodeGraph 可用时：
  ⭐ 优先用 codegraph_explore 替代 Read + Grep
  ⭐ 一次 codegraph_explore 顶一组 search + node
  ⭐ 不要开 Explore 子 agent 读文件（浪费）
  ⭐ 看到 ⚠️ stale banner 时，对那一个文件直接 Read，其它继续信任
```

### 6. OpenSpec 集成

```
- 变更必须用 /opsx:propose 创建，不用手动操作 changes/
- 验证用 openspec validate --specs
- 归档用 openspec archive <name>
- 与 openspec-assistant 双向同步：changes/ ↔ tasks.md
```

### 7. 文件编辑铁律

```
- 更新已有文档用 Edit（精准替换），不用 Write（全量覆盖）
- 创建全新文件才用 Write
- 禁止全量覆盖导致内容丢失
```

---

## 六、检查清单

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
- [ ] 不在 commit 中列 Claude 为 co-author

---

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
❌ 静默跳过 skill 步骤 → 技能执行违规
❌ 强假设推断（看到部分就推断完成）→ 自审违规
```
