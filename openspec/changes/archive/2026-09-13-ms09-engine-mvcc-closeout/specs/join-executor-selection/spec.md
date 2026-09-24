# join-executor-selection Specification

## Purpose

约束 JOIN 执行器的算法选择与 Nested Loop Join 能力：等值 ON 保持既有 Hash Join 路径，非等值/无等值腿的 ON SHALL 经新增 NLJ 可达（解锁非等值 JOIN），选择 SHALL 为计划期启发式（不含代价模型）。来源：MS09-T02（I015 NLJ 部分；change `2026-09-13-ms09-engine-mvcc-closeout`，用户裁定 2026-09-13 非等值 JOIN 一并解锁）。

## ADDED Requirements

### Requirement: 纯等值 JOIN Hash 路径保持

ON 条件 AND 分解后全部腿均为可解析的列=列等值（既有 `extract_join_conditions` 接受面）时 SHALL 继续路由既有 Hash Join 执行器，计划形状、结果与既有行为逐字节等价。本 Requirement 不覆盖混合腿（等值 + 非等值）——混合腿按下一 Requirement 路由 NLJ（调查确认 Hash 执行器无残余谓词机制，纯等值边界即其现有能力面）。

#### Scenario: 等值 INNER JOIN 形状与结果保持

- **GIVEN** 表 r(a INT)、s(b INT) 及既有等值 JOIN 用例
- **WHEN** `SELECT ... FROM r JOIN s ON r.a = s.b`（含 AND 复合纯等值）
- **THEN** 计划选择 Hash Join，结果与现状一致（既有测试零修改锚点）

### Requirement: 非等值 JOIN 经 NLJ 可达

ON 条件含任一非等值腿（如 `<`、`>`、`<=`、`>=`、`!=`，或含字面量/表达式的腿）时 SHALL 生成 Nested Loop Join 并产出正确连接结果：对左侧输入每行与右侧输入每行的组合求值完整 ON 谓词（含混合 ON 中的等值腿），谓词为真（三值语义下非 Unknown 且非假）的组合 SHALL 进入结果。当前非等值形态的计划期拒绝（`Plan error: Unsupported expression type`，`ddl_dml.rs:74`，调查实证）SHALL 被该能力取代。

#### Scenario: 不等式 JOIN 产出语义连接结果

- **GIVEN** 表 r(a INT) 含 {1,2}、s(b INT) 含 {2,3}
- **WHEN** `SELECT r.a, s.b FROM r JOIN s ON r.a < s.b`
- **THEN** 产出 {(1,2),(1,3),(2,3)}（语义连接结果）

#### Scenario: 复合非等值条件正确评估

- **GIVEN** 同上表形
- **WHEN** `SELECT ... FROM r JOIN s ON r.a < s.b AND r.a >= 2`
- **THEN** 仅产出满足全部腿的组合 {(2,3)}（多腿 AND 全部评估）

#### Scenario: 空输入与空结果

- **GIVEN** 任一侧为空表
- **WHEN** 非等值 JOIN
- **THEN** 空结果集，不报错

### Requirement: NLJ 结果语义与三值 NULL 处理

NLJ SHALL 与 SQL 连接语义等价：ON 谓词求值遵循既有三值语义（NULL 参与 → Unknown → 该组合不进入 INNER 结果）；JOIN 类型支持面（INNER 与既有 Hash Join 已支持的类型面）SHALL 对 NLJ 对齐到实现调查确认的同一集合，超出既有支持面的 JOIN 类型（如 FULL OUTER，若现状不支持）SHALL NOT 因本 change 新增。

#### Scenario: NULL 键不进入连接结果

- **GIVEN** 表含 NULL 行
- **WHEN** 非等值 JOIN（谓词涉及 NULL 侧）
- **THEN** 该组合不产出（三值语义 Unknown 折叠为不匹配，与既有谓词语义一致）

### Requirement: 启发式选择为计划期判定

Hash 与 NLJ 的选择 SHALL 在计划期由 ON 条件结构判定（含等值腿 → Hash；仅非等值/无等值 → NLJ），SHALL NOT 引入运行时代价估算、统计信息或代价模型；选择结果 SHALL 可经既有计划观测面（plan 形状断言）验证。

#### Scenario: 选择可由计划形状断言

- **WHEN** 分别对等值与非等值 ON 构建计划
- **THEN** 计划节点分别呈现既有 Hash Join 形态与新 NLJ 形态（测试见证面）

### Requirement: 既有 JOIN 面零回归

既有 Hash Join、SemiJoin、AntiJoin、关联子查询注入路径与 JOIN 周边能力（投影、WHERE、ORDER BY、聚合组合）SHALL 全部保持；既有全量测试 SHALL 零修改通过。

#### Scenario: 全量回归零修改

- **WHEN** 默认配置运行全量测试
- **THEN** 既有基线零修改通过（既有 Hash Join 全形态用例保持绿）
