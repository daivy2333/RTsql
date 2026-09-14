# insert-column-list-mapping Specification

## Purpose

约束 INSERT 显式列清单的映射语义：列清单 SHALL 决定值到表列的对应关系（值按清单映射落位），非法清单 SHALL 在计划期响亮拒绝。来源：MS16（change `2026-09-12-ms16-correctness-batch`）Plan Review 审计裁定并入（BH-2，2026-09-12）——实施调查实证 `InsertNode.columns` 全链路无消费点、值按表列序位置解释：乱序清单静默错位（`(v, id) VALUES (1, 5.0)` 落库 `[1, 5.0]`）、部分清单触发 `compute_tuple_size` 断言 panic（`tuple.rs:38`，exit 101）、未知列静默接受（affected 1），三者均二进制探针实证。

## ADDED Requirements

### Requirement: INSERT 显式列清单映射与校验

INSERT 语句带显式列清单时，列清单 SHALL 恰为表列集合的一个排列（每项为已知列、无重复、数量与表列数相等）；满足时每行值 SHALL 按清单到表列的映射重排后写入（键位相关语义——唯一性预检与键位类型校验——SHALL 作用于重排后的键位值）。列清单不满足排列条件（未知列、重复列、数量不符）或无清单但值行长度与表列数不符时，SHALL 在计划期以明确错误拒绝（exit 3），不发生任何存储副作用、SHALL NOT 因值数与列数不符触发序列化断言 panic。

#### Scenario: 乱序清单值正确落位

- **GIVEN** 表 `p(id INT PRIMARY KEY, v INT)`
- **WHEN** `INSERT INTO p (v, id) VALUES (1, 2)`
- **THEN** 落库行 `(2, 1)`（id=2、v=1，修复前列清单被忽略、值按表列序错位落库为 `(1, 1)`——探针实证 `[1, 5.0]` 形态）

#### Scenario: 乱序清单键位越界被键位校验拒绝

- **GIVEN** 同上表形
- **WHEN** `INSERT INTO p (v, id) VALUES (1, 5.0)`
- **THEN** 报错 `KeyTypeMismatch`（重排后 id 收 5.0，`key-column-type-conformance` 键位校验生效；修复前 5.0 错位落非键列 v、id 收 1 静默成功）

#### Scenario: 部分清单计划期拒绝（panic 消除）

- **GIVEN** 同上表形
- **WHEN** `INSERT INTO p (v) VALUES (9)`
- **THEN** 计划期明确错误拒绝（exit 3；修复前触发 `tuple.rs:38` 断言 panic、exit 101）

#### Scenario: 未知列计划期拒绝

- **GIVEN** 同上表形
- **WHEN** `INSERT INTO p (id, zz) VALUES (7, 1)`
- **THEN** 计划期明确错误拒绝（exit 3；修复前静默接受 affected 1、zz 被忽略）

#### Scenario: 无清单但值数不符计划期拒绝

- **GIVEN** 同上表形
- **WHEN** `INSERT INTO p VALUES (1)`
- **THEN** 计划期明确错误拒绝（exit 3；修复前同源 panic）

#### Scenario: 清单与表列序一致行为保持

- **GIVEN** 同上表形
- **WHEN** `INSERT INTO p (id, v) VALUES (5, 100)`
- **THEN** 落库 `(5, 100)`，与既有行为一致（既有测试列清单均按表列序书写，行为保持锚点）
