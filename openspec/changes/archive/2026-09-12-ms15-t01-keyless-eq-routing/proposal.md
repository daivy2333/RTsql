# MS15-T01: I036 键位等值过滤对无键行可达（planner 路由修复）

## Why

tasks MS15-T01 + improvements I036：MS14 分发收口达成初版前，清零 4 项用户可见正确性缺陷的第一项——对含无键行（键位 NULL/非 Int，MS10-T05 001-rework 起落库不入索引）的表按键位等值过滤静默漏行。缺陷在 planner 路由层：`WHERE <键列> = <不可键控字面量>` 经 `extract_pk_from_where`（仅 Int 字面量返回索引键，`src/executor/value.rs:82-90`）落入"非 PK WHERE"分支后，`has_pk_equality`（`src/parser/planner/query.rs:822-846`）按纯结构判定（只查键列名出现在 Eq，不问字面量可键控性）保留 `Filter(Scan)`，ScanExecutor 走 `index_manager.scan_all()` 索引遍历——无键行不入索引，不可达，静默返回空集。

缺陷现场（2026-09-12 本会话新鲜探针复现，工作区基线 179228b + MS11-T03 实施，CLI 直连实测，全部 exit 0 静默空结果、对照路径正常）：

| 表形 | 查询 | 现状 | 对照 |
|---|---|---|---|
| `t1(s STRING, n INT)` 隐式 PK=s | `WHERE s = 'x'` | `rows:[]`（应返回 `("x",1)`） | `s != 'x'` 与无 WHERE 均可见行 |
| `t2(f FLOAT, n INT)` 隐式 PK=f | `WHERE f = 5.0` | `rows:[]`（应返回 `(5.0,1)`） | `f != 1` 与无 WHERE 均可见行 |
| `t3(s TEXT PRIMARY KEY, n INT)` | `WHERE s = 'x'` | `rows:[]` | — |

此前（MS10-T05 前）键位不可键控的行被 INSERT 静默丢弃，表恒为空、缺陷不可观察；存储面修复使行落库后，路由面缺陷暴露。I036 另有 MS10-T05 001-rework 与 MS11-T03 双重独立实证（归档 change 记录）。

本 change 严格按 I036 已记录方案（用户 milestone 批准文本："不可键控字面量禁用索引路由"）实施；调查中新发现的相邻形态（见 Out of Scope 形态 2）不在本 change 范围，处置待用户裁定。

## What Changes

- 新 capability spec `planner-key-equality-routing`（3 Requirement）：
  - R1 键位等值过滤对无键行可达：简单等值、AND 组合、Float 键列 Float 字面量、声明 `TEXT PRIMARY KEY`、restart 后共 5 场景，键位等值过滤 SHALL 经数据页行内求值覆盖无键行
  - R2 可键控字面量路由保持：Int 字面量的简单等值保持 `IndexScan` 点查、AND 组合保持 `Filter(Scan)`，行为与 plan 形状不变
  - R3 既有语义零回归：Int 列 + 非 Int 字面量空结果不变（路径 DataScan 化、可观察结果不变）；全量回归零修改
- 实现：`src/parser/planner/query.rs` SELECT 单表 WHERE 路由判定扩展——`has_pk_equality` 结构遍历升级为可键控性分类；键位等值腿存在不可键控字面量（`Expr::Value` 经 `value_from_sqlparser` 后 `to_key()==None`，即 String/Float/Bool/NULL）时不再保留 `Filter(Scan)`，落入既有 OR/下推臂：含 OR → `Filter(DataScan)`，否则谓词下推 `DataScan` 行内过滤

不改变：`extract_pk_from_where` 语义与 `IndexScan` 路径、存储/索引/执行器层、plan cache 键（SQL 文本规范化）、CLI 渲染层、无键行存储语义（MS10-T05 001-rework）。

## Scenario Sketch

见 delta spec `specs/planner-key-equality-routing/spec.md`（R1×5、R2×2、R3×2，共 9 场景）。关键形态：

- 前置：隐式/声明键列表 + 种子行（含无键行）；触发键位等值 SELECT；观察行集与 plan 形状。
- 失败边界：修复前上述三探针形态静默空集 exit 0；修复后返回正确行集；可键控字面量全部既有路由形状与结果逐字节保持。

## Out of Scope

- **形态 2（本次调查新发现，待用户裁定）**：可键控 Int 字面量 + 非 Int 键列（如 Float 隐式 PK 表 `WHERE f = 5`，行值 5.0）——`extract_pk_from_where` 成功返回键 5 → `IndexScan` 点查空索引 → 静默漏行（`Value::equals` Int↔Float 隐式转换本应匹配，探针已实锤 `rows:[]`）。I036 已批准方案按字面量维度判定，不覆盖此形态。处置候选：(a) 独立后续 change，按"键列类型感知"路由（planner `register_table` 需加性传递列类型，`pipeline.rs:996-1001` 调用点已有 `ColumnType` 可用）；(b) 扩入本 change（需用户批准扩范围）。用户裁定（2026-09-12）：采纳 (a) 独立后续 change，I 项登记由 docs-maintainer 执行。
- I037（UPDATE 键位索引维护）/ I034（裸 DataScan 表头）/ I039（dump 表名保真）——MS15 其余独立 change。
- I038（GC 无键链盲区）；列-列键位等值腿（键位 NULL/非 Int 行被三值语义排除，`Filter(Scan)` 结果正确，已分析）；OR→IndexScan 优化（M21）；多列 PK；no-FROM SELECT（I035）。
