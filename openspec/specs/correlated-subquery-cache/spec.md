# correlated-subquery-cache Specification

## Purpose

约束关联子查询的结果缓存语义：同一语句执行内，关联参数值相同的子查询求值 SHALL 复用缓存结果（SHALL NOT 逐行重复执行），不同参数值 SHALL 独立求值，缓存命中与直执行结果 SHALL 逐字节等价；缓存生命周期 SHALL 为单次语句执行；关联参数按名从外层行解析失败时 SHALL 显式报错点名该列（SHALL NOT 静默回落或丢弃）。来源：MS09-T04（I017；change `2026-09-13-ms09-engine-mvcc-closeout`）；关联参数外层绑定失败显式报错来源 change `2026-10-15-subquery-alias-binding-and-error-prefix`（T2，防御纵深加固）；AntiJoin 并入同一要求来源 change `2026-10-15-antijoin-correlated-param-hardening`（T1，四执行器收口）。

## Requirements

### Requirement: 相同关联参数值复用子查询结果

单条语句执行内，关联子查询对相同关联参数值序列的多次求值 SHALL 复用首次执行结果（每不同参数值至多执行一次子查询）；外层 N 行中参数值重复出现时 SHALL NOT 重复执行子查询。

#### Scenario: 重复参数值不重复执行

- **GIVEN** 外层表 N 行、关联子查询按外层列关联，其中参数值 v 出现 k 次（k>1）
- **WHEN** 执行外层查询
- **THEN** 子查询对 v 恰执行一次，k 行均获得相同结果（行为可经计数探针或结果一致性验证）

#### Scenario: 全部参数值互异时行为等价

- **GIVEN** 外层 N 行参数值互异
- **WHEN** 执行
- **THEN** 子查询执行 N 次（缓存全未命中），结果与直执行一致（缓存无收益场景零回归）

### Requirement: 不同参数值独立求值

不同关联参数值序列 SHALL 对应独立缓存项，SHALL NOT 命中他值结果；NULL 参数值 SHALL 作为键的合法成分参与区分（NULL ≠ 任何非 NULL 值，NULL 键项与 NULL 键项命中由键相等语义决定并以实现调查确认的一致语义落定）。

#### Scenario: 相异参数值不串结果

- **GIVEN** 外层两行关联参数值分别为 1 与 2，子查询对 1 与 2 返回不同结果
- **WHEN** 执行
- **THEN** 两行各得各自结果（修复前逐行直执行即此结果——等价锚点）

### Requirement: 缓存结果与直执行逐字节等价

缓存命中产出的结果 SHALL 与禁用缓存直执行逐字节一致（行集、行序、列形状）；缓存 SHALL NOT 改变错误语义（子查询计划错误、类型错误按原样传播，SHALL NOT 被缓存吞没或缓存错误对象跨参数值复用）。

#### Scenario: 命中与直执行结果一致

- **GIVEN** 同一查询分别以缓存启用与直执行路径求值
- **WHEN** 对照输出
- **THEN** 结果逐字节一致（同库同数据）

#### Scenario: 子查询错误不被缓存改变

- **GIVEN** 子查询对某参数值求值报错
- **WHEN** 外层执行遇该参数值
- **THEN** 错误按既有语义传播（缓存路径不改变错误类型与文案）

### Requirement: 缓存生命周期为单次语句执行

缓存 SHALL 仅在单条语句执行内存续，语句执行结束即弃；SHALL NOT 跨语句、跨事务或跨连接存续，SHALL NOT 引入跨语句失效协议。语句执行内的数据不变性由快照语义保证（同一执行内读取面一致），SHALL NOT 因缓存引入可见性偏差。

#### Scenario: 跨语句不残留

- **GIVEN** 语句 1 以某参数值执行子查询
- **WHEN** 语句 2 修改数据后以相同参数值再次执行含子查询的语句
- **THEN** 语句 2 基于新数据求值（无跨语句缓存残留）

### Requirement: 既有子查询语义零回归

关联子查询与非关联子查询的既有语义（相关值注入、结果形状、错误面、与表达式/谓词组合）SHALL 全部保持；既有全量测试 SHALL 零修改通过。

#### Scenario: 全量回归零修改

- **WHEN** 默认配置运行全量测试
- **THEN** 既有基线零修改通过（既有子查询用例保持绿）

### Requirement: 关联参数外层绑定失败显式报错

执行器（SemiJoin/AntiJoin/ScalarSubqueryFilter/SubqueryEval）按名从外层行解析相关参数列时，未命中 SHALL 报错点名该列（`ExecutionError` 携带列名），SHALL NOT 静默回落到外层首列继续执行，SHALL NOT 静默丢弃该参数继续执行。修复前：SemiJoin/ScalarSubqueryFilter/SubqueryEval 三处 `extract_param_values` 以 `unwrap_or(0)` 回落，未命中参数获外层首列值；AntiJoin `extract_param_values` 以 `filter_map` 丢弃未命中参数（右计划参数未注入求值 NULL，且 `correlated_rows_cache` 键短项化可致跨行缓存误共享）——均为别名幻影参数静默错结果的直接放大器（ISS16/ISS17/ISS18 执行器侧）。本要求仅约束绑定期望存在而解析失败的防御路径；正常 SQL 经计划期分类后参数列 SHALL 存在。

**2026-10-15 修改（change `2026-10-15-antijoin-correlated-param-hardening` T1）**：执行器枚举由三扩展为四——AntiJoin 的 `filter_map` 静默丢弃形态补入（前 change T2 收口三处的 `unwrap_or(0)` 回落时漏掉的同构点，经其 Plan Review Finding 1 发现）；错误类型、消息文本与 58000 兜底分类四执行器同模。

#### Scenario: 外层缺列时点名报错

- **GIVEN** 直接构造的 SemiJoin/AntiJoin/ScalarSubqueryFilter/SubqueryEval 执行器，其 correlated_params 引用外层行不存在的列
- **WHEN** 执行 next()
- **THEN** 返回错误且消息点名该列，不产出任何数据行（修复前：SemiJoin/ScalarSubqueryFilter/SubqueryEval 按首列值继续产出错误行集；AntiJoin 丢弃该参数继续执行产出错结果）

#### Scenario: 正常相关查询绑定期命中

- **GIVEN** 计划期分类正确的相关 EXISTS / NOT EXISTS / WHERE 标量（含别名内层）
- **WHEN** 执行
- **THEN** 参数按外层行正确注入，结果与子查询直执行逐字节等价（与「缓存结果与直执行逐字节等价」要求同锚）
