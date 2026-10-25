## ADDED Requirements

### Requirement: 格式版本是产品格式契约

文件格式版本 SHALL 是**产品格式契约**：版本号只声明产品当前写出的文件布局，不用于标记运行实例、构建产物或任何其它身份语义。产品 SHALL NOT 为同一次运行引入版本比对、指纹匹配、修订固定、运行标识或产物清单一类的身份机制。

当前支持的文件格式版本 SHALL 是唯一的产品格式版本；产品 SHALL NOT 声明、读取或嗅探比它更旧的格式版本。与更早版本产出的数据库文件的兼容 SHALL NOT 通过版本号区分，而 SHALL 通过系统表目录行的布局代际区分，其支持范围由 `legacy-format-compat` 域规定。文件格式版本高于当前支持版本、或不含文件头的文件 SHALL 按既有分类明确拒绝。

#### Scenario: 唯一的产品格式版本

- **GIVEN** 当前产品写出的数据库文件，其文件头声明的格式版本为 1
- **WHEN** 判定该版本号承担的语义
- **THEN** 它只声明文件布局，不承担任何运行身份或产物身份语义（既有用例 `tests/file_header_test.rs::test_new_database_starts_with_header_and_reopens` 断言新库头的版本字段为 1）
- **AND** 产品不声明更旧的格式版本，也不按版本号对旧布局做嗅探与迁移（既有用例 `tests/file_header_test.rs::test_version_zero_rejected` 与 `::test_headerless_legacy_file_rejected` 覆盖版本字段的既有分类）

#### Scenario: 高版本文件明确拒绝（引用既有域）

- **GIVEN** 一个文件格式版本高于当前支持版本的数据库文件
- **WHEN** `Database::open` 打开
- **THEN** 按既有域 `database-file-format-header` 的「格式错误显式拒绝 / 文件由新版创建」处理（返回 `NewerFileVersion`，CLI 以退出码 1 报「文件由新版 RTsql 创建」）——该判定由既有域规定，已由既有用例 `tests/file_header_test.rs::test_newer_version_rejected` 覆盖，本 change 不作为新主张重复声明

#### Scenario: 不含文件头的旧文件明确拒绝（引用既有域）

- **GIVEN** 一个文件格式版本引入之前创建的不含文件头库文件
- **WHEN** `Database::open` 打开
- **THEN** 按既有域 `database-file-format-header` 的「格式错误显式拒绝 / 旧无头文件统一拒绝」处理（返回 `NotADatabase`，不做旧格式嗅探与迁移）——该判定由既有域规定，已由既有用例 `tests/file_header_test.rs::test_headerless_legacy_file_rejected` 覆盖，本 change 不作为新主张重复声明
