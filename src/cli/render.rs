//! CLI 输出渲染 —— table/json/csv/tsv 四格式纯函数（无 IO、无 env、无时钟）

use serde_json::Value;

/// 输出格式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputKind {
    Table,
    Json,
    Csv,
    Tsv,
}

/// 查询结果载荷：行集（列名由调用方提供）或 DML 影响行数
pub enum QueryPayload {
    Rows(Vec<Vec<Value>>),
    Affected(u64),
}

/// 按 format 渲染为输出文本（不含末尾换行）。
///
/// NULL 渲染为空串；Bool 在 table/csv/tsv 渲染为 true/false，JSON 保持布尔；
/// JSON 形状为 `{"columns":[...],"rows":[[...]]}`，DML 为 `{"affected_rows":N}`。
pub fn render(kind: OutputKind, columns: &[String], payload: &QueryPayload) -> String {
    match payload {
        QueryPayload::Affected(count) => match kind {
            OutputKind::Json => format!(r#"{{"affected_rows":{}}}"#, count),
            // table/csv/tsv：单列 affected_rows + 数值行，与行集输出同构
            _ => render_rows(
                kind,
                &["affected_rows".to_string()],
                &[vec![Value::from(*count)]],
            ),
        },
        QueryPayload::Rows(rows) => render_rows(kind, columns, rows),
    }
}

fn render_rows(kind: OutputKind, columns: &[String], rows: &[Vec<Value>]) -> String {
    match kind {
        OutputKind::Json => serde_json::json!({
            "columns": columns,
            "rows": rows,
        })
        .to_string(),
        OutputKind::Table => render_table(columns, rows),
        OutputKind::Csv => render_delimited(columns, rows, ',', csv_escape),
        OutputKind::Tsv => render_delimited(columns, rows, '\t', tsv_escape),
    }
}

/// table 单元格显示宽度上限（字符数）：超限单元格截断为前 61 字符 + `...`
const CELL_WIDTH_CAP: usize = 64;

fn truncate_cell(cell: &str) -> String {
    if cell.chars().count() <= CELL_WIDTH_CAP {
        return cell.to_string();
    }
    let mut out: String = cell.chars().take(CELL_WIDTH_CAP - 3).collect();
    out.push_str("...");
    out
}

fn value_to_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn render_table(columns: &[String], rows: &[Vec<Value>]) -> String {
    let mut lines: Vec<Vec<String>> = vec![columns.to_vec()];
    lines.extend(
        rows.iter()
            .map(|r| r.iter().map(value_to_text).collect::<Vec<_>>()),
    );

    // 单元格显示宽度封顶（字符数，含表头）：超限截断为前 61 字符 + "..."，
    // 消除 ≥65536 字符单元格触发的 std::fmt 动态宽度 panic（ISS15）。
    for line in &mut lines {
        for cell in line {
            *cell = truncate_cell(cell);
        }
    }

    let n_cols = columns.len();
    let widths: Vec<usize> = (0..n_cols)
        .map(|i| {
            lines
                .iter()
                .filter_map(|l| l.get(i))
                .map(|c| c.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();

    let fmt_line = |cells: &[String]| -> String {
        (0..n_cols)
            .map(|i| {
                let cell = cells.get(i).cloned().unwrap_or_default();
                format!("{:<w$}", cell, w = widths[i])
            })
            .collect::<Vec<_>>()
            .join(" | ")
    };

    let mut out = Vec::with_capacity(lines.len() + 1);
    out.push(fmt_line(&lines[0]));
    out.push(
        widths
            .iter()
            .map(|w| "-".repeat(*w))
            .collect::<Vec<_>>()
            .join("-+-"),
    );
    for line in &lines[1..] {
        out.push(fmt_line(line));
    }
    out.join("\n")
}

fn render_delimited(
    columns: &[String],
    rows: &[Vec<Value>],
    delim: char,
    escape: fn(&str) -> String,
) -> String {
    let delim_s = delim.to_string();
    let mut lines = Vec::with_capacity(rows.len() + 1);
    lines.push(
        columns
            .iter()
            .map(|c| escape(c))
            .collect::<Vec<_>>()
            .join(&delim_s),
    );
    for row in rows {
        lines.push(
            row.iter()
                .map(|v| escape(&value_to_text(v)))
                .collect::<Vec<_>>()
                .join(&delim_s),
        );
    }
    lines.join("\n")
}

/// RFC 4180：含 `,`/`"`/换行的字段引号包裹、内部引号翻倍
fn csv_escape(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') || field.contains('\r') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// 字段内 `\t`/`\n`/`\r`/`\` 转义为字面序列（反斜杠先翻倍）
fn tsv_escape(field: &str) -> String {
    field
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rows(rows: Vec<Vec<serde_json::Value>>) -> QueryPayload {
        QueryPayload::Rows(rows)
    }

    #[test]
    fn test_table_alignment() {
        let out = render(
            OutputKind::Table,
            &["id".into(), "name".into()],
            &rows(vec![
                vec![json!(1), json!("Alice")],
                vec![json!(22), json!("Bob")],
            ]),
        );
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "id | name ");
        assert_eq!(lines[2], "1  | Alice");
        assert_eq!(lines[3], "22 | Bob  ");
    }

    #[test]
    fn test_table_null_bool() {
        let out = render(
            OutputKind::Table,
            &["a".into(), "b".into()],
            &rows(vec![vec![json!(null), json!(true)]]),
        );
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "a | b   ");
        assert_eq!(lines[2], "  | true");
    }

    #[test]
    fn test_json_shape() {
        let out = render(
            OutputKind::Json,
            &["id".into(), "name".into()],
            &rows(vec![
                vec![json!(1), json!("Alice")],
                vec![json!(null), json!(true)],
            ]),
        );
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed["columns"], json!(["id", "name"]));
        assert_eq!(parsed["rows"], json!([[1, "Alice"], [null, true]]));
    }

    #[test]
    fn test_json_affected() {
        let out = render(OutputKind::Json, &[], &QueryPayload::Affected(3));
        assert_eq!(out, r#"{"affected_rows":3}"#);
    }

    #[test]
    fn test_csv_escaping() {
        let out = render(
            OutputKind::Csv,
            &["name".into(), "note".into()],
            &rows(vec![vec![json!("a\"b,c"), json!("line1\nline2")]]),
        );
        assert_eq!(out, "name,note\n\"a\"\"b,c\",\"line1\nline2\"");
    }

    #[test]
    fn test_csv_null_bool() {
        let out = render(
            OutputKind::Csv,
            &["a".into(), "b".into()],
            &rows(vec![vec![json!(null), json!(false)]]),
        );
        assert_eq!(out, "a,b\n,false");
    }

    #[test]
    fn test_tsv_escaping() {
        let out = render(
            OutputKind::Tsv,
            &["note".into(), "v".into()],
            &rows(vec![vec![json!("x\ty\nz\\w\rq"), json!(7)]]),
        );
        assert_eq!(out, "note\tv\nx\\ty\\nz\\\\w\\rq\t7");
    }

    #[test]
    fn test_affected_text_formats() {
        let table = render(OutputKind::Table, &[], &QueryPayload::Affected(2));
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines[0], "affected_rows");
        assert_eq!(lines[2].trim_end(), "2"); // 列宽 = 表头宽，末列补空格
        assert_eq!(
            render(OutputKind::Csv, &[], &QueryPayload::Affected(2)),
            "affected_rows\n2"
        );
        assert_eq!(
            render(OutputKind::Tsv, &[], &QueryPayload::Affected(2)),
            "affected_rows\n2"
        );
    }

    #[test]
    fn test_table_cell_cap_boundary() {
        // 64 字符原样
        let s64: String = "a".repeat(64);
        let out = render(
            OutputKind::Table,
            &["v".into()],
            &rows(vec![vec![json!(s64)]]),
        );
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[2].trim_end(), "a".repeat(64));

        // 65 字符截断恰 64 以 "..." 结尾（61 内容字符 + 省略号）
        let s65: String = "a".repeat(65);
        let out = render(
            OutputKind::Table,
            &["v".into()],
            &rows(vec![vec![json!(s65)]]),
        );
        let lines: Vec<&str> = out.lines().collect();
        let cell = lines[2].trim_end();
        assert_eq!(cell.chars().count(), 64);
        assert!(cell.ends_with("..."));
        assert_eq!(&cell[..61], &"a".repeat(61));
    }

    #[test]
    fn test_table_cell_cap_huge_no_panic() {
        // 100,000 字符单元格不 panic，截断后以 "..." 结尾（ISS15）
        let huge: String = "x".repeat(100_000);
        let out = render(
            OutputKind::Table,
            &["v".into()],
            &rows(vec![vec![json!(huge)]]),
        );
        let lines: Vec<&str> = out.lines().collect();
        let cell = lines[2].trim_end();
        assert_eq!(cell.chars().count(), 64);
        assert!(cell.ends_with("..."));
    }

    #[test]
    fn test_table_cell_cap_cjk_no_partial_char() {
        // CJK 多字节串按字符截断，不产生部分字符
        let cjk: String = "中".repeat(100);
        let out = render(
            OutputKind::Table,
            &["v".into()],
            &rows(vec![vec![json!(cjk)]]),
        );
        let lines: Vec<&str> = out.lines().collect();
        let cell = lines[2].trim_end();
        assert_eq!(cell.chars().count(), 64);
        assert!(cell.ends_with("..."));
        assert!(cell[..cell.len() - 3].chars().all(|c| c == '中'));
    }

    #[test]
    fn test_table_header_cap_same_rule() {
        // 表头 >64 字符同规则封顶
        let long_header: String = "h".repeat(70);
        let out = render(
            OutputKind::Table,
            &[long_header],
            &rows(vec![vec![json!(1)]]),
        );
        let lines: Vec<&str> = out.lines().collect();
        let header = lines[0].trim_end();
        assert_eq!(header.chars().count(), 64);
        assert!(header.ends_with("..."));
    }

    #[test]
    fn test_delimited_and_json_full_fidelity_with_huge_value() {
        // csv/tsv/json 对超大值保真（机器消费契约，截断仅限 table）
        let huge: String = "x".repeat(100_000);
        let payload = rows(vec![vec![json!(huge.clone())]]);
        let cols = &["v".to_string()];
        assert_eq!(
            render(OutputKind::Csv, cols, &payload),
            format!("v\n{}", huge)
        );
        assert_eq!(
            render(OutputKind::Tsv, cols, &payload),
            format!("v\n{}", huge)
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&render(OutputKind::Json, cols, &payload)).unwrap();
        assert_eq!(parsed["rows"][0][0], json!(huge));
    }
}
