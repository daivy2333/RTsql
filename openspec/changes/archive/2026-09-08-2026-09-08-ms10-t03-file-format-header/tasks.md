# tasks: MS10-T03 文件 magic/格式版本头

> 关联 milestone：MS10-T03（`.claude/docs/tasks.md`）。规划依据：R19 分析 + 用户决策 2026-09-08（见 proposal）。

## Task List

### T1: 文件头模块（布局、编解码、常量）

- **目标**：`src/storage/file_header.rs` 新模块——64B 布局（D1）、`FileHeader { version, flags, page_size }`、`encode`/`decode` 纯函数、`HEADER_SIZE`/`FORMAT_VERSION`/`FLAG_ENCRYPTED`/`KNOWN_FLAGS_MASK` 常量、私有 `HeaderError` 分类（D2）；`src/storage/mod.rs` 导出。
- **Requirement**: R1（头布局与生命周期）
- **测试见证**：模块内 `#[cfg(test)]` 单测——encode→decode roundtrip、magic 错、version 越界分类（0 vs >1）、未知 flag 位、page_size 不符、salt/reserved 非 0 拒绝。先写测试观察 RED（模块不存在无法编译即为 RED 起点）。
- **验收**：单测全绿；模块零外部依赖（不引 error.rs 以外的 crate 路径）。

### T2: FileStorage 接线（校验/初始化 + 偏移平移）与存储层回归

- **目标**：`FileStorage::open` 按 D4 顺序接线（锁 → 头初始化/校验 → 页整除校验）；read/write/allocate 3 处偏移加 `HEADER_SIZE`（D3）；`src/storage/error.rs` 新增 `NotADatabase(String)` / `NewerFileVersion(u32)` / `IncompatibleHeader(String)`（D5）；`async_storage.rs` page_count 文档措辞更新（D6）；`tests/drop_table_free_test.rs:18-19` helper 扣头。
- **Requirement**: R1、R2、R3（顺序）、R4（零回归）
- **测试见证**：`tests/file_header_test.rs` 拒绝矩阵（D8——垃圾 8k panic 场景 RED 起步，实现后 GREEN）；`tests/storage_test.rs` / `tests/file_storage_io_test.rs` 既有断言零修改保持绿；`database_file_lock_test.rs` 增补"坏文件 + 锁被占 → DatabaseLocked"场景。
- **验收**：拒绝矩阵全绿；全量存储层回归绿。

### T3: CLI e2e 与全量验证

- **目标**：`tests/cli_test.rs` 增补——垃圾文件 exit 1 + stderr 文案、`NewerFileVersion` 场景 exit 1、拒绝不产生伴生文件、锁优先 exit 4；跑全量验证门。
- **Requirement**: R2、R3
- **测试见证**：cli_test 新用例 RED（当前实现：垃圾文件 panic/abort、无头校验）→ 实现后 GREEN。
- **验收**：`cargo test` 全量（636 基线 + 新增）0 failed；`cargo clippy -- -D warnings` 0；`cargo fmt --check` 0；`openspec validate` PASS。

## Iteration Plan

### Iteration 000: 带头文件格式生效——创建/重开/拒绝矩阵闭环

- Tasks: T1, T2, T3
- Depends on: None
- Stable baseline: 主库文件自带 64B 自描述头；非 RTsql/新版/未知 flag/截断文件在打开时干净拒绝（exit 1，不 panic）；0 字节新库与伴生文件语义不变；全量回归绿。MS10-T05 生命周期子命令与 MS12-T01 加密可依赖此基线。
- Verification boundary: `cargo test` 全量 0 failed（636 基线 + 新增约 15 用例）+ clippy/fmt/validate 全 0 + `tests/file_header_test.rs` 拒绝矩阵全绿。
- Diagnostic boundary: `src/storage/file_header.rs`（布局/编解码）、`src/storage/file_storage.rs`（open 顺序与偏移）、`src/storage/error.rs`（变体）、`src/cli/mod.rs`（仅错误 Display 途经，不改动）；测试在 `tests/file_header_test.rs` / `tests/cli_test.rs` / 既有存储测试。
- Non-goals: WAL/checkpoint 加头；旧文件迁移；free-list 持久化；MS12 加密实现（见 proposal Out of Scope）。

### 平衡审计

- **聚合**：T1/T2/T3 共同形成"带头格式生效"单一可验收结果——T1 单独无消费者、T2 单独缺 CLI 可观察面、T3 依赖 T2，合并为 000。
- **拆分检查**：无多故障域（全部集中在 FileStorage 打开路径 + 错误面）；无独立可验收的第二个结果；工作量与 MS10-T01 单 Iteration 相当（约 4 生产文件 + 2 新/2 改测试文件）。不拆分。

## Requirements Traceability Matrix

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R1 头布局与生命周期 | 新库带头 / v1 重开 / to_offset 纯数学 | D1, D2, D3, D6, D7 | T1, T2 | 000 | `file_header.rs`、`file_storage.rs::{open,read_page_blocking,write_page_blocking,allocate_page}`、`async_storage.rs::page_count` | `file_header_test` roundtrip + `storage_test` 既有断言零修改 | None | Covered |
| R2 格式错误显式拒绝 | 垃圾 8k / 新版版本 / 未知 flag / 截断 / 过小 / 旧无头 | D2, D4, D5, D8 | T1, T2, T3 | 000 | `file_header.rs::decode`、`file_storage.rs::open`、`error.rs`、CLI Display 途经 | `file_header_test` 拒绝矩阵 + `cli_test` exit 1 e2e | None | Covered |
| R3 打开顺序守卫 | 锁优先 / 拒绝不触碰伴生文件 | D4 | T2, T3 | 000 | `file_storage.rs::open`（锁→头→长度次序） | `database_file_lock_test` 增补 + `cli_test` 伴生文件断言 | None | Covered |
| R4 既有语义零回归 | 0 字节新库 / 伴生文件不变 | D3, D6, D7 | T2, T3 | 000 | `async_storage.rs`、`drop_table_free_test.rs` helper | 全量 `cargo test`（636 基线）+ `file_storage_io_test` EOF 守卫 | None | Covered |
