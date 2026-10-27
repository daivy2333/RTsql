# legacy-format-compat Delta

## MODIFIED Requirements

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
- **AND** 迁移路径的「去掉 `UNIQUE` 声明」形态 SHALL 与当前导出的渲染一致（当前导出对非整数唯一列不渲染 `UNIQUE`，与 `PRIMARY KEY` 同型条件）⇒ 当前导出的产物可直接被恢复，SHALL NOT 需要额外的文本改写

#### Scenario: 非空列含空值的旧 dump 明确拒绝

- **GIVEN** 一个旧版本导出的 dump 文本，其插入语句向 `NOT NULL` 列写入 NULL
- **WHEN** 把该 dump 恢复到新建的空库
- **THEN** 恢复以失败结束，错误信息点名非空约束违例与涉及的列名，并给出失败语句序号
- **AND** 迁移路径：回填该行的 NULL 后恢复，或清理（删除）该行后恢复
