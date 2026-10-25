# legacy-format-compat Specification

## Purpose

定义旧库格式与旧 dump 的支持范围：哪些更早版本产出的数据库文件与 dump 文本属于受支持范围、在范围内能做到什么（打开、查询、写入、恢复）、范围外输入给出什么形态的明确拒绝与什么迁移路径。文件格式版本本身的契约由 `database-file-format-header` 域规定；本域规定版本之外的那一维差异——系统表目录行的布局代际。

## Requirements

### Requirement: 旧库格式的支持范围按目录行布局代际定义

受支持的旧库文件 SHALL 是**文件格式版本仍为当前支持版本、但系统表目录行属于旧布局代际**的数据库文件。目录行布局代际 SHALL 按下列形态区分：

- **G1（最旧代际）**：`__tables` 行止于表数据链尾指针（不含唯一根段、不含二级索引段、不含表类型字节）；`__columns` 行不含默认值段。
- **G2**：`__tables` 行含唯一根段，不含二级索引段与表类型字节；`__columns` 行可含默认值段。
- **G3（当前代际）**：`__tables` 行含唯一根段、二级索引段与表类型字节；`__columns` 行可含默认值段。

G1、G2、G3 SHALL 全部属于受支持范围。产品 SHALL NOT 对旧布局做格式嗅探、猜测性迁移或写时全库重排；兼容 SHALL 由读取路径按「可选尾段缺失即取默认值」实现。文件格式版本高于当前支持版本、或不含文件头的文件 SHALL 按 `database-file-format-header` 域的既有分类明确拒绝，不属于本域的支持范围。

**逐代能力矩阵**：矩阵的「覆盖层级」标注**当前证据强度**，不改变上述支持范围本身；每格的证据来源见下方证据索引。

| 代际 | 打开 | 查询 | 写入 | 完整性 | 打开时重建 | dump | 恢复 |
|---|---|---|---|---|---|---|---|
| **G1（最旧）** | 文件级样本（E1） | 文件级样本（E1） | 文件级样本（E2） | 文件级样本（E1） | 文件级样本（E3） | 文件级样本（E4） | 文件级样本（E5） |
| **G2** | 文件级样本（E7） | 文件级样本（E7） | 文件级样本（E7） | 文件级样本（E7） | 文件级样本（E7） | 文件级样本（E7） | 文件级样本（E7） |
| **G3（当前）** | 当前构建原生路径 · 非兼容证据（E8） | 当前构建原生路径 · 非兼容证据（E8） | 当前构建原生路径 · 非兼容证据（E8） | 当前构建原生路径 · 非兼容证据（E8） | 当前构建原生路径 · 非兼容证据（E8） | 当前构建原生路径 · 非兼容证据（E9） | 当前构建原生路径 · 非兼容证据（E10） |

矩阵列的含义：`dump` 指「该代际的库可被当前构建导出」；`恢复` 指「该代际的库可从伴生未决写入日志恢复出已提交行」。旧 dump **文本**的恢复属文本维度（dump 是纯 SQL 文本、不含目录行代际），由 E11 覆盖。

**证据索引**：

- **E1（G1 打开 / 查询 / 完整性）**：`tests/legacy_compat_test.rs::legacy_generation_db_is_in_supported_range`（目录行形态落在 G1、列定义可读）、`::legacy_db_opens_queries_and_writes`（打开成功、`SELECT * FROM users` 返回 3 行、写入后重开读回 4 行、完整性 error 级发现数 0）。
- **E2（G1 写入）**：`tests/legacy_compat_test.rs::legacy_db_write_upgrades_touched_table_row_to_current_layout`（写入跨过尾页容量、确实分配新页，既有已提交行与新增行在重开后都存活、完整性 0 error）；**写入后仍能再次写入**由 `tests/legacy_compat_test.rs::legacy_g1_db_accepts_two_consecutive_writes` 覆盖（同一 G1 旧库上连续两次写入，退出码均为 0，写入后查询可见：users 5 行、audit 2 行）。
- **E3（G1 打开时重建）**：`tests/legacy_compat_test.rs::legacy_db_open_rebuild_keeps_live_table_intact`（stderr 无「重建放弃」、活表行完整、完整性 0 error）；活页不进入可分配页集合的既有断言见 `tests/free_page_lifecycle_test.rs::rebuild_never_marks_live_pages_free`。
- **E4（G1 dump）**：旧构建产出的 dump 文本样本 `tests/fixtures/legacy/legacy_dump.sql`（其可恢复性由 E11 覆盖）；**当前构建导出 G1 旧库**由 `tests/legacy_compat_test.rs::current_build_dumps_legacy_g1_db` 覆盖（退出码 0、stderr 空、导出 3 表 + 全部行；导出文本可恢复到新库且行集等价）。
- **E5（G1 恢复）**：`tests/legacy_compat_test.rs::legacy_db_with_pending_wal_recovers_committed_rows`（伴生未决写入日志恢复出参考行数、种子行存活、完整性 0 error）。
- **E6（G2 行级既有覆盖）**：`src/storage/catalog.rs::legacy_layout_row_deserializes_with_empty_unique_roots`（缺唯一根段的目录行读为空唯一根）、`::legacy_row_without_kind_section_reads_as_table`（缺表类型字节读为 Table）、`::legacy_column_row_without_default_reads_none`（缺默认值段的列行读为 None）、`src/storage/data/table_manager.rs::legacy_row_with_int_unique_flag_opens_without_unique_index`（缺唯一根段的目录行打开后不建唯一索引）。
- **E7（G2 无专属断言）**：G2 形态已有**文件级样本**与断言：样本 `tests/fixtures/legacy/legacy_g2.db`（由 revision `6e8dcf5` 的旧构建在仓库之外产出，来源与再生步骤见 `tests/fixtures/legacy/README.md`）；用例 `tests/legacy_compat_test.rs::legacy_g2_catalog_rows_are_in_g2_generation`（目录行形态：含唯一根段、不含二级索引段与表类型字节）、`::legacy_g2_db_opens_queries_and_writes`（打开 / 查询 / 写入 / 完整性 + 被触及表的目录行升级）、`::legacy_g2_db_open_rebuild_keeps_live_table_intact`（打开时重建不放弃）、`::legacy_g2_dump_restores_into_current_db`（dump / 恢复往返等价）
- **E8（G3 当前构建原生路径，非兼容证据）**：当前构建自身写出的目录行走原生路径（行级往返见 `src/storage/catalog.rs::catalog_row_with_unique_roots_roundtrip`），文件级行为由既有当前格式测试套件承担。
- **E9（G3 dump，当前构建原生路径，非兼容证据）**：`tests/cli_test.rs::test_dump_pk_rendering_matches_enforced_semantics`。
- **E10（G3 恢复，当前构建原生路径，非兼容证据）**：`tests/recovery_test.rs::test_recover_commit_abort_marks`、`::test_recover_from_checkpoint`。
- **E11（旧 dump 文本维度）**：范围内 1 份（`legacy_dump.sql`）+ 范围外 3 份（`legacy_dump_non_integer_pk.sql`、`legacy_dump_non_integer_unique.sql`、`legacy_dump_null_in_not_null.sql`）；三份范围外文本的明确拒绝由 `tests/legacy_compat_test.rs` 的三个拒绝用例覆盖。

#### Scenario: 最旧代际库文件属于支持范围

- **GIVEN** 一个由 G1 代际产出、含已提交行的数据库文件
- **WHEN** 判定该文件是否落在本域声明的支持范围内
- **THEN** 判定为受支持（该文件的目录行缺少全部可选尾段，读取路径按缺省语义解析）
- **AND** 该判定不依赖任何运行时写入的文件版本标记

#### Scenario: 文件格式版本高于当前支持版本明确拒绝

- **GIVEN** 一个文件格式版本高于当前支持版本的数据库文件
- **WHEN** 打开该文件
- **THEN** 按 `database-file-format-header` 域的既有分类明确拒绝（`NewerFileVersion`），错误信息表明文件由新版创建
- **AND** 拒绝发生在任何页解析、伴生文件打开与恢复之前——该顺序由既有域 `database-file-format-header` 的「格式错误显式拒绝」与「打开顺序守卫」规定（既有用例 `tests/file_header_test.rs::test_newer_version_rejected` 覆盖拒绝分类与消息），本域不新增断言

#### Scenario: 不含文件头的旧文件明确拒绝

- **GIVEN** 一个文件格式版本引入之前创建的、不含文件头的数据库文件
- **WHEN** 打开该文件
- **THEN** 按 `database-file-format-header` 域的既有分类明确拒绝（`NotADatabase`），不做旧格式嗅探与迁移

### Requirement: 旧库文件的打开、查询与写入

受支持范围内的旧库文件 SHALL 能被打开、能被查询、能被写入。写入 SHALL NOT 降低或丢失该库中既有的已提交数据。被写入到的表，其系统表目录行 SHALL 被重写为当前布局代际（G3）；未被写入到的表 SHALL 保持其原有代际。写入后该库 SHALL 仍能打开、查询与写入，且其完整性扫描 SHALL 不产生 error 级发现。

#### Scenario: 旧代际库打开并查询

- **GIVEN** 一个 G1 代际的数据库文件，含一张已建表与若干已提交行
- **WHEN** 打开该库并查询该表
- **THEN** 打开成功，查询返回该表的全部已提交行且与写入时一致
- **AND** 打开过程不产生任何降级、放弃或需要迁移的提示

#### Scenario: 旧代际库写入并读回

- **GIVEN** 一个 G1 代际的数据库文件，含一张已建表与若干已提交行
- **WHEN** 向该表插入一行并重新打开该库查询该表
- **THEN** 新行与既有行都可查询，行的数量与内容与预期一致
- **AND** 既有已提交数据未因写入而丢失

#### Scenario: 写入使被写到的表目录行升级为当前布局

- **GIVEN** 一个 G1 代际的数据库文件，其表的目录行缺少可选尾段
- **WHEN** 该表发生目录行变更（表数据链尾指针或索引根变化）并被写回
- **THEN** 该表的目录行被重写为当前布局代际（可解析出唯一根段、二级索引段与表类型字节）
- **AND** 同库中未被写到的表仍保持其原有代际，且该库仍可打开与查询

#### Scenario: 旧代际库完整性扫描零 error

- **GIVEN** 一个受支持范围内的旧库文件（含已提交行）
- **WHEN** 对该库执行只读完整性扫描
- **THEN** 扫描完成且 error 级发现数为 0
- **AND** 该扫描的只读性质（不修改库文件内容）由既有域 `data-integrity-verification` 的「校验面与既有边界协调」规定（既有用例 `tests/check_command_test.rs::test_check_clean_db_single_line_file_unchanged` 断言校验前后库文件全字节不变）；本域对旧代际库**未新增**字节级断言——本场景的断言只覆盖 error 级发现数为 0

### Requirement: 旧库文件上的打开时重建与未决恢复

打开受支持范围内的旧库文件时，按持久目录推导可分配页集合的打开时重建 SHALL 成功：重建 SHALL NOT 把该库中仍被持久目录登记的页判成可分配页，SHALL NOT 放弃重建，既有表 SHALL 仍可查询，其完整性扫描 SHALL 不产生 error 级发现。

带未决写入记录的旧库文件被打开时，SHALL 恢复出已提交行，且恢复后的库 SHALL 可查询、完整性扫描 SHALL 不产生 error 级发现。

#### Scenario: 旧代际库打开时重建不放弃且活表完整

- **GIVEN** 一个 G1 代际的数据库文件，含一张活表（含数据页链与索引树页）与一张已被删除的表（遗留空闲页）
- **WHEN** 打开该库
- **THEN** 打开成功且不出现「重建放弃」类提示
- **AND** 活表的已提交行完整可查，其任何页都不进入可分配页集合
- **AND** 完整性扫描的 error 级发现数为 0

#### Scenario: 旧代际库带未决写入记录打开后恢复已提交行

- **GIVEN** 一个 G1 代际的数据库文件，其伴生写入日志含未决记录
- **WHEN** 打开该库
- **THEN** 打开成功并恢复出已提交行（未决记录按既有恢复语义处理）
- **AND** 恢复后该表可查询，完整性扫描的 error 级发现数为 0

### Requirement: 旧 dump 的恢复范围

旧版本导出的 dump 文本 SHALL 是纯 SQL 文本，不含自身的格式版本标记；其代际特征 SHALL 是 DDL 渲染差异（短类型名与长类型名等渲染形态）。旧 dump SHALL 能恢复到当前库：当其 DDL 与行数据被当前解析器与计划期规则接受时，恢复 SHALL 成功，且恢复出的行数据 SHALL 与源库等价。恢复 SHALL 沿用既有恢复合同（逐条自动提交、失败即停并定位到语句），本域不改变该语义。

#### Scenario: 旧代际 dump 恢复到当前库

- **GIVEN** 一个旧版本导出的 dump 文本，其 DDL 使用旧代际的类型名渲染，且行数据合法
- **WHEN** 把该 dump 恢复到新建的空库
- **THEN** 恢复成功（退出码 0），恢复出的表与行与源库等价
- **AND** 恢复出的库使用当前格式（导出按当前格式渲染长类型名），可继续写入，并可再次导出且导出内容与恢复出的库等价（证据：`tests/legacy_compat_test.rs::restored_legacy_db_accepts_further_writes` 承载「可继续写入」，`::legacy_dump_restores_into_current_db` 承载「可再次导出且等价」）

#### Scenario: 旧代际 dump 的 DDL 渲染差异不影响恢复

- **GIVEN** 一个旧版本导出的 dump 文本，其 DDL 含旧代际类型名渲染，并含日期、时间戳与布尔字面量
- **WHEN** 把该 dump 恢复到新建的空库
- **THEN** 恢复成功，各类型列的值与源库等价

### Requirement: 范围外的显式拒绝与迁移路径

不属于支持范围的输入 SHALL 给出**明确、定位到语句**的拒绝：拒绝信息 SHALL 点名拒绝原因（不是含糊错误），SHALL 给出失败语句在 dump 中的序号，SHALL 以失败退出码结束，且 SHALL NOT 静默地部分恢复后报告成功。对每一类范围外输入，产品文档 SHALL 给出可执行的迁移路径。

范围外输入至少包括：

- dump 中声明了非整数类型的 `PRIMARY KEY`；
- dump 中声明了非整数类型列的 `UNIQUE`；
- dump 中 `NOT NULL` 列的行值为 NULL。

#### Scenario: 非整数主键的旧 dump 明确拒绝

- **GIVEN** 一个旧版本导出的 dump 文本，其建表语句声明了非整数类型的 `PRIMARY KEY`
- **WHEN** 把该 dump 恢复到新建的空库
- **THEN** 恢复以失败结束，错误信息点名「非整数主键」这一拒绝原因并给出失败语句序号
- **AND** 迁移路径：去掉 dump 中的 `PRIMARY KEY` 声明后恢复——该形态与当前导出的渲染一致（当前导出对非整数主键表不渲染 `PRIMARY KEY`，见 `tests/cli_test.rs::test_dump_pk_rendering_matches_enforced_semantics`），恢复成功且行数据等价

#### Scenario: 非整数唯一列的旧 dump 明确拒绝

- **GIVEN** 一个旧版本导出的 dump 文本，其建表语句声明了非整数类型列的 `UNIQUE`
- **WHEN** 把该 dump 恢复到新建的空库
- **THEN** 恢复以失败结束，错误信息点名「唯一约束只支持整数列」这一拒绝原因并给出失败语句序号
- **AND** 迁移路径：去掉 `UNIQUE` 声明后恢复；或把被约束列改为整数类型**并同步转换该列的行值**后恢复——只改列类型而不转换行值仍被键列类型门拒绝（`key column '<列名>' expects INT, got String`，退出码 3）

#### Scenario: 非空列含空值的旧 dump 明确拒绝

- **GIVEN** 一个旧版本导出的 dump 文本，其插入语句向 `NOT NULL` 列写入 NULL
- **WHEN** 把该 dump 恢复到新建的空库
- **THEN** 恢复以失败结束，错误信息点名非空约束违例与涉及的列名，并给出失败语句序号
- **AND** 迁移路径：回填该行的 NULL 后恢复，或清理（删除）该行后恢复
