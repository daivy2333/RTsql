# 2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog

ISS20（索引写侧原子性）与 ISS21（DDL 目录事务化）的修复批：索引的结构变更、根页 id 发布与反向映射更新构成不可分割且串行的写单元，取消/回滚后被取消语句写入的全部键从索引释放；`ALTER TABLE DROP COLUMN` 的目录窄化延后到语句提交点之后生效，被放弃或失败的 DDL 不留下目录面变更。两个 Iteration 均于 2026-10-24 Review `accepted`，6 任务全部完成（全量 2171/0/2，134 个测试目标、13 批 M19 分批串行），change 已收尾归档。新增行为规格域 `index-write-side-atomicity`（4 Requirement/5 Scenario）与 `ddl-catalog-transactional`（3 Requirement/4 Scenario）。本 change 不对应任何 MSxx；ISS20 / ISS21 的关闭由 openspec-experience-recorder 执行。
