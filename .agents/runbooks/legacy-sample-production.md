# G2 旧格式样本的产出配方（真实来自旧构建，仓库之外生成）

- Status: active
- Last validated: 2026-10-25
- Environment: Linux x86_64（WSL2 6.18.40.1）、cargo 1.99.1 / rustc 1.99.1、git（`--no-hardlinks --shared` 本地共享克隆）、RTsql `tests/legacy_compat_test.rs` 集成测试
- Source: change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 007 / Cycle 000 Act Response 与 Plan Review（`openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/007-legacy-compat-evidence/000-initial.md`，2026-10-25，T16 / T17）；G1 的同形流程见 `tests/fixtures/legacy/README.md`

## 适用范围

- 需要为 `legacy-format-compat` 域的某个**代际**（G1 / G2 / …）产出一个**真实来自旧构建**的库文件样本，并固定入库为**固定输入**时使用。
- 需要复现或更新旧代际样本的**派生量**（表清单 / 行数 / 目录行长度构成 / 恢复行数参考值）时使用。
- 不适用：手工构造 / 伪造样本（**禁止**）；运行时做版本嗅探 / hash / revision 比对（**禁止**）；在仓库内构建旧 revision（**禁止**）；旧 dump 文本的兼容面（用 `.agents/runbooks/legacy-dump-compat-surface.md`）。

## 前置条件

- **目标代际对应的旧构建 revision 必须先独立核实**，不能凭 revision 号推断。G2 的 revision = `6e8dcf551aa53d2a08dc6f20097f1ad5e50fc464`（2026-09-26）。核实手法：读该 revision 的目录行序列化实现，确认其布局**含唯一根段、不含二级索引段与表类型字节**（这正是 G2 的判别特征）。G1 的 revision = `d8165fc634fa650d839811b1d6b0165e204cad01`（2026-09-24）。
- **离线可构建**：`cargo build --offline --bin rtsql` 必须成功。离线依赖不可用 ⇒ 回到 Plan，**不得**改用手工构造的样本。
- **工具链**：rustc / cargo 1.99.1（`--offline`）。
- **隔离克隆必须用 `--no-hardlinks --shared`**：确保**不触碰工作区**的未提交现场（旧 revision 的构建产物与工作区完全分离）。
- **样本规模 ≪ 300 行**（ISS23 约束）。
- **伴生 `.wal` / `.checkpoint` 不入库**（与 G1 的既有做法一致）。

## 操作步骤

1. **隔离克隆 + 构建旧版本**（全部在**仓库之外**执行）：

```bash
git clone --no-hardlinks --shared /home/daivy/projects/RTsql /tmp/rtsql-legacy-g2
cd /tmp/rtsql-legacy-g2 && git checkout 6e8dcf5
cargo build --offline --bin rtsql
OLD=/tmp/rtsql-legacy-g2/target/debug/rtsql
```

2. **用旧 CLI 产出样本**（在仓库之外的暂存目录内）：

```bash
F=/tmp/legacy-fixture-g2
rm -rf $F && mkdir -p $F
$OLD new $F/legacy_g2.db
$OLD $F/legacy_g2.db "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score FLOAT);"
$OLD $F/legacy_g2.db "INSERT INTO people VALUES (1, 'ada', 9.5), (2, 'bob', 7.25), (3, 'cyd', 8.0);"
$OLD $F/legacy_g2.db "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT, tag TEXT DEFAULT 'none');"
$OLD $F/legacy_g2.db "INSERT INTO notes (id, body) VALUES (1, 'first note');"
$OLD $F/legacy_g2.db "CREATE TABLE scratch (id INTEGER PRIMARY KEY, tmp TEXT);"
$OLD $F/legacy_g2.db "INSERT INTO scratch VALUES (1, 'to be dropped');"
$OLD $F/legacy_g2.db "DROP TABLE scratch;"
```

3. **只把样本本体拷入库**（不要拷伴生 `.wal` / `.checkpoint`）：

```bash
cp $F/legacy_g2.db tests/fixtures/legacy/legacy_g2.db
```

4. **更新来源说明**（`tests/fixtures/legacy/README.md`）：追加该代际的来源行与样本内容行 + 「再生步骤」节。必须写明：旧构建 revision、旧构建方式（隔离克隆 + `cargo build --offline --bin rtsql`）、构建产物路径、工具链、**样本的代际判别特征**、样本内容清单、**各样本的预期结论**、再生步骤（与上同形）。
5. **由断言锁定派生量**（在 `tests/legacy_compat_test.rs` 内）：为该代际建立**文件级**的常态化断言（打开 / 查询 / 写入 / 完整性 / 打开时重建 / dump / 恢复），每项走**真实入口**（`Database::open` + 真实 SQL / 真实 CLI `dump`）。派生量（表清单、行数、目录行长度构成等）**在断言常量与来源说明两处同源同步**。
6. **清理临时克隆与构建产物**：删除 `/tmp/rtsql-legacy-g2`（隔离克隆 + `target/`）、`/tmp/legacy-fixture-g2`（样本暂存）等临时目录，并复核已不存在。

## 验证

- 成功判据：`cargo test --test legacy_compat_test -- --test-threads=1` 退出码 0、failed 计数为 0；样本存在且可被当前构建打开；来源说明的「revision / 构建方式 / 代际特征 / 再生步骤」四要素逐条可对上。
- **派生量一致性**：断言常量与来源说明写**同一组数字**；不一致时**先核对样本字节是否被重新生成**，**不得**就地改断言常量迁就样本。
- **代际判别可观测**：样本的代际特征必须在**文件字节层**可观测（目录行布局）⇒ 不可观测 ⇒ 回到 Plan。
- **无身份机制**：入库的样本是**真实字节**；运行时不做版本嗅探 / 不伪造代际 / 不做 hash 或 revision 比对。⇒ 该样本的「来源无法被机械验证」是**设计如此**（公共规则禁止为证据引入身份比对机制），来源的权威记录是**来源说明 + 再生步骤**，不是字节内的标记。
- 判据落在**可观察状态**上（文件存在性、真实入口的语句结果、断言常量与说明的逐项一致）；判定层在 `cargo test` 内。

## 失败处理

- **`cargo build --offline` 失败**：离线依赖不可用 ⇒ 记入 Act Response 并**回到 Plan**（换 revision 或调整方案）；**不得**用手工构造 / 伪造的样本替代。
- **旧 CLI 产出的库无法被当前构建打开**：⇒ 回到 Plan；不要为「能打开」而在当前构建里做代际兼容改动。
- **代际特征在文件字节层无法观测**：⇒ 回到 Plan（该 revision 不是目标代际）。
- **派生量与来源说明不一致**：**先核对样本字节是否被重新生成**（重新生成会改变派生量，例如恢复行数参考值）⇒ 同步更新**两处**；不要只改一处。
- **误在仓库内构建旧 revision**：会污染工作区的未提交现场 ⇒ 清理构建产物并重新走隔离克隆流程。

## 回滚

- 样本是**固定输入**：入库后即被断言锁定。要换样本 ⇒ 重新走本配方第 1–6 步，并**同步**更新断言常量与来源说明两处。
- 旧构建产物与暂存目录在流程结束时**必须清理**；清理前确认样本本体已入库。

## 证据

- `openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/007-legacy-compat-evidence/000-initial.md` —— Act Response 的 T16（样本产出与入库，32832 字节）与 T17（七项能力断言 + 四个辅助函数与四个用例）；Plan Review 对「旧构建的可构建性」「派生量同步」的复核。
- G1 的同形流程与既有产物：`tests/fixtures/legacy/README.md`（G1 的 revision / 构建方式 / 产物路径 / 样本内容清单 / 再生步骤第 1–6 步 / 已知限制）。
- 见证代码：`tests/legacy_compat_test.rs`（G1 9 条 + G2 4 条用例；G2 的 `g2_tables_row_len` / `g2_tables_unique_count` / `columns_type_tag_offset` / `columns_default_segment` 辅助与 `COL_TAG_STRING` / `TUPLE_TAG_STRING` 常量）。
- 行为规格：`openspec/specs/legacy-format-compat/spec.md`（代际定义、能力矩阵与证据索引 E7）。
- 相关配方：`.agents/runbooks/legacy-dump-compat-surface.md`（旧 dump 的兼容面）。
