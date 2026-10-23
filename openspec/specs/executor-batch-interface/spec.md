# executor-batch-interface Specification

## Purpose

执行器批量取出接口：`Executor::next_batch(max)` 默认方法与逐行 `next` 同序——`max == 0` 返回空批且不推进内部游标；生产抽取路径是否改用批量实现由 criterion 测量决定（10000 行 DataScan 批量 256 对逐行基线平均时间至少低 10% 才保留接线），未达标保持逐行生产路径，两种路径的查询结果必须一致。MS32-T01 实施时时间门未达标，生产路径保持逐行，默认方法保留。来源：MS32-T01 / I081（change `2026-10-23-ms32-executor-batch-and-query-memory` Iteration 000，2026-10-23 收尾）。

## Requirements

### Requirement: 批量取出与逐行取出同序

执行器的批量取出 SHALL 返回与连续逐行取出相同的结果序列。批量大小为 0 时 SHALL 返回空批，且不得推进该执行器的内部游标。

#### Scenario: 分批取完

- **GIVEN** 一个会产出 5 行的执行器
- **WHEN** 以批量大小 2 连续取出，直到空批
- **THEN** 依次得到 2 行、2 行、1 行，然后空批，行内容与逐行取出一致

#### Scenario: 批量大小为零

- **GIVEN** 一个尚未取出的执行器
- **WHEN** 以批量大小 0 取出
- **THEN** 得到空批，随后的逐行取出仍从第一行开始

### Requirement: 批量生产路径不改变查询结果

若测量达标并启用 DataScan 的批量生产路径，经公共执行入口执行的查询 SHALL 与逐行路径返回相同的行和错误。未达标时 SHALL 保持逐行生产路径，默认批量取出仍然满足上一要求。

#### Scenario: 无 WHERE 的全表查询

- **GIVEN** 一张含已知行的表，且没有 WHERE
- **WHEN** 执行 `SELECT * FROM` 该表
- **THEN** 行集合与启用批量路径之前一致

#### Scenario: 测量未达标

- **GIVEN** 1 万行 DataScan、批量大小 256 的基准平均时间没有比逐行基线低至少 10%
- **WHEN** 结束本轮实现
- **THEN** 生产抽取仍走逐行，查询结果不变
