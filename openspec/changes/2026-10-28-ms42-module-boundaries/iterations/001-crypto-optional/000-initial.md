# Iteration 001 / Cycle 000: crypto 可选化与关闭态行为

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: MS42 需求 R3（crypto 可选组合）及 R1 的默认组合等价不变量；proposal 默认假设 1–5 继续有效；Iteration 000 的稳定结果（features 表可用、network 门控先例、C1 组合命令面）直接依赖
- Excluded scope: cli 门控（T4）、五组合矩阵与全量（T5/T6）、`database-encryption` 既有合同的任何修改、`rand` 可选化、KDF 参数与加密格式变更、测试断言改写

**Objective**

crypto feature 门控落地后：C2（no-crypto）组合构建门与验证集通过，关闭态行为（加密头拒绝面、携带密钥拒绝面、明文全功能）由先行测试锚定；默认组合加密面（`database-encryption` 全部合同）回归通过。

**Investigation Facts**

- Starting point: HEAD `ffd7482`（Iteration 000 accepted，Review 见 `../000-feature-base-and-network/000-initial.md` Plan Review）。features 表已就绪——`crypto = ["dep:argon2", "dep:aes-gcm"]`，两依赖已 optional（T1），lock 条目保留（optional 不删 lock）。C2 组合当前不可编译：`src/storage/mod.rs:7` 的 `mod crypto;` 无条件编译，argon2/aes-gcm 在 C2 未启用。
- Current-State Evidence（本 Cycle 规划时逐点核实，基线 ffd7482）:
  - crypto 符号接缝仅三个文件：`src/storage/crypto.rs`（整模块：`derive_key` KDF、`PageCipher`、`ENCRYPTED_PAGE_RECORD_SIZE = 4124`〔:17〕、`DEFAULT_KDF_M_KIB/T/P`、`validate_kdf_params`）；`src/storage/file_storage.rs`（21 处引用——`use super::crypto::{derive_key, PageCipher, DEFAULT_KDF_M_KIB, DEFAULT_KDF_P, DEFAULT_KDF_T}`〔:14〕、`cipher: Option<Arc<PageCipher>>` 字段〔:57〕、`initialize_header`〔:290-319，Some(key) 臂写加密头+派生密钥；None 臂纯明文〕、`validate_header`〔:324-376，`(encrypted, key)` 四臂 match：`(true,None)` → `InvalidKey("database is encrypted, supply --key or RTSQL_KEY: {path}")`、`(false,Some(_))` → `InvalidKey("database is not encrypted: {path}")`、`(true,Some)` 派生密钥、`(false,None)` 明文〕、`read_page_blocking`/`write_page_blocking` 的 `Some(cipher)` 臂〔:390-408 起〕、零页写 `Some` 臂〔:514〕）；`src/storage/file_header.rs`（1 处：`decode` :147 调 `super::crypto::validate_kdf_params`）。
  - 注意区分：executor/{delete:96,insert:299,update:238,430,upsert}、wal/recovery.rs:425、table_manager.rs:74 等处的 `derive_key` 是 `SecondaryIndex::derive_key`（B-Tree 二级索引键推导），与 crypto KDF 同名异物——非接缝，不得改动。
  - `file_header.rs` 关键结构：`FLAG_ENCRYPTED = 1`（:24，格式位常量，两组合都保留）；`KNOWN_FLAGS_MASK = FLAG_ENCRYPTED`（:27）；`decode` 判序为 magic → version → **unknown_flags（mask 判，:126-129）** → page_size → 加密分支（salt/参数校验）。mask=0 时 `FLAG_ENCRYPTED` 位置位在 unknown_flags 判即拒：`HeaderError::UnknownFlags(1)` → `header_rejection` → `StorageError::IncompatibleHeader("unknown feature flags: 0x1 ({path})")`（既有前向防护面，零新错误路径）。`current_encrypted()`（pub(crate)，:77）是纯编码器，除 crypto 模块外无依赖。
  - 锁序事实：`Database::open` 家族全部经 `construct_file_storage`（专用线程）进入 FileStorage 构造器（`open_with_lock_io_failpoint` / `open_readonly_with_lock`），**锁获取在头校验与密钥阶段之前**（顺序守卫，database-encryption R2）；`Database` 面错误类型为 `StorageError`（`storage::error::Result` 别名，`InvalidKey`/`IncompatibleHeader` 变体在 storage/error.rs:27,30）。`database.rs` 对 key 只透传 `Option<&str>`，零 crypto 类型外漏——database/cli/network/main 四处零改动。
  - CLI 映射既有事实：`StorageError::InvalidKey` → 退出码 5（`--key` 空值 exit 2 在开库前，不涉本 Cycle）。
  - 单测面：`file_header.rs` `#[cfg(test)]` 内加密正例 3 个函数（`encrypted_header_roundtrip`、`encrypted_header_salt_all_zero_rejected`、`encrypted_header_invalid_kdf_params_rejected`）与 `file_storage.rs` 内加密用例（:652 起，含 :688 `open_with_key(&path, Some("pw"))`；:661「明文库 `open` 与 `open_with_key(None)` 等价」用例不含密钥材料，两组合均保留）需按用例加 `#[cfg(feature = "crypto")]`；`tests/encryption_test.rs` 是默认组合的加密面锚定。
- Code and Critical Path: 门控改动 `src/storage/mod.rs` → `file_header.rs`（mask 与 decode 分支）→ `file_storage.rs`（use 行、cipher 类型与分支、携带密钥拒绝）；行为证明走 `tests/crypto_off_build_test.rs`（新增）。数据流无变化：明文路径的字节读写序列与默认组合一致。

**Implementation Guidance**

先写测试文件再实施门控（First test 见 Task Definition）。门控实现形态（分支 cfg 粒度、`cipher` 字段处理、`file_header` 不可达分支的重构与否）是 Act 的非实质选择；边界是「C2/C4 组合产物无加密代码路径、无 dead_code/unused 告警、明文路径与默认组合逐字节一致」。`current_encrypted` 在 no-crypto 组合若因调用点被门而闲置，优先以等效重构消除告警（如随调用点一并 cfg），确实不可消除时停止回报，不堆积 `#[allow]`。

**Task Definitions**

### T3: crypto feature 门控与关闭态行为

- Requirement/Scenario: R3（S3 明文全功能、S5 加密头拒绝、S6 携带密钥拒绝；S1 默认组合等价不变量持续成立）
- Depends on: Iteration 000 accepted
- Targets: `src/storage/mod.rs`、`src/storage/crypto.rs`、`src/storage/file_header.rs`、`src/storage/file_storage.rs`、`tests/crypto_off_build_test.rs`（新增）
- Current behavior: crypto 模块无条件编译；C2 组合构建失败（argon2/aes-gcm 未启用）；关闭态不存在（无该组合）；默认组合加密面由 `database-encryption` 合同约束并全部可用。
- Required behavior:
  1. `src/storage/mod.rs` 的 `mod crypto;` 加 `#[cfg(feature = "crypto")]`。
  2. `KNOWN_FLAGS_MASK` 按 feature 二值：crypto 开 = `FLAG_ENCRYPTED`，关 = `0`；`decode` 对 crypto 模块的引用按 feature 处理（mask=0 时加密分支不可达）。
  3. `file_storage.rs` 的 crypto use、`cipher` 相关类型与全部加密分支按 feature 门控；crypto 关闭时：携带密钥的打开——既有明文库路径（`validate_header` 的 `(·, Some)` 情形）与 0 字节新建路径（`initialize_header` 的 Some 臂之前）——在持锁之后返回 `StorageError::InvalidKey`，消息点名本构建未包含加密能力（含 `not available in this build` 语义），新建路径不写任何格式头字节（可能残留 0 字节空文件，与既有打开失败路径同类）；明文路径行为与默认组合逐字节一致。
  4. `file_header.rs`/`file_storage.rs` 中依赖加密能力的单测用例加函数级 `#[cfg(feature = "crypto")]`；不含密钥材料的用例保留两组合。
  5. 新增 `tests/crypto_off_build_test.rs`：文件级 `#![cfg(not(feature = "crypto"))]`，至少三个用例——(a) 合成 64 字节文件（magic `RTSQLDB\0`、version u32 LE=1、flags u32 LE=1、page_size u32 LE=4096、余零）以 `Database::open` 打开 → `StorageError::IncompatibleHeader` 且消息含 `unknown feature flags`；(b) 明文库 `open_with_key(Some(..))` 与 0 字节路径携带密钥打开 → `InvalidKey` 且消息含本构建未包含加密能力语义，新建路径文件长度保持 0；(c) 明文库建表/插入/查询/close/重开往返全部成功。
- Preserve: 默认组合错误消息逐字节不变（含 `(false, Some)` 的 `database is not encrypted: {path}` 原文）；`FLAG_ENCRYPTED` 常量两组合保留；`SecondaryIndex::derive_key` 及其调用点零改动；`database.rs`、`src/cli/`、`src/network/`、`src/main.rs`、`Cargo.toml` 零改动（features 表已就绪）；既有测试断言零改动。
- Forbidden: 修改 `database-encryption` 合同行为、KDF 参数、页记录格式；给 `rand` 做任何可选化；新增 `#[allow]` 压告警；改动 Iteration 000 已交付的门与文件。
- First test: 新建 `tests/crypto_off_build_test.rs`（Required behavior 第 5 条）后即运行 `cargo test --offline --locked --no-default-features --features cli,network --test crypto_off_build_test` → RED：编译失败（crypto 模块引用未启用的 argon2/aes-gcm），保留错误首行。
- GREEN condition: 门控后同命令退出码 0 且三用例通过；C2 构建门与验证集（Verification 第 2–4 条）通过；默认组合加密回归（第 5 条）通过。
- Verification:
  1. RED 记录（First test 命令与错误首行）。
  2. C2 构建门：`cargo check --offline --locked --no-default-features --features cli,network --lib --bins --tests` 退出码 0。
  3. C2 行为验证：`cargo test --offline --locked --no-default-features --features cli,network --lib` 通过；`cargo test --offline --locked --no-default-features --features cli,network --test crypto_off_build_test --test storage_test --test parser_test` 通过。
  4. C2 告警门：`cargo clippy --offline --locked --no-default-features --features cli,network --lib --tests -- -D warnings` 退出码 0（门控不得在关闭组合留下 dead_code/unused 告警；design §7 的 core-only clippy 在 T5 复验 C4 面）。
  5. 默认组合回归：`cargo test --offline --locked --test encryption_test --lib` 通过；`cargo check --offline --locked --lib --bins --tests` 与 `cargo fmt --check` 退出码 0。
  6. S9 事实核对：C2 验证集不含加密正例类目标（design §2.5 选定，非缺陷）。
- Stop when: 接缝出现上述三文件之外的 crypto 模块引用（grep `super::crypto\|PageCipher\|ENCRYPTED_PAGE_RECORD_SIZE\|DEFAULT_KDF` 于 src/ 复核）；默认组合任一回归命令失败（默认组合回归，立即停止）；C2 告警无法以等效重构消除；`SecondaryIndex::derive_key` 被误改。

**Invariants**

- 默认组合行为逐字节等价（含全部错误消息、退出码、磁盘格式、加密往返）。
- 锁序守卫：拒绝点在持锁之后（继承 database-encryption R2 的锁优先于密钥错误）。
- 不新增跨层依赖环边；新增代码不引入 `eprintln!`/`println!` 直写出口。
- M19：不跑全量；定向命令默认并行度。无判定层，判定只看 cargo 退出码与原生输出。

**Non-goals**

cli 门控、C3/C4 组合、组合矩阵收口、全量回归、加密功能与格式演进、`rand` 可选化、诊断接口实现。

**Acceptance**

- R3 逐 Scenario：加密头拒绝（用例 a ↔ delta spec「加密头文件被拒绝」）；携带密钥拒绝（用例 b ↔「携带密钥打开被拒绝」，含不写头）；明文全功能（用例 c ↔「明文库全功能」）；`argon2`/`aes-gcm` 不进 C2 依赖图（构建门隐含——两依赖 optional 且 crypto feature 未启用；如需显式证据，`cargo tree` 类输出不要求，以 optional 声明 + 构建通过为准）。
- 默认组合 `database-encryption` 合同零变化（Verification 第 5 条）。
- 映射：proposal 需求 3、design §3 裁定表、tasks.md T3 交付依据。

**Verification**

逐 Scenario 最简直接判定（真实入口）：

| Scenario | 判定 |
|---|---|
| S5 加密头拒绝 | 用例 (a)：`Database::open` 真实打开合成头文件，断言 `IncompatibleHeader` + 消息含 `unknown feature flags` |
| S6 携带密钥拒绝 | 用例 (b)：两个真实打开路径断言 `InvalidKey` + 构建缺加密语义消息 + 0 字节保持 |
| S3 明文全功能 | 用例 (c)：建表/写/读/close/重开往返 |
| S1 默认组合（触及面） | Verification 第 5 条三命令 + encryption_test |
| S9 组合验证集口径 | 事实核对（无命令） |
| S2/S4/S7/S8/S10/S11 | 不属本 Cycle |

**Gate 2 Readiness**

PASS（2026-10-28）。无 BLOCKED、无 SKIPPED。批准来自 goal 指令对 MS42 范围的授权（Iteration 000 accepted 后按 Map 展开本 Iteration）。RTM 对应行（R3）无 Missing；错误语义在 design §3 与本 Context 完全闭合，无 TBD。

**Persisted Evidence**

- Mode: none

命令、退出码与输出行写入 Act Response 即可；组合行为可低成本用 cargo 命令复现，不满足 required 白名单。

**Risks and Notes**

- 门控粒度与 `cipher` 字段处理形态留给 Act（design §9 第 1 条）；`current_encrypted` 闲置告警的处理路径见 Implementation Guidance。
- `file_storage.rs` 加密用例的函数级门清单以执行时实际用例为准（本 Context 列出已知锚点 :652/:661/:688，非穷举）；门控原则「依赖加密能力的用例加门、不依赖的保留」。
- C2 `--lib` 用例数将少于默认组合（加密单测剥除），差额在 Act Response 记录即可，不构成对账门槛。
- `Cargo.lock` 预期零变化（optional 声明已存在）；若离线解析异常，按实际错误回报，不做版本变更。

## Act Response

- Status: pending

## Plan Review

- Review Result: pending
