# Iteration 000 / Cycle 000：编码正确性与真实页评估

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks：T1、T2、T3。
- Acceptance gaps：None。
- Repair items：None。
- Inherited scope：R1 编码评估；R2–R6 在本轮只约束候选正确性、兼容边界和后继决策，不实施字符串 SQL。
- Excluded scope：生产 Key/B-Tree/文件头/catalog/DDL/DML/Planner 修改；字符串 PK/UNIQUE；组合索引/范围路由；全局状态、Issue/Runbook、其他暂存和归档文件。

**Objective**

形成三种布局的可复现原型、正确性测试和真实页数/缓存/读写吞吐比较，区分能测得的收益与尚未验证的实际树成本。交付实际测量与建议后停止，等待独立 Review 和用户选择生产路线。本轮不切换任何产品格式。

**Investigation Facts**

- Starting point：现工作区含此前已交付但未提交的修改，不能覆盖或清理。当前字符串 CREATE INDEX 被拒绝；仅 INT to_key 可达。用户在 2026-10-27 确认「按草案继续调查与规划」，允许本次计划调查，尚未批准详细执行计划。
- Current-State Evidence：当次执行 `cargo test --offline --lib storage::page_format::key::tests -- --test-threads=1`，6 passed/0 failed；`cargo test --offline --test secondary_index_test -- --test-threads=1`，27 passed/0 failed。测试覆盖旧 Key 序列化、INT 回填/查询/写入/回滚/重开/恢复/投影/删除。候选尚不存在，不能复用这些结果宣称候选正确；旧测试在覆盖未变化时可作为本轮既有回归结论沿用。
- 关键约束：M02 索引与数据分层；M03 固定 Key 是当前生产事实；M04 页槽；M09/M13 async 不跨 await 持阻塞锁；M18 恢复不信任未 checkpoint 索引树；M19 全量分批且本轮不消耗预算。无需沿引用回读正文才能执行下列任务。

Code and Critical Path：

| 文件/符号 | 当次读取确认的事实与本轮用途 |
|---|---|
| `src/storage/page_format/key.rs::Key` | Key::new 对 >32 字节 assert；serialize 固定 32 字节，Eq/Ord 比较 full_data；deserialize 去尾零推断 len。基线使用现有 Key/full_data，不能拿裸字符串尾零推断当完整字符串编码。 |
| `src/executor/value.rs::Value::to_key/equals` | to_key 只接受 Int，写 i64 BE；String 相等按完整 Rust String。旧 BE 在符号边界不数值保序；本轮刻画旧行为，不修它。 |
| `src/storage/page_format/slotted_page.rs` | 页头 16、槽 6、页 4096 字节；SlottedPage::init/add_slot、PageWriteError::PageFull；SlottedPageRef::get_slot_data_checked。用这些实际原语装页、装不下则换页；单记录本身装不下是候选容量边界，不能丢弃后继续报告完整负载。 |
| `src/storage/page_format/row_id.rs::RowId` | 实际 6 字节行位置。Leaf 记录尾部为 RowId；内部记录尾部是 u32 child；采用项目序列化方法，不按估算的固定页容量生成页数。 |
| `src/storage/page.rs::Page` | Page::new，4096 字节页数据，FileStorage/MemoryStorage 可实际读写。 |
| `src/storage/buffer_pool.rs::BufferPool` | new(capacity,Arc<dyn AsyncStorage>)；get_page 命中返回缓存，miss 最终 read_page；没有 hit/miss API。容量设 16/64；已知请求序列每次恰调用一次 get_page，计数 storage reads 后可得 hits。 |
| `src/storage/async_storage.rs::AsyncStorage` | read/write/allocate/free/sync/page_count；有释放生命周期默认方法。测试计数委托器转发给 FileStorage，只累计 read_page；不改变 trait、不加生产开关，不引入新缓存。 |
| `src/storage/file_storage.rs`、`memory_storage.rs` | 使用真实 FileStorage 作文件 IO 与缓存测量、MemoryStorage 可作装页回读测试；所有现场独立 TempDir，不调用 Database 初始化去增加无关 catalog/WAL。 |
| `src/storage/btree/node.rs`、`integrity.rs::check_tree_page` | 生产叶条目 38/内部 36，页类型 0x01/0x02，消费端有界读取与固定偏移。候选原型用测试的 0x03 SlottedPage 容器，不把新字节写成生产索引页。生产适配不是本轮任务。 |
| `src/storage/btree/index_manager.rs::insert` | blocking 闭包内结构+root+反向映射原子单元，write_gate 串行化。原型没有这个实际树成本，不可用页级数据宣称生产采用已通过。 |
| `src/parser/planner/ddl_dml.rs::build_create_index`、`query.rs::extract_secondary_index_equality` | 只放行 INT，裸列 Eq 字面量取 to_key；参数/AND/OR 等不走该路由。本轮保持。 |
| `create_index.rs::backfill_tree`、四 DML、`wal/recovery.rs::extract_index_keys`、transaction manager | 二级与 PK/唯一当前共用 to_key；回填和恢复都装配 overflow/default；未来只能二级分支加新派生，不能全局打开 String。当前不改这些路径。 |
| `Cargo.toml`、`benches/scale_bench.rs` | 已有 Criterion 0.5、tempfile、rand、async-trait 与 Tokio，无需新增依赖；增加一个 harness=false 的 bench 注册。现 scale_bench 只跑既有 SQL 不可用作候选布局吞吐的替代。 |

**Implementation Guidance**

共享原型放在新 `tests/support/index_key_encoding.rs`，由新集成测试 `tests/index_key_encoding_evaluation_test.rs` 和 `benches/index_key_encoding_bench.rs` 通过模块路径引用。判断在 cargo test/Criterion 内，报告在 Act Response；不新增 shell/Python 判定器、命令包装、JSON 协议、manifest、哈希或运行身份。

候选定义：

1. fixed32：Int 使用现 i64::to_be_bytes 与 Key::new/full_data；字符串先用下述完整字符串转义，只有编码 <=32 字节时放入现 Key。真实定长记录为 32 键字节加 RowId(6)/child(4)，仍计每槽6字节；不声称当前 SQL 已支持字符串。
2. ordered-variable：Int 0 编为80；正幅值最短 BE 长 L，头80+L，后接幅值；负数以 unsigned_abs 得幅值，头80-L，后接幅值逐字节取反。支持 i64::MIN，不做有符号 abs。String 非零 UTF-8 原样、00 转00 FF、末尾00 00。记录 `[编码长:u16 LE][完整键][RowId/child]`；不要预定生产 Key 用 Vec。
3. prefix-restart：用 ordered-variable 的完整编码按类型内逻辑顺序排序；每16个条目重启、页首也重启，格式 `[公共前缀长:u16 LE][后缀长:u16 LE][后缀][RowId/child]`。重启项前缀长为0；解码仅依赖同页前一条完整键，随机读最多向前15项，不能依赖前页。编码字节本身不要求压缩记录的字典序等于逻辑顺序，必须以重建后的完整键作比较。

固定页内头、长度字段、槽、RowId/child、重启成本全部进入实际占用；禁止只用键长除4096估算页数。超过容量或u16长度的输入明确返回候选不可容纳；不截断、不跳过后宣称全部回读。

**Task Definitions**

### T1：编码正确性与旧布局刻画

- Requirement/Scenario：R1/S01，R6；旧布局边界不是本轮缺陷修复。
- Depends on：None。
- Targets：新 `tests/index_key_encoding_evaluation_test.rs`、新 `tests/support/index_key_encoding.rs`。
- Current behavior：候选原型不存在；产品固定 Key 且字符串不入索引。
- Required behavior：增加同域编码/解码、顺序与相等性测试；分别刻画旧 fixed32 物理顺序与候选数值顺序，保留所有输入字节。覆盖 MIN、MAX、-256、-255、-1、0、1、255、256；字符串空、前缀、UTF-8、00/尾00、共享前缀及随机输入；prefix 重启边界15/16/17，解压完整键再比较。旧布局 unsupported 字符串不冒充成功记录。
- Preserve：产品所有代码、现存测试和规范主库；Int-only 的 to_key。
- Forbidden：直接改生产 Key/Value；对 String 取固定前缀当完整键；测试回避零字节；为旧行为引入范围排序承诺。
- First test：先添加 `integer_candidate_preserves_signed_order`、`string_candidate_preserves_full_value_and_lexical_order`、`prefix_restart_reconstructs_complete_keys`，候选入口可以只有最小未实现骨架；执行 `cargo test --offline --test index_key_encoding_evaluation_test -- --test-threads=1` 观察运行期 RED，不以编译错误当 RED，再实现原型。
- GREEN condition：上述测试及 `fixed32_matches_current_key_bytes` 均通过；同编码回读相等、不同值不碰撞；负数与非负数候选顺序对齐 Rust i64 比较，旧 fixed32 断言只要求旧 BE 数组顺序。
- Verification：同一新目标测试通过、无产品 diff；新功能的RED与GREEN实际结果写 Response。随机样本固定种子38用于可重现，不作为身份条件。
- Stop when：正确性需要修改生产键、定义跨类型排序、缩减已批准完整值语义，或候选算法不能覆盖边界；回 Plan，不悄悄变更方案。

### T2：真实页回读、容量与缓存测量

- Requirement/Scenario：R1/S01/S02、R6/S15 的原型前提；生产损坏页语义不在本任务实现。
- Depends on：T1。
- Targets：上述两个测试文件；仅测试内 AsyncStorage 委托器与页装配，复用项目原语。
- Current behavior：无候选装页负载；BufferPool 没有计数接口。
- Required behavior：Leaf/内部记录分别用真实 SlottedPage 装页、完整回读，统计实际分配页数与记录/槽占用。逐条保留 (逻辑值,RowId/child) 并验证多重集合完整。场景包括恰好页满、下一条换页、单条超页、长度字段截短及prefix失去重启条件；不得切越界片段。错误处理只针对被测候选格式，不新增产品判定层。
- Preserve：生产 trait、缓存淘汰与IO、索引页类型；FileStorage 真实读写，独立 TempDir；不把 OS 页缓存当 BufferPool 命中。
- Forbidden：复制 B-Tree、实现新缓存、给生产 BufferPool 加监控字段、伪造数据库文件版本或把原型页挂到 catalog。
- First test：增加 `candidate_page_roundtrip_checks_lengths_and_capacity`、`page_metrics_cover_equal_inputs`、`cache_counts_observe_real_storage_reads`，按首次运行失败的行为逐项实现。新记录装页/读入骨架存在但运行期断言不满足时观察 RED，再完成实现；不能把只打印数据的用例当正确性测试。
- GREEN condition：每种可容纳布局回读的值与位置等于输入；超容量完整拒绝、不计为完整负载；requests>=reads，hits=requests-reads；全冷首次读正确、同一缓存页后续命中不再触发 read_page。计数器只转发真实存储，读阶段没有其他隐含读入口。
- Verification：`cargo test --offline --test index_key_encoding_evaluation_test -- --test-threads=1 --nocapture`，新测试全过，输出布局/负载/叶与内部页数/字节数/请求/读取次数；无新专用 CLI 或日志文件。与T1覆盖未变化的GREEN可沿用，只运行新增或受改动影响的用例，最后一次完整新目标GREEN即可。
- Stop when：容量外输入需隐式截断、跳过、增设溢出树或修改真实存储行为；转 Plan。底层读不等于该阶段miss时修正测量边界，不得补身份归属机制。

### T3：Criterion 测量与推荐

- Requirement/Scenario：R1/S02；R2/S05 的后续物理可容纳域输入。
- Depends on：T2。
- Targets：新 `benches/index_key_encoding_bench.rs`、Cargo.toml 的单一新 bench 注册；复用已测试原型；Act Response 的测量摘要。
- Current behavior：没有可对比候选的 benchmark；现有 scale_bench 仅能测产品 SQL。
- Required behavior：真实测量编码/解码、装页、写页+sync、从BufferPool读取并解码的吞吐；记录页数与实际计数，给出筛选结论及原型未覆盖的B-Tree成本。不得擅自选择生产格式或进入后续Task。
- Preserve：现有依赖、bench 与产品构建行为；输入生成和临时目录创建不进入计时；错误立即失败，结果被black_box消费或完整遍历，不能被编译器消掉。
- Forbidden：忽略IO错误、只用内存字节数声称文件收益、删掉退化负载、固定格式伪测 >32 字节键、反复重测达标、新增框架外评判程序。
- First test：T1/T2 的候选正确性GREEN为合法先行证据；先运行 `cargo bench --offline --bench index_key_encoding_bench --no-run` 保证注册与调用正确，再执行一次下列指定测量。新增bench只增加测量，不修改已测试算法；若变更算法，先运行受影响测试观察行为再回到测量。
- GREEN condition：同域各项有真实值或区间；长串明确可容纳域与无fixed32分母；清楚给出达标/未达标/不确定，包含缓存口径和限制；没有生产行为变更。候选没有收益也是合格评估结果，不能把候选必须获胜作为任务完成条件。
- Verification：指定Criterion命令及T2直接输出；最终检查 `git diff --check -- Cargo.toml tests/support/index_key_encoding.rs tests/index_key_encoding_evaluation_test.rs benches/index_key_encoding_bench.rs`，并检查新增文件与目标范围。保留既有Key6/二级索引27的未失效结果，不重新跑它们以增强信心。
- Stop when：发现量化需要第二套树实现、新公共计数API或更改性能门；不扩范围。测量失败先诊断，三次同类失败遵守公共规则；不以全量测试代替该测量。

**Invariants**

- 首轮只有测试原型、新benchmark与Cargo注册；产品运行输出、持久字节、索引状态与现有约束不变。
- 完整值比较；零字节不得与填充混同；NULL不参与候选键序。整数和字符串分开测，不建立跨类型排序。
- 输入负载不按环境或revision识别，不创建hash、manifest或冻结条件；固定种子只复现值。
- 所有断言在项目测试内，计数来自真实调用；Criterion只报告其直接统计，不另包验收工具。

**Non-goals**

生产格式、全局Key重构、字符串SQL、迁移、索引溢出页、并发算法、范围扫描、组合索引、长期诊断框架。没有需要人工验证的场景；不存在应由用户执行的测试项。

**Acceptance**

- A1 / R1 / T1：三布局的编码正确性或旧布局刻画全部通过，完整字符串无碰撞，候选整数保序；RED→GREEN可核对。
- A2 / R1,R6 / T2：实际SlottedPage装页与回读完整，容量边界不截断，无遗漏；页数来自真实页，缓存读数可直接解释。
- A3 / R1 / T3：同域页数、实际读请求/底层读取、页级读写吞吐均有实测；有符号/宽值/长串代价披露，原型与生产树结论分开。
- A4 / R2–R6边界 / T3：Response写明支持域、筛选建议、实际树采用门和后续决策；生产行为无变化。不能把页级初筛通过写成整体MS38完成。

**Verification**

负载定义，每组16384项：

- 正short-int=1..16384；另组对称负正整数；wide-int使用种子38的全域i64并注入MIN/MAX等边界。
- common-string：8/24字节UTF-8样本，含公共前缀与重复值，完整转义<=32字节，三布局同域；零字节与尾零另有正确性样本。
- long-string：32/64/256字节，候选之间比较，不以fixed32不支持作为零吞吐去计算虚假提升；4096字节与约1MiB仅做容量/完整值边界断言。
- 叶与内部分开输出指标。按每种布局的正确顺序装页，保存逻辑输入位置至页/槽映射用于相同逻辑访问；映射是原型数据索引，不参与运行归属，也不冒充实际树下降。
- 读测量前，FileStorage页已写好并sync，旧pool释放，新pool容量16或64。一次顺序遍历全部逻辑行，每行一次get_page并解码；另阶段取同一4096逻辑行热点集合重复4遍，每次一次get_page并解码。各阶段重建pool、重置仅该阶段的底层计数，冷热状态明确。requests来自相同访问序列，reads来自真实read_page；hits=requests-reads。
- prefix随机读从本页最近重启项重建到目标，必须计入最多15次解码；不得直接取预先解码的键来报告读吞吐。
- Criterion每配置一次采样，sample-size10、warm-up1秒、measurement1秒；计时不含输入生成/TempDir。冷读批次以未缓存的新pool开始，pool构造放在iter_batched未计时setup；热点批次预热口径单列，不能将冷读和热读混合汇总。写批次从未使用的新文件开始，计时含编码、装页、write_page和sync，构造文件和pool在setup；不得只测已刷盘的空工作。

命令：

```sh
cargo test --offline --test index_key_encoding_evaluation_test -- --test-threads=1 --nocapture
cargo bench --offline --bench index_key_encoding_bench --no-run
cargo bench --offline --bench index_key_encoding_bench -- --sample-size 10 --warm-up-time 1 --measurement-time 1
```

首命令在对应任务的实际RED/GREEN阶段执行；通过后只在覆盖有改动时定向重跑。Criterion采用同一次输出的时间点估计与区间，不另跑求更好结果。拟议筛选：short-int/common-string同域页数降低>=10%，读写吞吐各不退化>5%，缓存命中率不下降>2个百分点；wide-int单列代价。区间跨越退化边界为不确定，交Plan/用户；不自行重测至通过。prefix相对variable的空间/随机读代价也单列，不用均值遮蔽。

当前没有新格式实际树测量，初筛即使通过也不能授权采用。最终生产采用必须经下一轮实际树原语读写/分裂/合并/恢复与同口径采用门。未达初筛、正确性不成立或无法可靠比较时，推荐保留现布局；这不阻塞本轮诚实交付。

**RTM**

| Requirement | Task | First Test | Status |
|---|---|---|---|
| R1 编码评估 | T1–T3 | integer_candidate_preserves_signed_order / page_metrics_cover_equal_inputs | Covered |
| R6 本轮原型判界前提 | T2 | candidate_page_roundtrip_checks_lengths_and_capacity | Covered |

R2–R5产品行为及R6生产布局安全分配在后续路线，当前不实施、不宣称通过。

**Gate 2 Readiness**

PASS：当前轮实现事实、任务、RTM、验证和停止边界已闭合；2026-10-27 用户回复「批准」，详细计划审计通过。没有跳过项。后续物理方案需要实测后选择，已设为下一轮进入边界，不由本轮 Act 处理。

**Persisted Evidence**

- Mode: none

Act Response足以容纳命令、RED/GREEN、页数与缓存读数、Criterion关键区间和限制。原型可重跑，没有不可再生环境或需要完整日志保留的决定性结构；不创建evidence目录。

**Risks and Notes**

- 本轮不会交付字符串SQL；整项工作仍需后继路线，不能在无后续可执行任务时宣称变更完成。
- 原型没有树下降、分裂、WAL、catalog/root与回滚开销；如没有后续实际树采用门，不得更换生产格式。
- 具体局部模块拆分和名称不构成接口要求；可做等价安排，但测试和bench必须共用同一候选实现，所有测试入口命令仍可运行。
- 既有未提交和暂存差异归原现场，本轮不得清理。没有新Issue或Runbook授权。

## Act Response

- Status: reported
- Tasks: T1 完成，T2 完成，T3 完成。T4–T8 未开始。
- Persisted Evidence: None required
- Experience Candidates: None
- Issue 候选: None
- Blocker: 无

初筛未通过。建议保留现有 32 字节固定键，不切换生产格式。页数和写吞吐达到拟议门槛，但热读没有全部守住 5% 吞吐线。页级结果也不能代替实际索引树的分裂、合并和恢复。

### 改动

新增原型，产品代码未改。

- `tests/support/index_key_encoding.rs`：三种布局的编码、真实 `SlottedPage` 装页、`CountingStorage` 读计数和文件读测量。页类型使用 `0x03`。
- `tests/index_key_encoding_evaluation_test.rs`：正确性、容量、回读和缓存计数。
- `benches/index_key_encoding_bench.rs`：编码、装页、写加 sync、冷读、热读和随机读。
- `Cargo.toml`：注册 `index_key_encoding_bench`，`harness = false`。

候选行为：整数有序变长编码按幅值最短大端，0 为 `0x80`，负数用 `unsigned_abs` 后逐字节取反，覆盖 `i64::MIN`。字符串非零字节原样，`00` 编成 `00 FF`，结尾 `00 00`。`fixed32` 只有转义后不超过 32 字节才放入现有 `Key`，否则返回不支持。prefix 每页每 16 条重启，页首也重启；顺序扫描沿同页上一条完整键解码，随机读从本页最近重启项重建。单条超过页容量或编码长度超过 `u16` 时整批拒绝。

### 与计划的偏差

- 记录下来的 Criterion 数字来自修正顺序扫描之后的那一次完整运行。修正前有一次运行把 prefix 的每一条顺序访问都从重启项重建；计划把这次回退留给随机读。那次输出不作为结论。随机读仍然从重启项重建，最多回看 15 条。
- 吞吐计时使用容量 64 的新缓冲池。容量 16 和 64 的请求数、底层读数在测试输出里。
- 宽整数的内部页，以及 32 字节和 256 字节长串的写、读，没有逐项进入 Criterion。它们的页数和缓存计数在测试里。长串写读计时用 64 字节叶记录代表。
- `fixed32` 的全零 32 字节同时是整数 0 和空字符串。两边分域测试，同域没有碰撞。这是该候选的边界，不是本轮要修的产品缺陷。

### 自检

Gate 3：三任务的目标行为有测试或实测。产品键、B-Tree、文件头、catalog、DDL、DML、Planner 未改。容量外输入返回错误，不截断。缓存计数只包在测试用的存储委托上。没有新增产品警告。遗留的表面点是共享原型文件开了 `dead_code` 允许，因为测试和 benchmark 各包含一份、用到的函数不同。

Gate 4：下面的命令是最终代码上的结果。Gate 5 未触发。

### 验证

T1 先行失败是运行期失败，不是编译失败。三个测试里，整数顺序断言左右值不同，字符串编码返回 `BadEncoding`，prefix 装页返回 `BadEncoding`。随后 `fixed32_matches_current_key_bytes` 在取整数键时返回 `BadEncoding`。

T2 先行失败：`cache_counts_observe_real_storage_reads` 冷读底层次数为 0，页数为 3。容量测试当时已通过。补上 `read_page` 计数后该测试通过。

最终：

```text
cargo test --offline --test index_key_encoding_evaluation_test -- --test-threads=1 --nocapture
```

7 passed，0 failed，16.03s。测试输出没有新增警告。

```text
cargo bench --offline --bench index_key_encoding_bench --no-run
```

注册和调用编译通过。

```text
cargo bench --offline --bench index_key_encoding_bench -- --sample-size 10 --warm-up-time 1 --measurement-time 1
```

修正后的运行约 280s，退出码 0。下面的区间是这次输出里的绝对时间 `[低, 估计, 高]`。

```text
git diff --check -- Cargo.toml tests/support/index_key_encoding.rs tests/index_key_encoding_evaluation_test.rs benches/index_key_encoding_bench.rs
```

退出码 0。既有 `storage::page_format::key::tests` 的 6 通过和 `secondary_index_test` 的 27 通过未重跑。

### 页数和缓存

每组 16384 条。顺序扫描按逻辑顺序每行一次 `get_page`。热点是排序后的前 4096 条：先单独走一遍作为预热，再在同一缓冲池上重复 4 遍。顺序扫描不回到已离开的页，所以两次容量的顺序底层读数都等于页数。

叶页数：

| 负载 | fixed32 | ordered-variable | prefix-restart |
|---|---:|---:|---:|
| short-int | 179 | 69 | 69 |
| symmetric-int | 179 | 69 | 69 |
| wide-int | 179 | 93 | 93 |
| common-string | 179 | 129 | 75 |
| long-string-32 | 不支持 | 193 | 86 |
| long-string-64 | 不支持 | 322 | 94 |
| long-string-256 | 不支持 | 1093 | 147 |

内部页数：short-int 169/61/61，common-string 169/121/67，wide-int 169/85/85，long-32 不支持/187/77，long-64 不支持/316/86，long-256 不支持/1093/138。

容量 64、叶、顺序扫描的命中：short-int 为 16205/16384、16315/16384、16315/16384。common-string 为 16205/16384、16255/16384、16309/16384。容量 64 的热点重复在这些负载上底层读数为 0。容量 16 的 common-string 叶热点重复读数为 180、164、80。候选的命中率没有比 fixed32 低过 2 个百分点。

4096 字节字符串：fixed32 不支持；另外两种 `RecordDoesNotFit`。约 1MiB 字符串：`LengthExceedsU16`。

### 吞吐

写计时含编码、装页、分配、`write_page` 和 `sync`。冷读每次用新的空池。热读的预热不在计时内。门槛按吞吐：候选时间的整个区间都必须不慢于 fixed32 的 1/0.95。区间整体慢于这条线算未达标，跨过算不确定。

short-int 叶，写：fixed32 `[27.906, 28.503, 29.029] ms`，variable `[13.101, 13.412, 13.731] ms`，prefix `[12.865, 13.373, 13.905] ms`。冷读：`[11.367, 11.757, 12.162]`、`[5.6842, 5.9088, 6.2942]`、`[5.6201, 5.8227, 6.0181] ms`。热读：`[1.5910, 1.6067, 1.6198]`、`[1.7771, 1.7831, 1.7936]`、`[2.1027, 2.1133, 2.1324] ms`。

common-string 叶，写：`[28.569, 29.363, 29.969]`、`[22.446, 22.993, 23.575]`、`[16.712, 17.558, 18.423] ms`。冷读：`[12.400, 12.951, 13.481]`、`[9.7470, 10.098, 10.315]`、`[7.4354, 7.8938, 8.2831] ms`。热读：`[3.1548, 3.1588, 3.1622]`、`[3.0577, 3.0613, 3.0649]`、`[3.5059, 3.5121, 3.5201] ms`。

内部页的写和冷读同样快于 fixed32。热读里，short-int variable 估计 1.671 ms，对照 fixed32 的 1.520 ms；prefix 为 1.973 ms。common-string 内部热读：fixed32 3.112 ms，variable 2.984 ms，prefix 3.440 ms。

对照结论：两种候选的页数降幅都超过 10%，写和冷读都快于 fixed32，命中率没有下降超过 2 个百分点。ordered-variable 的 short-int 热读区间整体大约只有 fixed32 的 91%，未达标；common-string 热读达标。prefix 的四组热读区间整体都低于 95%，未达标。因此初筛未通过。

宽整数叶页是 179/93/93。写估计 28.664/20.876/22.325 ms，冷读 11.063/6.750/6.905 ms，热读 1.616/1.884/2.273 ms。编码估计 278/665/663 µs。它比 short-int 更费编码时间，热读也慢于 fixed32；这项不并进上面的达标判断。

prefix 相对 variable，不取平均：short-int 叶都是 69 页，prefix 的负载字节略多。common-string 叶是 75 对 129，long-64 是 94 对 322，long-256 是 147 对 1093。热读在缓存能放下热点时，prefix 比 variable 慢大约 15% 到 18%。随机读在容量 64、预热过所触页之后：short-int 估计 19.836 ms 对 18.435 ms，区间重叠；common-string 估计 97.696 ms 对 35.573 ms，prefix 更快，因为页更少。64 字节长串没有 fixed32 分母：写 45.875 ms 对 19.747 ms，冷读 21.838 ms 对 10.217 ms，热读 23.706 ms 对 4.616 ms。热读差距主要来自 variable 的热点页放不进 64 个槽。

### 建议和未覆盖成本

保留现有固定 32 字节键。两种候选都没有同时守住页数、读写吞吐和命中率。原型没有树下降、分裂、合并、WAL、catalog 根、回滚和恢复。即便某一维后来达标，也不能据此更换生产格式。后续若继续评估，要用实际索引树原语按同一口径重测。

未解决问题：生产路线仍待选择。本轮没有阻塞项。

## Plan Review

- Review Result: accepted

**Findings**

阻塞 Acceptance：没有。

非阻塞：

1. 偏差原因是 Act 偏离。`tasks.md` 仍写「实施：not-started」，T1–T3 仍为 `pending`。这是计划交接时的状态。Act Response 写 T1–T3 完成。测试、benchmark 和本 Cycle 记录与完成声明一致。本审查不把 `tasks.md` 改成完成。
2. 偏差原因是 Act 偏离，Act Response 已经写明。Criterion 的装页、写和读没有覆盖对称整数，也没有覆盖宽整数内部页，以及 32 字节和 256 字节长串的写和读。长串写读用 64 字节叶记录代表。编码和解码测量包含这些负载。页数和缓存计数来自 `page_metrics_cover_equal_inputs`。初筛只使用 short-int 和 common-string，这两项的叶页和内部页都在 benchmark 里。
3. 偏差原因是 Act 偏离，出在报告粒度。内部页热读只给了点估计。叶页 short-int 和 common-string 的完整区间已经使热读未达标。保留固定键不依赖内部页区间。
4. 偏差原因是 Act 偏离。`benches/index_key_encoding_bench.rs` 的 `scan` 在随机访问抽到同页相邻槽时，用上一条已解码的完整键做单步解码。计划要求随机读从本页最近重启项重建。不相邻的非重启槽仍经 `read_slot` 从重启项重建。这组数字不进入通过或失败门槛。
5. 偏差原因是 Act 偏离，自检已经记录。`tests/support/index_key_encoding.rs` 整文件允许 `dead_code`。测试里的顺序读仍每次调用 `read_slot`，prefix 会从重启项重建。缓存计数只看 `read_page`，不受解码次数影响。吞吐使用 benchmark 里的相邻键路径。

**Evidence**

沿用 Act Response 的测试和 Criterion 结论，没有重跑。本审查开始时，Cycle 与四个产物两次内容校验一致，没有其他进程占用。结论之后这些文件没有再改。没有出现需要重跑的情形。

独立核对 `git diff --cached`：`Cargo.toml` 只增加 `index_key_encoding_bench`，且 `harness = false`。另外三个文件是新增。对这四个路径执行 `git diff --check --cached`，退出码 0。`src/` 不在这四个路径的 diff 里。

顺序命中与页数闭合：16384 − 179 = 16205，16384 − 69 = 16315。这与 Act Response 里容量 64 的顺序命中一致。

short-int 叶热读里，fixed32 估计 1.6067 ms，1.6067 / 0.95 = 1.6913 ms。ordered-variable 的下界是 1.7771 ms，整段区间慢于这条线。common-string 叶热读里，ordered-variable 快于 fixed32。prefix 的两组叶热读都慢于 95% 吞吐。页数降幅 (179 − 69) / 179、(179 − 129) / 179 和 (179 − 75) / 179 都大于 10%。初筛未通过，与「保留现有 32 字节固定键」一致。候选没有胜过门槛，不构成 Acceptance 失败。

编码和装页与计划一致。`encode_ordered_int` 和 `encode_ordered_str` 按幅值最短大端和 `00` / 结尾转义实现。装页使用 `SlottedPage::add_slot`。`CountingStorage::read_page` 计数。超长分别返回 `Unsupported`、`RecordDoesNotFit` 或 `LengthExceedsU16`。原型页类型 `0x03` 是计划指定的容器。内部 child 和 `RowId` 使用小端，与 `node.rs`、`RowId::serialize` 一致。

**Follow-up Decision**

A1 到 A4 有对应代码，测量结论来自未失效的 Act Response。初筛失败是计划允许的评估结果。不改筛选门槛，也不为了通过而重测。没有需要新执行依据的修复，也不把有限修复留在当前 Cycle。Iteration Plan 保持不变。001 仍只写在 `tasks.md` 里，本审查不展开。展开 001 仍要先有布局选择。

**Next Cycle**

None

**Next Iteration**

None
