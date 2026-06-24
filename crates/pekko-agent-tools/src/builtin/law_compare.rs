//! 국가법령정보센터(law.go.kr) OPEN API 신구법비교 조회 도구.
//!
//! 법령명을 받아 ① 신구법비교 목록 조회로 신구법 일련번호(MST)를 찾고,
//! ② 신구법비교 본문 조회로 구·신 조문을 가져와 변경된 조문만 추려
//! 프론트엔드 `OldNewComparison` 구조(snake_case)와 동일한 JSON으로 반환한다.

use async_trait::async_trait;
use pekko_agent_core::{Tool, ToolContext, ToolDefinition, ToolError, ToolOutput};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use tracing::{info, warn};

const LIST_URL: &str = "http://www.law.go.kr/DRF/lawSearch.do";
const SERVICE_URL: &str = "http://www.law.go.kr/DRF/lawService.do";

/// 국가법령정보센터 신구법비교 조회 도구.
pub struct LawCompareTool {
    client: reqwest::Client,
    oc: String,
}

#[derive(Debug, Deserialize)]
struct LawCompareInput {
    law_name: String,
}

impl LawCompareTool {
    /// `oc`: 국가법령정보 OPEN API 인증키(이메일 아이디 부분).
    pub fn new(oc: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self { client, oc }
    }

    /// 신구법비교 목록을 조회해 가장 적합한 항목의 MST(신구법일련번호)와 법령명을 반환.
    async fn find_mst(&self, law_name: &str) -> Result<(String, String), String> {
        let body = self
            .client
            .get(LIST_URL)
            .query(&[
                ("OC", self.oc.as_str()),
                ("target", "oldAndNew"),
                ("type", "JSON"),
                ("query", law_name),
                ("display", "20"),
                ("sort", "ddes"),
            ])
            .send()
            .await
            .map_err(|e| format!("법령 목록 조회 요청 실패: {e}"))?
            .text()
            .await
            .map_err(|e| format!("법령 목록 응답 읽기 실패: {e}"))?;

        let root: Value = serde_json::from_str(&body)
            .map_err(|e| format!("법령 목록 JSON 파싱 실패: {e}"))?;
        let search = &root["OldAndNewLawSearch"];
        let items = as_vec(&search["oldAndNew"]);
        if items.is_empty() {
            return Err(format!("'{law_name}'에 대한 신구법비교 정보를 찾을 수 없습니다."));
        }

        // 선택 우선순위: 법령명 정확 일치 > 법령구분명이 "법률" > 첫 번째.
        let exact = items
            .iter()
            .find(|it| str_field(it, "신구법명").as_deref() == Some(law_name.trim()));
        let statute = items
            .iter()
            .find(|it| str_field(it, "법령구분명").as_deref() == Some("법률"));
        let chosen = exact.or(statute).unwrap_or(&items[0]);

        let mst = str_field(chosen, "신구법일련번호")
            .ok_or_else(|| "신구법일련번호(MST)를 찾을 수 없습니다.".to_string())?;
        let name = str_field(chosen, "신구법명").unwrap_or_else(|| law_name.to_string());
        Ok((mst, name))
    }

    /// MST로 신구법비교 본문을 조회해 `OldAndNewService` 객체를 반환.
    async fn fetch_detail(&self, mst: &str) -> Result<Value, String> {
        let body = self
            .client
            .get(SERVICE_URL)
            .query(&[
                ("OC", self.oc.as_str()),
                ("target", "oldAndNew"),
                ("type", "JSON"),
                ("MST", mst),
            ])
            .send()
            .await
            .map_err(|e| format!("신구법 본문 조회 요청 실패: {e}"))?
            .text()
            .await
            .map_err(|e| format!("신구법 본문 응답 읽기 실패: {e}"))?;

        let root: Value = serde_json::from_str(&body)
            .map_err(|e| format!("신구법 본문 JSON 파싱 실패: {e}"))?;
        Ok(root["OldAndNewService"].clone())
    }
}

#[async_trait]
impl Tool for LawCompareTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "law_oldnew_comparison".to_string(),
            description: "국가법령정보센터(law.go.kr) 신구법비교 조회. 법령명(예: 산업안전보건법)을 받아 \
                          최신 개정의 신·구 조문 비교(변경된 조문)를 반환한다. 법령 개정 내용, 최신 법령, \
                          신구법 비교, 조문 변경을 묻는 질문에 사용하라."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "law_name": {
                        "type": "string",
                        "description": "조회할 법령명. 예: 산업안전보건법, 산업안전보건법 시행령, 중대재해처벌법"
                    }
                },
                "required": ["law_name"],
                "additionalProperties": false
            }),
            required_permissions: vec!["ehs.regulations.read".to_string()],
            timeout_ms: 15000,
            idempotent: true,
        }
    }

    async fn execute(
        &self,
        input: Value,
        _ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let params: LawCompareInput = serde_json::from_value(input)
            .map_err(|e| ToolError::ValidationFailed(format!("파라미터 오류: {e}")))?;
        let law_name = params.law_name.trim().to_string();
        if law_name.is_empty() {
            return Ok(ToolOutput::error("법령명(law_name)이 비어 있습니다."));
        }

        info!(law_name = %law_name, "Fetching old/new law comparison");

        let (mst, resolved_name) = match self.find_mst(&law_name).await {
            Ok(v) => v,
            Err(e) => {
                warn!(law_name = %law_name, error = %e, "law list lookup failed");
                return Ok(ToolOutput::error(e));
            }
        };

        let service = match self.fetch_detail(&mst).await {
            Ok(v) => v,
            Err(e) => {
                warn!(mst = %mst, error = %e, "law detail fetch failed");
                return Ok(ToolOutput::error(e));
            }
        };

        let comparison = build_comparison(&service, &resolved_name);
        let changed = comparison["article_comparisons"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0);
        info!(law_name = %resolved_name, mst = %mst, changed_articles = changed, "Built comparison");

        if changed == 0 {
            return Ok(ToolOutput::error(format!(
                "'{resolved_name}'의 신구법비교에서 변경된 조문을 찾지 못했습니다."
            )));
        }

        Ok(ToolOutput::success(comparison))
    }

    fn validate_input(&self, input: &Value) -> Result<(), ToolError> {
        match input.get("law_name").and_then(|v| v.as_str()) {
            Some(s) if !s.trim().is_empty() => Ok(()),
            _ => Err(ToolError::ValidationFailed("law_name은 필수입니다.".to_string())),
        }
    }
}

/// `OldAndNewService` JSON을 프론트엔드 `OldNewComparison` 형태로 변환한다.
fn build_comparison(service: &Value, fallback_name: &str) -> Value {
    let old_info = &service["구조문_기본정보"];
    let new_info = &service["신조문_기본정보"];

    let law_name = str_field(new_info, "법령명")
        .or_else(|| str_field(old_info, "법령명"))
        .unwrap_or_else(|| fallback_name.to_string());

    // no -> content 매핑 구성.
    let old_map = content_map(&service["구조문목록"]);
    let new_map = content_map(&service["신조문목록"]);

    // 두 목록의 no 합집합을 숫자 순으로 정렬.
    let mut nos: Vec<i64> = old_map
        .keys()
        .chain(new_map.keys())
        .filter_map(|k| k.parse::<i64>().ok())
        .collect();
    nos.sort_unstable();
    nos.dedup();

    let mut cur_no: Option<String> = None;
    let mut cur_title: Option<String> = None;
    let mut articles: Vec<Value> = Vec::new();

    for no in nos {
        let key = no.to_string();
        let old_raw = old_map.get(&key).map(String::as_str).unwrap_or("");
        let new_raw = new_map.get(&key).map(String::as_str).unwrap_or("");

        let old_clean = strip_tags(old_raw);
        let new_clean = strip_tags(new_raw);

        // 조문 헤더(제N조[의M](제목))가 보이면 변경 여부와 무관하게 현재 조문 컨텍스트를 갱신.
        if let Some((art_no, title)) = parse_article_header(&new_clean)
            .or_else(|| parse_article_header(&old_clean))
        {
            cur_no = Some(art_no);
            cur_title = title;
        }

        // law.go.kr은 실제 변경된 부분을 <P>...</P>로 감싼다. 변경 마커가 없으면
        // "(현행과 같음)/(생 략)" 컨텍스트 행이므로 건너뛴다.
        let changed = old_raw.contains("<P>") || new_raw.contains("<P>");
        if !changed {
            continue;
        }

        let change_type = if is_marker(&old_clean, "신설") {
            "신설"
        } else if is_marker(&new_clean, "삭제") {
            "삭제"
        } else {
            "개정"
        };

        articles.push(json!({
            "article_no": cur_no,
            "title": cur_title,
            "change_type": change_type,
            "old_text": old_clean,
            "new_text": new_clean,
        }));
    }

    json!({
        "source_mode": "law_oldnew",
        "law_name": law_name,
        "old_law": law_info(old_info),
        "new_law": law_info(new_info),
        "article_comparisons": articles,
    })
}

/// 기본정보 객체를 `OldNewLawInfo` 형태로 변환(날짜는 YYYY-MM-DD로 정규화).
fn law_info(info: &Value) -> Value {
    json!({
        "law_name": str_field(info, "법령명"),
        "promulgation_date": str_field(info, "공포일자").map(|d| fmt_date(&d)),
        "promulgation_number": str_field(info, "공포번호"),
        "enforcement_date": str_field(info, "시행일자").map(|d| fmt_date(&d)),
        "amendment_type": str_field(info, "제개정구분명"),
    })
}

/// `{"조문": [...]}` 또는 `{"조문": {...}}`에서 no -> content 매핑을 만든다.
fn content_map(list: &Value) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for item in as_vec(&list["조문"]) {
        let no = str_field(&item, "no");
        let content = str_field(&item, "content");
        if let (Some(no), Some(content)) = (no, content) {
            map.insert(no, content);
        }
    }
    map
}

/// JSON 값을 배열로 강제 변환(단일 객체면 1원소 배열, 그 외 빈 배열).
fn as_vec(value: &Value) -> Vec<Value> {
    match value {
        Value::Array(a) => a.clone(),
        Value::Null => Vec::new(),
        other => vec![other.clone()],
    }
}

/// 문자열 필드를 안전하게 추출(숫자로 와도 문자열화).
fn str_field(value: &Value, key: &str) -> Option<String> {
    match &value[key] {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// 본문 HTML 마크업(`<P>`, `<BR>` 등)을 제거한다. `<신  설>`처럼 한글로 시작하는
/// 꺾쇠 표기는 본문 내용이므로 보존한다(ASCII 영문/슬래시로 시작하는 태그만 제거).
fn strip_tags(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<' {
            // 다음 글자가 ASCII 영문이거나 '/'이면 HTML 태그로 보고 '>'까지 제거.
            if let Some(&next) = chars.get(i + 1) {
                if next.is_ascii_alphabetic() || next == '/' {
                    if let Some(close) = chars[i..].iter().position(|&c| c == '>') {
                        i += close + 1;
                        continue;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out.trim().to_string()
}

/// 공백/꺾쇠를 제거한 텍스트가 주어진 마커("신설"/"삭제")와 같은지 검사.
fn is_marker(clean: &str, marker: &str) -> bool {
    let normalized: String = clean
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '<' && *c != '>')
        .collect();
    normalized == marker
}

/// "제N조[의M](제목) ..." 형태의 머리말에서 조문번호와 제목을 추출.
fn parse_article_header(text: &str) -> Option<(String, Option<String>)> {
    let chars: Vec<char> = text.trim_start().chars().collect();
    if chars.first() != Some(&'제') {
        return None;
    }
    let mut i = 1;
    let num_start = i;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    if i == num_start || chars.get(i) != Some(&'조') {
        return None;
    }
    i += 1;
    // 선택적 "의<숫자>"
    if chars.get(i) == Some(&'의') {
        let mut j = i + 1;
        let ds = j;
        while j < chars.len() && chars[j].is_ascii_digit() {
            j += 1;
        }
        if j > ds {
            i = j;
        }
    }
    let article_no: String = chars[..i].iter().collect();

    // 바로 뒤 괄호가 있으면 제목으로 추출(중첩 괄호 대응).
    let mut title = None;
    if chars.get(i) == Some(&'(') {
        let mut depth = 0i32;
        let mut end = None;
        for (idx, &c) in chars.iter().enumerate().skip(i) {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
                if depth == 0 {
                    end = Some(idx);
                    break;
                }
            }
        }
        if let Some(e) = end {
            let t: String = chars[i + 1..e].iter().collect();
            if !t.trim().is_empty() {
                title = Some(t.trim().to_string());
            }
        }
    }
    Some((article_no, title))
}

/// "20260601" -> "2026-06-01". 형식이 다르면 원본 유지.
fn fmt_date(value: &str) -> String {
    if value.len() == 8 && value.chars().all(|c| c.is_ascii_digit()) {
        format!("{}-{}-{}", &value[0..4], &value[4..6], &value[6..8])
    } else {
        value.to_string()
    }
}
