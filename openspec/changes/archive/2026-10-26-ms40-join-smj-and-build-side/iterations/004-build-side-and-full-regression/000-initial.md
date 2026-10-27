# Iteration 004 / Cycle 000: 较小构建侧与全量回归

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T3，以及 tasks.md 的 4.2 全量回归
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 排序合并执行器已经能执行。写出的整数 `PRIMARY KEY` 才走排序合并，隐式首列整数键仍走哈希。`JoinExecutor::normalize_signed_zero_key` 已在树里：键向量里每个 `Value::Float(f)` 且 `f == 0.0` 的元素换成 `Value::Float(0.0)`，NaN 不换，桶内行和输出行保持输入原值。T5 的两条当前代目录行长度断言已经是 `g1_tables_row_len + 10`，尾段以 `0x01, 0x00` 结束。
- Excluded scope: 不改排序合并算法、计划期分类、`Value::Hash`、`ValueRef::Hash`、`compare_floats`、`compare_join_value`、`sort.rs` 的 `compare_values`、半连接、反连接、聚合。不改 `serialize_catalog_row`。不改 `iterations/002-signed-zero-match/000-initial.md` 和 `iterations/003-current-catalog-row-length/000-initial.md`，这两个文件的 Review Result 保持 accepted。不读写 `openspec/changes/archive/2026-10-26-ms40-add-column-catalog/`。不改 `scale_join` 的等行数基准。

**Objective**

等值哈希连接在 criterion 平均时间不超过「始终构建右输入」基线的 90% 时，改为用行数较少的一侧构建。达不到则撤回这次接线。撤回之后，生产路径仍构建右输入，本变更留下的产品行为只有已经完成的排序合并和已经完成的负零键归一。无论哪一个结局，都按下面的分批方式做本变更唯一的一次全量回归。缺一批即未通过。

**Investigation Facts**

- Starting point: Iteration 002 的 `iterations/002-signed-zero-match/000-initial.md` 为 reported，Review Result 为 accepted。那次把 `build_hash_key_right` 和 `build_hash_key_left` 交给哈希表之前的键收成正零。修改前 `cargo test --test sort_merge_join_test signed_zero_matches_across_hash_and_sort_merge` 退出码 101，排序合并有这对行，哈希连接没有。修改后该过滤命令退出码 0，1 passed。`cargo test --test sort_merge_join_test` 退出码 0，15 passed。Iteration 003 的 `iterations/003-current-catalog-row-length/000-initial.md` 为 reported，Review Result 为 accepted。修改后两条 `legacy_compat_test` 和两条 lib 往返测试都是退出码 0。本次规划没有重跑这些命令，没有跑全量，没有跑 criterion。
- Current-State Evidence:
  - `JoinExecutor::next`（`src/executor/join.rs`）在 `executed` 为假时进入 `JoinPhase::BuildRight`。该阶段把 `right_executor` 读完，非 NULL 键行经 `charge_bucket_insert` 进入 `right_hashmap`。然后 `JoinPhase::Output` 逐行读 `left_executor`，用左键 `HashMap::get`，按桶下标取右行。左行只留在 `current_left`，不计费。两个键函数在返回前都调用 `normalize_signed_zero_key`。
  - `pipeline.rs` 对 `PhysicalPlan::Join` 构造 `JoinExecutor::new(...).with_budget(...)`。排序合并走 `SortMergeJoinExecutor`，不进这个状态机。半连接和反连接有自己的 `BuildRight`。
  - 行序：无 `ORDER BY` 时，输出按探测侧的输入顺序，同一探测行上的匹配行按构建侧进入桶的顺序。正整数主键的 `scan_all` 是 `i64::to_be_bytes` 字典序，本 Cycle 用到的正 id 上就是 id 升序。`test_join_basic_hash_join` 左右都是 3 行，行数相等，接线保留后仍构建右输入，现有 Alice、100 / Alice、150 / Bob、200 的顺序不变。`test_join_one_to_many` 左 1 行、右 4 行；接线保留后构建左输入、探测右输入，但只有一行左表，金额顺序仍是 100、200、300、400。
  - `tests/query_memory_test.rs` 的 `small_right_large_left_join_succeeds_under_small_limit`：`l(id INT PRIMARY KEY, pad INT)` 20 行，`r(id INT PRIMARY KEY, ref INT)` 一行 `(100, 3)`，语句 `SELECT l.id, r.id FROM l JOIN r ON l.id = r.ref`，上限 64。连接键 `r.ref` 不是 `r` 的主键，留在哈希路径。当前只驻留右行 32 字节加键 16 字节，共 48，测试期望成功并得到 `(3, 100)`。`join_right_side_fails_under_small_limit` 是左 2 行、右 4 行、同一形状、上限 64，期望失败。`default_zero_limit_allows_existing_queries` 不设上限，`t` 50 行 JOIN `s` 2 行必须成功。
  - `benches/scale_bench.rs` 的 `scale_join` 对两表各插入相同的 `n`，语句是 `orders INNER JOIN customers ON orders.customer_id = customers.id`。`benches/common/mod.rs` 的 `create_join_tables` 建的是 `customers(id INT PRIMARY KEY, name STRING)` 和 `orders(id INT PRIMARY KEY, customer_id INT, amount INT)`。`insert_join_data` 两侧各 `n` 行。Cargo.toml 里 `scale_bench` 的 `harness = false`，没有 `[[test]]` 改名。
  - 时间门先例是 MS32：`cargo bench --bench data_scan_bench -- next_batch/10000 --save-baseline before-ms32-batch --sample-size 10 --measurement-time 2 --noplot`，对比把 `--save-baseline` 换成 `--baseline`。判定用 criterion 打印的平均时间，也就是 `time:` 三个数里中间那个。当时基线 19.467 ms，对比 19.061 ms，没有降到 90%，生产攒批被撤回。
- Code and Critical Path: `execute_sql` 对纯等值且未选中排序合并的语句构造 `PhysicalPlan::Join`，再进入 `JoinExecutor::next`。构建侧这一轮只改这个状态机，以及为它新增的基准和测试。全量在这条接线要么留下、要么撤回之后才跑。

**Implementation Guidance**

先写测试并看到 `hash_join_probes_larger_side` 失败，此时还不要改 `src/executor/join.rs`。然后只加基准并保存基线。基线保存之后到对比命令结束之前，产品代码只允许出现这次构建侧接线；基准文件、`Cargo.toml` 和其他产品文件在这两次 bench 之间不得再改。对比结束后才按通过或撤回收测试。全量放在收完之后，并且先重算 N。

**Task Definitions**

### T3: 时间门决定哈希连接是否按行数选择构建侧

- Requirement/Scenario: 纯等值 JOIN Hash 路径保持 / 右输入更大且接线保留；时间门未过；哈希 JOIN 不驻留整表左输入 / 接线保留时小右表加大左表因左输入驻留而失败，或接线撤回时小右表加大左表仍成功
- Depends on: None。T2 和 T4 的产品代码已经在树上，本任务不再实现它们。
- Targets:
  - `src/executor/join.rs::JoinExecutor::next`
  - `benches/scale_bench.rs` 新增的 `build_side_join` 基准，以及 `criterion_group!` 对它的注册
  - `tests/hash_join_build_side_test.rs`
  - 仅在时间门通过后：`tests/query_memory_test.rs::small_right_large_left_join_succeeds_under_small_limit`
- Current behavior: 哈希连接总是读完右输入建成哈希表，再按左输入逐行探测。左输入不整表驻留。`normalize_signed_zero_key` 已经作用在两个键函数上。没有 1000 对 100000 的构建侧基准，也没有 `hash_join_build_side_test`。
- Required behavior: 四条顺序测试都经 `execute_sql`，各自使用独立的临时目录。表是 `l(id INT PRIMARY KEY, k INT, tag STRING)` 和 `r(id INT PRIMARY KEY, k INT, tag STRING)`。语句固定为 `SELECT l.tag, r.tag FROM l INNER JOIN r ON l.k = r.k`，没有 `ORDER BY`。所有行的 `k` 都是 1。`id` 从 1 起按插入顺序递增，`tag` 与下面的期望一一对应。正 id 的扫描序就是 id 升序。
  - `hash_join_equal_counts_probe_left`：左 `L1`,`L2`，右 `R1`,`R2`。期望顺序 `(L1,R1),(L1,R2),(L2,R1),(L2,R2)`。接线保留或撤回都是这个顺序。
  - `hash_join_larger_left_still_probes_left`：左 `L1` 到 `L5`，右 `R1`,`R2`。期望按左行分组：`(L1,R1),(L1,R2)` 直到 `(L5,R1),(L5,R2)`。右表更小，两种结局都构建右输入。
  - `hash_join_probes_larger_side`：左 `L1`,`L2`，右 `R1` 到 `R5`。期望按右行分组：`(L1,R1),(L2,R1)` 直到 `(L1,R5),(L2,R5)`。这是接线保留后的顺序。
  - `hash_join_builds_right_when_right_is_larger`：数据与上一条相同。期望按左行分组：`(L1,R1)` 到 `(L1,R5)`，然后 `(L2,R1)` 到 `(L2,R5)`。这是当前顺序，也是撤回后的顺序。
  两条右表更大的测试不能同时留下。时间门通过后删除 `hash_join_builds_right_when_right_is_larger`。时间门不通过后删除 `hash_join_probes_larger_side`。
  接线本身只在 `JoinExecutor` 内：读出两侧的 `ExecResult::Row`，行数含 NULL 键行。左行数小于右行数时，用左行构建、用右行探测。否则用右行构建、用左行探测。构建仍走对应一侧的键函数和 `charge_bucket_insert`。探测侧的非 NULL 键行在产出第一行之前按 `row_bytes` 入账一次。NULL 键行不进表、不留在探测缓冲、不计费。非 `Row` 结果仍跳过。子执行器返回错误，或入账超限，则返回该错误且不产出行。`build_output_row` 仍接收原来的左行和右行。`normalize_signed_zero_key` 的比较和两处调用保持不变。
  基准加在 `scale_join` 之外。组名 `build_side_join`，函数名 `customers_1000_orders_100000`。调用现有 `create_join_tables`。customers 插入 id 1 到 1000。orders 插入 id 1 到 100000，`customer_id = ((i - 1) % 1000) + 1`，因此每个 customers.id 都出现。测量闭包里只执行 `SELECT * FROM customers INNER JOIN orders ON orders.customer_id = customers.id`，并对 `Result` 使用 `unwrap`。不要改 `scale_join`、`insert_join_data` 和 `create_join_tables`。
  平均时间是 criterion 在该基准 `time:` 行打印的三个数的中间一个。两次运行的单位不同时，先换成同一单位再比。通过条件是对比平均时间 ≤ 基线平均时间的 90%。等于 90% 算通过。两个平均时间都写入 Act Response。`target/criterion` 不抄进变更目录。
  通过后还要改 `small_right_large_left_join_succeeds_under_small_limit`：表、插入、SQL 和 64 字节上限都不改，断言改为返回的错误含 `Execution error: query memory limit exceeded (limit 64 bytes)`。不把上限调高。
- Preserve: 行数相等时的左探测顺序。右表行数更少时的左探测顺序。`normalize_signed_zero_key` 对两个零和 NaN 的现有结果。`join_right_side_fails_under_small_limit` 仍失败。`default_zero_limit_allows_existing_queries` 仍成功。排序合并、计划期选择、目录尾字节、半连接和反连接的行为。
- Forbidden: 基线命令与对比命令之间不得修改除 `src/executor/join.rs` 里这次接线以外的产品代码，也不得修改基准文件和 `Cargo.toml`。不得用缩小行数、改语句或改 criterion 参数来换通过。不得改 `Value::Hash`。不得让排序合并参与这道门。不得在时间门结果出来之前删除两条互斥测试中的任何一条，也不得提前改内存测试。不改 Iteration 002、003 的 Cycle 文件。不改已归档的加列变更。
- First test: 在修改 `src/executor/join.rs` 之前，逐条执行：
  1. `cargo test --test hash_join_build_side_test hash_join_probes_larger_side` 必须失败，实际顺序是按左行分组的 10 行，不是按右行分组。
  2. `cargo test --test hash_join_build_side_test hash_join_equal_counts_probe_left` 退出码 0。
  3. `cargo test --test hash_join_build_side_test hash_join_larger_left_still_probes_left` 退出码 0。
  4. `cargo test --test hash_join_build_side_test hash_join_builds_right_when_right_is_larger` 退出码 0。
  第 1 条已经通过，或第 2 到第 4 条失败，或实际顺序对不上上面写的分组，都停止并返回计划。
- GREEN condition: 接线写入 `join.rs` 之后、对比之前，第 1 条变为退出码 0，第 2 条和第 3 条仍是退出码 0。第 4 条此时失败是预期，先留着，不要为了它撤回接线。对比命令退出码 0，并且 Act Response 里有两个平均时间。
- Verification: 定向测试可用默认并行度。基准命令的参数放在 `--` 之后。
  - 基线，此时 `join.rs` 仍构建右输入：`cargo bench --bench scale_bench -- build_side_join --save-baseline before-ms40-build-side --sample-size 10 --measurement-time 2 --noplot`
  - 对比，此时产品代码相对基线只多了构建侧接线：`cargo bench --bench scale_bench -- build_side_join --baseline before-ms40-build-side --sample-size 10 --measurement-time 2 --noplot`
  通过则保留接线，删除 `hash_join_builds_right_when_right_is_larger`。先跑 `cargo test --test query_memory_test small_right_large_left_join_succeeds_under_small_limit`：它必须因仍期望成功而失败。已经成功则说明探测侧没有按驻留计费，停止并返回计划。确认这次失败之后，才把该断言改成期望超限。然后这四条退出码 0：`cargo test --test hash_join_build_side_test`，`cargo test --test query_memory_test small_right_large_left_join_succeeds_under_small_limit`，`cargo test --test query_memory_test join_right_side_fails_under_small_limit`，`cargo test --test query_memory_test default_zero_limit_allows_existing_queries`。
  不通过则把 `JoinExecutor::next` 恢复为先 `BuildRight`、再按左输入探测的现有循环，`normalize_signed_zero_key` 和两处调用保留。删除 `hash_join_probes_larger_side`。不改内存测试。然后 `cargo test --test hash_join_build_side_test` 退出码 0，并且 `hash_join_builds_right_when_right_is_larger` 仍是按左行分组的 10 行。`cargo test --test query_memory_test small_right_large_left_join_succeeds_under_small_limit` 退出码 0，结果仍是 `(3, 100)`。`join_right_side_fails_under_small_limit` 仍失败。
  两种结局都再跑一次 `cargo test --test join_test`。退出码 0 就结束这条。若失败只是因为无 `ORDER BY` 的哈希连接行序变了、多重集合没变，把该断言改成多重集合比较，不改 SQL，再跑这一条失败测试一次。这次仍失败，或失败原因不是行序，停止并返回计划。时间门不通过时，`join_test` 应直接退出码 0；需要改断言才能通过，也停止并返回计划。
- Stop when: 先行测试的顺序或通过情况和上面不一致；基线或对比没有打印平均时间；测量语句返回 `Err`；两次 bench 之间除 `join.rs` 的接线外还有产品代码或基准文件变化；需要改排序合并、键归一、计划期或目录序列化才能让定向测试通过；全量还没开始就去跑全量。

### 4.2: 本变更唯一的一次分批全量回归

- Requirement/Scenario: 覆盖 T1、T2、T3、T4、T5 的最终树。T3 撤回时，树上仍有排序合并和负零键归一，没有较小构建侧接线。
- Depends on: T3
- Targets: 不改产品代码。只执行测试。时间门通过后，若某条既有测试仅仅因为无 `ORDER BY` 的哈希连接行序改变而失败，允许把该断言改成多重集合比较；这是本任务里唯一允许的文件修改。
- Current behavior: 本变更还没有跑过全量。项目模型 M19 要求全量分批、每批 `--test-threads=1`、批间串行，缺一批不得声明通过。M19 正文里的 2231、143 和 14 是当时的记录，不是这次的 N。
- Required behavior: 在 T3 的最终树上，从仓库根目录执行 `LC_ALL=C ls tests/*.rs | sort`。N 是这个列表的文件数。只统计这一次的输出，不沿用计划、M19 或任何以前的个数。B = ceil(N / 12)，整数计算用 `(N + 11) / 12`。大小为 11 的批次数 S11 = B×12 − N。大小为 12 的批次数 S12 = N − 11×B。列表按该排序从头切：前 S11 批各 11 个文件，后 S12 批各 12 个文件，连续、不重、不漏。`--test` 的名字是文件名去掉目录和 `.rs`。
  开始跑之前检查：N ≥ 1，S11 ≥ 0，S12 ≥ 0，S11 + S12 = B，11×S11 + 12×S12 = N，并且每一个实际产生的文件批的文件数都在 8 到 12。S11 或 S12 为 0 表示没有该大小的批，不是越界。任一条件不成立，停止并返回计划，不改批大小，也不运行全量命令。
  第 00 批先跑：`cargo test --lib --bins -- --test-threads=1`。然后按批序号跑文件批：`cargo test --test <名> --test <名> ... -- --test-threads=1`。前一批进程退出后才开始下一批。同一时刻只有一批。不使用不带 `--lib`、`--bins` 或 `--test` 的 `cargo test`。
  每一批记录退出码、passed 和 failed。通过条件是退出码 0 且 failed 为 0。一批没有运行，就是缺批，本任务失败。任一批 failed 不是 0，本任务失败。失败的一批可以按公共规则复核一次；复核退出码 0 且 failed 为 0，则记明首跑失败并按通过处理。复核仍失败，或对已经通过的批再跑，都不把本任务算通过。不得为了通过而改产品代码。时间门通过时，只有「无 `ORDER BY` 的哈希连接、多重集合相同、仅行序不同」这一种失败可以改断言；改完后只重跑该批一次。其他失败停止并返回计划。时间门不通过时，连这种断言也不改，失败即返回计划。
- Preserve: T3 结束时的产品代码。T5 的目录断言。负零归一。排序合并的选择规则。
- Forbidden: 不沿用历史 N。不把批大小改成 8 到 12 以外的数来凑覆盖。不并行跑两批。不把 benches 算进 N。不把 `tests/` 子目录里的夹具算进 `tests/*.rs`。不创建 Evidence 目录，不把完整测试输出抄进变更。不改 Iteration 002、003 的文件。不改已归档的加列变更。
- First test: 本任务没有产品行为变更，不适用新的失败测试。开始前 T3 的收尾命令已经退出码 0。若还没有，先停止，不跑全量。
- GREEN condition: 第 00 批和全部 B 个文件批都已运行，每一批退出码 0 且 failed 为 0。Act Response 写明 N、B、S11、S12，以及每批的命令、退出码、passed 和 failed。
- Verification: 命令就是上面的第 00 批和各文件批。passed 与 failed 以该批 cargo 摘要里的 `test result:` 为准。缺一批、任一批 failed 不是 0、或批大小检查没有通过就跑了，都是未通过。
- Stop when: N 的检查失败；某批的目标名不被 cargo 接受；失败不属于允许的那一种行序断言；需要改产品代码才能让某批通过；T3 的接线状态在全量期间又被改动。

**Invariants**

`Float(-0.0)` 与 `Float(0.0)` 在哈希连接和排序合并里都命中，输出位模式不变。写出的整数主键等值内连接仍是排序合并。隐式首列、非主键列、外连接和非等值的选择不变。上限为 0 时查询内存不计费。缺目录尾字节的旧行仍读成没写出 PRIMARY KEY。半连接和反连接仍构建右输入。

**Non-goals**

不引入统计信息、连接重排或结果流式化。不改 NaN 排序。不改计费口径的 16 字节。不把全量拆成两次。不在本 Cycle 收尾变更或刷新项目现状。

**Acceptance**

- Act Response 有基线平均时间和对比平均时间，并写明用的是哪一个。
- 对比平均时间 ≤ 基线的 90% 时：`hash_join_probes_larger_side` 按右行分组通过，等行数和左表更大的两条仍按左行分组通过，`hash_join_builds_right_when_right_is_larger` 已删除。`small_right_large_left_join_succeeds_under_small_limit` 在 64 字节上限下失败并带有指定错误文本。`join_right_side_fails_under_small_limit` 仍失败。`default_zero_limit_allows_existing_queries` 仍成功。
- 对比平均时间大于基线的 90% 时：`JoinExecutor::next` 恢复为构建右输入、探测左输入，负零归一还在。`hash_join_builds_right_when_right_is_larger` 按左行分组通过，`hash_join_probes_larger_side` 已删除。小右表测试仍成功并返回 `(3, 100)`。
- 两种结局都跑过 `cargo test --test join_test`，退出码 0。
- 全量的 N、B、S11、S12 来自执行当时的 `tests/*.rs`。检查通过后，第 00 批和全部文件批都运行过，每批退出码 0 且 failed 为 0。缺一批或任一批 failed 不是 0，则全量未通过。
- 两次 bench 之间的产品 diff 只有 `src/executor/join.rs` 的构建侧接线。

**Verification**

顺序测试和基准命令写在 T3。全量命令写在 4.2。判定看退出码、failed 计数和 criterion 的平均时间。不另写脚本汇总。不跑第二次全量来增加信心。

**Gate 2 Readiness**

PASS。无 BLOCKED，无 SKIPPED。2026-10-26 用户要求只展开 Iteration 004，时间门沿用 MS32 的 criterion 10% 先例，不过门就撤回接线，基线与对比之间不再改其他产品代码，全量按执行当时的 `tests/*.rs` 重算 N，越界或批大小不在 8 到 12 就停止，缺一批或任一批 failed 不是 0 即未通过。调查确认当前 `JoinExecutor::next` 仍先构建右输入，负零归一已经在两个键函数上。探测侧一旦整表留下就必须计费，所以接线保留时小右表用例改为期望超限；撤回时该用例保持成功。这个分支写进了 design 的 D2 和 `specs/query-memory-budget/spec.md`。RTM 无 Missing。Persisted Evidence 为 none。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- 1000 与 100000 的插入发生在测量闭包之外，但本身会花时间。不得把行数改小。命令没有打印平均时间，或测量语句 `unwrap` 失败，就停止。
- `HashMap` 的迭代顺序不决定输出。输出顺序只由探测侧顺序和桶内插入顺序决定。
- 规划时没有统计 `tests/*.rs` 的个数。执行时如果 S12 小于 0，直接返回计划，不要临时改成别的批大小。
- 工作区里基线之前就已经存在的未提交改动，两次 bench 都带着。窗口内不得再改它们。

## Act Response

- Status: reported

**实际改动**

时间门未过，哈希连接的生产路径恢复为先构建右输入、再按左输入探测。`JoinExecutor::normalize_signed_zero_key` 和它在 `build_hash_key_left`、`build_hash_key_right` 里的两处调用保持原样。`src/executor/join.rs` 与基线保存前的内容一致。

留下的新增文件和符号：

- `benches/scale_bench.rs`：`insert_build_side_rows`、`build_side_join`，并注册进 `criterion_group!`。组名 `build_side_join`，基准名 `customers_1000_orders_100000`。测量闭包只执行 `SELECT * FROM customers INNER JOIN orders ON orders.customer_id = customers.id`，并对 `Result` 使用 `unwrap`。装数在测量闭包之外：customers 的 id 为 1 到 1000，orders 的 id 为 1 到 100000，`customer_id = ((i - 1) % 1000) + 1`。`scale_join`、`insert_join_data`、`create_join_tables` 未改。
- `tests/hash_join_build_side_test.rs`：留下 `hash_join_equal_counts_probe_left`、`hash_join_larger_left_still_probes_left`、`hash_join_builds_right_when_right_is_larger`。删除 `hash_join_probes_larger_side`。
- `tests/query_memory_test.rs` 未改。

**与计划的偏差**

新测试第一次编译时，`Database::open` 需要 `&Path`，四条命令都是退出码 101，还没有跑到顺序断言。补上借用后重新执行，顺序和通过情况与 Task Definition 一致。这没有改产品代码，也没有改测试策略。

**自检**

Gate 3：撤回后的树符合时间门不通过的验收。排序合并、计划期分类、目录序列化和负零归一未改。基线与对比之间，`benches/scale_bench.rs` 与 `Cargo.toml` 的内容未变，产品代码只多了当时的构建侧接线；对比结束后该接线已撤回。没有新增判定脚本或身份证明机制。

没有遗留的 Critical 或 Important 问题。无 Minor。

**验证**

基线（接线之前，仍构建右输入）`cargo bench --bench scale_bench -- build_side_join --save-baseline before-ms40-build-side --sample-size 10 --measurement-time 2 --noplot` 退出码 0。`time:` 行为 `[282.46 ms 284.79 ms 287.43 ms]`。平均时间取中间值，**284.79 ms**。

接线写入后、对比之前：`hash_join_probes_larger_side`、`hash_join_equal_counts_probe_left`、`hash_join_larger_left_still_probes_left` 退出码 0。`hash_join_builds_right_when_right_is_larger` 退出码 101，实际顺序已是按右行分组的 10 行。

对比 `cargo bench --bench scale_bench -- build_side_join --baseline before-ms40-build-side --sample-size 10 --measurement-time 2 --noplot` 退出码 0。`time:` 行为 `[316.46 ms 323.46 ms 329.87 ms]`。平均时间 **323.46 ms**。两次单位都是 ms。基线的 90% 是 256.311 ms。323.46 ms 高于该值，接线撤回。

撤回后的定向测试：

- `cargo test --test hash_join_build_side_test` 退出码 0，3 passed，0 failed。`hash_join_builds_right_when_right_is_larger` 按左行分组通过。
- `cargo test --test query_memory_test small_right_large_left_join_succeeds_under_small_limit` 退出码 0，1 passed，0 failed。结果仍是 `(3, 100)`。
- `cargo test --test query_memory_test join_right_side_fails_under_small_limit` 退出码 0，1 passed，0 failed。查询本身仍因超限失败。
- `cargo test --test join_test` 退出码 0，9 passed，0 failed。没有改断言。

全量在撤回后的树上执行。`LC_ALL=C ls tests/*.rs | sort` 得到 N = 150。B = (150 + 11) / 12 = 13。S11 = 13×12 − 150 = 6。S12 = 150 − 11×13 = 7。S11 + S12 = 13，11×6 + 12×7 = 150。前 6 批各 11 个文件，后 7 批各 12 个文件，都在 8 到 12。没有缺批。

| 批 | 退出码 | passed | failed |
|---|---:|---:|---:|
| 00 `cargo test --lib --bins -- --test-threads=1` | 0 | 425 | 0 |
| 01 至 06，各 11 个 `--test`，`--test-threads=1` | 0 | 149、191、155、125、153、87 | 0 |
| 07 至 13，各 12 个 `--test`，`--test-threads=1` | 0 | 99、143、228、91、204、157、97 | 0 |

第 00 批有两条 `test result:`：库测试 425 passed、0 failed，二进制测试 0 passed、0 failed。每个文件目标的 `test result:` 都是 failed 为 0。文件批从排序后的 `tests/*.rs` 连续切分：01 从 `aggregate_extension_test` 到 `cache_perf_test`，06 到 `join_test`，07 从 `key_type_conformance_test` 到 `network_protocol_test`，13 从 `view_test` 到 `write_type_conformance_test`。

**Persisted Evidence**

None required。

**Experience Candidates**

None。

**未解决问题**

None。Issue 候选：None。

## Plan Review

- Review Result: accepted

**Findings**

无阻塞项。时间门未过，接线已撤回，撤回分支的既有 Acceptance 已满足。

`JoinExecutor::next` 仍先进入 `JoinPhase::BuildRight`，读完右输入后进入 `Output`，再逐行读左输入。文件里没有按行数改构建侧的分支。`normalize_signed_zero_key` 仍把等于零的浮点收成 `Value::Float(0.0)`，`build_hash_key_left` 与 `build_hash_key_right` 返回前都调用它。`integer_pk_base_scan_join` 仍在两侧都是基表扫描、且每条等值键都是写出的整数主键时发出 `PhysicalPlan::SortMergeJoin`。`tests/hash_join_build_side_test.rs` 只剩三条测试，没有 `hash_join_probes_larger_side`；右表更大时断言按左行分组。`small_right_large_left_join_succeeds_under_small_limit` 仍期望 `(3, 100)`。`users` 与 `people` 的目录行长度仍是 `g1_tables_row_len + 10`，尾段以 `0x01, 0x00` 结束。`scale_join` 的函数体未改，新基准加在它后面。

Minor：`tasks.md` 的需求矩阵仍把 T3 的 First Test 写成 `hash_join_probes_larger_side`。该测试已按撤回条件删除，现存的顺序证明是 `hash_join_builds_right_when_right_is_larger`。这是计划在时间门结果出来之前写下的矩阵，不是 Act 偏离，不阻塞本 Iteration。本审查不改 `tasks.md`。

Issue 候选：None。

**Evidence**

沿用本 Cycle Act Response，未重跑 criterion，未重跑全量，未重跑定向测试。`src/executor/join.rs`、`tests/hash_join_build_side_test.rs`、`benches/scale_bench.rs` 的修改时间都早于本 Cycle 文件。`tests/query_memory_test.rs` 与 `tests/join_test.rs` 相对 HEAD 无差异。声明与当前代码一致。

时间门：基线 `time:` 中间值 284.79 ms，对比 323.46 ms，90% 线 256.311 ms。对比高于该线，接线撤回。

撤回后沿用：

- `cargo test --test hash_join_build_side_test` 退出码 0，3 passed，0 failed
- `cargo test --test query_memory_test small_right_large_left_join_succeeds_under_small_limit` 退出码 0，1 passed
- `cargo test --test query_memory_test join_right_side_fails_under_small_limit` 退出码 0，1 passed
- `cargo test --test join_test` 退出码 0，9 passed。`tests/join_test.rs` 里有 9 个测试函数

全量沿用 Act Response：N = 150，B = 13，S11 = 6，S12 = 7，每批退出码 0、failed 0，第 00 批库测试 425 passed。本次 `LC_ALL=C ls tests/*.rs | sort` 仍是 150 个文件。第 1 个是 `aggregate_extension_test`，第 11 个是 `cache_perf_test`，第 66 个是 `join_test`，第 67 个是 `key_type_conformance_test`，第 78 个是 `network_protocol_test`，第 139 个是 `view_test`，第 150 个是 `write_type_conformance_test`，与 Act 写的批次边界一致。Persisted Evidence 为 none，没有 Evidence 目录。

代码位置：`src/executor/join.rs` 的 `JoinPhase::BuildRight`（12–16 行）、`next` 的构建阶段（168–193 行）、`normalize_signed_zero_key`（139–148 行）。`src/parser/planner/query.rs` 的排序合并分支（891–897 行）和 `integer_pk_base_scan_join`（3399–3416 行）。`tests/hash_join_build_side_test.rs` 52–79 行。`tests/query_memory_test.rs` 214–223 行。`tests/legacy_compat_test.rs` 的 `users`（442–455 行）和 `people_row`（1091–1099 行）。

**Follow-up Decision**

撤回分支的 Acceptance 已满足，无需当前 Cycle 修复。Iteration Plan 保持不变。不创建后继 Cycle。`tasks.md` 的 Iteration Plan 停在 Iteration 004，没有下一项，因此不展开下一 Iteration。

**Next Cycle**

None

**Next Iteration**

None
