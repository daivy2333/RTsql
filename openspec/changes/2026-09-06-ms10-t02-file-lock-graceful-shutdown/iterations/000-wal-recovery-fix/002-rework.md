# Iteration 000 / Cycle 002-rework: B-Tree 多页规模正确性修复与见证精确化

## Plan Context

- Status: ready
- Iteration: 000-wal-recovery-fix（WAL 恢复逐帧无歧义与重放正确性）
- Cycle: 002-rework
- Cycle Type: rework
- Parent cycle: 001-replan（Act Response `reported`；Plan Review `rework-required`）
- Gate 2 批准：2026-09-07 14:36，用户原话「**更改gate状态，开始实施**」通过 `/openspec-act` 显式授权。风险记录：3 个 B-Tree 多页规模修复项（R2/R3/R4）尚未经代码层验证为可定位到根因；Act 将以 TDD RED-first 推进，三次失败后回退至设计/需求层，符合 `openspec-act` Gate 6 协议。

> **Rework 注记**：父 Cycle 的 T0b 核心修复（位置寻址重放）与 T1 主体验证成立（sigkill e2e 转绿、5/5 见证绿、clippy/fmt/validate PASS，均经 Plan 独立复跑）。Plan Review 深度验证揭示三个**既有 B-Tree 多页规模缺口**进入验收路径（最小键搜索盲区 / delete 重平衡 Page-full 泄漏 / update 内部节点未实现），并裁定父 Cycle 见证 ② 的断言放宽违反契约。本 Cycle 以 4 个 repair item 完成既有 Acceptance，不修改 Iteration Map。

**Cycle Scope**

- Trigger: rework（父 Cycle Plan Review）
- Acceptance gaps: ① R1-S2「重复 PK 被拒」普遍性不成立（最小键 id=0 在 10k 树上 search 未命中，重复插入被接受——运行树与恢复重建树一致）；② R2-S1「任意形态」不成立（无中位点 checkpoint 的删除型 WAL 重开 `delete redo ... Page full` → 打开失败；大树重放点的 Update 记录 → `RedoFailed`）；③ 父 Cycle 见证 ② 断言放宽（计数 ±5、UPDATE 效果断言移除）待以精确断言恢复
- Repair items: R-T0b-R1（见证 ② 精确化）、R-T0b-R2（最小键搜索盲区）、R-T0b-R3（BTree::delete Page-full 泄漏）、R-T0b-R4（BTree::update 内部节点递归）——repair item 不作为新的全局 change task，不修改 Iteration Map
- Inherited scope: 父 Cycle（001-replan）全部不变量与已落地成果（reader 修复、T0b 重放实现、5 个见证用例骨架）；D7 语义不变
- Excluded scope: 锁/信号（Iteration 001）；WAL 格式与页格式；do_flush 并发；非 PK 索引能力；B+Tree 并发/节点级锁（D-candidates）

**Objective**

B-Tree 在多页规模（~10k 键）下对最小键的 search 正确、delete 不再泄漏 Page-full、update 支持内部节点递归；混合 DML（INSERT/UPDATE/DELETE）在**无中位点 checkpoint 的驱逐规模 WAL** 下崩溃恢复精确（行数 = 已提交终态、更新可见新值、删除不可见、索引判重含最小键）；见证 ② 以精确断言恢复契约。

**Current Baseline**

- 工作区（未提交，位于 `590fdc6` 之上）：reader.rs（T0）、recovery.rs（T0b，+278/-31）、wal_recovery_large_test.rs（5 用例）、cli_test.rs（160k 标定）——父 Cycle 产物，Plan 独立审查通过，**保留为本 Cycle 基线**。
- 测试基线（Plan 独立复跑，2026-09-07）：`wal_recovery_large_test` 5/5 绿；`cli_test` 14 passed / 3 failed（白名单）/ 2 ignored（sigkill 绿）；clippy 0 / fmt 0 / validate PASS。

**Current-State Evidence**（全部来自 Plan Review 独立探针与代码审查，2026-09-07；临时探针测试已删除，recipe 如下）

- **G1 最小键搜索盲区**：10k 键 B-Tree 上 `search(最小键)` 未命中——运行树：重复 `INSERT INTO t VALUES (0)` 被**接受**（`insert.rs:101-109` 判重经 `index_manager.search` 返回 None）；id=1/42/86/99/100/5000/9999 全部正确拒绝。恢复重建树（T0b redo 重建，同一 insert 序）同样仅 id=0 失明 → 缺口在 B-Tree 本体（split/search 的左边界处理），与重放路径无关。Runtime recipe：create → checkpoint → 10k auto-commit INSERT → 重复 INSERT 各键观察响应。Rebuilt recipe：同 Act 夹具（50 行/显式事务 ×200，flush_all 后 drop 不 close）→ 重开 → 重复 INSERT 各键。
- **G2 `BTree::delete` Page-full 泄漏**：运行期 DELETE 偶发 `Page full`（Act 夹具 id=242；同形态诊断 3 中 id=300/1000/9000 全部成功——状态依赖）；恢复路径：无中位点 checkpoint 的混合 WAL（create→checkpoint→10k INSERT+50 DELETE→drop 不 close）重开必失败：`WalError("WAL redo failed: delete redo of table 't' row RowId { page_id: 8, slot_id: 67 } failed: Page full")`。失败点 = `index_manager.delete`（recovery.rs Delete 臂的索引清理，错误映射 "delete redo of table '{}' row {:?} failed"）；墓碑路径（`data_page.rs:103-128` 原地覆写）不可能产生 Page full，已排除。`btree.rs` 中 `StorageError::PageFull` 仅在 insert 分裂路径被处理（`:250/:333`）；delete 侧 `delete_from_page`（`:408`）→ underflow → `redistribute_leaf_right/left`（`:530-568`，read_leaf_pair + 整页 `rebuild_leaf` 重建）与 `merge_leaves`（`:570`）——泄漏点在 delete 重平衡族，需根因定位。运行期 recipe：10k 树上批量 DELETE（≥50 键，含 200..250 区段）捕获 `Page full`；恢复 recipe：G2 形态 WAL 重开。
- **G3 `BTree::update` 内部节点未实现**：`btree.rs:1027-1047` `update_in_page` 非叶直接 `Err("Internal node update not implemented yet")`。运行期 10k 树 UPDATE（id=42/220/5000/9999）全部失败；恢复侧 Update 重放（`recovery.rs` Update 臂 `index_manager.update`）在大树重放点同样 `RedoFailed` → 更新后置的混合 WAL 打开失败。id=0 的 UPDATE 失败原因是 G1（search None → `update.rs:72 KeyNotFound`），G1 修复后归并为 G3。
- **G4 已提交终态推导口径**：恢复精确性断言必须以「已提交终态」为期望值——运行期失败的语句（G2/G3 导致）不计入。Plan 探针实测：Act 夹具形态（100 UPDATE 全失败 + 49/50 DELETE 提交）恢复 COUNT=9951 = 10000 − 49 精确吻合；父 Cycle 预期值 9950 系未计失败语句的推导错误。
- **代码面**：`src/storage/btree/btree.rs`（search/split/delete/redistribute/merge/update）、`src/storage/btree/node.rs`（LeafNode/InternalNode）、`src/storage/btree/index_manager.rs`（包装层）。T0b 的 `recovery.rs` 不再是本 Cycle 修改面（其 Delete/Update 臂的既有错误传播语义保持）。

**Relevant Code**

| 文件/符号 | 职责与本 Cycle 关系 |
|---|---|
| `src/storage/btree/btree.rs::search 系` | G1 修复面（左边界/分裂分离键处理） |
| `src/storage/btree/btree.rs::delete_from_page / redistribute_* / merge_* / handle_child_merge` | G2 修复面（Page-full 泄漏根因） |
| `src/storage/btree/btree.rs::update_in_page` | G3 修复面（内部节点递归补全） |
| `src/storage/btree/node.rs` | 叶/内节点原语（按 G1-G3 根因需要） |
| `tests/btree_scale_test.rs`（新建） | G1/G2/G3 的规模级 RED→GREEN 见证 |
| `tests/wal_recovery_large_test.rs::mixed_dml_recovery_semantics` | R-T0b-R1 重设计面（精确断言恢复） |
| `src/wal/recovery.rs`、`src/wal/reader.rs` | **禁止修改**（父 Cycle 已冻结） |
| `src/storage/page_format/*`、`src/wal/{writer,record,buffer,checkpoint}.rs` | **禁止修改**（页格式/WAL 格式零变更） |

**Critical Path**

G1/G2/G3 根因定位（读 btree.rs 对应路径 + 最小化 RED 复现）→ 各自修复（保持 B-Tree 不变量：有序、平衡、分离键与子指针一致）→ 规模级回归（新建 btree_scale_test）→ R-T0b-R1 见证 ② 重设计（依赖 G1-G3，见依赖注记）→ 全量验证门。

**Implementation Guidance**

- G3（最明确）：`update_in_page` 非叶分支按 `InternalNodeRef` 找到目标子页递归下探（与 `search` 的下探同构），叶上 `leaf.update` 既有实现复用；KeyNotFound 语义保持（键不存在报 KeyNotFound，K05 对齐）。
- G1/G2：先根因后修复——以最小化 RED（如 10k 顺序 INSERT 后仅对最小键断言 search 命中；批量 DELETE 复现 Page full）定位到具体分支（怀疑面：左边界分裂后根/子指针或分离键维护；redistribute 的 `rebuild_leaf` 容量/`update_parent_separator(child_index - 1)` 偏移），修复不得引入逐键线性扫描或全树重建等 O(n) 回归。
- 关键取舍（已定）：三缺口全部在 `btree.rs`/`node.rs` 本体修复，不在恢复层绕行（跳过索引清理或容错 Page-full 都会造成索引/数据分叉，违反 K05 与 R1-S2）；B-Tree 修复不改变页格式与磁盘结构（纯逻辑层）；repair item 顺序 G1→G2→G3（相互独立，可并行验证）→ R1（依赖三者）。
- 非实质留给 Act：RED 测试的具体断言形态、根因修复的具体分支实现、btree_scale_test 的夹具规模微调。

**Behavioral Change**

| 输入 | 现行为 | 目标行为 |
|---|---|---|
| 10k 树重复 INSERT 最小键 | 被接受（唯一性失效） | DuplicateKey 拒绝（运行树与重建树一致） |
| 10k 树 UPDATE（任意已存在键） | `Internal node update not implemented` | 成功且索引指向新位置 |
| 10k 树批量 DELETE | 偶发 `Page full` | 全部成功 |
| 无中位点 checkpoint 混合 WAL 重开 | `delete redo ... Page full` 打开失败 | 恢复精确（终态计数/更新可见/删除不可见） |
| 见证 ② | ±5 容差 + UPDATE 断言缺失 | 精确断言（计数/新值/不可见） |

**Change Surface**

| Repair item | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| R-T0b-R2 | R1-S2 普遍性 | `btree.rs` search/split 左边界 | 最小键不可达 | 根因修复 + 规模 RED |
| R-T0b-R3 | R2-S1 任意形态 | `btree.rs` delete 重平衡族 | Page-full 泄漏 | 根因修复 + 规模 RED |
| R-T0b-R4 | R2-S1 任意形态 | `btree.rs::update_in_page` | 内部节点未实现 | 递归补全 + 规模 RED |
| R-T0b-R1 | R2-S1 见证精确化 | `tests/wal_recovery_large_test.rs` | 放宽断言 | 精确断言重设计（依赖 R2-R4） |
| 验证门 | 全部 | 无新代码 | — | 四命令复跑 |

**Task Contracts**

### R-T0b-R2: 最小键搜索盲区修复（G1）

- Requirement/Scenario: `wal-recovery-replay-integrity` R1-S2 普遍性
- Depends on: None
- Targets: `src/storage/btree/btree.rs`（search/split 左边界，根因定位后可含 `node.rs`）
- Current behavior: 10k 键树上 `search` 对最小键未命中（运行树与 redo 重建树一致）；重复 INSERT 最小键被接受
- Required behavior: 任意键规模的 B-Tree `search` 对全部已插入键命中；重复 INSERT 最小键被 DuplicateKey 拒绝
- Required changes: 根因定位 + 修复（G1 Implementation Guidance）；不改页格式
- Preserve: 既有 `tests/btree_test.rs` 10 用例零修改全绿；search 复杂度量级不回退（不得全树扫描）
- Forbidden: 页格式/WAL 格式变更；恢复层绕行；O(n) search 回归
- Test witness（RED 先行）: 新建 `tests/btree_scale_test.rs` 用例 ① `min_key_searchable_at_scale`——lib 侧建库 10k 键（50/显式事务）→ 重复 INSERT 最小键（0）必须 DuplicateKey、次小键（1）与中部/尾部键对照必须拒绝（RED：id=0 被接受，Plan 已实测）；恢复重建树同断言（同用例第二段或独立用例）
- GREEN condition: 用例绿 + `cargo test --test btree_test` 零回归
- Verification: `cargo test --test btree_scale_test` 输出记 Act Response
- Stop when: 根因定位到分裂/搜索结构性设计缺陷且局部修复会破坏平衡不变量 → 返回 Plan

### R-T0b-R3: BTree::delete Page-full 泄漏修复（G2）

- Requirement/Scenario: `wal-recovery-replay-integrity` R2-S1（恢复可用性）
- Depends on: None（与 R2 独立）
- Targets: `src/storage/btree/btree.rs`（delete_from_page/redistribute_*/merge_*/handle_child_merge，根因定位后可含 `node.rs`）
- Current behavior: 10k 树批量 DELETE 偶发 `Page full`；无中位点 checkpoint 混合 WAL 重开 `delete redo ... Page full` 整体失败
- Required behavior: 任意规模批量 DELETE 全部成功；G2 形态 WAL 重开成功且计数精确
- Required changes: Page-full 泄漏根因定位 + 修复（重平衡路径的容量/偏移处理）
- Preserve: B-Tree 平衡不变量；K05（恢复层错误传播语义零修改）；`merge_leaves` 释放页语义（free_page）
- Forbidden: 恢复层容错/跳过；页格式变更
- Test witness（RED 先行）: `tests/btree_scale_test.rs` 用例 ② `bulk_delete_at_scale`——10k 键建库 → 批量 DELETE ≥200 键（含跨区段）全部 AffectedRows(1) 且事后 COUNT 精确（RED：偶发 Page full，Plan 实测 id=242；若单次运行不复发，以 200 键全量+断言提高触发面）+ 用例 ③ `mixed_wal_no_midpoint_checkpoint_reopens`——create→checkpoint→10k INSERT+50 DELETE→drop 不 close→重开 COUNT 精确 9950（RED：`delete redo ... Page full` 打开失败）
- GREEN condition: ②③ 绿 + 既有套件零回归
- Verification: `cargo test --test btree_scale_test` + `--test wal_recovery_large_test` 记 Act Response
- Stop when: 泄漏根因在 merge/redistribute 的结构性设计（如父子指针协议）无法局部消解 → 返回 Plan

### R-T0b-R4: BTree::update 内部节点递归补全（G3）

- Requirement/Scenario: `wal-recovery-replay-integrity` R2-S1（恢复可用性）
- Depends on: None（与 R2/R3 独立）
- Targets: `src/storage/btree/btree.rs::update_in_page`
- Current behavior: 非叶根直接 `Err("Internal node update not implemented yet")`（`btree.rs:1041-1047`）；10k 树 UPDATE 全部失败；恢复侧大树重放点 Update 记录 `RedoFailed`
- Required behavior: 多层树 UPDATE 沿内部节点递归下探至叶更新；键不存在保持 KeyNotFound
- Required changes: 递归下探补全（与 search 下探同构，复用 `leaf.update`）
- Preserve: KeyNotFound 语义；`update` 公共签名
- Forbidden: 页格式变更；全树重建式实现
- Test witness（RED 先行）: `tests/btree_scale_test.rs` 用例 ④ `update_works_at_scale`——10k 键建库 → UPDATE 中部/尾部/边界若干键（跳过最小键——G1 未修时最小键不可达，G1 修复后可含）全部 AffectedRows(1) 且点查/判重反映新位置（RED：`Internal node update not implemented`，Plan 已实测）
- GREEN condition: 用例绿 + 既有套件零回归
- Verification: `cargo test --test btree_scale_test` 记 Act Response
- Stop when: 递归补全需要变更节点磁盘布局 → 返回 Plan（页格式禁区）

### R-T0b-R1: 见证 ② 精确化（依赖 R2/R3/R4）

- Requirement/Scenario: R2-S1 完整语义（更新可见/删除不可见/计数精确）
- Depends on: R-T0b-R2, R-T0b-R3, R-T0b-R4
- Targets: `tests/wal_recovery_large_test.rs::mixed_dml_recovery_semantics`（重写该用例）
- Current behavior: COUNT ±5 容差、UPDATE 效果断言缺失、`let _` 吞 UPDATE 返回（父 Cycle ACT-DEVIATION，Plan Review 裁定）
- Required behavior: 自然序夹具（create→checkpoint→10k INSERT→UPDATE 100 行（含最小键与中部/尾部）→DELETE 50 行→drop 不 close）→ 重开断言全部精确：COUNT == 9950、被更新行 v=9999 可见（点查与全量口径）、被删行 0 行、重复 INSERT 最小键被拒（G1 闭环进见证）
- Required changes: 重写用例（断言精确化 + 移除 `let _`）；既有其余 4 用例零修改
- Preserve: 父 Cycle 既有 4 用例语义零修改
- Forbidden: 任何容差断言；跳过失败语句不计入期望值的口径（期望值 = 已提交终态，运行期失败语句使夹具构造失败即失败）
- Test witness（RED 先行）: 重设计后先于 R2-R4 落地观察 RED（UPDATE 运行期失败 → 夹具构造失败/断言失败），R2-R4 后 GREEN——Act 按「先 RED 记录、后 GREEN」执行
- GREEN condition: 用例绿 + 其余 4 用例绿 + 全量门
- Verification: `cargo test --test wal_recovery_large_test` 记 Act Response
- Stop when: R2-R4 落地后夹具仍无法精确构造（实质问题）→ 返回 Plan

### R-Gate: 全量验证门（T1 收尾重跑）

- Requirement/Scenario: 全部（回归门）
- Depends on: R-T0b-R1
- Targets: 无新代码；`cargo test --all`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate 2026-09-06-ms10-t02-file-lock-graceful-shutdown`
- Current behavior: 父 Cycle 白名单口径全绿（sigkill 绿）
- Required behavior: 白名单（3 信号用例）之外全部通过（新增 btree_scale_test 4 用例 + 重设计的 mixed_dml 全绿）；clippy 0 / fmt 0 / validate PASS
- Test witness: 各命令决定性输出（≤20 行）与退出码
- GREEN condition: 四项达标
- Verification: 输出记 Act Response
- Stop when: 白名单之外的回归失败且无法归因于本 Cycle 修复 → BASELINE-CHANGED 返回 Plan

**依赖注记**：R2/R3/R4 相互独立可任意序实施（各自 RED→GREEN）；R1 严格在后（其夹具依赖三缺口的运行期修复）；R-Gate 收尾。

**Invariants**

- 页格式、WAL 格式、reader/recovery 语义（父 Cycle 冻结面）零变化
- K05 显式报错语义不变；恢复层不新增容错/跳过
- 既有测试断言语义零修改（父 Cycle 5 用例中仅 `mixed_dml_recovery_semantics` 按本契约重写）；4 文件夹具锁适配例外不变
- B-Tree 修复保持平衡不变量与既有磁盘结构；search/delete 复杂度量级不回退
- 不引入哈希/校验和/内容指纹新增；不新建 Evidence 占位目录

**Non-goals**

非 PK 索引、B+Tree 并发/节点级锁（D-candidates）、锁/信号（Iteration 001）、do_flush 并发、页格式与 WAL 格式、多行 INSERT Page full（观察项，若 G2 根因涉及共享容量逻辑可在 Act Response 记录关联性但不扩大修复面）。

**Acceptance**

1. R1-S2 普遍性：最小键重复 INSERT 在运行树与恢复重建树均被拒——`btree_scale_test` ①。
2. R2-S1 任意形态：无中位点 checkpoint 混合 WAL 重开精确——`btree_scale_test` ③ + 重设计见证 ②。
3. 运行期规模正确性：批量 DELETE 与 UPDATE 在 10k 树全部成功——`btree_scale_test` ②④。
4. 回归门：`cargo test --all` 白名单口径全绿、clippy 0、fmt 0、validate PASS——R-Gate。

**Verification**

- `cargo test --test btree_scale_test`（4 用例，先 RED 后 GREEN）
- `cargo test --test wal_recovery_large_test`（5 用例，含重设计 ②）
- `cargo test --all`（白名单口径）+ clippy/fmt/validate
- 全部输出（每项 ≤20 行）+ 退出码记 Act Response

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | G1-G3 全部有 Plan 独立复现数据（探针 recipe 与错误原文在 Current-State Evidence）；代码面定位到具体函数（btree.rs:1027-1047/530-568/250,333）；G4 口径修正有精确对账（9951 = 10000−49） |
| Design | PASS | 三缺口在 B-Tree 本体修复的取舍已定（恢复层绕行被拒：K05/索引一致性）；G3 实现路径明确（与 search 同构递归）；G1/G2 根因定位为 Act 首步并以最小化 RED 锚定；无 TBD 阻塞 |
| Iteration Plan | PASS | Map 不变；repair item 形式；平衡审计：单一「B-Tree 多页正确性 × 恢复验收」成果域，故障域集中 btree.rs |
| Cycle Scope | PASS | 既有 Acceptance 的必要条件（iteration-planning 判断问题 = 是）；**B-Tree 修复面扩大需用户 Gate 2 明确批准**（超出原 T0b Change Surface） |
| Task Contracts | PASS | 5 个 repair item/gate 均含 Targets/行为/Preserve/Forbidden/RED-GREEN/Verification/Stop |
| Traceability | PASS | R1-S2→R2；R2-S1→R3/R4/R1；回归门→R-Gate； witnessed by btree_scale_test + mixed_dml 重设计 |
| Verification | PASS | 验证直接证明目标行为；Persisted Evidence = none |

**Persisted Evidence**

- Mode: none

Act Response 承载全部决定性输出；RED 场景（G1/G2/G3）由固定夹具低成本重跑。

- Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个 Evidence 文件；单个文本文件最多 500 行且不超过 256 KiB。

**Risks and Notes**

1. G1/G2 的根因尚未定位到行级（Plan 定位到函数族与复现形态）——Act 首步为最小化 RED + 根因；若命中结构性设计缺陷按各自 Stop-when 返回 Plan，不以局部 hack 消解。
2. `btree_scale_test` 各用例 10k 规模在 debug 模式实测建库 ~2s（Plan 探针），4 用例总时长可接受。
3. G2 的「偶发」性（id=242 复现、id=300/1000/9000 不复现）——RED 用例以批量 DELETE（≥200 键）提高触发面；若仍不稳定，以恢复路径形态（用例 ③，必现）为 RED 锚，运行期用例作回归守护。
4. 修复 G1 后，父 Cycle 见证 ① 的判重探针（id=42）逻辑不变；`mixed_dml` 重设计新增最小键判重使 G1 进入恢复见证闭环。
5. `WALBuffer::do_flush` 并发互斥、多行 INSERT Page full 维持既有观察项不变。
6. 本 Cycle 完成后 Iteration 000 才算达成 D7 修订版 Stable baseline；Iteration 001（锁/停机）不受影响。

## Act Response

- Status: pending

（Act 填写）

## Plan Review

- Review Result: pending

（Plan 填写）
