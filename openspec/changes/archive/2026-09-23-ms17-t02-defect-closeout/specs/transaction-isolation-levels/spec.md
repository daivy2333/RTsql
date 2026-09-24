# transaction-isolation-levels Specification（delta）

## ADDED Requirements

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
