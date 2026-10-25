# 索引树字节级损坏页的注入夹具配方

- Status: active
- Last validated: 2026-10-25
- Environment: Linux x86_64（WSL2 6.18.40.1）、cargo 1.99.1 / rustc 1.99.1、RTsql `tests/release_path_bounded_read_test.rs` 与 `tests/index_read_bounded_read_test.rs` 集成测试、tokio 多线程 test runtime、单用例自带 `tempfile::tempdir()` 现场
- Source: change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 002（释放路径面）与 Iteration 006（读 / 写路径面）/ 各 Cycle 000 Act Response 与 Plan Review（`openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/002-release-path-bounded-read/000-initial.md` 与 `.../006-index-read-bounded-read/000-initial.md`，2026-10-25）

## 适用范围

- 需要在**真实文件库**上造出一个**字节级损坏**的索引树页（槽偏移越界），并观察两条路径**各自**的表现时使用：
  - **释放路径**（`DROP TABLE` / `TRUNCATE` / `DROP INDEX`）⇒ 尽力而为：**告警 + 跳过**该页 / 该链，返回已收集的部分，不 panic；
  - **读 / 写路径**（`SELECT` / `INSERT` / `DELETE` / `UPDATE` 的索引下降与条目提取）⇒ **明确错误**：点名**损坏的页**与**该索引树的根页**，不 panic、不跳过。
- 需要验证「页级判据（`integrity::check_tree_page`）能捕获 / 不能捕获某类损坏」时使用（本配方的损坏形态落在**槽偏移**上）。
- 不适用：页**内容**损坏之外的其它故障形态（断电、掉电、页丢失）；逻辑层错误（约束、类型）；进程崩溃语义（用 `.agents/runbooks/in-process-crash-simulation.md`）；全量回归的分批执行（用 `.agents/runbooks/m19-batched-full-regression.md`）。

## 前置条件

- **页内槽目录项的字节布局**：`[logical_id:2][offset:2][length:2]`（每项 6 字节）；第 `index` 项落在页尾 `PAGE_SIZE - (index + 1) * Slot::SIZE` 处，**offset 字段再 +2**。
- **文件偏移**：`page_file_offset(page, in_page) = HEADER_SIZE + page * Page::PAGE_SIZE + in_page`。
- **注入的越界值**：`0xFFFF`（65535）——超过页长（4096）⇒ `get_slot_data` 的 `&self.data[start..end]` 必然越界。
- **必须「关闭 → 改字节 → 重开」**：损坏字节要在库关闭后写进文件；**重开**这一步依赖 `src/database.rs` 的「重建失败只告警（`[free-page-rebuild] rebuild abandoned`）」行为 ⇒ 损坏的数据页**不会**让打开失败 ⇒ 损坏夹具可以「写库 → 关闭 → 改坏页字节 → 重开」后直接执行语句。
- 既有损坏注入先例：`tests/check_command_test.rs` 的 `poke(&path, page_file_offset(head, 0), 0x42)`（按文件偏移改写页字节）；本配方沿用同一手法，只把损坏点从页首移到**槽目录项的 offset 字段**。
- 夹具不携带任何运行身份（无 run-id / 指纹 / 时间戳）。

## 操作步骤

1. **建表并写入若干行**，取到目标索引树页的**页号**（主键索引的根页，或二级索引的根页）。
2. **关闭库**（让脏页落盘，得到稳定的文件字节）。
3. **按文件偏移改坏某个槽的 offset 字段**：

```rust
fn slot_offset_field_offset(slot_index: usize) -> usize {
    Page::PAGE_SIZE - (slot_index + 1) * Slot::SIZE + 2
}

fn page_file_offset(page: u32, in_page: usize) -> u64 {
    (HEADER_SIZE + page as usize * Page::PAGE_SIZE + in_page) as u64
}

/// 把某页某槽的 offset 字段改成越界值（0xFFFF 超过页长）——真实改坏页字节。
fn corrupt_slot_offset(path: &Path, page: u32, slot_index: usize) {
    use std::io::{Seek, SeekFrom, Write};
    let mut f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.seek(SeekFrom::Start(page_file_offset(
        page,
        slot_offset_field_offset(slot_index),
    )))
    .unwrap();
    f.write_all(&0xFFFFu16.to_le_bytes()).unwrap();
    f.flush().unwrap();
}
```

4. **重开库**。
5. **走释放路径**（Iteration 002 的面）：执行 `DROP TABLE t` / `TRUNCATE t` / `DROP INDEX <idx>` ⇒ 断言**语句返回**（成功或既有错误形态）、**进程未 abort**，且告警已报出、损坏页 / 该链被跳过、其余部分照常释放。
6. **走读 / 写路径**（Iteration 006 的面）：执行 `SELECT` / `INSERT` / `DELETE` / `UPDATE` ⇒ 断言返回**明确的错误**，错误文本**同时点名损坏的页与该索引树的根页**（形态：`corrupted index page {page_id} (index root page {index_root})`）⇒ **不得**被当作「查不到该行」或「没有这个子树」跳过。
7. **（对照）未损坏路径零变化**：同形状的库不做字节改坏，跑同样的语句 ⇒ 行为与修复前**逐项一致**（读 / 写正常、释放的页集合不遗漏不重复）。

## 验证

- 成功判据：释放路径用例 `cargo test --test release_path_bounded_read_test -- --test-threads=1` 与读 / 写路径用例 `cargo test --test index_read_bounded_read_test -- --test-threads=1` 各自退出码 0、failed 计数为 0（2026-10-25 实测分别为 7/0 与 5/0）。
- **修复前 RED 形态**：越界切片 panic，位点 `src/storage/page_format/slotted_page.rs:128` = `SlottedPageRef::get_slot_data` 里的 `&self.data[start..end]`；实测消息 `range start index 65535 out of range for slice of length 4096` ⇒ 与注入的 `offset = 0xFFFF` 完全对应；panic 发生在异步执行路径 ⇒ 伴随 `PoisonError` ⇒ `signal: 6, SIGABRT`（进程 abort）。
- **页级判据不误报**：半满合法叶页（N=46 条、条目 38 字节）与满叶页（N=92，`16 + 44n ≤ 4096`）经 `check_tree_page` 均**无 violation** ⇒ 判据对合法页零误报。
- 判据全部落在**可观察状态**上（语句返回 / 错误文本 / 进程是否 abort / 释放的页集合 / 完整性报告）；判定层在 `cargo test` 内，无项目测试框架之外的判定脚本。

## 失败处理

- **重开就失败（不是告警而是报错）**：确认损坏点确实落在**槽目录项的 offset 字段**（页尾 `PAGE_SIZE - (i+1)*6` 起的 6 字节中的第 3–4 字节），而不是页头（`HEADER_SIZE` 之后的固定头字段）⇒ 页头被改坏会让打开期的重建失败被吞掉之外的行为暴露出来。
- **执行语句时 panic 在别处**：确认改的是**目标索引树页**（`DROP INDEX` 的释放路径只读**索引树页与目录**，不读数据页 ⇒ 该语句的损坏点必须落在索引树页上）。
- **读路径返回「查不到该行」而不是错误**：说明读路径把损坏页当成了「没有这个子树 / 没有这个条目」⇒ 该形态是**静默错读**，正是本配方要区分的失败。
- **释放路径整体失败而不是告警 + 跳过**：释放路径的既有语义是尽力而为 ⇒ 整体失败说明该语义被改动（该语义由行为规格 `openspec/specs/release-path-bounded-read/spec.md` 承载）。
- **合法页被误报为损坏**：核对注入的页号确实是目标索引树的页（索引树的身份由**其根页号**唯一确定；根分裂时 `root_page_id` 随新根更新）。

## 回滚

不适用（测试夹具，无产品状态变更）。夹具只作用于本用例的库文件；`TempDir` 随用例结束清理。

## 证据

- `openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/002-release-path-bounded-read/000-initial.md` —— Act Response 的 T8 / T9（损坏注入手法、释放路径的告警 + 跳过语义、RED 形态）与 Plan Review 的独立复核。
- `openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/006-index-read-bounded-read/000-initial.md` —— Act Response 的错误语义定死（`corrupted index page {page_id} (index root page {index_root})`）、22 处页级校验点的枚举与 RED 复核（panic 位点与消息）。
- 见证代码：`tests/release_path_bounded_read_test.rs`（`slot_offset_field_offset` / `page_file_offset` / `corrupt_slot_offset`）、`tests/index_read_bounded_read_test.rs`。
- 修复落点：`src/storage/btree/node.rs`（`bounded_slot_data_ref` / `bounded_slot_data_mut`）、`src/storage/btree/btree.rs` 与 `src/storage/btree/index_manager.rs`（22 处页级校验闸门）、`src/storage/error.rs`（损坏索引页的专属错误变体）。
- 行为规格：`openspec/specs/release-path-bounded-read/spec.md`（释放路径：告警 + 跳过）、`openspec/specs/index-read-bounded-read/spec.md`（读 / 写路径：明确错误）。
- 相关配方：`.agents/runbooks/index-write-side-atomicity-repro.md`（索引写侧锚点式暂停）、`.agents/runbooks/ddl-catalog-commit-point.md`（DDL 目录提交点）。
