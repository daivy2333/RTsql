## ADDED Requirements

### Requirement: 非 INTEGER PRIMARY KEY 计划期拒绝

`CREATE TABLE` 显式声明 PRIMARY KEY 的列 SHALL 声明 INTEGER 类型；声明为 FLOAT / TEXT / BOOLEAN / DATE / TIMESTAMP 的显式 PRIMARY KEY SHALL 计划期点名拒绝（错误携带能力名，对齐既有约束诚实化拒绝面），零副作用。未显式声明主键的「未声明主键 → 首列隐式主键」既有引擎语义 SHALL 零变化（INTEGER 首列重复 DuplicateKey 强制、非 INTEGER 首列行落非键行重复合法——文档化行为保持，不被本拒绝面波及）。本修复前已存在的含非 INTEGER PK 声明的存量表 SHALL 继续可用：打开、读写、删除、dump 正常，行集不变。

#### Scenario: 显式非 INTEGER PK 建表拒绝

- **GIVEN** 空库
- **WHEN** `CREATE TABLE s (name TEXT PRIMARY KEY, x INTEGER)`（FLOAT / BOOLEAN / DATE / TIMESTAMP PK 同型矩阵）
- **THEN** 计划期点名拒绝 exit 3，零副作用（表不存在）；错误消息点名能力
- **AND** `CREATE TABLE t (id INTEGER PRIMARY KEY, ...)` 全形态（含既有类型拼写面）保持可达

#### Scenario: 隐式主键语义零变化

- **GIVEN** `CREATE TABLE t (a INTEGER, b TEXT)`（无 PK 声明）与 `CREATE TABLE u (name TEXT, x INTEGER)`（无 PK 声明）
- **WHEN** 建表并插入重复首列值
- **THEN** `t` INTEGER 首列重复 DuplicateKey fail-fast；`u` TEXT 首列重复合法（与今日行为一致）

#### Scenario: 存量表继续可用

- **GIVEN** lib 通道构造的存量 TEXT PRIMARY KEY 表（含行数据）
- **WHEN** 打开、读写、dump
- **THEN** 全部正常，行集不变（拒绝面仅作用于新 DDL 计划期，不触碰存量表）
