# storage-io-optimization Specification

## Purpose
TBD - created by archiving change 2026-09-05-ms08-t01-t02-pread-prefetch. Update Purpose after archive.
## Requirements
### Requirement: 页 I/O 使用位置参数调用

`FileStorage` 的页读写 SHALL 使用 `FileExt::read_exact_at` / `write_all_at`（pread64/pwrite64）完成，每次页 I/O 恰好 1 次 read/write class syscall，不再调用 `lseek`。读写的错误类型与语义（含短读报 `UnexpectedEof`）SHALL 与改造前的 `seek + read_exact/write_all` 路径一致。

#### Scenario: 页内容读写往返等价

- **GIVEN** 已打开的 `FileStorage`，已分配 ≥3 页并各自写入可识别的不同模式内容
- **WHEN** 依次 `read_page` 读回每页
- **THEN** 每页内容与写入逐字节一致，返回 `Page.id` 正确

#### Scenario: 越界读取报错语义不变

- **GIVEN** 文件长度为 N 页
- **WHEN** `read_page` 请求页 id ≥ N（读取越过文件末尾）
- **THEN** 返回 `Err`（IO 错误，短读语义），不 panic、不死循环

#### Scenario: 并发冷读不同页无错读

- **GIVEN** 已写入 ≥16 页可识别内容的 `FileStorage`，读取全部直接走 storage 层（不经 BufferPool）
- **WHEN** 16 个任务并发多次（高频交错）`read_page` 各自不同的页并校验内容
- **THEN** 每任务每次读到的内容都与该页写入的模式一致（改造前共享偏移 seek 存在错读窗口，本场景为回归守卫）

#### Scenario: 并发读写混合不串页

- **GIVEN** 多页 `FileStorage`，页 X 与页 Y 各自写入可识别模式
- **WHEN** 任务 A 反复 `write_page` 页 X，同时任务 B 并发多次 `read_page` 页 Y 并校验内容
- **THEN** B 每次读到的都是页 Y 的正确内容（改造前写路径的 seek 同样扰动共享偏移，存在读串页窗口）

#### Scenario: syscall 序列位置参数化

- **GIVEN** 一次触发若干页读的运行（如 bench 或测试进程）
- **WHEN** 用 strace 统计 `pread64/pwrite64` 与 `lseek` 调用数
- **THEN** 页读路径的 lseek 计数为 0（WAL 等其他子系统除外），页读写由 pread64/pwrite64 完成

### Requirement: 零接口零格式变更

页 I/O 改造 SHALL NOT 改变 `AsyncStorage` trait 签名、`FileStorage` 公开方法签名、磁盘页布局或任何查询结果。全部既有测试（577）SHALL 零修改通过。

#### Scenario: 既有行为全量回归

- **GIVEN** 改造完成的工作区
- **WHEN** `cargo test --all`
- **THEN** 全部既有测试零修改通过（≥577，含新增测试），无新 warning

### Requirement: DataScan 数据页链预取（可选能力，默认关闭）

`DataScanExecutor` SHALL 支持对 `next_page_id` 链后继页的预取（装入 BufferPool 缓存，使 miss 加载与当前页行处理重叠），经 `with_prefetch(true)` 显式启用。默认构造（`new`）SHALL NOT 发起任何预取——2026-09-05 实测（Iteration 001 bench）：默认路径预取为净回退（data_scan/1000 +40~47%、/10000 +17~18%，p<0.05，同套件对照组无变化；机制为暖缓存下 spawn/wake 开销大于可隐藏的加载延迟），测量结果决定默认值。启用预取时 SHALL NOT 改变行序、查询结果、可见性判定或错误语义：预取只写缓存，真实读取仍走 `with_page_data` 正常路径；预取任务的错误与结果被丢弃；同一时刻至多 1 个预取在途。全表扫描结果（无论开关）SHALL 与无预取时逐行一致。

#### Scenario: 默认构造不发起预取

- **GIVEN** 默认 `DataScanExecutor::new` 构造的扫描器
- **WHEN** 执行全表扫描
- **THEN** 全程不发起预取（默认路径与 `with_prefetch(false)` 行为一致），不承担预取调度开销
- **AND** 默认路径扫描性能与无预取基线无可分辨差异（bench p>0.05）

#### Scenario: 预取下全表扫描行序与结果等价

- **GIVEN** 已插入 N 行（N 跨 ≥3 个数据页）的表
- **WHEN** 分别用启用与禁用预取的路径执行 `DataScan` 全表扫描
- **THEN** 两者输出的行序列（含行内容与顺序）完全一致

#### Scenario: 预取不破坏谓词下推与 LIMIT 语义

- **GIVEN** 同一数据集，含带 WHERE 谓词与 LIMIT 的查询
- **WHEN** 执行扫描（预取启用）
- **THEN** 结果与改造前（无预取）一致：谓词过滤行集一致、LIMIT 截断行数一致（含 limit=0 立即结束）

#### Scenario: 链尾页不发起无效预取

- **GIVEN** 扫描推进到链尾页（`next_page_id == 0`）
- **WHEN** 该页行处理完成
- **THEN** 不对 `PageId(0)` 或任何无效页发起预取，扫描正常结束

#### Scenario: 预取与并发 miss 共存不饥饿

- **GIVEN** BufferPool miss 信号量 16 permits
- **WHEN** 扫描预取与其他并发 miss 加载同时发生
- **THEN** 预取在途数 ≤ 1，普通 miss 加载仍可获得 permits（单游标扫描预取占用不超过 1/16 预算）

#### Scenario: 预取任务错误被丢弃且不影响正确性

- **GIVEN** 预取发起后、真实读取前，目标页读取在底层失败（如 I/O 错误）
- **WHEN** 扫描继续推进到该页的真实读取
- **THEN** 错误在真实读取时显式报告（与无预取时一致），预取任务的错误不 panic、不改变错误类型

