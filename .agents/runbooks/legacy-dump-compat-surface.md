# 旧 dump 的兼容面配方（恢复路径 / 范围外拒绝与迁移路径 / 可执行命令写法）

- Status: active
- Last validated: 2026-10-25
- Environment: Linux x86_64（WSL2 6.18.40.1）、cargo 1.99.1 / rustc 1.99.1、RTsql `target/debug/rtsql` CLI、RTsql `tests/legacy_compat_test.rs` 集成测试
- Source: change `2026-10-24-ms36-legacy-format-and-dump-compat` Iteration 001 / Cycle 000 Act Response 与 Plan Review（`openspec/changes/archive/2026-10-24-ms36-legacy-format-and-dump-compat/iterations/001-contract-and-baseline/000-initial.md`，2026-10-25，迁移路径表与 M1 记录失真）；change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 007 / Cycle 000 Act Response 与 Plan Review（`.../iterations/007-legacy-compat-evidence/000-initial.md`，2026-10-25，T19 命令列更正与 IC-3）

## 适用范围

- 需要在**真实 CLI** 上验证「旧 dump → 新库 restore」是否成立，以及**范围外**输入的拒绝形态与**迁移路径**是否真的可执行时使用。
- 需要在文档 / 记录里写下 `rtsql` 命令并希望**可被字面执行**时使用（见「操作步骤」第 5 步的路径形式规则）。
- 不适用：旧代际**库文件**样本的产出（用 `.agents/runbooks/legacy-sample-production.md`）；分批全量回归（用 `.agents/runbooks/m19-batched-full-regression.md`）。

## 前置条件

- 当前构建的 CLI：`target/debug/rtsql`（旧构建产物的用法见 `.agents/runbooks/legacy-sample-production.md`）。
- 旧 dump 样本：`tests/fixtures/legacy/legacy_dump.sql`（**只读复制**到工作目录使用，不要在仓库内原地改写）。
- **恢复是逐条 auto-commit**（`Executor::execute` 的隐式提交）⇒ 后段语句被拒时**前段已提交**（既有合同，见 `.agents/issues/ISS32-multi-table-dump-partial-restore.md`）。

## 操作步骤

### 1. 范围内：旧 dump 恢复到新库

```bash
cd <工作目录>                     # 仓库样本先复制到这里
cp /home/daivy/projects/RTsql/tests/fixtures/legacy/legacy_dump.sql ./legacy_dump.sql
rtsql new ./s1.db                # 路径形式，不是裸名
rtsql restore ./s1.db ./legacy_dump.sql
rtsql ./s1.db "INSERT INTO users VALUES (4, 'dave', 6.0);"
rtsql ./s1.db "SELECT * FROM users;"
rtsql ./s1.db "INSERT INTO audit VALUES (2, 'after-restore');"
rtsql ./s1.db "SELECT COUNT(*) FROM audit;"
```

旧 dump 用的**短类型名**（`INT` / `STRING` / `BOOL` / `FLOAT` / `DATE` / `TIMESTAMP`）与**旧字面量**（`DATE '…'` / `TIMESTAMP '…'` / `TRUE`）当前构建**照常接受**；当前构建的 `dump` 则渲染**长类型名**（`INTEGER` / `TEXT` / `BOOLEAN`）⇒ 两套写法都是合法输入。

### 2. 范围外：三类拒绝与各自迁移路径（实测退出码 3）

| 拒绝原因（错误文本） | 迁移路径 | 复现用的最小编辑 |
|---|---|---|
| `NON-INTEGER PRIMARY KEY` | 去掉 dump 中的 `PRIMARY KEY` 声明后重新 restore | `sed 's/ PRIMARY KEY//'` |
| `UNIQUE (INT columns only)` | ① 去掉 `UNIQUE` 声明后重新 restore；② 或把被约束列改为整数类型**并同步转换该列的行值** | ① `sed 's/ STRING UNIQUE//'`；② `sed -e 's/"email" STRING UNIQUE/"email" INT UNIQUE/' -e "s/VALUES (1, 'a@b.c')/VALUES (1, 42)/"` |
| `NOT NULL constraint violation: column '<列名>'` | ① 回填该行的 `NULL` 后重新 restore；② 或删除该行后重新 restore | ① `sed "s/VALUES (2, NULL)/VALUES (2, 'backfilled')/"`；② `grep -v "VALUES (2, NULL)"` |

**迁移路径 2 的重要区分（实测）**：**只改列类型不足**——`sed 's/"email" STRING UNIQUE/"email" INT UNIQUE/'` 仍以退出码 **3** 结束，stderr 为 `key column 'email' expects INT, got String; statement: INSERT INTO "niuq" VALUES (1, 'a@b.c'); previous statement(s) were committed` ⇒ **改类型必须同步转换该列的行值**。

### 3. 范围外的另外两类（退出码 1）

- `NewerFileVersion`：文件格式版本高于当前支持版本 ⇒ 退出码 **1**。
- `NotADatabase`：不含文件头的文件 ⇒ 退出码 **1**。

### 4. 拒绝的定位要求

范围外的拒绝必须**定位到语句**：错误文本点名**拒绝原因**并给出**失败语句序号**（形态：`statement 2 of 2 failed: …` / `statement 3 of 3 failed: …`）⇒ 记录迁移路径时，序号与原因都要抄进文档。

### 5. 命令列必须写成可直接执行的形式（路径形式）

`src/cli/resolve.rs` 的 `resolve_db_path`：**含 `/`** 的参数按路径直开；**裸名**一律解析到 `$RTSQL_HOME/db/<name>.db`（未设 `RTSQL_HOME` 时为 `$HOME/.rtsql/db/<name>.db`）⇒ **按裸名字面执行会以 `does not exist` 退出 1**。

⇒ 文档 / 记录里的命令一律写**含 `/` 的路径形式**（`./p1.db` 或绝对路径），并在表前加一句说明点明工作目录与 `./` 的相对基准；仓库样本用**绝对路径**引用，工作目录内的输入是样本的**副本**。

## 验证

- 成功判据：范围内路径的每条命令退出码 **0**，且 `restore` 后的行**逐值等价**（旧 dump 的 `users` 3 行 + `audit` 1 行 + `events` 1 行）；恢复出的库**可继续写入**（写入后再查可见）、**可再次 dump**。
- 范围外路径的判据：退出码 **3**，错误文本**同时**含拒绝原因与失败语句序号；按迁移路径编辑后重跑 ⇒ 退出码 **0** 且行等价。
- 2026-10-25 实测记录（在 `/tmp` 用真实 CLI 独立复现，仓库样本只读复制，复现后已清理）：路径 1（去 `PRIMARY KEY`）→ 退出 0，行 `[["k1","x"]]`；路径 2a（去 `UNIQUE`）→ 退出 0，行 `[[1,"a@b.c"]]`；**路径 2b（只改列类型）→ 退出 3**，stderr `key column 'email' expects INT, got String`；路径 2c（改类型 + 转换行值）→ 退出 0，行 `[[1,42]]`；路径 3a（回填 NULL）→ 退出 0；路径 3b（删除该行）→ 退出 0。
- 回归承载：`cargo test --test legacy_compat_test -- --test-threads=1`（旧 dump 恢复用例 `legacy_dump_restores_into_current_db`：`DATE` / `TIMESTAMP` / `BOOL` 逐值断言；`legacy_dump_with_null_in_not_null_is_rejected`：退出码 3 + `NOT NULL constraint violation` + `column 's'` + `statement 3 of 3 failed`）。
- 判据落在**可观察状态**上（进程退出码、stderr 文本、恢复后的行值）；判定层在 `cargo test` 内与 CLI 自身的退出码内，无项目测试框架之外的判定脚本。

## 失败处理

- **命令以退出码 1 结束且提示 `does not exist`（但你确信文件存在）**：多半是**裸名**被解析到 `$RTSQL_HOME/db/<name>.db` ⇒ 改用含 `/` 的路径形式。
- **迁移路径「改了列类型还是退出 3」**：缺的是**行值转换**（实测 `key column 'email' expects INT, got String`）⇒ 按 2c 的两条 `sed` 同时改。
- **`restore` 前段成功、后段失败**：既有合同（逐条 auto-commit）⇒ 前段**已提交**；不要把它当成新的缺陷（见 `.agents/issues/ISS32-multi-table-dump-partial-restore.md`）。
- **旧 dump 恢复成功但 `dump` 的 DDL 与旧 dump 不同**：当前构建渲染**长类型名**（`INTEGER` / `TEXT` / `BOOLEAN`），旧 dump 用短类型名 ⇒ **两套都合法**，不是错误。
- **非整数 `UNIQUE` 的旧表 dump 后无法恢复**：见 `.agents/issues/ISS33-dump-renders-non-integer-unique.md`（`dump` 对非整数 `UNIQUE` 无条件渲染 ⇒ 该类 dump 不可恢复；既有行为 + 已记录的可选改进）。

## 回滚

不适用（验证执行方法，无产品状态变更）。复现用的工作目录与副本在复现后清理；仓库内的样本保持**只读**。

## 证据

- `openspec/changes/archive/2026-10-24-ms36-legacy-format-and-dump-compat/iterations/001-contract-and-baseline/000-initial.md` —— Act Response 的迁移路径表（各路径的编辑命令 / 退出码 / 结果）与 Plan Review 的 M1（记录保真：退出码与结果本身复现全部成立，失配的只是记录里的**命令写法**）。
- `openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/007-legacy-compat-evidence/000-initial.md` —— Act Response 的 T19（命令列改为路径形式 + 工作目录说明；**退出码列与结果列逐条未动**）与 Issue 候选 IC-3；Plan Review 的 F5（更正彻底性与裸名排查）。
- 见证代码与样本：`tests/legacy_compat_test.rs`（旧 dump 恢复与拒绝用例）、`tests/fixtures/legacy/legacy_dump.sql`、`tests/fixtures/legacy/README.md`。
- 行为规格：`openspec/specs/legacy-format-compat/spec.md`（范围外一律显式拒绝 + 迁移路径；`SHALL NOT` 静默地部分恢复后报告成功）。
- 相关配方：`.agents/runbooks/legacy-sample-production.md`（旧代际库样本的产出）、`.agents/runbooks/cli-subprocess-witnessing.md`（CLI 子进程见证）。
