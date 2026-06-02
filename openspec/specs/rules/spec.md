# Rules — 编码规范

> 版本：v1.0 | 最后更新：2026-06-02
> 由 openspec-init 从 `.claude/docs/rules.md` 迁移。
> 这是三大规则的唯一事实来源。CLAUDE.md 只做索引。

---

## Purpose

定义 RTsql 项目的编码规范、行为约束和工作流程框架。整合 Karpathy Guidelines、务实编码原则和 Workflow Designer 三大规则体系，确保代码质量和开发效率。

---

## Requirements

### Requirement: 代码符合命名规范

所有代码 SHALL 遵循项目命名规范。

#### Scenario: 新增 Rust 代码
- **WHEN** 编写新的 Rust 模块、类型或函数
- **THEN** 模块名用 snake_case，类型名用 PascalCase，常量用 SCREAMING_SNAKE_CASE，布尔值用 `is_`/`has_`/`can_` 前缀

#### Scenario: 检查命名合规
- **WHEN** 代码审查时发现命名不规范
- **THEN** 标记为违规并要求修正

### Requirement: 函数单一职责

每个函数 SHALL 只做一件事，理想长度 < 20 行。

#### Scenario: 函数过长
- **WHEN** 函数超过一屏（~50 行）
- **THEN** 拆分为多个职责单一的子函数

#### Scenario: 函数有副作用
- **WHEN** 函数修改了输入参数或产生隐式副作用
- **THEN** 重构为纯函数或显式标注副作用

### Requirement: 测试覆盖核心逻辑

所有核心逻辑变更 SHALL 有测试见证（TDD Iron Law）。

#### Scenario: 新增功能
- **WHEN** 实现新功能
- **THEN** 先写失败测试（RED），再写实现（GREEN），最后重构（REFACTOR）

#### Scenario: 修复 Bug
- **WHEN** 修复 Bug
- **THEN** 先写复现测试（RED），再修复（GREEN）

#### Scenario: 提交前检查
- **WHEN** 准备提交代码
- **THEN** 运行 `cargo fmt` + `cargo clippy` + `cargo test` 全部通过

### Requirement: 需求完整性

需求完整性 SHALL 优先于实现简化。所有需求裁剪必须用户明确 approval。

#### Scenario: 发现需求裁剪
- **WHEN** AI 模型试图用"实现简单"偷换"需求满足"
- **THEN** 立即报告，等待用户 approval 后才能继续

---

## Karpathy Guidelines（行为约束）

### 1. Think Before Coding
**不假设。不隐藏困惑。暴露权衡。**

- 实现前明确陈述假设，不确定就问
- 多种解读存在时，全部呈现，不 silently 选择
- 更简单的方法存在时说，必要时 push back
- 不清楚时立即 STOP，命名困惑点并询问

### 2. Simplicity First
**最小代码解决问题。无投机性功能。**

- 不添加未被要求的功能
- 单次使用代码不抽象
- 未要求的"灵活性"或"可配置性"不加
- 不可能场景的错误处理不加
- 若能将 200 行减到 50 行，重写

### 3. Surgical Changes
**只改必须改。只清理自己的烂摊子。**

- 不"改进"相邻代码、注释、格式
- 不重构没坏的东西
- 匹配现有风格，即使与你做法不同
- 删除 own 改动导致未用的 import/变量/函数
- 每行改动应直接追溯到用户请求

### 4. Goal-Driven Execution
**定义成功标准。循环直到验证。**

- 任务转化为可验证目标
- 多步任务简述计划：`1. [步骤] → verify: [检查]`
- 强成功标准 → 可独立循环
- 不局限于单一功能测试，而是模块级别的端到端测试

### 5. Requirements Integrity（需求完整性）
**需求完整性优先于实现简化。所有需求裁剪必须用户明确 approval。**

- 模型不得用"实现简单"偷换"需求满足"
- 需求范围的简化必须用户 explicit approval
- 违规处理：发现未经用户确认的需求裁剪 → 立即报告

---

## 务实编码原则（代码质量）

### 十大铁律

#### 1. 命名即文档
- 名称揭示意图，非实现细节
- 避免缩写、单字母变量
- 使用领域语言（Page、Buffer、Transaction）
- 布尔值用 `is_`、`has_`、`can_` 前缀
- 集合用复数形式

#### 2. 函数单一职责
- 理想 < 20 行，不宜超过一屏
- 只做一件事，做好它
- 无副作用：不修改输入参数
- 抽象层级一致

#### 3. DRY & 正交性
- 三次法则：复制两次后，第三次必须抽象
- 模块独立，一个模块的改变不影响其他模块
- 业务逻辑与基础设施分离

#### 4. 显式胜于隐式
- 依赖通过参数或构造函数显式传入
- 常量命名：用 `MAX_RETRY_COUNT` 而非 `5`
- 避免全局状态和隐式上下文

#### 5. 健壮边界
- 高层模块不依赖低层模块，都依赖抽象
- 外部依赖通过接口封装
- 核心业务与框架解耦

#### 6. 可测试设计
- 纯函数优于有状态函数
- 依赖可注入
- 先写失败测试，再写实现（TDD 推荐）

#### 7. 尽早重构
- 看到坏味道立即小步重构
- 每次提交让代码比之前更好

#### 8. 务实破窗
- 发现问题立即修复
- 先实现端到端最小可用功能，再完善
- 避免过度设计

#### 9. 自动化检查
- 提交前运行 `cargo fmt`
- 静态分析 `cargo clippy`
- 运行测试 `cargo test`

#### 10. 注释解释意图
- 好代码是自文档的
- 注释只解释"为什么"，不解释"做什么"
- 过时注释比无注释更糟

---

## Workflow Designer（流程框架）

### 核心概念
- **Phase**: 逻辑分组，有进入/退出条件
- **Gate**: 检查点，PASS/BLOCK
- **Task**: 最小执行单元，可验证
- **Loop**: 重复机制，有循环/退出条件

### 执行铁律
1. Phase 进入前必须 Gate PASS
2. Task 开始前必须 Gate PASS
3. Task 完成必须展示证据
4. Loop 退出必须条件 PASS
5. Gate BLOCK 必须记录原因
6. 声明完成必须验证证据

---

## 核心执行约束（8 条）

```
1. 不探索清楚不实现
2. 不计划清楚不实现
3. 不完整覆盖需求不实现
4. 不测试通过不提交
5. 不验证成功不声明
6. 三次失败必须反思
7. 不见证据不变更（TDD Iron Law）
8. 不见场景缺口不进设计（BDD 智能缺口）
```

---

## 项目特定规范

### 命名规范
- 模块名：snake_case（`buffer_pool`、`slotted_page`）
- 类型名：PascalCase（`PageGuard`、`WalRecord`）
- 常量：SCREAMING_SNAKE_CASE（`MAX_RETRY_COUNT`）
- 布尔值：`is_`、`has_`、`can_` 前缀
- 集合：复数形式（`pages`、`slots`）

### 代码结构
- 源码目录：`src/`
- 测试目录：`tests/`（集成测试）+ 文件内 `#[cfg(test)]`（单元测试）
- 基准测试：`benches/`（criterion）
- 存储层：`src/storage/`（buffer_pool、btree、page_format、file_storage）
- 执行器：`src/executor/`（每个执行器独立文件）
- 解析器：`src/parser/`（planner、ast）

### 测试规范
- 单元测试：`#[cfg(test)] mod tests` 在每个模块内
- 集成测试：`tests/` 目录，端到端验证
- 基准测试：criterion，6 套（micro/concurrent/scale/sqlite_compare/single/precise_compare）
- 测试命名：描述行为，非 `test1`、`test2`
- 测试覆盖：核心逻辑必须有测试

### 提交规范
- 格式：`feat(scope): description` 或 `fix(scope): description`
- 提交前：`cargo fmt` + `cargo clippy` + `cargo test`
- 不在提交中列为共同创作者

---

## 检查清单

每次提交前确认：

- [ ] 命名清晰，揭示意图
- [ ] 函数 < 20 行，单一职责
- [ ] 无重复代码
- [ ] 无魔法数字/字符串
- [ ] 依赖显式注入
- [ ] 核心逻辑有测试覆盖
- [ ] 注释解释"为什么"
- [ ] 已运行格式化和静态分析
- [ ] 代码比来时更干净
- [ ] 只改必须改的代码
- [ ] 不添加未要求的功能

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
```
