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
}
