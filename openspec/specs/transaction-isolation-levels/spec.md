# transaction-isolation-levels Specification

## Purpose

约束隔离级别的配置面与 Read Committed 语义：隔离级别 SHALL 经 lib API 参数配置（默认 Repeatable Read 逐字节等价），Read Committed 模式下每条语句 SHALL 持语句开始时点的已提交视图（其他事务未提交写不可见，自身未提交写可见）。来源：MS09-T01（I014 Read Committed 部分；change `2026-09-13-ms09-engine-mvcc-closeout`，用户裁定 2026-09-13 仅 lib API 配置面）。调查基线事实：现状全部查询扫描无快照（`pipeline.rs:457/468/483/496`），RR 默认路径维持该现状、零回归；RC 为其上新增的更严格读语义。

## Requirements

### Requirement: 隔离级别经 lib API 配置

`Database` 的打开面 SHALL 提供带隔离级别参数的打开方式，取值为 `Repeatable Read`（默认）与 `Read Committed`；既有无参打开方式 SHALL 等价于 Repeatable Read 且全部既有行为逐字节不变（既有调用点零修改）。隔离级别在打开时确定，SHALL NOT 要求支持运行中切换。本 change SHALL NOT 改变默认（Repeatable Read）路径的任何可见性行为。

#### Scenario: 默认路径零回归

- **GIVEN** 经既有无参方式打开数据库
- **WHEN** 运行既有全量测试（892 基线）
- **THEN** 零修改通过，可见性行为与现状一致（无快照语义保持）

#### Scenario: Read Committed 可选打开

- **GIVEN** 以 Read Committed 打开数据库
- **WHEN** 执行 auto-commit 查询与显式事务语句
- **THEN** 可见性按 RC 语义（见下），DML 写路径行为不变

### Requirement: Read Committed 语句级已提交视图

Read Committed 模式下，每条读取语句 SHALL 基于语句开始时点的视图求值：其他事务在语句开始时尚未提交的任何变更（插入、更新、删除）SHALL 不可见；其他事务已提交的变更 SHALL 按 MVCC 语义可见（含墓碑抑制语义，见 `mvcc-tombstone-visibility`）；语句自身所属事务的未提交写 SHALL 可见（`is_visible_self` 语义）。auto-commit 单语句与显式事务内语句均按本语义；显式事务内每条语句独立取视图（语句间其他事务的提交对后续语句可见）。写路径（DML、冲突、WAL）SHALL 不因隔离级别改变，本 change SHALL NOT 引入写写冲突检测。

#### Scenario: 他事务未提交写不可见（RC 定义性场景）

- **GIVEN** 连接 A 以 RC 模式查询某表；连接 B 在显式事务内 INSERT 行 X 尚未提交
- **WHEN** 连接 A 查询
- **THEN** 行 X 不可见（现状默认路径下同一序列行 X 可见——脏读，RC 予以排除）

#### Scenario: 语句间提交立即可见

- **GIVEN** 连接 A 以 RC 模式在显式事务内 SELECT（未见行 X）；连接 B 插入行 X 并提交
- **WHEN** 连接 A 同事务内再次 SELECT
- **THEN** 行 X 可见（语句级视图，每语句独立取已提交水位）

#### Scenario: 语句间提交删除立即消失

- **GIVEN** 连接 A 以 RC 模式读得行 Y；连接 B 删除行 Y 并提交
- **WHEN** 连接 A 同事务内再次 SELECT
- **THEN** 行 Y 不可见（已提交墓碑对新鲜视图抑制整链，与 `mvcc-tombstone-visibility` 互为见证）

#### Scenario: 自身未提交写可见

- **GIVEN** RC 模式显式事务内 INSERT 行 Z（未提交）
- **WHEN** 同事务内 SELECT
- **THEN** 行 Z 可见（is_visible_self；与既有显式事务行为一致）

#### Scenario: auto-commit 单语句与默认路径行为一致

- **GIVEN** 全部变更均已提交
- **WHEN** RC 模式与默认模式执行同序列 auto-commit 查询
- **THEN** 结果一致（单语句即单视图，无可见性差异）

### Requirement: RC 重启后可见性高水位健全（checkpoint 水位持久化）

Read Committed 的语句可见性高水位以事务 id 分配器当前值为来源，SHALL 在任何重启路径下保持「分配器当前值 ≥ 重启前已分配的一切事务 id」。checkpoint 截断 WAL 使恢复观测不到前缀事务 id 时，SHALL 由 checkpoint 位点文件持久化的水位补足：位点在 LSN 捕获之后读取的分配器当前值随位点写入，恢复侧以 WAL 观测最大 id 与位点水位的最大值推进分配器。干净关闭（close → checkpoint → 截断）后重开，重启前已提交行 SHALL 对 Read Committed 语句立即可见，SHALL NOT 依赖重启后的新写入抬升水位。位点文件 SHALL 向后兼容读取：不足 16B 视为无效（既有语义），16B..24B 旧格式按无水位处理，≥24B 携带水位。Repeatable Read 路径与无位点/旧位点文件行为不受影响。

#### Scenario: 干净关闭后重开已提交行对 RC 立即可见

- **GIVEN** `open_with_isolation(ReadCommitted)` 打开的库，表 `t` 含三行已提交数据，执行 `close()`（checkpoint 截断 WAL）
- **WHEN** 以 ReadCommitted 重新打开后立即执行 `SELECT * FROM t`（不插入任何其他 DML）
- **THEN** 返回三行（修复前：分配器归零 → 高水位 0 → 0 行，需先经其他 DML 抬升水位才可见）

#### Scenario: 位点文件水位往返与旧格式兼容

- **GIVEN** `write_checkpoint_site` 以 24B（lsn u64 LE + timestamp u64 LE + tx watermark u64 LE）写入
- **WHEN** `read_site_file` 读取
- **THEN** 返回携带水位的结果；同函数读取 16B 旧格式位点返回 (lsn, timestamp) 且不携带水位；不足 16B 返回 None（既有语义不变）

#### Scenario: 重启后新事务 id 不复用

- **GIVEN** checkpoint 位点携带水位 W（≥ 重启前一切已分配 id）
- **WHEN** 重开后恢复以 max(WAL 观测最大 id, W) 推进分配器，随后执行新 INSERT
- **THEN** 新分配 id > W；新行与重启前行同库共存，同一条 RC 语句 SELECT 全部返回

#### Scenario: 既有 checkpoint/恢复语义零回归

- **GIVEN** 既有 `tests/checkpoint_test.rs`、`tests/checkpoint_redo_reduction_test.rs`、`tests/recovery_test.rs`、`tests/recovery_e2e_test.rs` 与全量套件
- **WHEN** 全量测试运行
- **THEN** 除 `checkpoint_test.rs` 直接构造/调用点的签名机械适配（断言集不变）外全部零修改通过；位点消费过滤（`≥ lsn`）、checkpoint 九步崩溃窗口次序、RR 无快照路径语义不变

### Requirement: 既有默认路径零回归

Repeatable Read（默认）路径 SHALL NOT 因 RC 支持而变化：扫描构造的快照参数、版本链遍历、墓碑处理（`mvcc-tombstone-visibility` 修复本身除外）、事务回滚与 WAL 恢复语义全部保持。既有全量测试 SHALL 零修改通过。

#### Scenario: 全量回归零修改

- **WHEN** 默认配置运行全量测试
- **THEN** 既有基线零修改通过
