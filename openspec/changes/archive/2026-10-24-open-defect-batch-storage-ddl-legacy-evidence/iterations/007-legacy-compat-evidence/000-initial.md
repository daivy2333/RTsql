# Iteration 007 / Cycle 000: 旧格式兼容的证据补齐

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T16, T17, T18, T19, T20, T21
- Acceptance gaps: None
- Repair items: None
- Inherited scope:
  - 本 change 前七个 Iteration 的**已验收**成果全部为前置：Iteration 000（索引页归属唯一性 / 下降终止）、Iteration 001（列改写的数据链一致性与放弃可见性）、Iteration 002（释放路径的有界读）、Iteration 003（改写窗口的读一致性）、Iteration 004（释放可分配判定的耐久性前提）、Iteration 005（DML 墓碑的放弃窗口收敛）、Iteration 006（索引树读路径的有界读与显式拒绝）。
  - **零产品代码改动**是本 Iteration 的形态约束（D10）：交付物是**固定样本 + 常态化用例 + 记录更正 + 规格对齐 + 收尾全量回归**。
  - 既有域 `legacy-format-compat`（MS36 新增域）的**支持范围与代际定义**不变（G1 / G2 / G3 全部在范围内；兼容由读取路径的「可选尾段缺失即取默认值」实现；不做格式嗅探 / 猜测性迁移 / 写时全库重排）。
  - 既有域 `release-path-bounded-read`（本 change 新增域，Iteration 002 产出）与既有域 `mvcc-tombstone-visibility`（MS35 前既有域，Iteration 005 的 AR2 落点）的**行为语义**不变；本 Iteration 只对齐**措辞**与**证据索引**。
  - M19 分批串行全量回归纪律（公共规则 + `.agents/runbooks/m19-batched-full-regression.md` R48）是 T21 的执行依据；本 change 的全量回归预算**只有这一次**。
- Excluded scope:
  - G1 / G3 的样本扩充与代际定义的修改（MS36 已验收）。
  - 旧 dump 文本的支持范围（MS36 已验收；范围内 1 份 + 范围外 3 份的判定不变）。
  - 加密旧库的专项断言（MS36 已记录为不覆盖）。
  - 产品代码的行为变更（本 Iteration **零产品代码改动**；任何需要改产品代码的发现 ⇒ 回到 Plan）。
  - Iteration 006 登记的两条 Issue 候选（IC-1 叶页条目长度判据 / IC-2 其余页型的无界读点）——由 Recorder 择承载，**不在**本 Iteration。
  - `openspec/specs/` 的合并（属 `openspec-docs-maintainer`；本 Iteration 只交付**增量与交接清单**）。

**Objective**

本 Cycle 完成后应达到的可验证结果：

1. **G2 代际有了真实来自旧构建的文件级样本**（`tests/fixtures/legacy/legacy_g2.db`），样本固定入库为**固定输入**，来源说明（`tests/fixtures/legacy/README.md`）与样本**同源同步**；
2. **G2 样本有了常态化断言**（打开 / 查询 / 写入 / 完整性 / 打开时重建 / dump / 恢复），**派生量**（行数 / 恢复行数 / 目录行长度等）由断言锁定并与来源说明同步；
3. **MS36 的三条一次性兼容条款转为常态化用例**（恢复出的库可继续写入 / 旧 G1 可由当前构建 dump / G1 可连续写两次），落在 `tests/legacy_compat_test.rs`；
4. **MS36 迁移路径实测记录的命令列**改为**可直接执行**的形式（路径形式），并保留「结论本身可复现、失配只在命令写法」的事实；
5. **本 change 的规格增量与验收边界对齐**（含 Iteration 005 的 AR2 措辞澄清、Iteration 002 遗留的「尽力而为」AND 子句、Iteration 006 登记的 IRBR Requirement 措辞收敛）+ **收尾交接清单**就绪；
6. **本 change 的收尾全量回归**（M19 分批串行）跑完：**全部批次各自 0 failed**、覆盖对账不重不漏、与基线的差额由实际结果说明。

**Investigation Facts**

- Starting point:
  - HEAD `0a08b14`（ahead of origin 9）。工作区带有**未提交**的改动：MS34 + issue-fix + MS35 + MS36 + 本 change 七个 Iteration 的成果（`src/` 下 33 个条目、`tests/` 下 25 个条目、`tests/fixtures/legacy/`、`README.md` / `README.zh-CN.md`、`.agents/` 与 `openspec/specs/` 下的文档改动）。**该现场属既有状态**：本 Iteration 不回滚、不清理、不提交。
  - Iteration 006 已 `accepted`（Plan Review 2026-10-25，`iterations/006-index-read-bounded-read/000-initial.md`）：改动 = `src/storage/btree/{node,btree,index_manager}.rs` + `src/storage/error.rs` + 新增 `tests/index_read_bounded_read_test.rs`；验证结论 = `index_read_bounded_read_test` 5/0、`index_manager_test` 3/0、`secondary_index_test` 27/0、`index_page_ownership_test` 5/0、`index_page_ownership_stack_overflow_test` 1/0、`release_path_bounded_read_test` 7/0、`cargo test --lib` 420/0，另重跑 `ddl_rewrite_consistency_test` 9/0、`release_durability_precondition_test` 2/0、`dml_tombstone_abandon_window_test` 3/0、`mvcc_tombstone_visibility_test` 11/0、`free_page_lifecycle_test` 12/0。
  - **本 change 至今未跑过全量回归**：全量预算在本 Iteration（T21）。MS36 于 2026-10-24 实测的基线（`tests/*.rs` 135 个文件 + `--lib --bins` = **136 个目标 / 13 批**；终态 **2192 passed / 0 failed / 2 ignored**）**对当前工作区已失效**——此后有七个 Iteration 的改动（含 6 个新增测试文件与既有文件的改动）。
  - 已知限制：MS36 的「未决日志样本恢复行数参考值 133」是**已提交样本字节的属性**（确定值）；重新生成样本会改变该值，届时须同步更新来源说明与用例常量。
- Current-State Evidence:
  - **G1 样本的来源与再生流程（可直接照搬到 G2）**：`tests/fixtures/legacy/README.md`——旧构建 revision `d8165fc634fa650d839811b1d6b0165e204cad01`（2026-09-24）；旧构建方式 = `git clone --no-hardlinks --shared <repo> /tmp/rtsql-legacy && git checkout d8165fc && cargo build --offline --bin rtsql`（**构建在仓库之外的隔离目录**，不触碰工作区）；旧构建产物 `/tmp/rtsql-legacy/target/debug/rtsql`；工具链 rustc / cargo 1.99.1（`--offline`）；样本内容清单（`legacy_g1.db` 41024 字节、`legacy_g1_pending_wal.db` / `.wal`、`legacy_dump.sql` 及三份范围外 dump 文本）；再生步骤第 1–6 步；已知限制（恢复行数参考值、`null_in_not_null` 是旧 dump 文本的一处最小编辑、`discarded` 表的空闲页、样本规模 ≪ 300、不覆盖加密旧库）。
  - **G1 样本的派生量与断言位点**：`tests/legacy_compat_test.rs`——`RECOVERED_ROWS_REFERENCE: usize = 133`（`:601`）、`g1_tables_row_len`（`:164`）/ `g1_columns_row_len`（`:205`）锁定 G1 目录行布局、`find_table_row`（`:175`）/ `newest_table_row`（`:182`）/ `column_row_names`（`:188`）定位目录行；现有用例 `legacy_generation_db_is_in_supported_range`（`:229`）/ `legacy_db_opens_queries_and_writes`（`:315`）/ `legacy_db_write_upgrades_touched_table_row_to_current_layout`（`:403`）/ `legacy_db_open_rebuild_keeps_live_table_intact`（`:549`）/ `legacy_db_with_pending_wal_recovers_committed_rows`（`:610`）/ `legacy_dump_restores_into_current_db`（`:656`）+ 三份范围外 dump 的拒绝用例（`:747` / `:777` / `:808`）。
  - **G2 的代际定义**：`openspec/changes/2026-10-24-ms36-legacy-format-and-dump-compat/specs/legacy-format-compat/spec.md:12-14`——**G2** = `__tables` 行**含唯一根段**、**不含二级索引段与表类型字节**；`__columns` 行**可含**默认值段。
  - **G2 的旧构建 revision（已独立核实）**：`6e8dcf551aa53d2a08dc6f20097f1ad5e50fc464`（2026-09-26，「MS23+MS24 收尾统一入库」）。核实方式：`git show 6e8dcf5:src/storage/catalog.rs` 的 `serialize_catalog_row`（该 revision 的 `:614-646`）布局为 `u16 name_len | name_bytes | u32 data_page_head | u32 index_root_page_id | u32 pk_index | u16 pk_col_len | pk_col_bytes | u32 column_count | u32 data_page_tail | u32 unique_count | unique_count × u32 root` ⇒ **唯一根段恒在**（G2 的判别特征）且**二级索引段 / 表类型字节不在**（该 revision 的 `CatalogRow` 无 `secondary_indexes` / `kind` 字段）⇒ **该 revision 产出的库即 G2 代际**；同 revision 的 `serialize_column_row` 追加可选 DEFAULT 尾段（MS24）⇒ 与 G2 定义「`__columns` 行可含默认值段」一致。
  - **G2 的证据缺口（本 Iteration 的承载目标）**：MS36 `legacy-format-compat` 规格的证据索引 **E7** 明确记载：「**没有**按 G2 形态构造的用例，**仓库内也没有 G2 布局的文件级样本**」；能力矩阵的 G2 行除「打开 / 查询」外全部为「无 G2 专属断言（E7）」⇒ T16（样本）+ T17（断言）正是关闭 E7 的动作。
  - **三条一次性条款的承载目标（T18）**：MS36 `legacy-format-compat` 的证据索引 **E2**（「写入后仍能再次写入」由「本 change 的真实 CLI 实测记录」支撑）、**E4**（「当前构建导出 G1 旧库」由「本 change 的真实 CLI 实测记录」支撑），以及 **S1**「恢复出的库可继续写入」——三者的支撑记录在 MS36 `iterations/001-contract-and-baseline/000-initial.md:314-317` 的「S1…与另两处子句的实测补齐」表 ⇒ 需转为 `tests/legacy_compat_test.rs` 内的常态化用例。
  - **命令列失配的承载目标（T19）**：MS36 `iterations/001-contract-and-baseline/000-initial.md:300-317` 的两张表的「命令」列写的是**裸名**形式（`rtsql new p1.db` / `rtsql new p2a.db` / `rtsql restore p2b.db p2b.sql` / `rtsql new s1.db` / `rtsql dump g1_dump_probe.db`）。`src/cli/resolve.rs:30` 的 `resolve_db_path` 对**不含 `/`** 的参数解析为 `$RTSQL_HOME/db/<name>.db`（`$RTSQL_HOME` 未设置时 `$HOME/.rtsql/db/<name>.db`）⇒ **按记录原文字面执行会以「does not exist」退出 1**；须用**路径形式**（含 `/` ⇒ 按路径直接使用）。MS36 的 Plan Review 已把这一点记为 **M1**（同文件 `:475`）：「**所记录的退出码与结果本身经本 Review 复现全部成立**，失配的只是记录里的命令写法」。
  - **两条遗留的承载目标（T20）**：① **AR2 措辞澄清**——Iteration 005 的 Plan Review **F3** 裁定「Acceptance 2 的字面读法与 Acceptance 1 自相矛盾」，实际实现取**唯一自洽读法**（无残留条目：同键条目数恰好 1 ⇒ 行真正删除后同键可再插入 ⇒ 再插入后点查仍命中 1 行），AR2 以该读法为准 ⇒ 需在**既有域 `mvcc-tombstone-visibility`** 的措辞上澄清；② **RPBR 的「尽力而为」AND 子句**——Iteration 002 遗留：既有域 `release-path-bounded-read` 的「删索引」Scenario **缺少**另两个 Scenario 都有的「按尽力而为语义结束」平行 AND 子句。
  - **Iteration 006 登记的第三条对齐项（T20，Plan Review 追加）**：IRBR 增量 `specs/index-read-bounded-read/spec.md` 的 Requirement 句「读取到损坏的索引页时，系统 SHALL 返回明确的错误」比**已验收**的行为略宽——页级判据（`integrity::check_tree_page`）对**叶页**只校验槽边界、不校验条目长度 ⇒ 短于条目格式（38 字节）的叶槽不被判为损坏、其条目被**静默跳过**（详见该 Cycle 的 Plan Review F6 / Issue 候选 IC-1）⇒ 需把该句**收敛为「未通过页级判据的索引树页」**（行为级措辞，不引入实现术语），使并入 `openspec/specs/` 的规范只陈述**已验收**的行为。
  - **全量目标清单（T21 的对账基准，按实际文件列表重算）**：`tests/*.rs` 当前 **142** 个文件（`ls tests/*.rs | xargs -n1 basename | sort` 的实际输出）⇒ 加 `--lib --bins` = **143 个目标**；与 MS36 基线（135 + 1 = 136 目标 / 13 批）相差 **7 个目标**，且基线的 2192/0/2 计数**已失效**（七个 Iteration 的新增用例与改动之后）⇒ 差额**必须**由实际结果说明，不得以基线数字反推结论。
- Code and Critical Path:
  - **G2 样本产出链**：仓库之外的隔离克隆 → `git checkout 6e8dcf5` → `cargo build --offline --bin rtsql` → 旧 CLI（`new` / `CREATE TABLE` / `INSERT` / `DROP TABLE` / `dump`）在 `/tmp/...` 内产出库文件 → 只把**样本本体**复制进 `tests/fixtures/legacy/`（伴生 `.wal` / `.checkpoint` 不入库）→ 运行时由测试复制到临时目录再打开。样本是**固定输入**：运行时**不得**伪造版本号 / 行布局，**不得**用 hash / revision 比对。
  - **G2 断言链**：`tests/legacy_compat_test.rs` 的夹具（`fixture_path` `:42` / `copy_fixture` `:51` / `run_cli` `:71` / `cli_error_message` `:85` / `cli_rows` `:96` / `db_rows` `:107` / `read_chain_rows` `:120`）→ `Database::open` → 查询 / 写入 / 完整性扫描 / 打开时重建 / `dump` / 恢复。
  - **目录行代际的可观测形态**：`__tables` 行的长度与字段构成（唯一根段的在场与否）可由 `read_chain_rows` + `tables_row_name` / `g1_tables_row_len` 同形态的辅助读出 ⇒ G2 的判别特征（唯一根段在、二级索引段与表类型字节不在）**可由断言直接观测**，不靠版本号或 revision。
  - **命令解析链**：`src/cli/resolve.rs:30` `resolve_db_path`（含 `/` ⇒ 按路径；裸名 ⇒ `$RTSQL_HOME/db/<name>.db`）⇒ 记录的命令列必须写成**含 `/` 的路径形式**才可字面执行。
  - **T21 的执行链**：`cargo test --lib --bins -- --test-threads=1`（独立一批）+ 每批一次 `cargo test --test <f1> --test <f2> … -- --test-threads=1`；判定只看 cargo 自己的退出码与原生输出，计数用**一步通用管道**读出。

**Implementation Guidance**

建议顺序（每个 Task 独立可验证、可诊断）：

1. **T16**（样本）→ **T17**（断言）：先固定**样本字节**，再让断言锁定它的**派生量**。顺序不可颠倒——先写断言再产样本会让断言去迁就字节。
2. **T18**（三条常态化用例）：复用 `tests/legacy_compat_test.rs` 既有的夹具与断言风格（G1 样本已在库内 ⇒ 无需产新样本）。
3. **T19**（记录更正）：**只改 MS36 记录的命令列写法**，不改任何已记录的退出码与结果；更正后须保留「失配只在命令写法」这一事实的表述。
4. **T20**（规格对齐 + 交接清单）：三条措辞对齐（AR2 / RPBR / IRBR）+ 交接清单；**不改**任何域的**行为要求**，只收敛措辞与更新证据索引指针。
5. **T21**（收尾全量）：**最后**执行；本 change 的唯一一次全量预算。

关键技术细节与取舍：

- **G2 样本的产出纪律**（C-1）：样本必须**真实来自旧构建**——`git clone --no-hardlinks --shared <repo> /tmp/rtsql-legacy && git checkout 6e8dcf5 && cargo build --offline --bin rtsql`，**在仓库之外**的隔离目录执行（不触碰工作区现场）；构建产物用于产出样本后，只把**样本本体**复制入库。**禁止**：运行时伪造版本号或行布局；禁止 hash / revision 比对（样本只是固定输入）；禁止在仓库内构建旧 revision。
- **派生量**：G2 样本的派生量（表清单 / 行数 / 目录行长度构成 / 若做未决日志样本则恢复行数参考值）**必须**由断言锁定，并**与 `tests/fixtures/legacy/README.md` 的来源说明同步**（两处写同一组数字）。派生量不一致时**先核对样本字节是否被重新生成**，不得就地改断言常量迁就样本。
- **T18 的三条用例**必须是**规范化**的自动化用例（进入 `cargo test` 的判定面），不得继续只靠一次性 CLI 实测记录支撑；断言要走**真实入口**（`Database::open` + 真实 SQL）。
- **T21 的纪律**见下方 T21 的 Task Definition（12 条，逐条写死）。

**Task Definitions**

### T16: G2 代际的文件级样本产出并固定入库

- Requirement/Scenario: 既有域 `legacy-format-compat`（MS36 新增域）的 G2 行能力矩阵与证据索引 **E7**（「仓库内也没有 G2 布局的文件级样本」「无 G2 专属断言」）；本 change 的 T16。
- Depends on: None
- Targets: `tests/fixtures/legacy/legacy_g2.db`（**新增样本**，固定输入）；`tests/fixtures/legacy/README.md`（**新增 G2 的来源行与样本内容行 + 再生步骤**）。
- Current behavior: 仓库内只有 G1 代际的文件级样本（`legacy_g1.db` 等，见 `tests/fixtures/legacy/README.md` 的样本清单）；G2 只有行级覆盖（E6），**没有**按 G2 形态构造的文件。
- Required behavior: 由**旧构建**（revision `6e8dcf5`，2026-09-26）产出的一个**真实库文件**入库为固定输入；该库的 `__tables` 行**含唯一根段、不含二级索引段与表类型字节**（= G2）；来源说明写明：旧构建 revision、旧构建方式（隔离克隆 + `cargo build --offline --bin rtsql`）、构建产物路径、工具链、**样本的代际判别特征**、样本内容清单与**各样本的预期结论**；再生步骤写全（与 G1 的第 1–6 步同形）。
- Preserve: G1 样本与其来源说明**逐字不变**（本任务只**追加** G2 的行）；样本规模 ≪ 300 行（ISS23 约束）；伴生 `.wal` / `.checkpoint` **不入库**（与 G1 的既有做法一致）；**不触碰**工作区的未提交现场。
- Forbidden: **不得**在仓库内构建旧 revision 或修改工作区来产样本；**不得**在运行时伪造版本号 / 代际标记 / 行布局；**不得**用 hash / revision / 时间戳比对样本；**不得**手工改写样本字节去凑某个断言；**不得**改动产品代码。
- First test: 无先行产品测试（本任务交付的是**固定输入**与其来源说明；**派生量的断言在 T17**）。先行性体现在：**先固定样本字节，再由 T17 的断言锁定其派生量**。
- GREEN condition: `tests/fixtures/legacy/legacy_g2.db` 存在且可被当前构建打开；来源说明含 G2 的 revision / 构建方式 / 产物路径 / 工具链 / 代际判别特征 / 内容清单 / 预期结论 / 再生步骤。
- Verification: `cargo test --test legacy_compat_test -- --test-threads=1` ⇒ 本任务**不新增**断言 ⇒ 既有 G1 用例仍 0 failed（回归不破）；样本的可打开性由 T17 的用例最终判定。**人工核对项**（最短操作 + 直接可观察结果）：读 `tests/fixtures/legacy/README.md` 的 G2 行 ⇒ 能逐条对上「revision / 构建方式 / 代际特征 / 再生步骤」四要素。
- Stop when: 旧 revision **无法在离线条件下构建**（`cargo build --offline` 失败）或**无法产出可被当前构建打开的库** ⇒ 记入 Act Response 并**回到 Plan**（换 revision 或调整方案），**不得**用手工构造 / 伪造的样本替代；样本的代际特征**无法**在文件字节层被观测 ⇒ 回到 Plan。

### T17: G2 样本的常态化断言（派生量由断言锁定并与来源说明同步）

- Requirement/Scenario: 同 T16（既有域 `legacy-format-compat` 的 G2 能力矩阵行 + 证据索引 E7 的关闭）；本 change 的 T17。
- Depends on: T16
- Targets: `tests/legacy_compat_test.rs`（**新增 G2 用例**，沿用既有夹具与断言风格）。
- Current behavior: `tests/legacy_compat_test.rs` 的 9 条用例全部针对 G1 样本（打开 / 查询 / 写入 / 行升级 / 打开时重建 / 未决日志恢复 / dump 恢复 / 三份范围外 dump 拒绝）⇒ G2 在文件级**零覆盖**。
- Required behavior: 为 G2 样本建立**文件级**的常态化断言，覆盖能力矩阵的 G2 行：**打开 / 查询 / 写入 / 完整性 / 打开时重建 / dump / 恢复**七项；每项走**真实入口**（`Database::open` + 真实 SQL / 真实 CLI `dump`）；**派生量**（表清单、行数、目录行长度构成等）由**断言**锁定，并与 `tests/fixtures/legacy/README.md` 的「各样本的预期结论」**同源同步**（两处写同一组数字）。
- Preserve: G1 的既有 9 条用例与其断言**逐条不变**（G2 用例为**新增**）；G1 样本与其派生量常量**不变**；MS36 已验收的支持范围与代际定义**不变**；样本规模 ≪ 300。
- Forbidden: **不得**放宽或改写 G1 的既有断言来迁就 G2；**不得**用「行数 ≥ 1」这类无判别力的断言代替派生量锁定（MS36 的既有教训：只断言「≥ 1」时未决日志未被重放也会通过）；**不得**把 G2 的断言写成只依赖 G1 样本的间接断言；**不得**改动产品代码。
- First test: 新增 G2 用例后**先观察 RED**：在 T16 的样本**尚未入库**时（样本文件不存在），G2 用例必须**失败**（夹具找不到样本）⇒ 证明用例确实依赖 G2 样本，而不是恒真。
- GREEN condition: `cargo test --test legacy_compat_test -- --test-threads=1` ⇒ G2 用例全部 passed / 0 failed，且 G1 的 9 条用例仍 0 failed；G2 的派生量与来源说明**逐项一致**。
- Verification: `cargo test --test legacy_compat_test -- --test-threads=1`（全部 0 failed）。
- Stop when: G2 样本的某个能力项在**当前构建**上行为与代际定义不符（例如打开失败 / 查询不命中 / 写入后目录行形态异常）⇒ **回到 Plan**（该形态可能超出「兼容读分支」的既有承载），**不得**改判据或改断言迁就；派生量无法锁定（每次运行都不同）⇒ 回到 Plan。

### T18: 三条一次性兼容条款转为常态化用例

- Requirement/Scenario: 既有域 `legacy-format-compat` 的证据索引 **E2**（写入后仍能再次写入）、**E4**（当前构建导出 G1 旧库）、**S1**（恢复出的库可继续写入）——三者当前**只**由 MS36 的一次性 CLI 实测记录支撑（`openspec/changes/2026-10-24-ms36-legacy-format-and-dump-compat/iterations/001-contract-and-baseline/000-initial.md:314-317`）；本 change 的 T18。
- Depends on: None
- Targets: `tests/legacy_compat_test.rs`（**新增三条常态化用例**）。
- Current behavior: 三条条款各自有一次性实测记录（退出码与结果均成立），但**没有**进入 `cargo test` 的常态化判定面 ⇒ 后续回归无法保护它们。
- Required behavior: 三条用例在 `tests/legacy_compat_test.rs` 内落地，走**真实入口**：① **恢复出的库可继续写入**——把 `tests/fixtures/legacy/legacy_dump.sql` 恢复到当前空库后**继续写入**并读回（断言写前后的行集合变化）；② **旧 G1 可由当前构建 dump**——当前构建对 G1 旧库 `dump` **退出码 0**、导出 3 表 + 全部行，且导出的文本可被当前构建恢复（往返等价）；③ **G1 可连续写两次**——同一 G1 旧库上**连续两次**写入，两次退出码均为 0，写入后查询可见。
- Preserve: G1 / G2 样本与其派生量**不变**；MS36 已验收的**范围外**判定（三份范围外 dump 的退出码 3 与错误文本）**不变**；既有 9 条用例**不变**；磁盘格式**不变**。
- Forbidden: **不得**只做一次性 CLI 实测了事（必须是常态化用例）；**不得**只断言「退出码 0」而不断言**可观察的结果**（导出的表清单与行、写入后读回的行）；**不得**改动产品代码；**不得**把一次性记录删掉而只留用例（记录与用例并存，记录是历史现场）。
- First test: 新增用例后**先观察 RED**：把用例断言中的**可观察结果**临时改成与实现不符的期望（例如期望恢复出的库写入后行数比实际多 1）⇒ 必须失败 ⇒ 证明断言有判别力；随后改回正确期望。
- GREEN condition: `cargo test --test legacy_compat_test -- --test-threads=1` ⇒ 三条新用例 passed / 0 failed；既有 9 条 + G2 用例仍 0 failed。
- Verification: `cargo test --test legacy_compat_test -- --test-threads=1`（全部 0 failed）。
- Stop when: 三条条款中任一条在**当前构建**上已**不成立**（行为与 MS36 的实测记录不一致）⇒ **回到 Plan**（属行为变更，不属本 Iteration 的证据补齐），**不得**改断言迁就。

### T19: 迁移路径实测记录的命令列更正为可直接执行的形式

- Requirement/Scenario: MS36 `iterations/001-contract-and-baseline/000-initial.md` 的 **R5 三条迁移路径逐条实测**表与 **S1…另两处子句的实测补齐**表（`:300-317`）的「命令」列；MS36 Plan Review 的 **M1**（「所记录的退出码与结果本身经本 Review 复现全部成立，失配的只是记录里的命令写法」）；本 change 的 T19。
- Depends on: None
- Targets: `openspec/changes/2026-10-24-ms36-legacy-format-and-dump-compat/iterations/001-contract-and-baseline/000-initial.md`（**只改「命令」列的写法**）。
- Current behavior: 命令列写的是**裸名**形式（`rtsql new p1.db` / `rtsql new p2a.db` / `rtsql restore p2b.db p2b.sql` / `rtsql restore p3a.db p3a.sql` / `rtsql new s1.db` / `rtsql dump g1_dump_probe.db`）⇒ `src/cli/resolve.rs:30` 把裸名解析为 `$RTSQL_HOME/db/<name>.db` ⇒ **按记录原文字面执行会以「does not exist」退出 1**，与记录的退出码 0 不符。
- Required behavior: 命令列改为**含 `/` 的路径形式**（例如 `rtsql new /tmp/ms36-migration-paths/p1.db` → `rtsql restore /tmp/ms36-migration-paths/p1.db /tmp/ms36-migration-paths/p1_edited.sql`），使**照字面执行即可复现**；**已记录的退出码与结果逐条不变**；记录中保留「失配只在命令写法、结论本身可复现」这一事实（可引用 M1）。
- Preserve: MS36 该 Cycle 的**已冻结**内容不被改写（Plan Context 与 Act Response 原文**不改**；只改**记录正文**的「命令」列）；已记录的退出码 / 结果 / 结论**逐字不变**；记录中其余表格与段落**不动**。
- Forbidden: **不得**改动退出码或结果以迁就新写法；**不得**改动该文件的 Plan Context / Act Response 段；**不得**改产品代码；**不得**顺手修改 MS36 的其它记录（超出命令列的更正**不在本任务范围**）。
- First test: 更正**之前**，按记录原文的命令列**字面执行**一次 ⇒ 必须观察到与记录**不符**的失败（退出码非 0 / 「does not exist」）⇒ 证明失配真实存在；该观察写入 Act Response。
- GREEN condition: 更正后命令列的每一条命令**含 `/`**；按更正后的命令列字面执行 ⇒ 退出码与记录一致（0 / 3 / 0 逐条）；记录中**未**出现任何裸名形式的 `rtsql <name>.db`。
- Verification: **人工核对项**（最短操作 + 直接可观察结果）：读该文件的 `:300-317`，逐条检查「命令」列每个 `rtsql` 调用**第一个路径参数含 `/`**、且**退出码列**未变；并把「字面执行 → 退出码与记录一致」作为一条可观察结论记入 Act Response。
- Stop when: 更正所需的路径形式与 MS36 记录当时的工作目录 / 临时目录约定**冲突**（例如记录明确实现在特定目录下的相对路径）⇒ 回到 Plan。

### T20: 规格增量与验收边界对齐 + 收尾交接清单

- Requirement/Scenario: ① **AR2 措辞澄清**——既有域 `mvcc-tombstone-visibility`（Iteration 005 的 Plan Review **F3**：Acceptance 2 的字面读法与 Acceptance 1 自相矛盾，实际实现取「无残留条目」读法）；② **RPBR 的「尽力而为」AND 子句**——既有域 `release-path-bounded-read`（Iteration 002 遗留：「删索引」Scenario 缺另两个 Scenario 都有的「按尽力而为语义结束」平行 AND 子句）；③ **IRBR Requirement 措辞收敛**——本 change 新增域 `index-read-bounded-read`（Iteration 006 的 Plan Review **F6**：页级判据对叶页只校验槽边界 ⇒ 短于条目格式的叶槽不被判为损坏、其条目被静默跳过 ⇒ Requirement 的「损坏的索引页」须收敛为「**未通过页级判据的索引树页**」）；④ 本 change 的**收尾交接清单**；本 change 的 T20。
- Depends on: None
- Targets: `openspec/specs/mvcc-tombstone-visibility/spec.md`（既有域的措辞对齐）、`openspec/specs/release-path-bounded-read/spec.md`（既有域的并行子句补齐）、`openspec/changes/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/specs/index-read-bounded-read/spec.md`（本 change 的增量措辞收敛）、`openspec/changes/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/tasks.md`（收尾交接清单 + RTM 的状态同步）。
- Current behavior: ① AR2 的字面读法与 AR1 矛盾（Iteration 005 已按唯一自洽读法实现并留待本任务澄清）；② RPBR 的「删索引」Scenario 缺平行 AND 子句；③ IRBR 的 Requirement 比已验收行为略宽；④ 收尾交接清单未成文。
- Required behavior: 三处**只改措辞 / 只补子句**，**不改任何行为要求**；交接清单写明：各增量域的最终状态、**`legacy-format-compat` 证据索引更新**（新增 G2 的文件级证据指针 E-G2，并更新能力矩阵 G2 行）**交 `openspec-docs-maintainer`** 的交接项、本 change 的 T21 全量结论、以及 Iteration 006 登记的两条 Issue 候选（IC-1 / IC-2）的移交说明。
- Preserve: 各域的**行为要求**逐条不变（只收敛措辞与补齐并行子句）；既有域 `mvcc-tombstone-visibility` / `release-path-bounded-read` 的**已验收场景**不变；本 change 的**增量域集合**不变（只新增 `index-read-bounded-read`，不新开域）；`openspec/specs/` 的**合并**归 `openspec-docs-maintainer`（本任务只交**增量 + 交接清单**，不代做合并）。
- Forbidden: **不得**借措辞对齐**新增或放宽**任何行为要求；**不得**改动产品代码；**不得**在 `openspec/specs/` 中把尚未验收的行为写成已验收；**不得**代替 `openspec-docs-maintainer` 做合并或全局状态维护。
- First test: 纯文档任务，**无先行产品测试**；先行性体现在**先写出三处「改前 / 改后」措辞对照**再落笔 ⇒ 对照中的每条改后措辞都必须是**已验收行为**的准确表述（逐条指向 Iteration 002 / 005 / 006 的已验收结论）。
- GREEN condition: 三处措辞对齐落地；改后措辞**不引入**任何新的行为断言；交接清单覆盖上述四项。
- Verification: **人工核对项**（最短操作 + 直接可观察结果）：逐条读三处改后措辞 ⇒ 每条都能对应到本 change 或既有域的**已验收**结论；读交接清单 ⇒ 四项齐备。
- Stop when: 措辞对齐**需要改动行为要求**（而非措辞）⇒ 回到 Plan；交接清单需要 `openspec-docs-maintainer` 的**全局状态**（SNAPSHOT / tasks / M / R / I）才能写完 ⇒ 只写**交接项**，把全局状态维护留给 Maintainer。

### T21: 本 change 的收尾全量回归（M19 分批串行）

- Requirement/Scenario: change 的「收尾验证：M19 分批全量回归」；`.agents/runbooks/m19-batched-full-regression.md`（R48）；公共规则 › 验证；本 change 的 T21。
- Depends on: T16, T17, T18, T19, T20
- Targets: `tests/*.rs` 的**全部**测试目标 + `--lib --bins`；`openspec/changes/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/tasks.md`（全量结论与对账记录）。
- Current behavior: 本 change 至今**未跑过全量**（预算在本任务）；MS36 于 2026-10-24 的基线（136 目标 / 13 批 / **2192 passed / 0 failed / 2 ignored**）**对当前工作区已失效**（此后有七个 Iteration 的改动；当前 `tests/*.rs` = **142** 个文件 ⇒ 目标 **143** 个）。
- Required behavior: 按下列 **12 条纪律**执行一次全量回归，并**逐批记录** passed / failed、做覆盖对账、说明与基线的差额。
  1. `--lib --bins` **单独成一批**。
  2. `tests/*.rs` 按**文件名字典序**分组，每批 **8–12 个测试目标**。
  3. 每批**一次独立调用**：`cargo test --test <f1> --test <f2> … -- --test-threads=1`。
  4. 批间**顺序执行**，同一时刻**只有一批**在跑。
  5. **禁止**一次无目标过滤的全量调用（无论串行还是默认并行度）。
  6. **禁止**用默认并行度跑任何全量批次（每批必须带 `--test-threads=1`）。
  7. 每批**退出码 0** 且其 `test result:` 的 **failed 计数为 0**。
  8. 计数只用**一步管道**（通用工具）读出（例：`… 2>&1 | grep -E "^test result:"`），管道**只做计数不做判定**；**不得**新建判定脚本、封装命令或判定层。
  9. **逐批**记录 passed / failed（批次号、目标清单、退出码、passed、failed）。
  10. 与**目标清单**做**覆盖对账**：目标 = 执行时 `tests/*.rs` 的**实际**全部文件 + `--lib --bins` ⇒ **不重不漏**（每个目标恰好出现一次）。
  11. 全量 Gate 等价于「**全部批次各自 0 failed**」；**缺一批不得声明全量通过**。
  12. 与**基线**做总数对账：基线 = MS36 的 **2192 passed / 0 failed / 2 ignored**（13 批 / 136 目标），但**该基线对当前工作区已失效**（此后有 7 个 Iteration 的改动）⇒ 差额**必须由实际结果说明**（新增用例数 + 目标数变化），**不得**用基线数字反推结论，也**不得**把差额当作失败。
- Preserve: 本 change 的**已验收行为**（八个 Iteration 的成果）零回归；测试目标的**覆盖范围**不因分批而缩小（对账不重不漏）；`--lib --bins` 独立成批（不与 `tests/*.rs` 混批）；既有 `cargo test` 命令的**判定面**不被替换（退出码 + 原生输出）。
- Forbidden: **不得**用一次无目标过滤的全量调用替代分批；**不得**用默认并行度跑任何一批；**不得**并发跑多批；**不得**为判定新建脚本 / 封装命令 / 判定层；**不得**用 hash / revision / 时间戳 / 计时阈值作为通过判据；**不得**因某批失败而缩小目标清单重跑并只报通过的那部分（失败须如实记录并回到 Plan）。
- First test: 不适用（非行为改动）；本任务以**真实运行**为唯一证据。
- GREEN condition: 全部批次**各自**退出码 0 且 failed 计数为 0；覆盖对账**不重不漏**；逐批记录齐全；与基线的差额已由实际结果说明。
- Verification: 按上述 12 条执行（每批一次独立调用 + 一步管道读计数）。
- Stop when: 任一批失败 ⇒ **如实记录失败批次与失败用例**，**保留**该轮结果，回到 Plan（**不得**缩小清单重跑取巧）；目标清单在执行期间发生变化（新增 / 删除测试文件）⇒ 重算清单与对账后再判定；出现需要改产品代码才能通过的失败 ⇒ 回到 Plan。

**Invariants**

- **零产品代码改动**：本 Iteration 不改 `src/` 下的任何文件；任何需要改产品代码的发现 ⇒ 回到 Plan。
- **样本即固定输入**：入库的样本是**真实字节**；运行时不做版本嗅探 / 不伪造代际 / 不做 hash 或 revision 比对；样本与来源说明**同源同步**。
- **兼容面不变**：MS36 的 G1 / G2 / G3 支持范围、代际定义与「可选尾段缺失即取默认值」的兼容读语义**不变**；不引入格式嗅探 / 猜测性迁移 / 写时全库重排。
- **磁盘格式不变**；`free_page` 集合语义不变；`IndexManager` / `BTree` 的对外签名不变；MS34 的写侧串行化语义不变。
- **已验收行为零回归**：八个 Iteration 的成果（含 Iteration 006 的索引树读路径显式拒绝）**不因**本 Iteration 的文档 / 夹具 / 记录改动而变化。
- **判定面纪律**：所有判定落在 `cargo test` 与 cargo 的原生输出内；无身份证明机制（hash / 指纹 / revision pin / run-id / manifest / 时间顺序证明）、无框架外判定层、无 timer 阈值行为判据。
- **全量纪律**：M19 的 12 条纪律逐条成立；全量只跑一次（本 change 的预算）。

**Non-goals**

- G1 / G3 的样本扩充与代际定义的修改（MS36 已验收）。
- 旧 dump 文本的支持范围（MS36 已验收：范围内 1 份 + 范围外 3 份）。
- 加密旧库的专项断言（MS36 已记录为不覆盖）。
- 任何**产品代码**的行为变更。
- Iteration 006 登记的 Issue 候选 IC-1 / IC-2 的修复（由 Recorder 择承载）。
- `openspec/specs/` 的合并与全局状态维护（SNAPSHOT / tasks / M / R / I）——属 `openspec-docs-maintainer`。

**Acceptance**

1. **G2 有真实来自旧构建的文件级样本，且固定入库**（C-1）：`tests/fixtures/legacy/legacy_g2.db` 存在；由 revision `6e8dcf5` 的旧构建在**仓库之外**产出；来源说明写明 revision / 构建方式 / 产物路径 / 工具链 / 代际判别特征 / 内容清单 / 预期结论 / 再生步骤；**运行时无版本伪造、无 hash / revision 比对**。映射：T16、既有域 `legacy-format-compat` 的 G2 行与证据索引 E7。
2. **G2 样本的七项能力有常态化断言，派生量由断言锁定并与来源说明同步**（C-1）：打开 / 查询 / 写入 / 完整性 / 打开时重建 / dump / 恢复各一，走**真实入口**；派生量在**断言**与**来源说明**两处同源同步。映射：T17。
3. **三条一次性兼容条款转为常态化用例**（C-2）：恢复出的库可继续写入 / 旧 G1 可由当前构建 dump / G1 可连续写两次 ⇒ `tests/legacy_compat_test.rs` 内三条用例，断言**可观察结果**（不止退出码）。映射：T18、既有域 `legacy-format-compat` 的 E2 / E4 / S1。
4. **迁移路径实测记录的命令列可直接执行**（C-3）：MS36 `:300-317` 的「命令」列每条 `rtsql` 调用的路径参数**含 `/`**；**已记录的退出码与结果逐条不变**；「失配只在命令写法」的事实保留。映射：T19。
5. **规格增量与验收边界对齐 + 收尾交接清单就绪**：AR2 措辞澄清（既有域 `mvcc-tombstone-visibility`）、RPBR 的「尽力而为」AND 子句（既有域 `release-path-bounded-read`）、IRBR Requirement 措辞收敛（本 change 增量域 `index-read-bounded-read`）；三处**只改措辞 / 只补子句**；交接清单覆盖增量域状态、`legacy-format-compat` 证据索引更新交 Maintainer、T21 全量结论、IC-1 / IC-2 的移交。映射：T20。
6. **收尾全量回归通过（M19 分批串行）**：12 条纪律逐条成立；**全部批次各自 0 failed**；逐批 passed / failed 记录齐全；覆盖对账不重不漏（目标 = 执行时 `tests/*.rs` 实际全部文件 + `--lib --bins`）；与基线（2192 / 0 / 2，13 批 / 136 目标，**已失效**）的差额由实际结果说明。映射：T21、change 的「收尾验证」。

**Verification**

- Acceptance 1–5 走**真实入口**：`cargo test --test legacy_compat_test -- --test-threads=1` ⇒ 全部 0 failed（含 T16–T18 的新增用例）；T19 / T20 的**人工核对项**按各自 Task Definition 的最短操作步骤执行，所见即判定，结果以一句话记入 Act Response（不要求填表 / 截图 / 汇总工具）。
- Acceptance 6 走**真实运行**：按 T21 的 12 条纪律执行；判定只看 cargo 的退出码与原生输出，计数用**一步通用管道**读出。
- 全量目标清单 = **执行时** `tests/*.rs` 的实际全部文件 + `--lib --bins`（Plan 时点的清单为 142 + 1 = 143 个目标，仅作参考）。
- **无人工验证项**被计入完成声明之外的内容；无身份证明机制；无框架外判定层；无 timer 阈值行为判据。
- Persisted Evidence = `none`（全部结论可低成本重跑：一条定向测试目标 + 一次分批全量）。

**Gate 2 Readiness**

- PASS——需求与场景：T16–T21 各自的 Requirement/Scenario 已逐条映射到既有域 `legacy-format-compat`（G2 行 + E2 / E4 / E7 / S1）、既有域 `mvcc-tombstone-visibility`（AR2）、既有域 `release-path-bounded-read`（RPBR 子句）、本 change 增量域 `index-read-bounded-read`（Requirement 措辞）与 change 的「收尾验证」（T21）⇒ 已映射到 Acceptance 1–6（BLOCKED: 无）。
- PASS——调查：G1 的来源 / 再生流程与派生量位点、G2 的代际定义与旧构建 revision（**已独立核实** `6e8dcf5` 的 `serialize_catalog_row` 布局）、E7 的缺口、三条一次性条款的记录位点、命令解析的裸名规则（`src/cli/resolve.rs:30`）、两条遗留的来源（Iteration 005 F3 / Iteration 002）、当前测试目标清单的实际数量，均已落到**当前工作区的行号**（BLOCKED: 无）。
- PASS——设计闭合：C-1 / C-2 / C-3 三个承载方式、T20 的三条措辞对齐、T21 的 12 条纪律均已定死，Act 不需再决定形态或判据（BLOCKED: 无）。
- PASS——任务分轮：T16 → T17 → T18 / T19 / T20 → T21，六轮可独立验证与诊断（BLOCKED: 无）。
- PASS——追踪：Cycle RTM（见下）与 change `tasks.md` 的 RTM 行一致（T16–T21 ↔ 既有域 `legacy-format-compat` + `mvcc-tombstone-visibility` + `release-path-bounded-read` + 本 change 增量域 `index-read-bounded-read`）（BLOCKED: 无）。
- PASS——当前轮验证：各 Task Definition 的 First test / GREEN condition / Verification 已写明；T21 的 12 条纪律逐条写死；Persisted Evidence = `none`（BLOCKED: 无）。
- 无 BLOCKED / SKIPPED 项。⇒ **Gate 2 通过**，Plan Context 状态置 `ready`。

**Cycle RTM**

| Requirement / Scenario | Task | First Test（计划名） | Status |
|---|---|---|---|
| 既有域 `legacy-format-compat` 的 G2 行能力矩阵 + 证据索引 E7（无 G2 文件级样本） | T16, T17 | `tests/legacy_compat_test.rs` 新增 G2 用例（计划名 `legacy_g2_db_opens_queries_and_writes` / `legacy_g2_db_open_rebuild_keeps_live_table_intact`；Act 可改名并记录实际名） | Covered |
| 既有域 `legacy-format-compat` 的 E2 / E4 / S1（写入后仍能再次写入 / 当前构建导出 G1 旧库 / 恢复出的库可继续写入） | T18 | 同文件新增三条用例（计划名 `restored_legacy_db_accepts_further_writes` / `current_build_dumps_legacy_g1_db` / `legacy_g1_db_accepts_two_consecutive_writes`） | Covered |
| MS36 迁移路径实测记录的**命令列**可直接执行（T19；MS36 Plan Review M1） | T19 | 同文件 MS36 `:300-317` 的命令列更正（记录层修正，无产品测试；字面执行 → 退出码与记录一致为人工核对项） | Covered |
| 既有域 `mvcc-tombstone-visibility` 的 AR2 措辞澄清（Iteration 005 F3）；既有域 `release-path-bounded-read` 的「尽力而为」AND 子句（Iteration 002 遗留）；本 change 增量域 `index-read-bounded-read` 的 Requirement 措辞收敛（Iteration 006 F6）+ 收尾交接清单 | T20 | 纯文档任务（三处「改前 / 改后」措辞对照） | Covered |
| change 的「收尾验证：M19 分批全量回归」 | T21 | 真实运行（分批串行；12 条纪律逐条） | Covered |

**Persisted Evidence**

- Mode: none

全部结论可低成本重跑（一条定向测试目标 `cargo test --test legacy_compat_test -- --test-threads=1` + 一次分批全量 + 若干人工核对项），命令与结果写入 Act Response 即可承载；不存在「Act Response 装不下 / 无法低成本重跑 / 缺了就无法作决定」的情形 ⇒ 不创建 `evidence/` 目录。

**Risks and Notes**

- **旧构建的可构建性**：G2 样本依赖 revision `6e8dcf5` 在**离线**条件下可构建（`cargo build --offline`）。若离线依赖不可用 ⇒ 回到 Plan（**不得**改为手工构造样本）。仓库之外的隔离克隆（`--no-hardlinks --shared`）确保不触碰工作区的未提交现场。
- **派生量同步**：样本的派生量在**两处**出现（断言常量 + `tests/fixtures/legacy/README.md`）⇒ 任一处更新必须同步另一处；不一致时**先核对样本字节是否被重新生成**，不得就地改断言常量迁就样本。
- **样本规模**：G2 样本的行数 ≪ 300（ISS23 约束）；若 G2 的构造需要用到 `UNIQUE`（唯一根段在代际上是 G2 的判别特征，但**唯一根计数为 0 的表**同样落在 G2）⇒ 优先用**计数为 0** 的普通表，保持规模最小；若确需非零唯一根段，必须说明该派生量的可观测形态。
- **MS36 记录的冻结面**：T19 只改**记录正文**的「命令」列；该 Cycle 的 Plan Context 与 Act Response 段**不改写**（已交接产物不可改写）。
- **T20 的措辞纪律**：三处对齐**只改措辞 / 只补子句**；任何需要**新增或放宽行为要求**的写法 ⇒ 回到 Plan。`openspec/specs/` 的合并与全局状态维护**不属本 Iteration**，只交**增量 + 交接清单**。
- **T21 的预算与不可逆性**：全量在本 change 只跑一次 ⇒ 执行前先确认 T16–T20 的目标文件已就位（`tests/*.rs` 清单在此刻冻结并记录），避免跑完再改目标造成对账失效。
- **基线失效**：MS36 的 2192 / 0 / 2 基线**不得**作为通过判据；差额必须由实际结果说明（新增用例 + 目标数变化）。目标数在 Plan 时点为 143（142 + `--lib --bins`），执行时以**实际列表**重算。
- **规模 / 范围外的既有缺陷**：Iteration 006 登记的 IC-1 / IC-2 不在本 Iteration；全量若暴露**新的**失败，须如实记录并回到 Plan，不得在本 Iteration 内顺手改产品代码。

## Act Response

- Status: reported

### 完成任务清单

| 任务 | 状态 | 涉及文件与符号 |
|---|---|---|
| **T16**（G2 文件级样本产出并固定入库） | done | 新增 `tests/fixtures/legacy/legacy_g2.db`（32832 字节）；更新 `tests/fixtures/legacy/README.md`（样本清单表 + 「G2 代际样本」节 + 「G2 样本的再生步骤」节） |
| **T17**（G2 样本的常态化断言） | done | `tests/legacy_compat_test.rs`：新增 `g2_tables_row_len` / `g2_tables_unique_count` / `columns_type_tag_offset` / `columns_default_segment` 四个辅助函数 + `COL_TAG_STRING` / `TUPLE_TAG_STRING` 两个常量 + 四个用例 `legacy_g2_catalog_rows_are_in_g2_generation` / `legacy_g2_db_opens_queries_and_writes` / `legacy_g2_db_open_rebuild_keeps_live_table_intact` / `legacy_g2_dump_restores_into_current_db` |
| **T18**（三条一次性条款常态化） | done | `tests/legacy_compat_test.rs`：新增 `restored_legacy_db_accepts_further_writes` / `current_build_dumps_legacy_g1_db` / `legacy_g1_db_accepts_two_consecutive_writes` |
| **T19**（迁移路径记录的命令列更正） | done | `openspec/changes/2026-10-24-ms36-legacy-format-and-dump-compat/iterations/001-contract-and-baseline/000-initial.md`：R5 实测表 + S1 / E4 / E2 实测表的**命令列**与表前说明（**已记录的结论逐字未动**） |
| **T20**（规格增量与验收边界对齐 + 收尾交接清单） | done | `specs/release-path-bounded-read/spec.md`（`DROP INDEX` 场景补 AND 子句）、`specs/index-read-bounded-read/spec.md`（Requirement 句 + Purpose 同词收敛）、`tasks.md`（RTM 段同步 + 新增「收尾交接清单（T20）」节 + 任务状态 T16–T21 → done） |
| **T21**（收尾全量回归，M19 分批串行） | done | 14 批 / 143 目标 / **2231 passed / 0 failed / 2 ignored**；逐批记录见下「T21 的分批执行记录」 |

零产品代码改动（`src/` 未触碰）。

### C-1：G2 样本的落地证据

| 项 | 值 |
|---|---|
| 样本路径 | `tests/fixtures/legacy/legacy_g2.db` |
| 来源 revision | `6e8dcf551aa53d2a08dc6f20097f1ad5e50fc464`（2026-09-26，MS23+MS24 收尾统一入库） |
| 产出命令 | `git clone --no-hardlinks --shared /home/daivy/projects/RTsql /tmp/rtsql-legacy-g2` → `cd /tmp/rtsql-legacy-g2 && git checkout 6e8dcf551aa53d2a08dc6f20097f1ad5e50fc464` → `cargo build --offline --bin rtsql` → 旧 CLI `new` / `CREATE TABLE` / `INSERT` / `DROP TABLE` 产出库文件 → `cp` 样本本体入库 |
| 构建结果 | `Finished dev profile [unoptimized + debuginfo] target(s)`（rustc / cargo 1.99.1，**离线**构建成功） |
| 固定方式 | 样本本体（库文件）固定入库为固定输入；**运行期不做任何版本 / 哈希 / 修订比对**，也不新增任何身份证明机制；断言只锁定**派生量**（目录行形态构成、列行 DEFAULT 段、表清单与行数） |
| 字节规模 | **32832 字节**（行数 ≪ 300：`people` 3 行 + `notes` 1 行；另有 `scratch` 表由旧构建删掉） |
| **是否在工作区内生成** | **否**。样本在 `/tmp/legacy-fixture-g2/` 的**仓库之外**隔离目录产出，只把样本本体 `cp` 进仓库；工作区全程未被旧构建写入 |
| 临时克隆 / 构建产物的清理 | 已清理：`/tmp/rtsql-legacy-g2`（隔离克隆 + 765MB `target/`）、`/tmp/legacy-fixture-g2`（样本暂存）、`/tmp/ms36-cmd-probe`（T19 实测目录）、`/tmp/m19_targets*.txt`（批计划临时清单）——全部经 `mavis-trash` 移入回收站，复核 `ls` 均已不存在 |

**样本的代际判别（由断言锁定，非人工声称）**：`__tables` 行长度恰为 **G1 固定段 + 4 字节**，该 4 字节是唯一根计数（= 0），其后**没有**二级索引段与表类型字节 ⇒ 落在 **G2**；`__columns` 共 6 行（2 表 × 3 列），只有 `notes.tag` 带 DEFAULT 段（**8 字节**：有默认值标记 + 字符串标签 + u16 长度 + `"none"`），段内解出字面量 `'none'`，其余 5 行止于 G1 固定段。断言用**结构式**表达（G1 固定段 + 4 / + 8），不是魔数常量。

**派生量与来源说明同步**：`tests/fixtures/legacy/README.md` 的「G2 代际样本」节写明同一组数字（文件 32832 字节、表清单 `people`（3 行）/ `notes`（1 行）、`__columns` 6 行、目录行 +4 字节、DEFAULT 段 +8 字节且字面量 `'none'`），并记录来源 revision、产出命令与再生步骤。

### C-2：三条新增常态化用例的判别力与实际结果

三条用例走**真实入口**（真实 CLI 子进程 `CARGO_BIN_EXE_rtsql` + 真实库文件），断言强度不低于原一次性实测记录：

| 用例 | 覆盖的一次性条款 | 判别力证据 | 实际断言 |
|---|---|---|---|
| `restored_legacy_db_accepts_further_writes` | R4 S1「恢复出的库可继续写入」 | **表征型首次 GREEN + 临时错位期望的 RED**：把「恢复后 users 行数」期望由 4 改为 5 ⇒ 用例**失败**（`left: 4, right: 5`）⇒ 断言有判别力；改回 4 后 GREEN | `restore` 退出码 0；`INSERT INTO users` 退出码 0 且 `{"affected_rows":1}`；`SELECT * FROM users` 得 **4 行**（含 `(4,"dave",6.0)`）；`INSERT INTO audit` 退出码 0；`SELECT COUNT(*) FROM audit` 得 **2** |
| `current_build_dumps_legacy_g1_db` | E4「旧 G1 可由当前构建 dump」 | 表征型首次 GREEN（dump 退出码 0 + stderr 空 + 3 张 `CREATE TABLE` + 4 个行值字面量 + 恢复往返后的行集逐行断言，任一不符即失败） | `dump` 退出码 **0**、stderr **空**；导出 `users` / `audit` / `events` **3 表** + 全部行（含 `'alice'` / `'bob'` / `'carol'` / `'kept'`）；导出的文本经 `restore` 进新库（退出码 0）后 `users` **3 行**逐行相等、`events` **1 行** |
| `legacy_g1_db_accepts_two_consecutive_writes` | E2「G1 可连续写两次」 | 表征型首次 GREEN（两次写入的退出码 + `affected_rows` + 计数 + 行集四重断言） | 第一次 `INSERT INTO users (4,'dave',6.0)` 退出码 **0**、`{"affected_rows":1}`；第二次 `INSERT INTO users (5,'erin',5.5)` 退出码 **0**、`{"affected_rows":1}`；`SELECT COUNT(*) FROM users` = **5**；`INSERT INTO audit (2,'second-round')` 退出码 **0**；`SELECT * FROM audit` = **2 行**（含新行 `(2,"second-round")`） |

**实际 `test result:` 行**（`cargo test --test legacy_compat_test -- --test-threads=1`，当次运行）：

```
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.65s
```

（16 = 既有 9 条 + G2 四条 + 本次三条；该文件在 T21 的 B06 批内再次以 **16 passed / 0 failed** 通过。）

### C-3：命令列更正前后的实际文字对照

**更正前**（字面执行的结果，探针目录 `/tmp/ms36-cmd-probe`，仓库样本只读）：

```
$ rtsql dump g1_dump_probe.db
{"error":"/home/daivy/.rtsql/db/g1_dump_probe.db.db does not exist"}
exit=1
$ rtsql g1_dump_probe.db "SELECT COUNT(*) FROM users;"
{"error":"database 'g1_dump_probe.db' does not exist; create it first with: rtsql new g1_dump_probe.db"}
exit=1
```

⇒ 与记录**不符**：裸名被 CLI 解析到 `$RTSQL_HOME/db/<name>.db`（未设 `RTSQL_HOME` 时为 `$HOME/.rtsql/db/<name>.db`），**字面执行退出 1**；记录的结论（退出码 0、导出 3 表 + 全部行）无法由该文字复现。

**更正后**（路径形式；同一探针目录实测，退出码均为 **0**）：

```
$ rtsql dump ./g1_dump_probe.db          → exit 0（导出 3 张 CREATE TABLE + 全部行）
$ rtsql ./g1_dump_probe.db "SELECT COUNT(*) FROM users;"  → exit 0，{"columns":["count_star"],"rows":[[3]]}
$ rtsql new ./s1.db                      → exit 0
$ rtsql restore ./s1.db ./legacy_dump.sql → exit 0
$ rtsql ./s1.db "INSERT INTO users VALUES (4, 'dave', 6.0);" → exit 0，{"affected_rows":1}
$ rtsql ./s1.db "SELECT * FROM users;"   → exit 0，4 行（alice/bob/carol/dave）
$ rtsql ./s1.db "INSERT INTO audit VALUES (2, 'after-restore');" → exit 0，{"affected_rows":1}
$ rtsql ./s1.db "SELECT COUNT(*) FROM audit;" → exit 0，{"columns":["count_star"],"rows":[[2]]}
```

记录表的**命令列**据此改为路径形式（`./` 相对工作目录 `/tmp/ms36-migration-paths/`，仓库样本用绝对路径引用），并在表前加一句说明点明「裸名形式不可用」的原因。**已记录的退出码与结果本身逐字未动**（T19 的执行依据：只改记录文本）。

### T20：三处规格措辞的改动前后对照与理由

| # | 位置 | 改动前 | 改动后 | 理由 |
|---|---|---|---|---|
| ① | 既有域 `mvcc-tombstone-visibility` 的 Requirement「未提交与已回滚墓碑不抑制、评估回溯前驱」（`openspec/specs/mvcc-tombstone-visibility/spec.md`） | 「删除者事务尚未提交（读取时仍在活跃集合，或 RC 视图下语句开始时仍活跃）时，该墓碑 SHALL NOT 抑制版本链，评估回溯产出删除前对该读者可见的最新版本。墓碑事务 ROLLBACK 后，该墓碑 SHALL 中性化（不再作为已提交删除抑制链），行为与该删除未发生一致（扫描路径）。」（**未改**——本 change 不修改 `openspec/specs/`） | 见 `tasks.md`「收尾交接清单」第 5 条的**改动后**全文（只加解释性括注：行仍可见 ⇒ 键仍被占用 ⇒ 同键插入被拒是键被占用的正常结果、不是残留条目造成的假阳性；「无残留条目」指该键索引条目数为 1；只有该删除真正提交后同键才可再插入） | Iteration 005 的 Plan Review F3：AR2 的字面读法与 AR1（放弃窗口后该行仍可见）自相矛盾 ⇒ 以「条目数恰好 1 / 真正提交删除后同键可再插入 / 再插入后点查命中 1」三条为准澄清既有域措辞。**形态为交接项**（`openspec/specs/` 按本 change 执行依据不改；措辞原文随交接清单成文，由 Maintainer 应用）——见「Deviations from Plan」第 1 条 |
| ② | 本 change 增量 `specs/release-path-bounded-read/spec.md` 的 Scenario「损坏页上的删索引不 panic」 | `- **THEN** 语句 SHALL NOT panic，SHALL NOT 导致进程 abort` | 同句 + `- **AND** 语句 SHALL 按释放路径既有的尽力而为语义结束（告警并跳过不可读的部分，返回成功或明确的错误）` | Iteration 002 遗留：同域另两个场景（`DROP TABLE` / `TRUNCATE`）都带该子句，唯独 `DROP INDEX` 缺 ⇒ 场景面读起来像「不保证结束语义」。**只补子句**，与 `DROP TABLE` 场景同形；Scenario 数不变（4） |
| ③ | 本 change 增量 `specs/index-read-bounded-read/spec.md` 的 Requirement 正文（+ Purpose 同词） | 「读取到损坏的索引页时，系统 SHALL 返回**明确的错误**……」；Purpose「损坏的索引页 SHALL NOT 导致进程 abort，且 SHALL 给出**明确的错误**……」 | 「读取到**未通过页级判据**的索引树页时，系统 SHALL 返回**明确的错误**……」；Purpose「**未通过页级判据**的索引树页 SHALL NOT 导致进程 abort，且 SHALL 给出**明确的错误**……」 | Iteration 006 的 Plan Review F6：`integrity::check_tree_page` 的叶页分支只校验槽边界（内部页分支另有长度校验）⇒ **短于 38 字节的叶条目不被判为损坏**，会被 `filter_map` 静默跳过 ⇒ 原措辞「损坏的索引页」**超出已验收范围**（已验收的是「页级判据判定为损坏的页给出明确错误」）。收紧为**行为级措辞**（页级判据），不引入实现术语；Scenario 数不变（5），已验收的四个损坏页 Scenario 仍逐字成立 |

三处**只改措辞 / 只补子句**，不新增、不放宽任何行为要求（①为解释性括注，既有 SHALL 逐条不变）。

### T21 的分批执行记录

**纪律**：①`--lib --bins` 单独成批；②`tests/*.rs` 按**文件名字典序**分组、每批 8–12 个目标（切分算法：剩余 ≤ 12 取剩余，否则取 11）；③每批**一次独立调用** `cargo test --offline --test <f1> … -- --test-threads=1`；④批间**顺序执行**（同一时刻只有一批在跑）；⑤未使用任何无目标过滤的全量调用；⑥未使用默认并行度（每批均带 `--test-threads=1`）；⑦每批退出码 0 且 `test result:` 的 failed 计数为 0；⑧计数只用一步管道 `… 2>&1 \| grep -E "^test result:"` 读出（只计数、不判定），退出码取 `PIPESTATUS[0]`，**未新增判定脚本 / 封装命令 / 专用参数 / 判定层**；⑨每批计数均记录；⑩目标清单**按执行时实际的 `tests/` 文件集合重新生成**（`ls tests/*.rs | sort` = 142 个文件），并做覆盖对账；⑪全量结论 = 全部批次各自 0 failed（14 批全过）；⑫与基线 2192/0/2 的差额由实际结果说明。

| 批 | 目标文件（`--test` 名） | 退出码 | passed | failed | ignored |
|---|---|---|---|---|---|
| B00 | `--lib --bins` | 0 | 420（lib）+ 0（bins） | 0 | 0 |
| B01 | aggregate_extension_test, aggregate_output_shape_test, aggregate_test, alter_table_test, attach_session_test, btree_merge_test, btree_scale_test, btree_split_test, btree_test, busy_timeout_test, cache_perf_test | 0 | 149 | 0 | 0 |
| B02 | catalog_durability_test, chain_join_test, check_command_test, checkpoint_command_test, checkpoint_redo_reduction_test, checkpoint_skip_test, checkpoint_test, cli_test, concurrent_test, connection_limit_test, constraint_enforcement_test | 0 | 210 | 0 | **2** |
| B03 | cross_db_dml_test, cross_db_query_test, ctas_test, cte_test, database_file_lock_test, datetime_function_test, datetime_type_test, ddl_catalog_transactional_test, ddl_crash_recovery_test, ddl_rewrite_consistency_test, default_declaration_test | 0 | 144 | 0 | 0 |
| B04 | default_precheck_test, derived_scan_alias_test, dml_tombstone_abandon_window_test, dml_tx_id_test, drop_persistence_test, drop_table_free_test, e2e_test, encryption_test, executor_test, explain_test, explicit_tx_test | 0 | 161 | 0 | 0 |
| B05 | expression_e2e_test, file_header_test, file_storage_io_test, free_page_lifecycle_test, full_table_dml_test, gc_test, group_by_expr_test, import_input_surface_test, index_manager_test, index_page_ownership_stack_overflow_test, index_page_ownership_test | 0 | 101 | 0 | 0 |
| B06 | index_read_bounded_read_test, index_write_atomicity_test, insert_column_list_test, insert_select_test, isolation_level_test, join_output_resolution_test, join_test, key_type_conformance_test, keyless_eq_routing_test, keyless_row_test, legacy_compat_test | 0 | 106 | 0 | 0 |
| B07 | limit_test, memory_db_test, mvcc_abort_test, mvcc_commit_test, mvcc_record_test, mvcc_tombstone_visibility_test, nested_loop_join_test, network_protocol_test, network_server_test, no_from_select_test, order_by_alias_sort_test | 0 | 73 | 0 | 0 |
| B08 | outer_join_test, overflow_page_test, param_binding_test, parser_test, pg_integration_test, pg_messages_test, pg_protocol_test, pipeline_test, plan_cache_test, plan_exec_test, planner_test | 0 | 238 | 0 | 0 |
| B09 | predicate_test, prefetch_test, prepared_statement_test, projection_expression_test, projection_test, pushdown_test, query_memory_test, readonly_session_test, recovery_e2e_test, recovery_error_surface_test, recovery_replay_convergence_test | 0 | 117 | 0 | 0 |
| B10 | recovery_tail_persistence_test, recovery_test, recovery_wal_convergence_test, release_durability_precondition_test, release_path_bounded_read_test, runtime_test, scalar_function_test, scalar_subquery_comparison_test, schema_persistence_test, secondary_index_equality_test, secondary_index_test | 0 | 110 | 0 | 0 |
| B11 | select_distinct_test, self_join_test, set_operations_test, sort_test, sql_differential_test, statement_cancellation_test, storage_test, structural_property_test, subquery_alias_binding_test, subquery_test, subset_insert_test | 0 | 168 | 0 | 0 |
| B12 | sync_loader_test, table_alias_resolution_test, table_manager_test, truncate_test, tx_statement_test, typed_api_test, update_expression_test, update_index_maintenance_test, upsert_test, value_test, version_chain_test | 0 | 151 | 0 | 0 |
| B13 | view_test, visibility_test, wal_auto_checkpoint_test, wal_buffer_test, wal_handle_test, wal_integration_test, wal_record_test, wal_recovery_large_test, wal_writer_test, write_type_conformance_test | 0 | 83 | 0 | 0 |

**批次总数 = 14**（1 个 `--lib --bins` 批 + 13 个文件批；每批 10–11 个目标，全部落在 8–12 区间）。**目标数 = 143**（142 个 `tests/*.rs` 文件 + `--lib --bins` 一组；对应 144 个测试二进制 / 144 条 `test result:` 行）。**终态 = 2231 passed / 0 failed / 2 ignored**。

**覆盖对账**（基准 = 执行时实际的 `ls tests/*.rs | sort` 集合，142 个文件；执行前后该清单 `diff` 一致）：**missing 0 / extra 0 / duplicate 0**（142 个文件各出现恰好一次）。

**与基线 2192 / 0 / 2 的总数差额**：`2231 − 2192 = **+39**`，逐项归因：

| 归因项 | 用例数 | 说明 |
|---|---|---|
| 本 change 新增的 7 个测试文件 | **+32** | `index_page_ownership_test` 5 + `index_page_ownership_stack_overflow_test` 1 + `ddl_rewrite_consistency_test` 9 + `release_path_bounded_read_test` 7 + `release_durability_precondition_test` 2 + `dml_tombstone_abandon_window_test` 3 + `index_read_bounded_read_test` 5（均为 T21 批次的实际计数） |
| 既有 `legacy_compat_test` 的增量 | **+7** | 9 → 16（本 Cycle 新增 G2 四条 + 常态化三条） |
| 其余既有文件的用例数 | **0** | lib 420 = 基线 420；其余文件计数差额无需归因 |
| 合计 | **+39** | 与实测差额**精确相等** |

**目标数变化**：135 → 142 个文件（+7 个本 change 新增文件）⇒ 136 → 143 个目标（+7）。基线的 2192/0/2 对当前工作区**已失效**（由本次执行重新建立），差额按上表由实际结果说明，**判据未因对不上而调整**。

**全量结论**：全部 14 批各自退出码 0 且 failed 计数为 0 ⇒ 本 change 的收尾全量回归**通过**（无缺批）。

### Gate 3 / Gate 4 逐项结论

**Gate 3（Task Review）— PASS**

- **Spec compliance**：T16 的样本真实来自旧构建（隔离克隆 + 离线 `cargo build --offline --bin rtsql`）且在仓库之外生成；T17 的 G2 断言覆盖 7 个能力面（打开 / 查询 / 写入 / 完整性 / 打开时重建 / dump / 恢复）并锁定派生量（目录行 = G1 固定段 + 4 字节且唯一根计数为 0；`notes.tag` 列行 = G1 固定段 + 8 字节 DEFAULT 段且字面量 `'none'`；表清单与行数），与来源说明**同数字**；T18 三条用例走真实入口、断言强度不低于原一次性记录，并有**判别力证据**（临时错位期望 ⇒ RED）；T19 只改记录正文命令列（结论逐字未动），并有更正前（退出 1，`does not exist`）与更正后（退出 0）的实测对照；T20 三处措辞只改措辞 / 只补子句、Scenario 数与 Requirement 数不变（RPBR 1/4、IRBR 1/5）；T21 纪律 12 条逐条执行。
- **Code quality**：diff 仅涉及 1 个测试文件（`tests/legacy_compat_test.rs`）、1 个新增样本（`tests/fixtures/legacy/legacy_g2.db`）、1 个样本说明（`tests/fixtures/legacy/README.md`）、2 个本 change 的增量规格、change 的 `tasks.md` 与 MS36 的一处记录正文；**产品代码零改动**；未新增依赖、未新增身份证明机制（无 hash / 指纹 / revision pin / run-id / manifest / 时间序证明）、未新增判定层或封装命令、断言全部落在 `cargo test` 集合内；无 timer 阈值行为判据；测试用临时目录操作样本副本，不写仓库内样本。
- **Critical / Important 问题**：**0 项未解决**（见下节）。

**Gate 4（Verified Completion）— PASS**

- T16–T20 的定向判据 `cargo test --test legacy_compat_test -- --test-threads=1` 当次运行 **16 passed / 0 failed**；T21 的 14 批全部当次运行、各自退出码 0 且 failed 计数为 0（逐批计数见上表）；`cargo test --lib` 的 420 基线**被本次 B00 批重新确认**（420 passed / 0 failed / 0 ignored）——本 Cycle 未改 `src/`，该结论原已可沿用，现由本次全量重新建立；完成声明全部基于真实运行；**无人工验证项**（T19 的命令对照为真实 CLI 实跑）；Persisted Evidence 模式为 `none`，无 `required` Evidence。

### 自检发现、已修复内容与遗留 Minor 问题

- **已修复（实施过程中的自检发现）**：
  1. **G2 写入用例的目录行升级断言落空**：首版断言「写入后被触及的表目录行 = 当前布局」在只写 1 行的场景下**失败**（实测 35 字节 = G1 + 4，仍是 G2）。根因：目录行只在**数据链增长（分配新页）**时被重写，单行追加落在既有尾页内 ⇒ 不重写。改为按 G1 既有做法的**会分配新页的写入**（120 行批量插入 + 断言库文件增长），升级断言随之成立（`people` 行 = G1 + 9，尾段 `[0×8, 0x01]`），未触及的 `notes` 行仍停在 G2（G1 + 4）。
  2. **目录读与库句柄的锁冲突**：G2「打开时重建」用例在 `Database` 打开期间直接读目录链，触发 `DatabaseLocked`（`FileStorage::open` 与已打开的 `Database` 争同一把库锁）。改为**先读目录链、再打开 `Database`**（与 G1 同域用例的顺序一致）后通过。
  3. **T18 断言判别力的验证**：按 Task Definition 临时把「恢复后 users 行数」期望由 4 改为 5 ⇒ 用例失败（`left: 4, right: 5`）⇒ 证明断言不是恒真；随后改回正确期望。
- **遗留 Minor 问题**：
  1. **T21 的目标数口径**：Plan 与 tasks.md 记的是「143 个目标」（142 文件 + `--lib --bins` 一组），而 `cargo` 实际产出 **144** 个测试二进制（lib 与 bins 各一）与 144 条 `test result:` 行。两者各自成立（口径不同），与 Runbook R48 的「134 个目标 / 135 条 `test result:` 行」同型；已在交接清单第 3 条写明口径。
  2. **G2 样本的 `__tables` 行覆盖面**：样本的 `people` / `notes` 两张表唯一根计数均为 0（按 Plan 的「优先用计数为 0 的普通表」），因此**未**覆盖「非零唯一根段」的 G2 形态；该形态的目录行解析由既有行级单测（`src/storage/catalog.rs::legacy_layout_row_deserializes_with_empty_unique_roots` 等）承担，文件级样本不承担。
  3. **`legacy_compat_test.rs` 的模块头注释**：已补 G2 样本与三条常态化用例的一句说明，但未重写全文（原文以 MS36 的样本为主线）；不影响任何断言。

### Deviations from Plan

1. **T20 ① 的落点由「就地改写既有域」改为「交接项」**：Plan Context 的 T20 Targets 把既有域 `mvcc-tombstone-visibility` 的措辞澄清指向 `openspec/specs/mvcc-tombstone-visibility/spec.md`，而本 change 的执行依据（`tasks.md` 的 T20 执行要点「只对齐本 change 的规格增量文本与验收边界，**不改** `openspec/specs/`」；Plan Context 的 Preserve「`openspec/specs/` 的**合并**归 `openspec-docs-maintainer`（本任务只交**增量 + 交接清单**，不代做合并）」；本会话的交付约束同样禁止修改 `openspec/specs/`）三者一致要求不写该文件。故**措辞原文（改动前 → 改动后）成文于 `tasks.md` 的「收尾交接清单」第 5 条**，交 Maintainer 在合并时应用；**只改措辞、不改行为要求**的纪律不变。**提请 Plan Review 裁定**：若要求就地应用，可由 Maintainer 按该交接条目执行（本 change 不代做）。另两处（② RPBR、③ IRBR）落在本 change 的增量域，**已就地改写**。
2. **T17 的用例名与计划名的差异**：Plan Context 列的计划名为 `legacy_g2_db_opens_queries_and_writes` / `legacy_g2_db_open_rebuild_keeps_live_table_intact`（已按此命名，**未改名**）；另**新增两条**用例 `legacy_g2_catalog_rows_are_in_g2_generation`（代际判别：目录行形态本身）与 `legacy_g2_dump_restores_into_current_db`（dump / 恢复往返）——两条均属 Acceptance 2 明列的 7 个能力面（完整性、dump、恢复）中**尚无专门断言**的部分，属**覆盖补齐**而非范围扩张（均在 `tests/legacy_compat_test.rs` 内）。
3. **T21 的批次切分用 `cargo test --offline`**：Plan / Runbook 的命令形态未含 `--offline`；本执行加上该标志以避免网络访问（MS36 同类收尾全量的既有做法亦为 `--offline`），不影响测试语义与判定口径（判定仍只看 cargo 退出码与原生 `test result:` 行）。
4. **T21 的一处断言口径补充**：「目录行升级为当前布局」需以**会分配新页**的写入为前提（自检发现 1），故该断言比首版多了一条「库文件增长」的前置断言——这是**产品既有行为**的如实记录，不是新增判据。

### Persisted Evidence

`None required`——Plan 的 `Persisted Evidence` 模式为 `none`。全部结论可低成本重跑（1 个测试文件 + 1 个固定样本 + 14 批全量），不存在「Act Response 装不下 / 无法低成本重跑 / 缺了就无法作决定」的情形 ⇒ **未创建** `evidence/` 目录。

### Experience Candidates

1. **Runbook 候选（既有配方可扩充）**：`.agents/runbooks/m19-batched-full-regression.md`（R48）的「批计划与对账」节可直接复用，但可补两条本次遇到的实测细节：①「目标数」与「`test result:` 行数」是两个口径（142 文件 + `--lib --bins` = 143 目标 / 144 条结果行），收尾记录里两者都要写明；②基线失效后的差额归因公式（`新增文件用例数 + 既有文件增量`）本次再次精确成立（32 + 7 = 39）。证据：本 Cycle 的 T21 批次表与对账结果。
2. **Issue 候选（范围外，IC-1 的同形面）**：叶页槽负载短于条目格式（38 字节）时不被页级判据判为损坏 ⇒ 该条目被静默跳过（Iteration 006 的 Plan Review F6 已登记为 IC-1）。本 Cycle 的 IRBR 措辞收敛已把该行为排除在已验收主张之外，但**缺陷本身仍在**。承载方式：另立 change 承载「叶页条目长度的页级判据」（读 / 写路径 = 明确错误、释放路径 = 计数 + 报出 + 跳过）。**本 change 不登记**（由 Recorder 按用户指令处置）。
3. **Issue 候选（本 Cycle 观察到的既有行为，无缺陷）**：`rtsql` 的裸名参数解析（`$RTSQL_HOME/db/<name>.db`）与「含 `/` 才按路径直开」的规则**本身是既有契约**（`src/cli/resolve.rs`），但它使**文档 / 记录里的裸名命令**在字面执行时必然失败（本 Cycle 的 T19 探针实测退出 1）。若后续仍有记录 / 文档出现裸名库参数，可复用本 Cycle 的更正形态（路径形式 + 一句说明）。**不是缺陷**，不登记为 Issue。

### 未解决问题与停止条件

- 未解决项：**无**（T16–T21 全部完成；Gate 3 / Gate 4 均通过）。两项待 Plan Review 裁定的判断已记录：① T20 ① 的落点（交接项 vs 就地应用，见「Deviations from Plan」第 1 条）；② T20 ③ 的范围补充是否留在 T20 内（Plan Review 在交接指令之外追加，按 T20 自身标题范围「规格增量与验收边界对齐」执行，**未移出**，判断记录于本节）。
- 停止条件：不适用——本 Cycle 全部 task 正常完成；按 Phase 边界，Act 写入响应后**终止**，不创建下一 Cycle / 下一 Iteration、不归档 change、不维护全局状态、不登记 Issue。等待用户审计与 Plan Review 填写 `Review Result`（本文件不得由 Act 填写）。

## Plan Review

- Review Result: accepted

**Findings**

Plan Review 2026-10-25（`openspec-plan`，Review 模式；Iteration 007 / Cycle 000——本 change 的**最后一个** Cycle）。独立阅读了当前 Cycle 的 `Plan Context` 与 `Act Response` 全文、change 的 `tasks.md` / `design.md` / `proposal.md` 与 5 份规格增量全文、`tests/legacy_compat_test.rs` 全文（1405 行）、`tests/fixtures/legacy/README.md` 全文、MS36 change 的 `iterations/001-contract-and-baseline/000-initial.md` 的工作区 diff，以及本 Cycle 的全部工作区 diff；**未**以 Act Self-Review 代替独立检查；**未**重复运行 T21 的 14 批全量（覆盖范围自 Act 运行后未变化，按公共规则「验证」直接沿用），**只**为核实 G2 样本的**字节级**结构主张（读码无法判定二进制夹具）当次运行了**一个**定向测试目标。

- **F1（通过·零产品代码改动，既有样本未被改动）** `find src -newermt "2026-10-25 17:00:00"` 的**全部**输出只有一行：`src/storage/btree/node.rs`（mtime `17:00:30`）——早于本 Act 会话的起点（17:24）⇒ Iteration 007 内**无任何 `src/` 改动**成立（Act 的自述成立，且本 Review 以 mtime 独立核验）。既有 7 个样本（4 份 `.sql` + `legacy_g1.db` / `legacy_g1_pending_wal.db` / `.wal`）的 `git status` 为 `A `（已暂存且**工作区 == 暂存区**）⇒ 本 Cycle **未改动任何既有样本字节**；本 Cycle 的测试侧 diff 为 `git diff --numstat` 的 **567 + 56 = 623 行插入、0 行删除** ⇒ **纯增量**，既有 9 条 G1 用例与 G1 来源说明逐字节未变（T17 / T18 的 Preserve 成立）。临时克隆（`/tmp/rtsql-legacy-g2`）、staging 与 probe 目录**已不存在**（`ls -d /tmp/rtsql-legacy*` 无输出）⇒ 现场清理成立。
- **F2（通过·G2 样本的判别是结构化锁定，无身份机制）** 独立读码 `tests/legacy_compat_test.rs` 的 G2 判别链：`g2_tables_row_len`（`:860`）= `g1_tables_row_len(row) + 4`（G1 固定段 + 唯一根计数 4 字节）、`g2_tables_unique_count`（`:865`）读该 `u32` 且断言为 0、`columns_default_segment`（`:882`）只在行尾存在时解析（`rest[0] == 1` 有默认值标志 + `rest[1] == TUPLE_TAG_STRING` + 长度 + 载荷）⇒ `__tables` 行**无**二级索引段与表类型尾字节、`__columns` 6 行中**只有** `notes.tag` 带 8 字节 DEFAULT 段（解码为 `'none'`）。**全文件 grep** `hash|digest|crc|revision|commit|mtime|modified\(|sha|md5|run_id|manifest|elapsed|Instant::now|duration_since` ⇒ 命中项只有 `std::collections::HashSet`（标准库集合）与模块头注释里的来源 revision（`d8165fc` / `6e8dcf5`）——**无任何断言**比较 hash / revision / 版本 / 时间戳 ⇒ **无身份证明机制**，判别全部落在**可观察的目录行结构**上。Act 的「结构化锁定」自述与代码一致。
- **F3（通过·G2 样本的字节级结构主张——本 Review 当次运行的唯一一次定向验证）** `cargo test --offline --test legacy_compat_test -- --test-threads=1` ⇒ **16 passed / 0 failed / 0 ignored**（当次运行，输出含 16 条用例名，其中 `legacy_g2_catalog_rows_are_in_g2_generation` 等 4 条 G2 用例与 3 条常态化用例均为 `ok`）⇒ G2 夹具在**磁盘上的实际字节**满足 F2 的全部结构断言（不是仅凭来源说明自证）。本 Review 未运行 14 批全量（沿用 Act 的 T21 结论，覆盖范围未变化）。**样本来源（真实旧构建）的不可机械验证性**：按公共规则禁止身份证明机制 ⇒ 「样本确由 revision `6e8dcf5` 的旧构建产出」这一**来源事实**只能由来源说明与产出步骤记录承载，不能由断言背书；本 Review 确认来源说明写明了 revision / 构建方式 / 产物路径 / 工具链 / 代际判别特征 / 内容清单 / 预期结论 / 再生步骤，且 Plan Context 已**独立核实**该 revision 的 `serialize_catalog_row` / `serialize_column_row` 布局 ⇒ 记录与判别互洽，**不构成阻塞**（记为观察，见 F13）。
- **F4（通过·C-2 三条常态化用例的判别力）** 逐条独立读码 `restored_legacy_db_accepts_further_writes`（`:1227`）/ `current_build_dumps_legacy_g1_db`（`:1286`）/ `legacy_g1_db_accepts_two_consecutive_writes`（`:1354`）：断言锚定的都是**真实入口的真实可观察结果**——`restore` / `dump` / `INSERT` 的**退出码 0**、`affected_rows: 1`、`SELECT` 的**具体行集**（users 4 行且第 4 行 = `("dave", 6.0)`、audit 计数 2、users 5 行）、`dump` 的**stderr 为空** + 3 张 `CREATE TABLE` + 4 行值 + 导出文本**恢复回新库后行集等价** ⇒ 断言强度**不低于**原一次性实测记录（T18 的执行依据）。Act 的「故意改错期望」判别力验证（期望 4 → 5 得 RED `left: 4, right: 5`）**可信**：该断言比对的是**行数**这一可变量，改错期望即失败 ⇒ 非恒真；且三条用例的其余断言（退出码、`affected_rows`、行集成员、stderr 空）均非恒真。
- **F5（通过·C-3 更正彻底，且已记录的结论未被改动）** MS36 Cycle 文件的工作区 diff 为 **12 insertions / 10 deletions**，逐行核对：**全部**落在两张表的「命令」列（`rtsql new p1.db` → `rtsql new ./p1.db` 等）与新增的一句说明（工作目录 = `/tmp/ms36-migration-paths/`、`./` 的相对基准、仓库样本用绝对路径、工作目录内的输入是样本副本）⇒ **退出码列与结果列逐条未动**（`0 / **0**`、`**3**`、`**0**` 全部原样保留）⇒ T19「只改记录文本，不改任何已记录的结论」成立。**其它裸名命令的排查**：全文件 grep `rtsql (new|restore|dump|import|recover)\s+[A-Za-z0-9_.-]+\.db` ⇒ **唯一**命中在 `:477`，位于**已冻结**的 Plan Review 的 M1 条目正文里（把旧写法**作为缺陷描述**引用），**不是可执行命令列** ⇒ T19 的 GREEN 条件「命令列的每条命令含 `/`」成立，**无其它遗留**。
- **F6（通过·Deviation 1 的裁定：T20 ① 的落点）** Act 的选择（**交接项**，不就地写入 `openspec/specs/`）**成立**。证据链三条一致：(a) change `tasks.md` 的 T20 执行要点（`:50`）明写「只对齐本 change 的规格增量文本与验收边界，**不改** `openspec/specs/`」；(b) 本 Cycle `Plan Context` 的 Preserve（`:145`）/ Forbidden（`:146`）/ Non-goals（`:195`）三处均写明 `openspec/specs/` 的合并归 `openspec-docs-maintainer`，本任务「只交**增量 + 交接清单**」；(c) 公共规则 `AGENTS.md` 的文档地图把 `openspec/specs/<domain>/spec.md` 的写入者列为 `openspec-docs-maintainer`（收尾合并），并规定「直接修改 `openspec/specs/` 不属于轻量工作」。Plan Context 的 T20 **Targets** 把落点指向 `openspec/specs/mvcc-tombstone-visibility/spec.md`，是**该 Plan Context 自身的措辞不自洽**（Targets 与 Preserve / Forbidden / Excluded scope 冲突），**不是**覆盖三条禁令的授权 ⇒ 归类为 **Plan 措辞误标（Plan 错误）**，**非 Act 偏离**，不要求返工。同类误标还有 RPBR：Plan Context 称其为「既有域 `release-path-bounded-read`」，实际它是**本 change 自己尚未合并的新增域**（`openspec/specs/` 下无该域）⇒ Act 把「尽力而为」AND 子句落在**本 change 的增量** `specs/release-path-bounded-read/spec.md` 是**唯一正确**的落点。
- **F7（通过·交接清单的措辞完整、可直接应用——逐字核对）** 交接清单第 5 条的「**改动前**」与 `openspec/specs/mvcc-tombstone-visibility/spec.md:49` 的正文**逐字节相等**（本 Review 用程序比对，`True`）；「**改动后**」= 改动前 + **一段**插入的解释性括注（首个差异位置在第 80 字，插入段以「该行仍可见 ⇒ 其键仍被占用…」开头），改动前的**全部 SHALL 子句在改动后原样存在** ⇒ 维持者可按「位置 + 改动前 → 改动后」直接 find-and-replace 应用，**无需**重新推导措辞。清单第 2 条的 E2 / E4「现文」与 MS36 增量 `specs/legacy-format-compat/spec.md` 的对应文本**逐字相等**（程序比对，`True`）⇒ 同样可直接替换。**E7 的「现文」是节选**（省略了「（唯一根段存在、二级索引段与表类型字节缺失）」与后半句「G2 与 G1 的唯一差别…」），**不能**按字符串替换、须整条替换 E7 条目——本 Review 已就地补注（见 F12）。
- **F8（通过·T21 的 M19 合规性——本 Cycle 最关键项）** 独立解析 Cycle 文件的 14 行批次表并与工作区实际文件清单对账：批次 **14**（B00 = `--lib --bins`，B01–B13 = `tests/*.rs`）；B01–B12 各 **11** 个目标、B13 **10** 个 ⇒ 全部落在纪律 2 的 **8–12** 区间内；把 B01→B13 的目标序列**拉平**后与文件名字典序比较 ⇒ **完全相等**（全局字典序成立，无乱序、无重复批次）；目标清单共 **142** 项、去重后 **142** 项，与 `tests/*.rs` 的实际 **142** 个文件**集合相等**（missing **0** / extra **0** / duplicate **0**）⇒ 纪律 10 的覆盖对账成立；全部 **14** 批的退出码列均为 **0**、failed 列均为 **0**；逐批 passed 求和 = **2231**、ignored 求和 = **2** ⇒ 与 Act 记录的 **2231 passed / 0 failed / 2 ignored** 一致；目标口径 **143** = 142 文件 + `--lib --bins` 一组，与 Runbook R48 的「目标数 ≠ `test result:` 行数」同型（Act 的 Minor 1 已记录，**非缺陷**）。**基线差额独立复算**：逐文件统计 7 个新增测试文件的用例数 = `index_page_ownership_test` 5 + `index_page_ownership_stack_overflow_test` 1 + `ddl_rewrite_consistency_test` 9 + `release_path_bounded_read_test` 7 + `release_durability_precondition_test` 2 + `dml_tombstone_abandon_window_test` 3 + `index_read_bounded_read_test` 5 = **32**；`legacy_compat_test` 的 `#[tokio::test]` 数 = **16**（基线 9 ⇒ **+7**）⇒ **32 + 7 = 39** = 2231 − 2192 ⇒ 纪律 12 的差额归因**独立复算吻合**。⇒ **M19 的 12 条纪律逐条成立**（纪律 1 独立成批、纪律 3 每批一次调用、纪律 4/5/6 串行且带 `--test-threads=1`、纪律 7 全部 0 failed、纪律 8 一步管道计数、纪律 9 逐批记录、纪律 10 对账不重不漏、纪律 12 差额已说明）。**本 Review 未重跑全量**（沿用结论，覆盖范围未变化）。
- **F9（通过·无身份证明机制与无框架外判定层）** 本 Cycle 的判定面全部落在 `cargo test` 与真实 CLI 内：G2 判别 = 目录行结构断言（结构化锁定，非版本比对）；C-2 = CLI 退出码 / JSON 输出 / 行集；T21 = `cargo` 退出码 + 原生 `test result:` 行；C-3 = 人工核对项（最短操作 + 直接可观察结果），无汇总工具、无截图依赖。全文无 hash / 指纹 / revision pin / run-id / manifest / 时间顺序证明；无框架外判定脚本、封装命令或判定层；无 timer 阈值行为判据（测试内的超时仅为夹具失控兜底）。**通过。**
- **F10（通过·9 份规格增量的可合并性）** 本 change 的 5 份增量（IPO 3/6、DRC 3/10、RPBR 1/4、RDP 2/7、IRBR 1/5）全部为 `## ADDED Requirements`、Requirement 数与交接清单第 1 条**逐项吻合**；逐条核对每个 Scenario 都有用例验收：IRBR 的 5 个 Scenario ↔ `tests/index_read_bounded_read_test.rs` 的 5 条用例**一对一**（`damaged_index_page_makes_{select,insert,delete,update}_fail_with_a_named_error` + `undamaged_index_reads_and_writes_are_unchanged`）⇒ 无「未验收写成已验收」。跨 change 的合并前提：`openspec validate --specs` ⇒ **75 passed / 0 failed**（既有 75 域不受影响）；MS36 的 `database-file-format-header` 增量为 `## ADDED` 且其 Requirement 标题「格式版本是产品格式契约」**不在**既有 4 条 Requirement 中 ⇒ ADDED 合法；MS35 的两条 MODIFIED 增量的 Requirement 标题与既有域**逐字相同** ⇒ 可直接替换。⇒ **9 份增量均可合并，彼此不冲突、与既有 75 域不冲突**（唯一的合并障碍在 MS35 的**另一份**增量，见 F13）。
- **F11（通过·Deviation 2 的裁定：额外 2 条 G2 用例 = 覆盖补齐）** Plan Context 的 Acceptance 2 明列 G2 的**七项能力**（打开 / 查询 / 写入 / 完整性 / 打开时重建 / dump / 恢复）各有常态化断言，而 Cycle RTM 只列出**两条**计划用例（`legacy_g2_db_opens_queries_and_writes` / `legacy_g2_db_open_rebuild_keeps_live_table_intact`）⇒ **dump / 恢复两项能力在计划名集合中无专属断言**。Act 追加的 `legacy_g2_dump_restores_into_current_db`（`:1158`）与 `legacy_g2_catalog_rows_are_in_g2_generation`（`:902`）恰好补上该缺口，且两者的断言内容与交接清单第 2 条 E7 的「更正为」文本**逐项对应**（文件级样本 + 四条 G2 用例名）⇒ 是 **Acceptance 2 的覆盖补齐**，不是范围扩张（仍在 `tests/legacy_compat_test.rs` 内、零产品代码、派生量与来源说明同源）。补充核实：七项能力中的**完整性**由 `legacy_g2_db_opens_queries_and_writes` 内的 `check_integrity` 零 error 断言承载（`:1072`）⇒ 七项能力均有专属断言，Acceptance 2 成立。
- **F12（非阻塞 Minor·change `tasks.md` 的两处交接清单缺陷——已就地修正，不改 Plan Context）** ① RTM 段（`:82`）把 MTV 的 AR2 澄清指向「收尾交接清单**第 4 条**」，而实际条目是**第 5 条**（第 4 条是 IC-1 / IC-2 的 Issue 候选移交）⇒ 维持者按该指针会取错条目；② 交接清单第 2 条的 E7「现文」是**节选**而非逐字原文 ⇒ 维持者若按「现文」做字符串替换会留下残留片段。两处均为**可直接定位的路径 / 文本修正**，不涉及行为要求 ⇒ 本 Review 已就地修正（`tasks.md`），**未**改写 `Plan Context`、**未**触碰 `openspec/specs/`。
- **F13（非阻塞 Minor·MS35 的 `free-page-lifecycle` 增量缺 delta 段头——阻塞 MS35 的归档，不阻塞本 change）** `openspec validate --changes --no-interactive` ⇒ **2 passed / 1 failed**：`change/2026-10-24-ms35-free-page-reuse-across-restart` 的 `free-page-lifecycle/spec.md` 报 **"No delta sections found. Add headers such as `## ADDED Requirements`"**——该增量用的是 `## Requirements`（无 delta 段头），而该域在 `openspec/specs/` 下**不存在**（75 域中无）⇒ 其语义应为 `## ADDED Requirements`。⇒ **MS35 在修正该段头前无法被 `openspec archive` 正常合并**。该缺陷属 **MS35**，不在本 change 范围 ⇒ 本 Review **不修改** MS35 的产物（不越权），**写入收尾交接说明**交 Maintainer / 上游处置。
- **F14（非阻塞 Minor·IRBR 的 Requirement 标题与 Scenario 标题仍用「损坏的索引页」）** `specs/index-read-bounded-read/spec.md` 的**规范正文**已收敛为「读取到**未通过页级判据**的索引树页时」（Purpose 同步），但 Requirement 的**标题**（「损坏的索引页在读 / 写路径上给出明确的错误」）与 4 个 Scenario 的标题 / GIVEN 仍用「损坏的索引页」。判定：正文是规范句、标题与场景是标签与示例；各 Scenario 的 GIVEN（槽目录项 offset 越界）**确实**未通过页级判据 ⇒ **未把未验收行为写成已验收**，**不阻塞**合并；仅为措辞残余张力，建议 Maintainer 合并时顺带对齐标题 / 场景措辞（**非必做**）。
- **F15（非阻塞 Minor·G2 样本的来源事实不可机械验证）** 见 F3 的说明：按公共规则禁止身份证明机制 ⇒ 「样本确由旧构建 `6e8dcf5` 产出」不可由断言背书，只能由来源说明与产出步骤承载。Plan Context 已独立核实该 revision 的目录行序列化布局与 G2 定义一致 ⇒ 记录与判别互洽。本 Review 已用当次运行坐实**样本字节**满足 G2 结构断言；**来源**一环仍属记录性证据。**不阻塞**（公共规则明确禁止为样本身份引入比对机制）。

**Acceptance Gaps**

无。Acceptance 1–6 逐条满足：

1. **G2 有真实来自旧构建的文件级样本且固定入库（C-1）** ✔ `tests/fixtures/legacy/legacy_g2.db` 存在（32832 B）；来源说明（`tests/fixtures/legacy/README.md`）写明 revision / 构建方式 / 产物路径 / 工具链 / 代际判别特征 / 内容清单 / 预期结论 / 再生步骤；**运行时无版本伪造、无 hash / revision 比对**（F2、F3、F15）。
2. **G2 样本的七项能力有常态化断言，派生量与来源说明同源（F11）** ✔ 打开 / 查询 / 写入 / 完整性 / 打开时重建 / dump / 恢复七项各有断言；派生量（目录行长度、唯一根计数、DEFAULT 段、恢复行数）在**断言**与**来源说明**两处同步。
3. **三条一次性兼容条款转为常态化用例（C-2）** ✔ `restored_legacy_db_accepts_further_writes` / `current_build_dumps_legacy_g1_db` / `legacy_g1_db_accepts_two_consecutive_writes`，断言可观察结果而非仅退出码（F4）。
4. **迁移路径实测记录的命令列可直接执行（C-3）** ✔ MS36 两张表的命令列每条 `rtsql` 调用的路径参数含 `/`；退出码与结果逐条未动；「失配只在命令写法」的事实保留（F5）。
5. **规格增量与验收边界对齐 + 收尾交接清单就绪** ✔ RPBR 的「尽力而为」AND 子句与 IRBR 的 Requirement 措辞收敛**已就地落地**（只补子句 / 只改措辞，不改行为要求）；MTV 的 AR2 澄清以**逐字可应用的**交接项成文（清单第 5 条），落点裁定见 F6。
6. **收尾全量回归通过（M19 分批串行）** ✔ 12 条纪律逐条成立；14 批各自 0 failed；逐批记录齐全；覆盖对账 missing 0 / extra 0 / duplicate 0；与基线的差额 **+39 = 32 + 7** 经独立复算吻合（F8）。

**Evidence**

- 本 Cycle 的工作区改动（`git diff --numstat`）：`tests/legacy_compat_test.rs` **+567 / −0**、`tests/fixtures/legacy/README.md` **+56 / −0**；新增未跟踪 `tests/fixtures/legacy/legacy_g2.db`（32832 B）；MS36 Cycle 文件 **+12 / −10**（仅命令列 + 一句说明）；本 change 的 `specs/release-path-bounded-read/spec.md` **+1 / −0**（「尽力而为」AND 子句）；`specs/index-read-bounded-read/spec.md` 与 `tasks.md` 的本轮改动。**`src/` 零改动**（F1）。
- 机制坐实的读码位点：`tests/legacy_compat_test.rs:860`（`g2_tables_row_len` = G1 + 4）/ `:865`（唯一根计数）/ `:882`（DEFAULT 段解析）/ `:902`（代际断言）/ `:971`（七能力之一）/ `:1110` / `:1158`（dump 与恢复）/ `:1227` / `:1286` / `:1354`（三条常态化用例）；`openspec/specs/mvcc-tombstone-visibility/spec.md:49`（AR2 正文，与交接清单「改动前」逐字节相等）；`openspec/changes/2026-10-24-ms36-legacy-format-and-dump-compat/specs/legacy-format-compat/spec.md:35-37`（E2 / E4 / E7 现文）。
- 验证结论：**当次运行**（本 Review 亲自执行）`cargo test --offline --test legacy_compat_test -- --test-threads=1` ⇒ **16 passed / 0 failed / 0 ignored**；`openspec validate --specs --no-interactive` ⇒ **75 passed / 0 failed**；`openspec validate --changes --no-interactive` ⇒ **2 passed / 1 failed**（失败项为 MS35 的 `free-page-lifecycle` 增量，见 F13）。**沿用**（覆盖范围自 Act 运行后未变化，未重复运行）：T21 的 14 批全量 **2231 passed / 0 failed / 2 ignored**（来源：Act Response 的 T21 批次表；本 Review 以程序独立重算批次结构、覆盖对账与差额归因，结论一致）。Persisted Evidence = `none` ⇒ 无 `evidence/` 目录，不据此提出问题。
- 本 Review **未修改任何产品代码**、**未新增 / 删除任何文件**；对 change 产物的写入仅限：本 Cycle 的 `Plan Review` 段、`tasks.md` 的两处交接清单修正与终态记录 ⇒ 被覆盖代码自 Act 运行后**未变化**。

**Issue 候选（范围外，只报告不登记）**

- **IC-1（沿用 Iteration 006 的登记，本 Cycle 未改变其状态）**：叶页槽负载短于条目格式（38 字节）时不被页级判据判为损坏 ⇒ 该条目被静默跳过。本 Cycle 的 IRBR 措辞收敛**已把该行为排除在已验收主张之外**，但**缺陷本身仍在**。**承载方式**：另立 change 承载「叶页条目长度的页级判据」，按**读 / 写路径 = 明确错误**、**释放路径 = 计数 + 报出 + 跳过**两条相反处置方向分别设计。**本 change 不承载**（T23 Forbidden + Iteration 006 的 Non-goals）。
- **IC-2（沿用 Iteration 006 的登记）**：存储层其余页型的无界读点（数据页 / 目录页 / 恢复面 / 缓冲池，同形态约 17 处）。**承载方式**：按**页型**分组另立 change，每组各自判定「损坏页在该页型的正确行为」；夹具沿用 `slot_offset_field_offset` / `page_file_offset` / `corrupt_slot_offset`。**本 change 不承载**。
- **IC-3（新增·非缺陷但影响记录可执行性）**：`rtsql` 的裸名参数解析（`$RTSQL_HOME/db/<name>.db`，`src/cli/resolve.rs:30`）是**既有契约**，但它使文档 / 记录中的裸名库命令在字面执行时必然失败（本 Cycle 的 T19 探针实测退出 1）。**承载方式**：**不是缺陷**、不登记为 Issue；按现有形态预防即可——记录命令一律用含 `/` 的路径形式 + 一句工作目录说明（本 Cycle 的 C-3 更正即该形态的样板）。
- **IC-4（新增·工具链类，非本 change 产物）**：MS35 的 `free-page-lifecycle` 增量缺 delta 段头 ⇒ `openspec validate --changes` 失败 ⇒ MS35 无法正常归档（见 F13）。**承载方式**：由 Maintainer / 上游在归档 MS35 **之前**把该增量首行改为 `## ADDED Requirements`（该域在 `openspec/specs/` 下不存在 ⇒ 语义为新增）。**本 change 不修改** MS35 的产物。

**Follow-up Decision**

接受当前 Cycle。判据：既有 Acceptance 1–6 **全部满足**且**无阻塞项**（F1–F11 为通过项；F12–F15 为**非阻塞 Minor finding**）；Act 的两条 Deviation **逐条裁定为合规**——Deviation 1（T20 ① 的落点）**成立**，其依据是 Plan Context 自身的 Targets 与 Preserve / Forbidden / Excluded scope 冲突（归类为 **Plan 措辞误标**，非 Act 偏离），交接措辞经逐字核对**可直接应用**；Deviation 2（额外 2 条 G2 用例）**属覆盖补齐**（Acceptance 2 的七项能力中 dump / 恢复在计划名集合中本无专属断言）。**不要求当前 Cycle 修复任何项**，不创建 rework / replan Cycle。Iteration 007 的 Iteration Plan **保持不变**（T16–T21、依赖 Iteration 003/004/005/006、验收边界、验证边界均无需修改）。当前 Iteration 为本 change 的**最后一个** Iteration ⇒ **Next Iteration: None**；change 的收尾（合并规格增量进 `openspec/specs/`、同步全局 tasks / SNAPSHOT / M / R / I、归档 change、登记 / 关闭 Issue）属 `openspec-docs-maintainer` 与 Recorder 的职责，**不由本 Review 执行**——逐条交接待见下。

**Iteration Plan Update**

None（Iteration 007 的 Iteration Plan 不变）。

**Next Cycle**

None。

**Next Iteration**

None——Iteration 007 是本 change 的**最后一个** Iteration（`openspec/changes/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/tasks.md` 的 Iteration Map 中 000–007 全部 `accepted`）。本 change 的**收尾**（规格增量合并 / 全局状态同步 / 归档 / Issue 登记与关闭）交 `openspec-docs-maintainer` 与 Recorder，见下节。

**收尾交接说明（交上游派发 `openspec-docs-maintainer`；本 Review 未执行其中任何一项）**

**A. 本 Review 明确不做、留给 Maintainer 的三件事**（AGENTS.md 阶段边界：Maintainer 收尾才合并行为规格、维护全局状态与归档）：

1. **合并本 change 的 5 份规格增量**进 `openspec/specs/`（均为 `## ADDED Requirements`，且 `openspec/specs/` 下无同名域 ⇒ 全新域）：`index-page-ownership`（3 Requirement / 6 Scenario）、`ddl-rewrite-consistency`（3 / 10）、`release-path-bounded-read`（1 / 4）、`release-durability-precondition`（2 / 7）、`index-read-bounded-read`（1 / 5）。合并后 `openspec/specs/` 由 75 域增至 **80 域**。
2. **同步全局状态**：`.agents/docs/SNAPSHOT.md`、`.agents/docs/tasks.md`（里程碑运行状态）、M / R / I 登记。
3. **归档本 change**（`openspec archive`）——**前置条件**：先完成 A.1 与 A.2，且先修正 IC-4（MS35 的 delta 段头）。

**B. T20 ① 的交接措辞（必须由 Maintainer 在合并时应用；改动前 → 改动后已逐字核对，可直接 find-and-replace）**

- **位置**：`openspec/specs/mvcc-tombstone-visibility/spec.md` 的 Requirement「未提交与已回滚墓碑不抑制、评估回溯前驱」正文（现为 `:49`）。
- **改动前**：「删除者事务尚未提交（读取时仍在活跃集合，或 RC 视图下语句开始时仍活跃）时，该墓碑 SHALL NOT 抑制版本链，评估回溯产出删除前对该读者可见的最新版本。墓碑事务 ROLLBACK 后，该墓碑 SHALL 中性化（不再作为已提交删除抑制链），行为与该删除未发生一致（扫描路径）。」
- **改动后**：「删除者事务尚未提交（读取时仍在活跃集合，或 RC 视图下语句开始时仍活跃）时，该墓碑 SHALL NOT 抑制版本链，评估回溯产出删除前对该读者可见的最新版本。**该行仍可见 ⇒ 其键仍被占用（对该键的插入按既有唯一性语义被拒绝——该拒绝是键被占用的正常结果，不是残留条目造成的假阳性；此处「无残留条目」指该键的索引条目数为 1，即墓碑不为同一键留下第二条条目，而不是指该键可被再次插入）；只有该删除真正被提交之后，同键的插入才可再次成功。** 墓碑事务 ROLLBACK 后，该墓碑 SHALL 中性化（不再作为已提交删除抑制链），行为与该删除未发生一致（扫描路径）。」
- **形态**：只加解释性括注，既有 SHALL 逐条不变（不新增、不放宽任何行为要求）；与同域「回滚后墓碑行的索引条目还原」Requirement 不冲突。完整原文另见本 change `tasks.md` 的「收尾交接清单」第 5 条。

**C. MS36 的 `legacy-format-compat` 证据索引更新（合并 MS36 时必须同批应用；本 change `tasks.md` 的交接清单第 2 条含逐条对照）**

- **E2（G1 写入）末句**：「**写入后仍能再次写入**由本 change 的真实 CLI 实测记录（同一 G1 旧库上连续两次写入，退出码均为 0，写入后查询可见）」 ⇒ 改为「**写入后仍能再次写入**由 `tests/legacy_compat_test.rs::legacy_g1_db_accepts_two_consecutive_writes` 覆盖（同一 G1 旧库上连续两次写入，退出码均为 0，写入后查询可见：users 5 行、audit 2 行）」。
- **E4（G1 dump）末句**：「**当前构建导出 G1 旧库**由本 change 的真实 CLI 实测记录（退出码 0、导出按当前格式渲染长类型名）」 ⇒ 改为「**当前构建导出 G1 旧库**由 `tests/legacy_compat_test.rs::current_build_dumps_legacy_g1_db` 覆盖（退出码 0、stderr 空、导出 3 表 + 全部行；导出文本可恢复到新库且行集等价）」。
- **E7（G2 无专属断言）**：**整条替换**（现文是节选，不可按字符串替换）⇒ 改为「G2 形态已有**文件级样本**与断言：样本 `tests/fixtures/legacy/legacy_g2.db`（由 revision `6e8dcf5` 的旧构建在仓库之外产出，来源与再生步骤见 `tests/fixtures/legacy/README.md`）；用例 `tests/legacy_compat_test.rs::legacy_g2_catalog_rows_are_in_g2_generation`（目录行形态：含唯一根段、不含二级索引段与表类型字节）、`::legacy_g2_db_opens_queries_and_writes`（打开 / 查询 / 写入 / 完整性 + 被触及表的目录行升级）、`::legacy_g2_db_open_rebuild_keeps_live_table_intact`（打开时重建不放弃）、`::legacy_g2_dump_restores_into_current_db`（dump / 恢复往返等价）」。
- **能力矩阵的 G2 行**：7 列由「行级既有覆盖（E6）/ 无 G2 专属断言（E7）」改为**文件级样本（E7）**（打开 / 查询 / 写入 / 完整性 / 打开时重建 / dump / 恢复七列均已覆盖）。
- **R4 S1** 的证据由 `tests/legacy_compat_test.rs::restored_legacy_db_accepts_further_writes` 承载；「可再次导出且等价」仍由 `::legacy_dump_restores_into_current_db` 承载。

**D. 三个 change 的收尾派发（并行事实：MS35 / MS36 / 本 change 均已 `accepted`，待统一派发 Maintainer）**

| change | 待 Maintainer 做的事 | 前置 / 注意 |
|---|---|---|
| `2026-10-24-ms35-free-page-reuse-across-restart` | 合并 3 份增量（`drop-persistence-boundary` MODIFIED 1/4、`drop-table-physical-free` MODIFIED 2/7、`free-page-lifecycle` ADDED 4/7）→ `openspec/specs/`；同步全局状态；归档 | **必须先修 IC-4**：把 `specs/free-page-lifecycle/spec.md` 的首行段头改为 `## ADDED Requirements`（该域在 `openspec/specs/` 下不存在）⇒ 否则 `openspec validate --changes` 失败、`openspec archive` 无法合并。两条 MODIFIED 的 Requirement 标题与既有域逐字相同，可直接替换 |
| `2026-10-24-ms36-legacy-format-and-dump-compat` | 合并 2 份增量（`database-file-format-header` ADDED 1/3、`legacy-format-compat` ADDED 5/14）→ `openspec/specs/`；**同批应用 C 节的证据索引更新**；同步全局状态；归档 | `legacy-format-compat` 的证据索引在合并**同一批**内更新，否则会先落一份「无 G2 文件级样本」的过期证据索引 |
| `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence`（本 change） | 合并 5 份增量 → `openspec/specs/`（75 → **80** 域）；**同批应用 B 节的 MTV AR2 澄清**；同步全局状态；归档 | 8 个 Iteration 全部 `accepted`；T21 全量结论 **2231 passed / 0 failed / 2 ignored**（14 批 / 143 目标）应记入收尾 |

**E. Issue 台账（`.agents/issues/`，只读核对；登记 / 关闭由 Recorder 按用户指令执行）**

| Issue | 现状 | 本 Review 的裁定 |
|---|---|---|
| **ISS22**（跨页容量边界的删列改写丢行） | `open` | 本 change 的 Iteration 001（T4 / T5）已修并验收 ⇒ **可关闭**（由 Recorder 更新指针 + 关闭） |
| **ISS23**（大规模删列改写索引下降栈溢出） | `open`（处置段仍写「待建 change 承载」） | Iteration 000（T3）已判定归属 = B-2 的**下游后果**并已修 ⇒ **可关闭**（由 Recorder 更新指针 + 关闭） |
| **ISS24**（被放弃的删列改写隐藏已改写的行） | `open` | Iteration 001（T6 / T7，DDL 面）+ Iteration 005（T14 / T15，DML 同形面）均已修并验收 ⇒ **可关闭** |
| **ISS25**（改写窗口内的崩溃-重开 / 提交后放弃仍按位置错位） | `open` | Iteration 003 只闭合「**并发读**」变体；「**崩溃-重开**」变体仍开放（关闭它需持久标记 ⇒ 改磁盘格式并破坏 MS36 的 G1/G2/G3 兼容面，或移动提交点 ⇒ 与 ISS21 / D5 冲突）⇒ 经 Plan Review 裁定 **保持 `open`**，随 Iteration 003 的登记一并保留 |
| **IC-1 / IC-2**（Iteration 006 登记，本 change 只报告不登记） | 未登记 | 由 Recorder 按用户指令择承载（另立 change：叶页条目长度的页级判据 / 其余页型的无界读点） |
| **IC-3**（裸名库参数的记录可执行性） | 非缺陷 | **不登记**；预防形态见 C 节与本 change 的交接清单 |
| **IC-4**（MS35 的 `free-page-lifecycle` 增量缺 delta 段头） | 未登记 | **不是产品缺陷**，是归档工具的格式障碍 ⇒ 并入 D 节的前置条件处理 |

**F. 本 Review 的产物写入**

- 本 Cycle 文件：追加 `## Plan Review` 段（含 `Review Result: accepted`、`Next Iteration: None`、上述交接说明）。
- change `tasks.md`：修正 RTM 段对 MTV 交接项的**交叉引用**（第 4 条 → **第 5 条**）、为交接清单第 2 条的 E7 补注「**现文是节选，须整条替换**」、写入 **Iteration Map 终态**与 **Next Iteration 终态**。
- **未**修改 `src/`、**未**修改 `openspec/specs/`、**未**修改 `Plan Context`、**未**填写 Act Response、**未**归档 change、**未**同步全局 tasks / SNAPSHOT / M / R / I。
