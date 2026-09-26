# insert-column-list-mapping Specification

## Purpose

约束 INSERT 显式列清单的映射语义：列清单 SHALL 决定值到表列的对应关系（值按清单映射落位），非法清单 SHALL 在计划期响亮拒绝。来源：MS16（change `2026-09-12-ms16-correctness-batch`）Plan Review 审计裁定并入（BH-2，2026-09-12）——实施调查实证 `InsertNode.columns` 全链路无消费点、值按表列序位置解释：乱序清单静默错位（`(v, id) VALUES (1, 5.0)` 落库 `[1, 5.0]`）、部分清单触发 `compute_tuple_size` 断言 panic（`tuple.rs:38`，exit 101）、未知列静默接受（affected 1），三者均二进制探针实证。MS24（change `2026-09-25-ms24-write-surface-completion`）修改：清单由「恰为表列排列」放宽为子集排列，省略列按 `sql-write-surface` 的 DEFAULT/NULL 语义填充为全宽行。

## Requirements

### Requirement: INSERT 显式列清单映射与校验

INSERT 语句带显式列清单时，列清单 SHALL 为表列集合的子集排列（每项解析为互异已知列，数量允许少于表列数）；满足时每行值 SHALL 按清单到表列的映射重排为全宽行后写入，被省略的列按 `sql-write-surface` capability 的 DEFAULT/NULL 语义填充（键位相关语义——唯一性预检与键位类型校验——SHALL 作用于重排后的键位值）。列清单含未知列或重复列，或无清单但值行长度与表列数不符时，SHALL 在计划期以明确错误拒绝（exit 3），不发生任何存储副作用、SHALL NOT 因值数与列数不符触发序列化断言 panic。

#### Scenario: 乱序清单值正确落位

- **GIVEN** 表 `p(id INT PRIMARY KEY, v INT)`
- **WHEN** `INSERT INTO p (v, id) VALUES (1, 2)`
- **THEN** 落库行 `(2, 1)`（id=2、v=1，值按清单映射落位）

#### Scenario: 乱序清单键位越界被键位校验拒绝

- **GIVEN** 同上表形
- **WHEN** `INSERT INTO p (v, id) VALUES (1, 5.0)`
- **THEN** 报错 `KeyTypeMismatch`（重排后 id 收 5.0，`key-column-type-conformance` 键位校验生效）

#### Scenario: 部分清单计划期拒绝（panic 消除）

（场景名保留 MS16 历史锚点；语义自本 change 起更新为子集合法 + 全宽填充。）

- **GIVEN** 表 `p(id INT PRIMARY KEY, v INT DEFAULT 7, w INT)`
- **WHEN** `INSERT INTO p (id, v) VALUES (9, 3)` 与 `INSERT INTO p (id) VALUES (10)`
- **THEN** 子集清单合法，缺省列按 DEFAULT/NULL 填充为全宽行落库 `(9, 3, NULL)` 与 `(10, 7, NULL)`；SHALL NOT 触发序列化断言 panic（MS16 消除的 exit 101 缺陷意图保持）；NOT NULL 且无 DEFAULT 的省略列由执行器门点名拒绝

#### Scenario: 未知列计划期拒绝

- **GIVEN** 同上表形
- **WHEN** `INSERT INTO p (id, zz) VALUES (7, 1)`
- **THEN** 计划期明确错误拒绝（exit 3）

#### Scenario: 无清单但值数不符计划期拒绝

- **GIVEN** 同上表形
- **WHEN** `INSERT INTO p VALUES (1)`
- **THEN** 计划期明确错误拒绝（exit 3）

#### Scenario: 清单与表列序一致行为保持

- **GIVEN** 同上表形
- **WHEN** `INSERT INTO p (id, v, w) VALUES (5, 100, 3)`
- **THEN** 落库 `(5, 100, 3)`，与既有行为一致
