# ms28-cli-concurrency-checkpoint — Proposal

## Why

- **并发 one-shot 硬失败（I077-①，2026-10-01 外部评审 #1）**：one-shot 形态下并发写同一库立即 `DatabaseLocked` exit 4（`WouldBlock` 仅 ~8ms 有界退避，`database-file-lock` R1）——多 agent/多进程「全部接受、串行完成、互不干扰」的语义缺席，硬失败与 `--format json/csv/tsv` 的自动化定位对立。advisory flock 本身已是跨进程串行化原语（调度器已存在），同步阻塞 CLI 下「等锁再执行」对调用者即等效于串行调度，只差可配置的等待预算。
- **显式落盘原语缺席（I079，2026-10-01 外部评审 #3）**：checkpoint 触发面只有 close()（隐式）、WAL 字节水位（自动，MS26）与 lib `Database::checkpoint()`——「已提交数据落稳后再做外部备份/拷贝」类操作无 CLI 入口；I072 备份边界实操序列缺一个干净同步点。
- 用户 2026-10-01 批准 MS28 重排新建（功能带首项；MS22 import 持锁窗口治理以本阶段等待语义为前提），并裁定单 change 交付（本 proposal）。

## What Changes

两任务面、两 Iteration 承载：

- **Iteration 000 — T01 锁等待策略可配置（busy-timeout）**（MODIFIED `database-file-lock` R1 + ADDED `cli-noninteractive-shell` 通道 Requirement）：
  - 锁获取单点 `FileStorage::try_lock_with_backoff`（独占 :107 / 只读共享 :155 两调用点）从固定 ~8ms 窗口改为「可见性窗口下限 + 可配置等待预算」；预算内周期性重试，耗尽后维持 `StorageError::DatabaseLocked` 既有错误面（CLI exit 4）。
  - 三通道：`--busy-timeout <MS>` 全局 flag（对齐 `--key` 的 global+env 先例）+ env `RTSQL_BUSY_TIMEOUT`（显式 flag 优先）+ lib 打开面参数（Phase 2 定稿具体签名，MS17 `open_with_key` 委托壳先例）。
  - 非法值（非数字/负数/空）open 前 exit 2（对齐 `RTSQL_WAL_CHECKPOINT_BYTES` 校验先例）；等待可被信号中断（沿用既有 open 阶段信号面）。
- **Iteration 001 — T02 显式 checkpoint 子命令**（ADDED `cli-noninteractive-shell` Requirement）：
  - `rtsql checkpoint <db>`：`resolve_existing_db`（缺失 exit 1）→ 独占开库（锁冲突经 busy-timeout 等待后 exit 4）→ `Database::checkpoint()` → stdout 一行确认（含 MS18 零工作早退情形的明示）。
  - 拒绝面沿用先例：`--readonly` / `:memory:` 组合 exit 2；加密库经既有全局 `--key` 支持、错钥 exit 5。

## 默认假设（Gate 1 审阅项；可逐项推翻）

1. **默认等待预算 = 5000ms**（无 flag/env/lib 配置时）：one-shot 持锁窗口为毫秒量级（MS18 实测 ~6.5ms 固定开销），5s 足以吸收正常串行排队且脚本超时判断仍可容忍；存量锁冲突测试按显式短预算机械校准（断言语义不变）。备选：0（默认立即拒绝、纯 opt-in，零行为变化）或 30000ms。
2. **`:memory:` × `--busy-timeout`/`RTSQL_BUSY_TIMEOUT` = 接受并忽略**：内存库无文件锁，等待语义无意义但无害；exit 2 拒绝（对齐 `--key`/`--readonly` 先例）会使全局导出 env 的用户无法使用任何 `:memory:` 组合，属通道性误伤。备选：exit 2 拒绝（拒绝矩阵一致性）。
3. **单位与命名**：毫秒（SQLite `busy_timeout` 先例）；flag `--busy-timeout <MS>`、env `RTSQL_BUSY_TIMEOUT`。
4. **timeout = 0 语义**：不额外等待，仅保留内部 ~8ms 可见性窗口下限（close→立即重开假 `DatabaseLocked` 消除的既有正确性机制，非用户等待策略的一部分）——即完全回到当前行为。
5. **作用面 = 全部开库路径**：主命令（读写/只读）、生命周期子命令（new/list/schema/dump/restore/import/stats/sample/profile）、ATTACH 附加库打开、checkpoint——均经 `FileStorage` open 面自动获得等待语义；`delete` 子命令的锁探测（`File::open`+`try_lock` 自有路径）**不引入等待**（破坏性操作保持立即探测立即拒绝），行为不变。
6. **等待预算下限**：内部 ~8ms 可见性窗口吸收始终生效（正确性机制）；用户预算 < 8ms 时仍保证窗口吸收，预算只增不减等待时长。
7. **checkpoint 输出形态**：stdout 一行确认（如含 WAL 截断/落盘结果或零工作早退明示）；不加 `--format` 支持（生命周期管理命令先例：`delete` 逐行报告亦无四态格式）；确切文案 Phase 2 定稿。
8. **迭代序 = 000 → 001**：T01 的通道与等待语义是 T02 锁冲突场景（等待后成功/超时拒绝）的依赖；两者无反向耦合。
9. **文档面**：`rtsql-docs/SKILL.md` 命令行功能目录 + README 双语随两任务同步（新 flag、新子命令、并发使用语义）；docs SKILL.md 当前有未提交修改（2026-10-01 milestone 重排），本 change 在其上继续编辑。

## 范围外（Non-goals）

- import 攒批提交 / I021 批量 INSERT（持锁窗口治理 → MS22 候选池，I077-②）。
- FIFO 票据队列（flock 非 FIFO，不自造公平队列）；`rtsql serve` 复活（I071，串行模型达天花板后的扩容路径）。
- 跨进程多写者；Server 面并发语义。
- 完整性校验命令 / 页校验和（MS27-T02）；`rtsql check`。
- lib 打开面既有入口签名与语义的破坏性变更（新增入口或加性参数，MS17 委托壳先例）。

## Impact

- **受影响代码面**（Phase 2 复核）：`src/storage/file_storage.rs`（`try_lock_with_backoff` 策略化 + open 家族参数穿线）、`src/database.rs`（open 家族 lib 通道）、`src/lib.rs`（导出面）、`src/cli/mod.rs`（`--busy-timeout` 全局 flag + env 校验 + `:memory:` 特判 + Command 枚举 checkpoint 臂 + 锁等待值穿线）、`rtsql-docs/SKILL.md`、`README.md`/`README.zh-CN.md`。
- **受影响 specs**：MODIFIED `database-file-lock`（R1 等待策略可配置）；MODIFIED `cli-noninteractive-shell`（+2 Requirement：busy-timeout 通道 / 生命周期子命令 checkpoint）；MODIFIED `readonly-open-mode`（R1 共享锁冲突「SHALL NOT 等待或重试」改为按 `database-file-lock` R1 预算执行——Phase 2 调查确认直接冲突，必须同步）。语料域数 61 不变。`in-memory-database` R3 无需改动（busy-timeout 不进其拒绝面，`:memory:` 接受并忽略是「无消费点」的自然结果）。
- **测试**：新增锁等待矩阵（两进程/同进程 × 读/写锁 × 等待成功/超时拒绝 × 三通道 × 非法值）+ checkpoint 命令矩阵（正常/零工作/缺库/锁冲突/`--readonly`/`:memory:`/加密）；存量 `database_file_lock_test`/`cli_test` 锁冲突用例按默认预算机械校准；全量零回归（change 创建时基线 1632 tests / 0 failures / 2 ignored，SNAPSHOT 2026-10-01）。

## 验证边界

- **T01**：锁矩阵 e2e（持有者跨窗口时第二方等待后成功；持有者不释放时超时 `DatabaseLocked` exit 4；独占与只读共享两侧；timeout=0 立即拒绝）+ 通道矩阵（flag/env 优先级、非法值 exit 2、lib 通道直达）+ close→重开既有回归 + 全量零回归。修复前以「等待语义缺失」用例见证 RED。
- **T02**：checkpoint 命令矩阵 e2e（写入后 checkpoint → WAL 有界/位点推进可见、零工作早退 exit 0、缺库 exit 1、锁冲突经等待后 exit 4、`--readonly`/`:memory:` exit 2、`--key` 成功/错钥 exit 5）+ 全量零回归。
- **文档**：SKILL.md 新增 flag/子命令与实现一致（人工核对 + 抽样自动执行）。
