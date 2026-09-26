## Purpose

记录有证据但尚未承诺实施的改进机会。条目使用 `Ixx` 编号，按 Phase 或主题分类。批准实施后创建 OpenSpec change 并标记 `promoted`。

## Requirements

### Requirement: 改进项可评估

改进项 SHALL 包含分类、问题、证据、影响、建议和状态。

#### Scenario: 发现未排期问题

- **WHEN** 已有证据表明存在改进机会但尚未批准实施
- **THEN** 使用递增 I 编号记录

#### Scenario: 批准实施

- **WHEN** 用户批准实施改进项
- **THEN** 创建 OpenSpec change 并把原条目标记 promoted

---

## Phase 1 基础设施（已完成）

**I001-I003 全部完成**（M41/M30/M38，详见 R08-R13；原 K14-16/D09-10 随 2026-09-24 K/D 退役入清理 carrier，经 knowledge/decisions spec 底部 arc 墓碑可解析）

## Phase 2 存储引擎核心（已完成）

**I004-I008 全部完成**（M20/M19/M21/M36/M19，详见 R10-R13 与 R28 分析沉淀；原 K17-19/K22/D11-12 随 2026-09-24 K/D 退役入清理 carrier）

## Phase 3 并发控制

## I012: M42 消息传递重构

- **分类**: 架构 / 可维护性
- **问题**: 多个模块用 `Arc<Mutex<_>>` 共享状态，实为生产者-消费者模式
- **方案**: WAL→mpsc，提交→oneshot，Checkpoint→Notify，BufferPool→watch
- **依赖**: M32（I011）
- **状态**: planned（P3）
- **Legacy**: O012

## Phase 4 上层功能

## I015: M25 多 Join 算法

- **分类**: 功能 / 查询优化
- **问题**: 只有 Hash Join
- **方案**: NLJ + SMJ + 启发式选择
- **依赖**: 无
- **状态**: promoted（NLJ 部分已于 MS09-T02 实施——`NestedLoopJoin` 执行器 + 计划期启发式（纯等值 Hash 保持 / 非等值·混合·字面量腿经 NLJ，用户裁定一并解锁），spec `join-executor-selection`，change 归档 `openspec/changes/archive/2026-09-13-ms09-engine-mvcc-closeout/`；SMJ 维持未排期）
- **Legacy**: O015

## I016: M26 代价模型 + Join 重排

- **分类**: 功能 / 优化器
- **问题**: 固定 join 顺序，无 cardinality/selectivity
- **方案**: `TableStatistics` + `CostEstimator` + DP/贪心重排
- **依赖**: M25（I015）
- **状态**: planned（P4）
- **Legacy**: O016

## I020: M37 clone 消除 Arc/Cow

- **分类**: 性能 / 分配
- **问题**: `Value::clone()` 在聚合/排序/JOIN 中反复调用
- **方案**: `Value::Text` 内部 `Arc<str>` + `Cow<'_, str>` 延迟分配
- **依赖**: M20（已完成）
- **状态**: planned（MS22 候选——2026-09-24 初版后第一批路线规划，量化定稿后立项；原 P4）
- **Legacy**: O020

## I021: M39 INSERT 批量执行

- **分类**: 性能 / 写入
- **问题**: 多值 INSERT 逐行执行
- **方案**: `bulk_insert(keys)` + `append_batch(records)`
- **依赖**: M20（已完成）
- **状态**: planned（MS22 候选——2026-09-24 初版后第一批路线规划，量化定稿后立项；先 bench 逐行路径占比再决定；原 MS08-T09 排期撤销，2026-09-14 MS08 剥离退还）
- **Legacy**: O021

## Phase 5 高级优化

## I024: M23 Varint Key 编码

- **分类**: 性能 / 存储
- **问题**: 固定 32B Key，INT PK 浪费 ~28B
- **方案**: `Key` 内部 `Vec<u8>` 变长编码
- **预期**: 索引空间 ~70% 缩减
- **依赖**: 无
- **状态**: planned（MS22 候选——2026-09-24 初版后第一批路线规划，量化定稿后立项；原 MS08-T05 排期撤销，2026-09-14 MS08 剥离退还）
- **Legacy**: O024, D02 successor

## I025: M33 B+Tree 节点级锁

- **分类**: 并发 / 锁
- **问题**: 每次操作 lock 整棵树
- **方案**: Semaphore 限流 + latch coupling（crabbing protocol）
- **依赖**: M23（I024）
- **状态**: planned（P5）
- **Legacy**: O025

## I026: M35 脏页 writev 批量写回

- **分类**: 性能 / Checkpoint
- **问题**: 逐页 `write_at()` + 单独 `fsync()`
- **方案**: 连续页合并写 + `writev()` 向量化
- **预期**: Checkpoint 5-10x
- **依赖**: M31（已完成）+ M48（I013）
- **状态**: planned（MS22 候选——2026-09-24 初版后第一批路线规划，量化定稿后立项；原 MS08-T03 排期撤销，2026-09-14 MS08 剥离退还）
- **Legacy**: O026, D12 下游

## I028: M45 io_uring 批量提交

- **分类**: 性能 / I/O
- **问题**: `tokio::fs` 底层 `spawn_blocking`，每次 I/O 一次 syscall
- **方案**: `tokio-uring` 批量提交（IOSQE_IO_LINK）
- **预期**: I/O 延迟 -30-50%
- **依赖**: Linux 5.1+
- **状态**: planned（P5）
- **Legacy**: O028, K36

## 长期方向（未规划具体里程碑）

## I029: M46 瘦内部节点

- **分类**: 性能 / 存储
- **问题**: B+Tree 内部节点只存 separator keys，不存完整 Key
- **难度**: 高，需重构 B+Tree 分裂/合并逻辑
- **依赖**: M23（I024）
- **状态**: long-term
- **Legacy**: O029

## I030: M47 合并 Tag byte

- **分类**: 性能 / 序列化
- **问题**: Slot Tag byte 合并进 VersionHeader，省 1 byte/slot
- **难度**: 低，但影响序列化格式兼容性
- **状态**: long-term
- **Legacy**: O030

## I031: B-Tree 撕裂树运行期根修（结构感知刷盘/驱逐改造）

- **分类**: 正确性 / 存储引擎
- **问题**: 页驱逐按 LRU 而非树拓扑刷盘——checkpoint 后的运行期修改使磁盘 B-Tree 含洞（父页指向未刷盘子页）与孤儿页（裸读实测 `scan_all` 184/10000 + `InvalidPageType` 洞页）。恢复侧已由 `redo_count > 0` 时索引去信任 + 重放后重建消解（MS10-T02 R7/R8），但运行期检查点间的磁盘树仍处于撕裂状态；MS12 整库加密若引入页级 transform 将放大对磁盘树一致性的依赖
- **候选方案**: 结构感知刷盘（子树后序）/ checkpoint 树快照 / no-steal 驱逐——均为 BufferPool 驱逐策略重设计，MS10-T02 design D10 拒绝并入
- **量化支撑**: D10 恢复重建代价实测 40k 行→8.86s、160k 行→40.7s（100 页池随机访存主导；撕裂树运行期根修可同时压缩该恢复代价）
- **状态**: planned（未排期——MS22 Non-goal，撕裂增长与恢复代价量化后另行评估；原 MS08-T07 排期撤销，2026-09-14 MS08 剥离退还）

## I038: GC 对无键行版本链不可达（gc_table scan_all 盲区）

- **分类**: 资源 / GC 覆盖面
- **问题**: `TableMeta::gc_table` 经 `index_manager.scan_all()` 枚举版本链，键位不可键控的行（MS10-T05 001-rework 起落库不入索引）不在索引中——其旧版本链永不被 GC 清理
- **证据**: MS10-T05 Iteration 001 001-rework Plan Context Risks 预判 + 实施后语义成立（2026-09-09，归档 change 同上）；`gc_table` 为可选维护路径（M10）
- **影响**: 含无键行的表长期频繁 UPDATE 场景下旧版本空间不回收；无正确性影响
- **方案**: GC 增加数据页链全扫模式（不经索引）或无键链登记结构；需评估成本后独立 change
- **状态**: planned（MS22 候选——2026-09-24 初版后第一批路线规划，量化定稿后立项；原 MS08-T08 排期撤销，2026-09-14 MS08 剥离退还）

## I045: 主 specs Purpose 占位与 TBD/TODO 残留（validate --specs 持续 WARNING）

- **分类**: 文档质量 / OpenSpec 语料库
- **问题**: 多个主 spec 的 `## Purpose` 仍为 `openspec archive` 自动写入的占位句或含 TBD/TODO 标记（grep 实证 10 文件：database-file-format-header、planner-module-decomposition、dml-transaction-lifecycle、drop-table-physical-free、wal-recovery-replay-integrity、pipeline-stage-decomposition、wal-writer-handle-reuse、database-file-lock、cli-noninteractive-shell、storage-io-optimization 等）——`openspec validate --specs` 持续 WARNING（passed/failed 不受影响）；2026-09-11 起新 spec `sql-scalar-functions` 已补真实 Purpose，不再新增占位
- **证据**: MS11-T03 Iteration 001 Act Remaining Issue #3 + Plan Review 复跑 validate 输出（2026-09-11，归档 change 同上）
- **影响**: 语料库能力入口可读性下降；validate 输出噪声持续
- **方案**: 逐 spec 补写真实 Purpose（一句能力定位 + 来源 change 引用，格式对齐 `sql-transaction-statements`/`sql-scalar-functions` 先例）；纯文档工作，可一次性小 change 或随下一次 docs 收尾顺带
- **状态**: planned

## I049: WAL fsync 合并（组提交）

- **分类**: 性能 / WAL
- **问题**: 提交路径逐事务 fsync（原 MS03 原范围项，曾排期 MS08-T06 未实施）
- **方案**: 组提交 / 多事务合并 fsync
- **前置**: 做前先验证 fsync 是否真瓶颈（原 MS08 实测纪律保留）
- **状态**: planned（MS22 候选——2026-09-24 初版后第一批路线规划，量化定稿后立项；2026-09-14 MS08 剥离退还登记）

## I050: RowLockTable DashMap 化

- **分类**: 性能 / 并发
- **问题**: 行锁表为非并发友好结构（原 MS03 原范围项，曾排期 MS08-T04 未实施）
- **方案**: RowLockTable 迁移 DashMap
- **前置**: 先做 mini-bench 决定是否值得做（原 MS08 实测纪律保留）
- **状态**: planned（MS22 候选——2026-09-24 初版后第一批路线规划，量化定稿后立项；2026-09-14 MS08 剥离退还登记）

## I051: CI workflow（构建/测试自动化流水线）

- **分类**: 分发 / CI
- **问题**: 无 CI workflow——构建、测试、静态检查靠本地手工执行（原 MS14-T01 组成项，2026-09-23 用户裁定「CI 没必要」裁剪）
- **方案**: GitHub Actions workflow（build + test + clippy/fmt）；按需求重启时规划
- **状态**: planned（2026-09-23 MS14 裁剪退还登记——未排期候选，用户裁定暂不做）

## I052: GitHub Releases 预编译矩阵

- **分类**: 分发 / 发布
- **问题**: 无预编译产物分发——x86_64-linux-gnu / musl 静态 / aarch64 / macOS 矩阵（原 MS14-T01 组成项，2026-09-23 用户裁定「Release 也没必要」裁剪）
- **方案**: Releases + cargo-zigbuild 或 runner 原生矩阵 + 各平台 smoke（`rtsql --version` + 基础 CRUD）；macOS 矩阵受阻可独立收口（原 MS14 split signal 随条目保留）
- **状态**: planned（2026-09-23 MS14 裁剪退还登记——未排期候选，用户裁定暂不做）

## I053: crates.io 发布（cargo install 可用）

- **分类**: 分发 / 发布
- **问题**: 未发布 crates.io——`cargo install rtsql` 不可用（原 MS14-T01 组成项，2026-09-23 用户裁定「暂时不发布」）
- **方案**: crates.io 元数据 + 发布流程；发布不可逆性与名额届时确认
- **状态**: planned（2026-09-23 MS14 裁剪退还登记——未排期候选，用户裁定暂不发布）

## I054: TTL 密钥缓存（sudo 式密钥管理层）

- **分类**: 安全便利层 / 密钥管理
- **问题**: 派生密钥每次打开需重输（原 MS12-T02 组成项：TTL 缓存 `~/.rtsql/keys/` 0600 + 过期重问，2026-09-23 用户裁定初版最小加密不含便利层）
- **方案**: 派生密钥（非明文）落盘 TTL 缓存 + 过期重问；依赖 MS17-T01 最小加密先行
- **状态**: planned（2026-09-23 MS12 裁剪退还登记——未排期候选，初版最小集外）

## I055: `rtsql key set/remove/status` 密钥管理子命令

- **分类**: 安全便利层 / 密钥管理 CLI
- **问题**: 无密钥管理子命令（原 MS12-T02 组成项，2026-09-23 裁剪）
- **方案**: key 三子命令（set/remove/status）；依赖 MS17-T01 与 I054 的密钥面定型
- **状态**: planned（2026-09-23 MS12 裁剪退还登记——未排期候选）

## I056: `--password-file <path>` 密钥通道

- **分类**: 安全便利层 / 密钥通道
- **问题**: 初版仅 `--key` 与 `RTSQL_KEY` 两通道（原 MS12-T02 三通道之一被裁，2026-09-23）；脚本场景可暂以 `RTSQL_KEY` 环境变量兜底
- **方案**: 补 `--password-file <path>` 通道；依赖 MS17-T01
- **状态**: planned（2026-09-23 MS12 裁剪退还登记——未排期候选）

## I057: 加密性能正式 bench 基线

- **分类**: 性能 / 加密（MS08「先量化再决定」纪律随条目保留）
- **问题**: 加密后打开延迟与吞吐影响仅 Act Response 顺带实测记录（MS17-T01），无正式 bench 基线设施（原 MS12-T03 组成项，2026-09-23 裁剪）
- **方案**: 加密开关两态对比 bench（open 延迟 / CRUD 吞吐，`--save-baseline` 纪律）；依赖 MS17-T01 落地后实施
- **状态**: planned（2026-09-23 MS12 裁剪退还登记——未排期候选，先量化再决定）

## I058: man 页生成（clap_mangen）

- **分类**: 分发 / 文档
- **问题**: 无 man 页（原 MS14-T01 组成项，2026-09-23 裁剪——v0.1 文档职责由双语 README 承担）
- **方案**: clap_mangen 从 Command 定义构建期生成 man 页；与 I052 发布形态一并规划为宜
- **状态**: planned（2026-09-23 MS14 裁剪退还登记——未排期候选）

## I060: WAL 与 checkpoint 伴生文件加密

- **分类**: 安全 / 存储
- **问题**: MS17-T01 只加密主数据库文件；`.wal` 仍含可重放记录，`.checkpoint` 仍公开位点与事务水位。加密数据库运行期间，伴生文件可能泄露表/行内容或恢复元数据
- **方案**: 独立 change 评估 WAL 帧与 checkpoint 载荷的加密/认证格式，明确密钥派生、nonce/tag、格式协商、旧明文库兼容、错误密钥拒绝与恢复失败边界；先确认威胁模型和兼容要求，再决定是否实施
- **状态**: planned（2026-09-24 MS17 初版收尾登记；用户裁定为初版非目标，未排期）

## I064: WAL 提交持久化语义文档化（每 commit 一次 write_batch + sync_all）

- **分类**: 文档 / 持久化语义
- **问题**: WAL 写路径每 batch 追加后整体 `sync_all`（`src/wal/writer.rs:188-211`，含 LSN+CRC32），提交路径每语句一次 write_batch → 语句级持久；该保证未在用户文档明示——对比测量中 INSERT ~52x 优势易被误读为「靠丢耐久换速度」（实际为单日志单次 fsync 对 SQLite 回滚日志的多文件多 fsync）
- **证据**: 代码面 `src/wal/writer.rs:72-79/188-211`；2026-09-24 对比测量（README 对比板块）
- **影响**: 无行为影响；文档缺失使性能声明缺乏耐久性语境、易受质疑
- **方案**: README/文档明确「每语句 commit 同步 fsync WAL，断电不丢已提交事务」及 checkpoint 的边界分工；纯文档小项，随下次文档变更顺带
- **状态**: planned（2026-09-24 登记随带）

## I065: 扫描结果流式化/分页（Response 物化架构，远期）

- **分类**: 性能 / 执行器与响应架构（long-term）
- **问题**: 全表扫描 1k 行 297µs vs SQLite 98µs（剔除 SQLite 侧 prepare 不对称后真实差距更大）——火山执行器逐行 MVCC 判定后将结果整体物化 `Vec<Vec<Value>>` 进 Response，大结果集的内存与时延随行数线性放大
- **证据**: 2026-09-24 SQLite 对比测量（README 对比板块、R26 runbook）；Response 物化架构见 SNAPSHOT pipeline 描述
- **影响**: 大表导出/分析吞吐受限；当前嵌入式单机与 agent 分析负载规模下不构成实际瓶颈
- **方案**: 远期评估流式响应（分页/chunk 协议）——牵动 `network::protocol::Response`、CLI 渲染与 plan cache 交互面，需独立设计与 change；先量化真实负载中全扫占比再决定
- **状态**: planned（long-term，2026-09-24 登记未排期）

## I066: 分配器评估与切换（jemalloc/mimalloc）

- **分类**: 性能 / 分配器
- **问题**: 当前使用系统默认分配器（glibc malloc，SNAPSHOT 技术栈无自定义分配器依赖）——执行器热路径大量 String/Vec 小分配（扫描物化 `Vec<Vec<Value>>`、JOIN/聚合 clone、sqlparser/plan 构造）的吞吐与 RSS 受分配器行为影响；原 tasks 长期方向 K37，2026-09-24 用户指令转入台账
- **证据**: tasks 长期方向 K37（原有记录）；2026-09-24 SQLite 对比测量（RSS 16.7MiB、扫描吞吐差距——分配器因素为推断，未单独 profile）
- **影响**: 分配密集路径的吞吐与常驻内存；无正确性影响
- **方案**: 先 profile 分配热点（MS08「先量化再决定」纪律），再评估 jemalloc/mimalloc 以 feature-gated 依赖引入（crate 化后可作为微内核拓扑的可选件 feature，呼应 R25）；与 I020（clone 消除）、I065（流式化减少物化）互补——先减分配次数还是先换分配器，以 profile 数据定序
- **状态**: planned（MS22 候选——2026-09-24 初版后第一批路线规划，量化定稿后立项）

## I067: REPL 交互模式与元命令

- **分类**: 功能 / CLI 交互形态
- **问题**: 无 REPL（readline/多行输入/历史，无任何 stdin 读取代码）与元命令（`.tables`/`.schema`/`.mode`/`.import`/`.dump`/`.quit` 等）；非交互生命周期子命令已覆盖 `.tables`/`.schema`/`.dump` 的多数场景（list/schema/dump），交互式探索场景仍缺位
- **证据**: R18 主题 3 差距清单两行（REPL「中：新 cli/ 模块；执行后端复用 lib（不经网络）」、元命令「小-中：REPL 层实现；.dump 需语句反序列化」）+ 主题 7 用户决策记录（2026-09-05：非交互优先，REPL 不是核心可作后续可选）
- **影响**: 交互式使用/教学场景不可用；agent/脚本场景不受影响（非交互面已完整）
- **方案**: 新 `src/cli/` REPL 模块（readline/多行/历史），执行后端复用 lib；元命令逐个独立验收（`.open`/`.databases` 见 R18 主题 6 设计空间表）
- **状态**: planned（用户裁定 2026-09-05 降为后续可选，未排期）

## I069: DECIMAL/BLOB 列类型

- **分类**: 功能 / 类型系统
- **问题**: 列类型 6 类（Int/String/Float/Bool + MS13 的 Date/Timestamp），无 DECIMAL/BLOB——精确小数以 Float 近似、二进制数据无承载类型
- **证据**: R18 主题 3 差距清单「DATE/TIMESTAMP/DECIMAL/BLOB | 大：改序列化格式，深水区应靠后」；前两类已由 MS13-T01 落地，DECIMAL/BLOB 未做
- **影响**: 金额类精确计算与二进制存储场景不可用；常规分析负载不受影响
- **方案**: tuple TAG/catalog COL_TAG 扩展的格式变更深水区，参照 MS13 日期类型先例独立规划
- **状态**: planned（long-term，未排期）

## I071: PG Extended Query 与 serve 子命令复活

- **分类**: 功能 / 网络协议
- **问题**: PG 层仅 Simple Protocol，无 Extended Query（prepared statement）；`psql` 及依赖协议扩展的生态工具不可用；server 保留为库能力，无 `rtsql serve` 入口
- **证据**: R18 主题 3 差距清单「PG Extended Query | 对内置 CLI 形态优先级下降（REPL 不需要）；对 psql 生态工具仍有价值」+ 主题 4 规划输入 3（2026-09-05 用户形态决策：server 代码停止投入，将来可选 `rtsql serve` 复活；原 MS09-T03 移除）
- **影响**: 仅影响 psql 生态消费者；非交互 CLI 形态与嵌入式库用法不受影响
- **方案**: 届时随 `rtsql serve` 子命令复活一并评估；协议扩展对 server 库能力独立成立
- **状态**: planned（用户裁定降级未排期，2026-09-05/06）

## I073: 主流平台构建验证（x86_64-linux-musl 静态与 macOS）

- **分类**: 分发 / 平台验证
- **问题**: RISC-V 64 musl 静态交叉构建已验证（`build-riscv64-musl.sh`，change `2026-09-24-riscv64-musl-build-artifacts`），但 x86_64-linux-musl 全静态与 macOS（`FileExt` 理论可用）均未实测，macOS 下测试套件未运行过；安装脚本宣称目标 Linux/macOS
- **证据**: R18 主题 8 事实（musl 全静态可行——「本机未装 musl target，未实测」）+ 未确认项 8（musl 全静态与 macOS 构建未实测）
- **影响**: macOS 支持无验证记录；musl 静态是容器/scratch 部署前提
- **方案**: 装对应 rustup target 实测 cargo build + 测试套件 + CLI 冒烟，结果回写 README/分发文档；与 I052（预编译矩阵）相关但独立——本条是平台可编译性验证，I052 是发布设施
- **状态**: planned（未排期）

## I074: workspace crate 化路线 B（微内核数据库形态）

- **分类**: 架构 / 模块化（long-term 方向）
- **问题**: 单 crate 单体（src/ 11 模块）；两条真实依赖环（database→pipeline→executor→database、storage↔transaction 经 VersionHeader）、Response 错层（core 反向依赖 network）、99 处 `pub(crate)` 可见性耦合——可选件（net/crypto/cli）feature 化与多拓扑组合在单体上不可达
- **证据**: R25 分析（2026-09-24 读码：F1 依赖图谱 / F2 依赖环 / F3 词表熔接点 Value/PhysicalPlan≈20 变体/ColumnType / F4 现成接缝 AsyncStorage/REGISTRY / F6 Response 错层 / F7 加密 feature-ready / F8 无 CI）+ 四刀迁移顺序草图（词表下沉→存储域→语言域→组装层+可选件）；2026-09-24 用户裁定初版后启动路线 B
- **影响**: 纯结构重构，无行为收益；断环可能重塑执行器装配协议，返工成本以「周」计（R25 边界段）
- **方案**: 按 R25 四刀顺序，每刀独立验收全量 GREEN→GREEN、磁盘格式/行为零变化硬锚；启动前置条件包含 I051（CI）落地或等价的本地 feature 组合门纪律（F8：组合无人跑则缺陷无人发现）；`PhysicalPlan` 词表化 vs trait 化为第三刀单点架构决策需专项设计；matklad 告诫 crate 墙有真实成本——每刀以「边界值得墙」准入，两刀后停留也是合法终态；与 I059（多文件会话抽象）、I066（可选件 feature 化）顺风
- **状态**: planned（2026-09-24 用户裁定初版后启动，未排期）

<!-- arc: ARC-202609092322 --> 7 条已归档 (2026-09-09) → openspec/changes/archive/2026-09-09-ARC-202609092322/proposal.md
<!-- arc: ARC-202609241843a --> 2 条已归档 (2026-09-24) → openspec/changes/archive/2026-09-24-ARC-202609241843a/proposal.md

<!-- arc: ARC-202609242151 --> 24 条已归档 (2026-09-24) → openspec/changes/archive/2026-09-24-ARC-202609242151/proposal.md
