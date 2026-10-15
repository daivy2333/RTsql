# Delta: correlated-subquery-cache

## ADDED Requirements

### Requirement: 关联参数外层绑定失败显式报错

执行器（SemiJoin/ScalarSubqueryFilter/SubqueryEval）按名从外层行解析相关参数列时，未命中 SHALL 报错点名该列（`ExecutionError` 携带列名），SHALL NOT 静默回落到外层首列继续执行。修复前：三处 `extract_param_values` 以 `unwrap_or(0)` 回落，未命中参数获外层首列值，是别名幻影参数静默错结果的直接放大器（ISS16/ISS17/ISS18 执行器侧）。本要求仅约束绑定期望存在而解析失败的防御路径；正常 SQL 经计划期分类后参数列 SHALL 存在。

#### Scenario: 外层缺列时点名报错

- **GIVEN** 直接构造的 SemiJoin/ScalarSubqueryFilter/SubqueryEval 执行器，其 correlated_params 引用外层行不存在的列
- **WHEN** 执行 next()
- **THEN** 返回错误且消息点名该列，不产出任何数据行（修复前：按首列值继续产出错误行集）

#### Scenario: 正常相关查询绑定期命中

- **GIVEN** 计划期分类正确的相关 EXISTS / WHERE 标量（含别名内层）
- **WHEN** 执行
- **THEN** 参数按外层行正确注入，结果与子查询直执行逐字节等价（与「缓存结果与直执行逐字节等价」要求同锚）
