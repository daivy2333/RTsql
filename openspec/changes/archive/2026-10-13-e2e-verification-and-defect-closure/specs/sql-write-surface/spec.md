## ADDED Requirements

### Requirement: CREATE TABLE 失败零残留与 DEFAULT 超限计划期拒绝

CREATE TABLE 的 catalog 写入 SHALL 以「列行全部写入成功 → 表行最后写入」为序（表行存在性 = 建表 commit point）；任何一步失败 SHALL 零残留——内存不注册半表、catalog 无可用半表（孤儿行清理）、已分配数据页/索引页释放、同名建表可立即重试。DEFAULT 字面量序列化超出 catalog 列行页内容量时 SHALL 计划期点名拒绝（文案点名 DEFAULT 与过大语义及列名），零副作用；ALTER ADD COLUMN 同款预检。dump/restore/import 建表通道经同一写入序继承。

#### Scenario: DEFAULT 超限计划期点名拒绝

- **GIVEN** 空库
- **WHEN** `CREATE TABLE t (id INTEGER, pad TEXT DEFAULT '<5000 字符>')`
- **THEN** 计划期拒绝、文案含 DEFAULT 过大语义（点名列），exit 3 零副作用（失败边界——修复前：执行期 `Execution error: Page full`）

#### Scenario: 建表失败零残留可重试

- **GIVEN** 空库
- **WHEN** 构造建表 catalog 写入中途失败（如未预检命中的执行期页满路径）后 `schema`、`INSERT INTO t …`、`CREATE TABLE t (id INTEGER)`
- **THEN** schema 无该表、INSERT 被拒（表不存在）、同名重建成功（失败边界——修复前：残留可写半表 + 同名重建被拒）

#### Scenario: ALTER ADD COLUMN 失败零可见变化

- **GIVEN** 已有表
- **WHEN** ALTER ADD COLUMN 的 DEFAULT 超限被预检拒绝（或执行期失败）
- **THEN** 表 schema 无变化、既有读写不受影响
