/// 경량 마크다운 → HTML 변환기
/// 지원: 테이블, **볼드**, `코드`, ```코드블록```, 줄바꿈
pub fn markdown_to_html(input: &str) -> String {
    let mut result = String::new();
    let lines: Vec<&str> = input.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // ── 코드 블록 (```) ──
        if line.trim_start().starts_with("```") {
            let lang = line.trim_start().trim_start_matches('`').trim();
            result.push_str("<pre><code");
            if !lang.is_empty() {
                result.push_str(&format!(" class=\"lang-{}\"", escape_html(lang)));
            }
            result.push('>');
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                result.push_str(&escape_html(lines[i]));
                result.push('\n');
                i += 1;
            }
            result.push_str("</code></pre>");
            i += 1;
            continue;
        }

        // ── 마크다운 테이블 ──
        if line.contains('|') && is_table_row(line) {
            // 테이블 시작 감지
            let mut table_rows: Vec<&str> = vec![line];
            i += 1;

            // 구분선 행 (|---|---| 등) 탐지
            let has_separator = i < lines.len() && is_separator_row(lines[i]);
            if has_separator {
                i += 1; // 구분선 건너뛰기
            }

            // 나머지 테이블 행 수집
            while i < lines.len() && lines[i].contains('|') && is_table_row(lines[i]) {
                table_rows.push(lines[i]);
                i += 1;
            }

            // 테이블 렌더링
            result.push_str("<div class=\"table-wrapper\"><table>");

            for (ri, row) in table_rows.iter().enumerate() {
                let cells = parse_table_cells(row);
                let tag = if ri == 0 && (has_separator || table_rows.len() > 1) {
                    "th"
                } else {
                    "td"
                };

                if ri == 0 {
                    result.push_str("<thead><tr>");
                } else if ri == 1 {
                    // 첫 번째 tbody 행: tbody 열기
                    result.push_str("<tbody><tr>");
                } else {
                    result.push_str("<tr>");
                }

                for cell in &cells {
                    result.push_str(&format!("<{tag}>{}</{tag}>", inline_format(cell.trim())));
                }

                if ri == 0 {
                    result.push_str("</tr></thead>");
                } else {
                    result.push_str("</tr>");
                }
            }

            if table_rows.len() > 1 {
                result.push_str("</tbody>");
            }
            result.push_str("</table></div>");
            continue;
        }

        // ── 일반 텍스트 행 ──
        if line.trim().is_empty() {
            result.push_str("<br/>");
        } else {
            result.push_str("<p>");
            result.push_str(&inline_format(line));
            result.push_str("</p>");
        }
        i += 1;
    }

    sanitize_html(&result)
}

/// 테이블 행인지 판단
fn is_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    // | 로 시작하거나 내부에 | 가 있으면 테이블 행으로 간주
    trimmed.starts_with('|') || trimmed.matches('|').count() >= 2
}

/// 구분선 행인지 판단 (|---|---| 형태)
fn is_separator_row(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return false;
    }
    // |, -, :, 공백만으로 구성되면 구분선
    trimmed
        .chars()
        .all(|c| c == '|' || c == '-' || c == ':' || c == ' ')
}

/// 테이블 행에서 셀 추출
fn parse_table_cells(row: &str) -> Vec<String> {
    let trimmed = row.trim();
    let inner = if trimmed.starts_with('|') && trimmed.ends_with('|') {
        &trimmed[1..trimmed.len() - 1]
    } else if trimmed.starts_with('|') {
        &trimmed[1..]
    } else if trimmed.ends_with('|') {
        &trimmed[..trimmed.len() - 1]
    } else {
        trimmed
    };
    inner.split('|').map(|s| s.to_string()).collect()
}

/// 인라인 포맷팅: **볼드**, `코드`
fn inline_format(text: &str) -> String {
    let escaped = escape_html(text).replace("&lt;br/&gt;", "<br/>");

    // **볼드** 처리
    let mut result = String::new();
    let parts: Vec<&str> = escaped.split("**").collect();
    if parts.len() > 1 {
        for (i, part) in parts.iter().enumerate() {
            if i % 2 == 1 {
                result.push_str(&format!("<strong>{part}</strong>"));
            } else {
                result.push_str(part);
            }
        }
    } else {
        result = escaped;
    }

    // `인라인 코드`
    let mut final_result = String::new();
    let code_parts: Vec<&str> = result.split('`').collect();
    if code_parts.len() > 1 {
        for (i, part) in code_parts.iter().enumerate() {
            if i % 2 == 1 {
                final_result.push_str(&format!("<code>{part}</code>"));
            } else {
                final_result.push_str(part);
            }
        }
    } else {
        final_result = result;
    }

    final_result
}

/// HTML 이스케이프
fn sanitize_html(input: &str) -> String {
    let mut result = String::new();
    let mut cursor = 0;

    while let Some(relative_start) = input[cursor..].find('<') {
        let start = cursor + relative_start;
        result.push_str(&input[cursor..start]);

        let Some(relative_end) = input[start..].find('>') else {
            result.push_str(&escape_html(&input[start..]));
            return result;
        };

        let end = start + relative_end;
        let tag = &input[start + 1..end];
        result.push_str(&sanitize_tag(tag));
        cursor = end + 1;
    }

    result.push_str(&input[cursor..]);
    result
}

fn sanitize_tag(tag: &str) -> String {
    let trimmed = tag.trim();
    if trimmed.is_empty() || trimmed.starts_with('!') || trimmed.starts_with('?') {
        return escape_html(&format!("<{tag}>"));
    }

    if let Some(closing) = trimmed.strip_prefix('/') {
        let name = tag_name(closing);
        if is_allowed_tag(name) && name != "br" {
            return format!("</{name}>");
        }
        return escape_html(&format!("<{tag}>"));
    }

    let name = tag_name(trimmed);
    if !is_allowed_tag(name) {
        return escape_html(&format!("<{tag}>"));
    }

    if name == "br" {
        return "<br/>".to_string();
    }

    let attrs = sanitize_attrs(name, trimmed.get(name.len()..).unwrap_or_default());
    format!("<{name}{attrs}>")
}

fn tag_name(tag: &str) -> &str {
    tag.trim_start()
        .split(|ch: char| ch.is_whitespace() || ch == '/' || ch == '>')
        .next()
        .unwrap_or_default()
}

fn is_allowed_tag(name: &str) -> bool {
    matches!(
        name,
        "div"
            | "table"
            | "thead"
            | "tbody"
            | "tr"
            | "th"
            | "td"
            | "pre"
            | "code"
            | "p"
            | "br"
            | "strong"
    )
}

fn sanitize_attrs(tag_name: &str, attrs: &str) -> String {
    if !matches!(tag_name, "div" | "code") {
        return String::new();
    }

    let Some(class_value) = extract_class_attr(attrs) else {
        return String::new();
    };

    let class_value = class_value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ' '))
        .collect::<String>();
    if class_value.is_empty() {
        String::new()
    } else {
        format!(" class=\"{}\"", escape_html(&class_value))
    }
}

fn extract_class_attr(attrs: &str) -> Option<String> {
    let mut rest = attrs.trim();
    while !rest.is_empty() {
        let lower = rest.to_ascii_lowercase();
        if lower.starts_with("class=") {
            let value = rest.get("class=".len()..)?.trim_start();
            if let Some(value) = value.strip_prefix('"') {
                return value.split('"').next().map(str::to_string);
            }
            if let Some(value) = value.strip_prefix('\'') {
                return value.split('\'').next().map(str::to_string);
            }
            return value.split_whitespace().next().map(str::to_string);
        }

        let Some(next_space) = rest.find(char::is_whitespace) else {
            return None;
        };
        rest = rest[next_space..].trim_start();
    }
    None
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_escapes_raw_html() {
        let html = markdown_to_html("<script>alert(1)</script><img src=x onerror=alert(1)>");

        assert!(!html.contains("<script"));
        assert!(!html.contains("<img"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("&lt;img src=x onerror=alert(1)&gt;"));
    }

    #[test]
    fn sanitizer_removes_unsafe_tags_and_attributes() {
        let html = sanitize_html(
            r#"<p onclick="alert(1)">safe</p><script>alert(1)</script><code class="lang-rust x" onclick="x">ok</code><a href="javascript:alert(1)">bad</a>"#,
        );

        assert_eq!(
            html,
            r#"<p>safe</p>&lt;script&gt;alert(1)&lt;/script&gt;<code class="lang-rust x">ok</code>&lt;a href=&quot;javascript:alert(1)&quot;&gt;bad&lt;/a&gt;"#
        );
    }

    #[test]
    fn markdown_preserves_allowed_table_markup() {
        let html = markdown_to_html("| A | B |\n|---|---|\n| **x** | `y` |");

        assert!(html.contains("<div class=\"table-wrapper\"><table>"));
        assert!(html.contains("<strong>x</strong>"));
        assert!(html.contains("<code>y</code>"));
    }
}
