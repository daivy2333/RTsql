# ISS15 table 渲染列宽未封顶，单元格字符数 ≥ 65536 时 CLI panic（exit 101）

- Status: closed
- Filed: 2026-10-14
- Source: 用户实测评测（2026-10-14 上手操作演示）
- Environment: Linux x86_64、rtsql 0.2.0（`~/.local/bin/rtsql` 安装二进制）、源码 HEAD `fbb5c54`、`rustc 1.90.0`（std 行为复现）

## 缺陷描述

`--format table`（TTY 默认格式）渲染行集时，`render_table` 以各列最大单元格字符数作为列宽，未封顶直接传入 `format!("{:<w$}", cell, w = widths[i])`（`src/cli/render.rs:85`）。Rust `std::fmt` 对动态宽度有 16 位上限：宽度 ≥ 65536 即 panic `Formatting argument out of range`。引擎明确支持无上限大值（overflow page 链），合法单行大值因此把整个 CLI 打崩。

预期：大值在 table 格式下正常输出（截断/省略号）或体面降级。实际：进程 panic，exit 101，stderr 泄漏 `src/cli/render.rs:85` 源码位置。

## 影响

任何单元格字符数 ≥ 65536 的结果集在 table 格式下不可用，受影响面：`SELECT`、`stats`、`sample`、`profile` 全部分析子命令。交互终端默认就是 table 格式，普通用户查询大值必踩。JSON/CSV/TSV 格式不受影响（同数据 exit 0）。数据无损，纯输出面崩溃；panic 信息对终端用户不可操作。

## 事件记录

- 2026-10-14：用户实操评测中 `rtsql --format table stats demo people`（含 100KB 文本值的表）与 `sample`、`profile`、裸 `SELECT v` 连续 panic（exit 101）；二分定位阈值恰为 65536；删除大值行后同命令恢复 exit 0。

## 处置

closed（2026-10-15）——change `2026-10-14-iss15-table-truncation-and-combination-gaps` 单 Iteration accepted 收尾归档：`render_table` 列宽封顶 `CELL_WIDTH_CAP=64`（表头同规则，超限单元格截断为前 61 字符 + `...`，chars 边界安全；widths 按截断后文本计，std::fmt 动态宽度 panic 类关闭；json/csv/tsv 保真逐字节不变）。同批补齐组合面见证：大值×四格式、大值×stats/sample/profile（本缺陷用户实测路径）、import×加密库、`--readonly`×分析子命令拒绝矩阵。规格合并 `cli-noninteractive-shell` 1 MODIFIED + 2 ADDED。全量 1999 tests / 0 failures / 2 ignored（1994 基线净增 5）。

## 证据

本次评测实测（RTSQL_HOME 隔离临时目录 `repro` 库）：

```bash
rtsql new repro
rtsql repro "CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)"
python3 -c "print('x'*100000)" # 生成值，INSERT INTO t VALUES (1, <值>)
rtsql --format table repro "SELECT v FROM t"        # exit 101，panic at src/cli/render.rs:85
rtsql --format table stats repro t                  # 同样 exit 101
rtsql --format json    stats repro t                # exit 0，输出正常
```

std 层根因复现（`rustc 1.90.0` 独立最小程序）：`format!("{:<w$}", cell, w = 65535)` 正常，`w = 65536` panic `Formatting argument out of range`。阈值与库内二分结果一致，根因确认为 std::fmt 动态宽度上限而非存储层。
