# 执行器路径错误文案双层前缀「Execution error: execution error:」（ISS11 同族残余）

- Status: closed（2026-10-15 fixed → change `2026-10-15-subquery-alias-binding-and-error-prefix` T3：`StorageError::ExecutionError` Display 去前缀、pipeline 漏斗唯一赋予单层前缀 + 锚点校准 + e2e 断言）
- Filed: 2026-10-15
- Source: 用户实测评测（2026-10-15 上手实操评测）
- Environment: Linux x86_64、rtsql 0.2.0（`~/.local/bin/rtsql` 2026-10-15 安装二进制，源自源码 HEAD `23495df`）、rustc 1.95.0-nightly (859951e3c)、隔离 RTSQL_HOME 临时目录

## 缺陷描述

ISS11（错误前缀双层包装）修复收敛了 parse/plan 漏斗点的单层赋予，但执行器子查询路径仍存在双层前缀：同一错误经文本格式渲染为 `Execution error: execution error: <message>`（首层大写、次层小写，为两个独立包装点而非重复字符串）。实测 witness：标量子查询多行错误（`Subquery returns multiple rows`）。

预期：全部错误路径单层前缀（ISS11 既定形态）。实际：子查询执行器漏斗点漏网。

位置（Inferred）：执行器错误上浆路径（`Response::Error` 或执行器 Result 包装点，与 ISS11 修复的 parse_stage 漏斗不同点）；登记时未独立追读确认。

根因：Unconfirmed。

## 影响

当前：纯错误文案质量缺陷，不影响退出码与 SQLSTATE；对终端用户是观感/排障噪音，对锁定错误文案的断言构成误报源（ISS11 同类断言校准面）。无数据影响。

## 事件记录

None（2026-10-15 用户实操评测中顺带发现）。

## 处置

无（Status: open）。修复范围预计与 ISS11 相同手法（定位该路径漏斗点单层化），可随 ISS16–ISS18 同 change 顺手收敛；独立关闭亦可。

## 证据

2026-10-15 评测实测（`RTSQL_HOME` 隔离临时目录，复用 ISS16 现场）：

```bash
rtsql --format table iss16 "SELECT a.id, (SELECT x.name FROM t x WHERE x.id = a.id) AS who FROM a"
# statement 1 of 1 failed: Execution error: execution error: Subquery returns multiple rows
#   (scalar subquery requires single row); statement: SELECT ... [exit 3]
```
