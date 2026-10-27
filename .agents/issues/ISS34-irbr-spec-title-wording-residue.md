# `index-read-bounded-read` 域的 Requirement 标题与 Scenario 标题措辞残余（正文已收紧为「未通过页级判据」）

- Status: closed（2026-10-26 fixed → `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure`）
- Filed: 2026-10-25
- Source: Act Response 的 Issue 候选（Iteration 007 的 T20 措辞收敛留下的标题残余）+ Plan Review 的 F14（不阻塞的措辞残余张力，建议 Maintainer 合并时顺带对齐），change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 007 Cycle `000-initial.md`（Review Result `accepted`）
- Environment: 规格文档 `openspec/specs/index-read-bounded-read/spec.md`；登记时工作区 HEAD `0a08b14`（ahead of origin 9）

## 缺陷描述

- 预期：规格的 Requirement / Scenario 标题与其正文、以及与已实现的行为**一致**；标题不得断言正文已收窄掉的行为。
- 实际：`openspec/specs/index-read-bounded-read/spec.md` 的 **Purpose** 与 Requirement **正文**已收敛为「读取到**未通过页级判据**的索引树页时」（这是**刻意**的收窄——叶页短条目**不被**页级判据捕获，因此该域不应断言这类情形），但：
  - Requirement **标题**仍是「损坏的索引页在读 / 写路径上给出明确的错误」；
  - **4 个 Scenario** 的**标题 / GIVEN** 仍以「损坏的索引页上的查询 / 插入 / 删除 / 更新」表述。
- 位置：`openspec/specs/index-read-bounded-read/spec.md`（Requirement 标题行 + 4 个 Scenario 标题 / GIVEN）。
- 根因：Confirmed（文档层——正文收窄与标题措辞未同步）。
- 触发条件：读者按标题理解该域时，会把「短于条目格式的叶条目被静默跳过」（ISS26）读作**已验收行为**；正文则相反。

## 影响

- 当前：规格**标题层**与**正文层**对同一行为的覆盖面表述不一致 ⇒ 引用该域时会把 ISS26 的形态误算为已覆盖。
- 非阻塞性：正文（规范性的 SHALL 条款）已收紧且**正确**；该残余只在**标题 / GIVEN** 层 ⇒ 不改变任何行为要求，但会误导读者与后续 change 的范围判断。
- 与 ISS26 的关系：ISS26（叶页条目长度判据缺口）是**产品侧**待修复的缺陷；本条是**规格侧**的措辞残余。两者不合并，但互为解释：ISS26 的存在正是正文必须收窄的原因。

## 事件记录

- 2026-10-25（缺陷修复 change Iteration 007，T20 措辞收敛）：IRBR 增量只改措辞、不补子句，**Scenario 数与 Requirement 数不变**；收敛后的正文不再把「短叶条目」计入该域的已验收主张。
- 2026-10-25（缺陷修复 change Iteration 007，Plan Review F14）：判为**不阻塞**——「正文已收敛、标题未同步」的措辞残余张力，建议 Maintainer 合并时顺带对齐标题 / 场景措辞（非必做）。
- 2026-10-25（本次登记复核）：该域已被合并进 `openspec/specs/`，且合并后**标题措辞仍存在** ⇒ 残余在当前工作区**仍然可见**，本条据此登记为待处理。

## 处置

- 处置指针：**待建 change 承载**（change 建好后由 Recorder 更新本指针）。
- 承载落点：落在**规格措辞对齐** → 由 `openspec-docs-maintainer` 在规格维护时对齐（**只改标题 / GIVEN 措辞**，不改 SHALL 条款、不改 Scenario 数与 Requirement 数）；或由独立 change 的规格增量承载。
- 建议的对齐措辞（与正文一致）：标题 / Scenario 的「损坏的索引页」改为「**未通过页级判据**的索引树页」，并保留「不 panic、不静默错读」的断言主体。
- 2026-10-26 关闭。原因：Requirement 标题与 4 个 Scenario 标题已对齐为「未通过页级判据的索引树页」。归档变更 Iteration 009 任务 T13 只交接标题（该 Cycle 写明 GIVEN 不改），Plan Review 为 `accepted`。当前主规格 `openspec/specs/index-read-bounded-read/spec.md` 的 Requirement 标题与 4 个 Scenario 标题已是这一措辞。处置指针：fixed → `openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure`。

## 证据

- 2026-10-26 关闭依据：`openspec/changes/archive/2026-10-25-ms37-defect-ledger-closure/iterations/009-corpus-and-full-regression/000-initial.md` 的 Plan Review 为 `accepted`（T13，只交接标题）。当前标题见 `openspec/specs/index-read-bounded-read/spec.md`。
- 调查与结论正文（不在本 Issue 复制）：`openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/007-legacy-compat-evidence/000-initial.md` —— Act Response 的 T20（IRBR 的措辞收敛）与 Plan Review 的 F14。
- 现状核对（登记时）：`openspec/specs/index-read-bounded-read/spec.md`（Purpose 与 Requirement 正文用「未通过页级判据」；Requirement 标题与 4 个 Scenario 标题 / GIVEN 用「损坏的索引页」）。
- 相关 Issue：`/home/daivy/projects/RTsql/.agents/issues/ISS26-index-leaf-entry-length-unchecked.md`（正文收窄的产品侧原因）。
