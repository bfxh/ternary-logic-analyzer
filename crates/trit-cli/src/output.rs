//! CLI 的输出格式：纯文本表格、JSON、CSV、Markdown。

use std::fmt::Write as _;

use trit_core::bits::TruthTable;
use trit_core::value::Ternary;
use trit_eval::eval::decode_assignment;

/// 输出格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    /// 终端友好的等宽表格。
    Table,
    /// JSON（机器可读）。
    Json,
    /// CSV（逗号分隔）。
    Csv,
    /// Markdown 表格。
    Markdown,
}

impl Format {
    /// 按格式渲染真值表。
    pub fn render_truth_table(self, tt: &TruthTable, title: &str) -> String {
        match self {
            Self::Table => render_table(tt, title),
            Self::Markdown => render_markdown(tt, title),
            Self::Csv => render_csv(tt),
            Self::Json => render_json(tt, title),
        }
    }
}

/// 把一行的取值拼成紧凑字符串。
fn row_string(tt: &TruthTable, index: usize) -> String {
    row_values(tt, index).into_iter().collect()
}

/// 一行里每个变量的取值（按列）。
fn row_values(tt: &TruthTable, index: usize) -> Vec<char> {
    let asg = decode_assignment(index as u64, &tt.variables);
    tt.variables
        .iter()
        .map(|v| asg.get(v).unwrap_or(Ternary::Unknown).as_compact_char())
        .collect()
}

/// 渲染 Markdown 表格。
pub fn render_markdown(tt: &TruthTable, title: &str) -> String {
    let mut out = String::new();
    if !title.is_empty() {
        let _ = writeln!(out, "### {title}");
        let _ = writeln!(out);
    }
    let _ = writeln!(out, "| {} | f |", tt.variables.join(" | "));
    // 分隔行：变量列 + 结果列
    let _ = writeln!(out, "|{}|", "---|".repeat(tt.variables.len() + 1));
    for i in 0..tt.values.len() {
        let mut cells: Vec<String> = row_values(tt, i)
            .into_iter()
            .map(|c| c.to_string())
            .collect();
        cells.push(tt.values[i].as_compact_char().to_string());
        let _ = writeln!(out, "| {} |", cells.join(" | "));
    }
    out
}

/// 渲染 CSV。
pub fn render_csv(tt: &TruthTable) -> String {
    let mut out = String::new();
    let mut header: Vec<String> = tt.variables.clone();
    header.push("f".to_string());
    let _ = writeln!(out, "{}", header.join(","));
    for i in 0..tt.values.len() {
        let mut cells: Vec<String> = row_values(tt, i)
            .into_iter()
            .map(|c| c.to_string())
            .collect();
        cells.push(tt.values[i].as_compact_char().to_string());
        let _ = writeln!(out, "{}", cells.join(","));
    }
    out
}

/// 渲染 JSON。
pub fn render_json(tt: &TruthTable, title: &str) -> String {
    let rows: Vec<Vec<String>> = (0..tt.values.len())
        .map(|i| {
            let mut r: Vec<String> = tt
                .variables
                .iter()
                .map(|v| {
                    let asg = decode_assignment(i as u64, &tt.variables);
                    asg.get(v).unwrap_or(Ternary::Unknown).as_char().to_string()
                })
                .collect();
            r.push(tt.values[i].as_char().to_string());
            r
        })
        .collect();
    let vars: Vec<&str> = tt.variables.iter().map(|s| s.as_str()).collect();
    serde_json::json!({
        "title": title,
        "variables": vars,
        "rows": rows,
    })
    .to_string()
}

/// 渲染终端表格。
pub fn render_table(tt: &TruthTable, title: &str) -> String {
    let mut out = String::new();
    if !title.is_empty() {
        let _ = writeln!(out, "{title}");
        let _ = writeln!(out, "{}", "=".repeat(display_width(title)));
    }
    let mut headers: Vec<String> = tt.variables.to_vec();
    headers.push("f".to_string());
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(tt.values.len());
    for i in 0..tt.values.len() {
        let mut r = row_string(tt, i)
            .chars()
            .map(|c| c.to_string())
            .collect::<Vec<_>>();
        r.push(tt.values[i].as_char().to_string());
        rows.push(r);
    }
    let all_rows = std::iter::once(&headers)
        .chain(rows.iter())
        .map(|r| r.iter().map(|s| s.as_str()).collect::<Vec<_>>())
        .collect::<Vec<_>>();

    // 计算每列宽度（按显示宽度，中文按 2 宽计）
    let mut widths: Vec<usize> = vec![0; headers.len() + 1];
    for row in &all_rows {
        for (i, cell) in row.iter().enumerate() {
            let w = display_width(cell);
            if w > widths[i] {
                widths[i] = w;
            }
        }
    }

    let sep = |widths: &[usize]| -> String {
        let mut s = String::new();
        for w in widths {
            let _ = write!(s, "+{}", "-".repeat(w + 2));
        }
        s.push_str("+\n");
        s
    };

    let _ = write!(out, "{}", sep(&widths));
    let _ = write!(out, "{}", format_row(&headers, &widths));
    for row in &rows {
        let _ = write!(out, "{}", format_row(row, &widths));
    }
    let _ = write!(out, "{}", sep(&widths));
    out
}

/// 渲染一行（带对齐与边框）。
fn format_row(cells: &[String], widths: &[usize]) -> String {
    let mut line = String::new();
    for (i, cell) in cells.iter().enumerate() {
        let pad = widths[i].saturating_sub(display_width(cell));
        line.push_str("| ");
        line.push_str(cell);
        line.push_str(&" ".repeat(pad));
        line.push(' ');
    }
    line.push_str("|\n");
    line
}

/// 终端显示宽度：CJK 字符按 2 宽计算。
pub fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| {
            let cp = c as u32;
            if (0x1100..=0x115F).contains(&cp)
                || (0x2E80..=0xA4CF).contains(&cp)
                || (0xAC00..=0xD7A3).contains(&cp)
                || (0xF900..=0xFAFF).contains(&cp)
                || (0xFE30..=0xFE4F).contains(&cp)
                || (0xFF00..=0xFF60).contains(&cp)
                || (0xFFE0..=0xFFE6).contains(&cp)
            {
                2
            } else {
                1
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use trit_core::algebra::Algebra;
    use trit_core::parser::Parser;

    fn sample() -> TruthTable {
        let e = Parser::parse("a & b").unwrap();
        trit_eval::eval::truth_table(&e, Algebra::Kleene)
    }

    #[test]
    fn csv_has_header_and_rows() {
        let s = render_csv(&sample());
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines[0], "a,b,f");
        assert_eq!(lines.len(), 10); // 表头 + 9 行
        assert!(lines[1..].iter().all(|line| line.split(',').count() == 3));
        assert_eq!(lines[1], "0,0,0");
        assert_eq!(lines[2], "1,0,0");
    }

    #[test]
    fn markdown_has_separator() {
        let s = render_markdown(&sample(), "t");
        assert!(s.contains("| a | b | f |"));
        assert!(s.contains("|---|---|---|"));
        assert!(s.contains("| 0 | 0 | 0 |"));
    }

    #[test]
    fn json_parses() {
        let s = render_json(&sample(), "t");
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["variables"], serde_json::json!(["a", "b"]));
        assert_eq!(v["rows"].as_array().unwrap().len(), 9);
    }

    #[test]
    fn display_width_counts_cjk_as_two() {
        assert_eq!(display_width("ab"), 2);
        assert_eq!(display_width("变量"), 4);
    }
}
