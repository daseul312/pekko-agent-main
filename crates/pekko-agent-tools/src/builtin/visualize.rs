//! 답변 내 데이터 시각화 도구.
//!
//! LLM이 DB 조회 결과 등에서 집계한 수치를 차트 스펙(JSON)으로 넘기면, 입력을
//! 검증·정규화해 프론트엔드 `ChartData` 구조(snake_case)와 동일한 JSON으로 그대로
//! 반환한다. 외부 호출은 없다 — api-gateway가 이 도구 결과를 캡처해
//! `QueryResponse.chart_data`로 부착하고, 프론트엔드가 Chart.js로 렌더한다.
//!
//! 기존 `law_oldnew_comparison`(law_compare.rs) → `old_new_comparison` 캡처 흐름과
//! 동일한 패턴이다.

use async_trait::async_trait;
use pekko_agent_core::{Tool, ToolContext, ToolDefinition, ToolError, ToolOutput};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 지원하는 차트 종류.
const CHART_TYPES: [&str; 3] = ["bar", "donut", "line"];
/// 인사이트 콜아웃 레벨.
const INSIGHT_LEVELS: [&str; 3] = ["info", "warn", "alert"];

/// 답변 내 차트 렌더링 도구.
pub struct VisualizeTool;

/// 입력 = 출력 스키마. 프론트엔드 `ChartData`(web-chat/src/types.rs)와 1:1 대응.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChartData {
    /// "bar" | "donut" | "line".
    chart_type: String,
    title: String,
    /// x축(bar/line) 또는 세그먼트(donut) 라벨.
    labels: Vec<String>,
    /// 1개 이상의 데이터 계열. donut은 보통 1개, line은 다계열 가능.
    series: Vec<ChartSeries>,
    /// "확인 필요" 콜아웃. 이상 신호가 없으면 생략.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    insight: Option<ChartInsight>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChartSeries {
    name: String,
    values: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChartInsight {
    /// "info" | "warn" | "alert".
    level: String,
    text: String,
}

#[async_trait]
impl Tool for VisualizeTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "visualize".to_string(),
            description: "답변에 포함된 집계 수치를 차트로 시각화한다. 부서/상태별 건수 비교는 \
                          bar, 분포(비율)는 donut, 기간별 추이는 line을 사용하라. labels와 각 \
                          series.values의 길이는 반드시 같아야 한다. 데이터에서 이상 신호(0건 \
                          누락, 기한 임박, 급증/급감)가 보이면 insight에 한 문장으로 경고하라."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "chart_type": {
                        "type": "string",
                        "enum": ["bar", "donut", "line"],
                        "description": "차트 종류. 비교=bar, 분포=donut, 시계열=line."
                    },
                    "title": {
                        "type": "string",
                        "description": "차트 제목. 예: 부서별 순회점검 건수"
                    },
                    "labels": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "x축(bar/line) 또는 세그먼트(donut) 라벨 목록."
                    },
                    "series": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string", "description": "계열 이름." },
                                "values": {
                                    "type": "array",
                                    "items": { "type": "number" },
                                    "description": "labels와 같은 길이의 수치 배열."
                                }
                            },
                            "required": ["name", "values"]
                        },
                        "description": "1개 이상의 데이터 계열."
                    },
                    "insight": {
                        "type": "object",
                        "properties": {
                            "level": { "type": "string", "enum": ["info", "warn", "alert"] },
                            "text": { "type": "string", "description": "이상 신호 한 문장 설명." }
                        },
                        "required": ["level", "text"],
                        "description": "선택. 이상 신호가 있을 때만 '확인 필요' 콜아웃으로 표시."
                    }
                },
                "required": ["chart_type", "title", "labels", "series"],
                "additionalProperties": false
            }),
            required_permissions: vec![],
            timeout_ms: 5_000,
            idempotent: true,
        }
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let mut data: ChartData = serde_json::from_value(input)
            .map_err(|e| ToolError::ValidationFailed(format!("차트 스펙 파싱 오류: {e}")))?;

        let chart_type = data.chart_type.trim().to_lowercase();
        if !CHART_TYPES.contains(&chart_type.as_str()) {
            return Ok(ToolOutput::error(format!(
                "chart_type은 {CHART_TYPES:?} 중 하나여야 합니다(받은 값: '{}').",
                data.chart_type
            )));
        }
        data.chart_type = chart_type;

        if data.labels.is_empty() {
            return Ok(ToolOutput::error("labels가 비어 있습니다."));
        }
        if data.series.is_empty() {
            return Ok(ToolOutput::error("series가 비어 있습니다."));
        }
        for s in &data.series {
            if s.values.len() != data.labels.len() {
                return Ok(ToolOutput::error(format!(
                    "계열 '{}'의 값 개수({})가 labels 개수({})와 다릅니다.",
                    s.name,
                    s.values.len(),
                    data.labels.len()
                )));
            }
        }
        if let Some(insight) = &mut data.insight {
            let level = insight.level.trim().to_lowercase();
            // 알 수 없는 레벨은 거부 대신 info로 정규화(렌더 실패 방지).
            insight.level = if INSIGHT_LEVELS.contains(&level.as_str()) {
                level
            } else {
                "info".to_string()
            };
        }

        let content = serde_json::to_value(&data)
            .map_err(|e| ToolError::ExecutionFailed(format!("차트 직렬화 오류: {e}")))?;
        Ok(ToolOutput::success(content))
    }

    fn validate_input(&self, input: &Value) -> Result<(), ToolError> {
        match input.get("chart_type").and_then(|v| v.as_str()) {
            Some(s) if CHART_TYPES.contains(&s.trim().to_lowercase().as_str()) => Ok(()),
            _ => Err(ToolError::ValidationFailed(
                "chart_type은 bar|donut|line 중 하나여야 합니다.".to_string(),
            )),
        }
    }
}
