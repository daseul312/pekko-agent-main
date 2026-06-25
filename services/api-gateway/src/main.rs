use axum::{
    extract::{Path, State, Json},
    routing::{get, post},
    Router,
    http::{StatusCode, header, HeaderValue, Method},
    response::IntoResponse,
};
use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;
use std::sync::Arc;
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tokio::sync::RwLock;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use tracing::{info, error, warn};

use pekko_agent_core::{Message, TokenUsage, ShortTermMemory, ToolContext, AgentInfo, AgentStatus};
use pekko_agent_llm::{LlmConfig, ClaudeClient, OpenAIClient, LlmRequest, ClaudeMessage, ContentBlock, ClaudeTool};
use pekko_agent_tools::{ToolRegistry, builtin::{PermitSearchTool, ComplianceCheckTool, EhsQueryTool, LawCompareTool}};
use pekko_agent_memory::{CoreDbConversationStore, InMemoryVectorStore, InMemoryEpisodicStore};
use coredb::{CoreDB, DatabaseConfig};
use pekko_agent_orchestrator::OrchestratorActor;
use pekko_agent_events::EventPublisher;
use pekko_agent_security::{RbacManager, TenantManager, AuditLogger};
use sqlx::PgPool;

mod auth;
use auth::{AuthIdentity, OptionalAuth};


/// Shared application state with all services
#[derive(Clone)]
struct AppState {
    tool_registry: Arc<RwLock<ToolRegistry>>,
    conversation_store: Arc<CoreDbConversationStore>,
    vector_store: Arc<InMemoryVectorStore>,
    episodic_store: Arc<InMemoryEpisodicStore>,
    orchestrator: Arc<RwLock<OrchestratorActor>>,
    event_publisher: Arc<EventPublisher>,
    rbac: Arc<RwLock<RbacManager>>,
    tenant_manager: Arc<RwLock<TenantManager>>,
    audit_logger: Arc<AuditLogger>,
    llm_config: LlmConfig,
    claude_client: Arc<ClaudeClient>,
    openai_client: Option<Arc<OpenAIClient>>,
    llm_provider: String,
    /// Postgres pool used by the login handler / auth (EmployeeInfo lookup).
    pg_pool: Arc<PgPool>,
    /// RSA private key used to decrypt the login password sent by the frontend.
    rsa_private_key: Arc<rsa::RsaPrivateKey>,
    /// Matching RSA public key (SPKI PEM) served to the frontend for encryption.
    rsa_public_key_pem: Arc<str>,
}

/// Request payload for agent queries
#[derive(Deserialize)]
struct QueryRequest {
    content: String,
    #[serde(default)]
    session_id: Option<Uuid>,
    #[serde(default = "default_tenant")]
    tenant_id: String,
    #[serde(default = "default_user")]
    user_id: String,
}

fn default_tenant() -> String {
    "default".to_string()
}

fn default_user() -> String {
    "anonymous".to_string()
}

/// Response from agent query
#[derive(Serialize)]
struct QueryResponse {
    session_id: Uuid,
    agent_id: String,
    response: String,
    tools_used: Vec<String>,
    token_usage: TokenUsage,
    /// How the request was routed: "direct", "smart_single", or "multi_agent".
    route_kind: String,
    /// The agent ids that actually served this request.
    routed_agent_ids: Vec<String>,
    /// 신구법비교(법령 개정 신·구 조문) 구조화 데이터. 법령 조회 도구가
    /// 호출됐을 때만 채워지고, 그 외에는 직렬화에서 생략된다.
    #[serde(skip_serializing_if = "Option::is_none")]
    old_new_comparison: Option<OldNewComparison>,
}

/// 프론트엔드 `OldNewComparison`(web-chat/src/types.rs)과 1:1로 대응하는
/// 신구법비교 구조. `law_oldnew_comparison` 도구가 만든 JSON을 그대로 역직렬화한다.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct OldNewComparison {
    source_mode: String,
    law_name: String,
    old_law: OldNewLawInfo,
    new_law: OldNewLawInfo,
    #[serde(default)]
    article_comparisons: Vec<OldNewArticleComparison>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OldNewLawInfo {
    law_name: Option<String>,
    promulgation_date: Option<String>,
    promulgation_number: Option<String>,
    enforcement_date: Option<String>,
    amendment_type: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OldNewArticleComparison {
    article_no: Option<String>,
    title: Option<String>,
    change_type: Option<String>,
    old_text: Option<String>,
    new_text: Option<String>,
}

/// Phase of a processing step on the wire.
#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum StepPhase {
    Active,
    Done,
}

/// Streaming progress event emitted while a query is processed.
/// Serialized as one JSON object per line (NDJSON) on the `/stream` endpoints.
///
/// `Step` is a generic, upsert-by-`id` progress item so the pipeline can report
/// fine-grained stages (analysis, DB query, result review, answer composition).
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ProcessEvent {
    Step {
        id: String,
        label: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        status: StepPhase,
    },
    /// Terminal event carrying the full query response.
    Final(QueryResponse),
    /// Terminal error event.
    Error { message: String },
}

/// Build a `Step` event.
fn step_event(id: &str, label: &str, detail: Option<String>, status: StepPhase) -> ProcessEvent {
    ProcessEvent::Step {
        id: id.to_string(),
        label: label.to_string(),
        detail,
        status,
    }
}

/// Remove tool-call artifacts that some providers (e.g. OpenAI gpt-5.5 harmony
/// format) leak into the assistant's text channel — raw function-call markers
/// like `to=functions.ehs_query` and standalone JSON argument objects such as
/// `{"sql": "..."}`. Keeps the genuine answer untouched.
fn sanitize_answer(text: &str) -> String {
    let cleaned: Vec<&str> = text
        .lines()
        .filter(|line| {
            let t = line.trim();
            // Harmony function-call channel marker.
            if t.starts_with("to=functions.") || t.starts_with("functions.") {
                return false;
            }
            // A standalone single-line JSON object is almost always leaked tool
            // arguments, not answer prose.
            if t.starts_with("{\"") && t.ends_with('}') {
                return false;
            }
            true
        })
        .collect();
    cleaned.join("\n").trim().to_string()
}

/// A friendly Korean description of a tool, used in DB-query step details.
fn tool_friendly_name(tool_name: &str) -> &'static str {
    match tool_name {
        "ehs_query" => "회사 EHS 데이터베이스",
        "permit_search" => "작업허가 데이터",
        "compliance_check" => "규정 준수 데이터",
        _ => "데이터",
    }
}

/// 테이블명 → 한글 라벨. `ehs_query.rs`의 ALLOWED_TABLES와 동기화 유지.
/// 미등록 테이블은 입력 이름을 그대로 돌려주므로 반환 수명은 입력에 묶인다.
fn table_friendly_name(table: &str) -> &str {
    match table {
        "dangerousworkmanagement" => "위험작업허가",
        "tbmmanagement" => "TBM",
        "accidentfreemanagement" => "무재해",
        "riskassessmentmanagement" => "위험성평가",
        "nearmissmanagement" => "아차사고",
        "safetyeducationmanagement" => "안전교육",
        "msdsmanagement" => "MSDS",
        "safetyhealthcommittee" => "안전보건위원회",
        "employeeinfo" => "직원정보",
        "worksitemanagement" => "작업장",
        "facilitymanagement" => "시설",
        "chemicalmanagement" => "화학물질",
        "protectiveequipmentmanagement" => "보호구",
        "healthcheckmanagement" => "건강검진",
        "environmentalmanagement" => "환경",
        _ => table, // 미등록 테이블은 원래 이름 그대로
    }
}

/// `ehs_query` tool_input의 `sql`에서 조회 대상 테이블명을 추출해
/// "한글명(테이블명)" 목록으로 반환한다. 알려진 테이블명을 단어 경계 단위로
/// 매칭하므로 취약한 SQL 파서 없이도 안정적이다.
fn describe_query_tables(input: &serde_json::Value) -> Vec<String> {
    /// `ehs_query.rs`의 ALLOWED_TABLES와 동일한 15개. 변경 시 함께 갱신.
    const KNOWN_TABLES: &[&str] = &[
        "dangerousworkmanagement",
        "tbmmanagement",
        "accidentfreemanagement",
        "riskassessmentmanagement",
        "nearmissmanagement",
        "safetyeducationmanagement",
        "msdsmanagement",
        "safetyhealthcommittee",
        "employeeinfo",
        "worksitemanagement",
        "facilitymanagement",
        "chemicalmanagement",
        "protectiveequipmentmanagement",
        "healthcheckmanagement",
        "environmentalmanagement",
    ];

    let Some(sql) = input.get("sql").and_then(|v| v.as_str()) else {
        return Vec::new();
    };
    let sql_lower = sql.to_lowercase();

    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut tables: Vec<String> = KNOWN_TABLES
        .iter()
        .copied()
        .filter(|t| {
            // 단어 경계로 포함되는지 확인 (부분 문자열 오매칭 방지)
            sql_lower.match_indices(*t).any(|(idx, m)| {
                let before_ok = sql_lower[..idx].chars().next_back().map_or(true, |c| !is_ident(c));
                let after_ok = sql_lower[idx + m.len()..].chars().next().map_or(true, |c| !is_ident(c));
                before_ok && after_ok
            })
        })
        .map(|t| format!("{}({})", table_friendly_name(t), t))
        .collect();
    tables.sort();
    tables.dedup();
    tables
}

/// A single extracted condition / normalized value in a query plan.
#[derive(Deserialize)]
struct PlanItem {
    #[serde(default)]
    label: String,
    #[serde(default)]
    value: String,
}

/// Structured analysis of the user's question produced before answering, used to
/// surface detailed processing steps (condition extraction, value normalization).
#[derive(Deserialize, Default)]
struct QueryPlan {
    #[serde(default)]
    conditions: Vec<PlanItem>,
    #[serde(default)]
    normalized: Vec<PlanItem>,
}

/// Format plan items as "라벨: 값, 라벨: 값" for a step detail line.
fn format_plan_items(items: &[PlanItem]) -> String {
    items
        .iter()
        .map(|item| {
            if item.value.is_empty() {
                item.label.clone()
            } else {
                format!("{}: {}", item.label, item.value)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Extract a JSON object substring and parse it as a `QueryPlan`.
fn parse_query_plan(text: &str) -> Option<QueryPlan> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end < start {
        return None;
    }
    serde_json::from_str::<QueryPlan>(&text[start..=end]).ok()
}

/// Ask the LLM to analyze the question and return extracted conditions and their
/// normalized DB values. Returns `None` on any failure (graceful degrade).
async fn generate_query_plan(state: &AppState, content: &str) -> (Option<QueryPlan>, TokenUsage) {
    let zero = TokenUsage {
        input_tokens: 0,
        output_tokens: 0,
    };
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let system_prompt = format!(
        "당신은 EHS(환경안전보건) 질의 분석기입니다. 사용자 질문을 분석해 JSON 객체 하나만 출력하세요. \
         설명, 코드블록, 마크다운 없이 순수 JSON만 출력합니다.\n\
         형식: {{\"conditions\":[{{\"label\":\"조건명\",\"value\":\"질문 속 표현\"}}],\
         \"normalized\":[{{\"label\":\"조건명\",\"value\":\"DB 조회용 변환값\"}}]}}\n\
         conditions 는 질문에서 추출한 조회 조건(기간, 점검유형, 상태, 부서 등), \
         normalized 는 그 조건을 DB 조회용 값으로 변환한 것입니다(예: 기간 -> 'YYYY-MM-DD ~ YYYY-MM-DD', \
         상태 -> 상태코드). 오늘 날짜는 {today} 입니다. 상대적 기간은 실제 날짜 범위로 변환하세요. \
         조회 조건이 없으면 두 배열 모두 빈 배열로 두세요."
    );
    let llm_request = LlmRequest {
        system_prompt,
        messages: vec![ClaudeMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::Text {
                text: content.to_string(),
            }],
        }],
        tools: vec![],
        max_tokens: 600,
        temperature: Some(0.0),
        cacheable: false,
    };

    let resp = if state.llm_provider == "openai" {
        match &state.openai_client {
            Some(client) => match client.send_message(&llm_request).await {
                Ok(resp) => resp,
                Err(e) => {
                    warn!(error = %e, "Query plan generation failed (openai)");
                    return (None, zero);
                }
            },
            None => return (None, zero),
        }
    } else {
        match state.claude_client.send_message(&llm_request).await {
            Ok(resp) => resp,
            Err(e) => {
                warn!(error = %e, "Query plan generation failed (claude)");
                return (None, zero);
            }
        }
    };

    let tokens = TokenUsage {
        input_tokens: resp.usage.input_tokens,
        output_tokens: resp.usage.output_tokens,
    };

    let mut plan_text = String::new();
    for block in &resp.content {
        if let ContentBlock::Text { text } = block {
            plan_text.push_str(text);
        }
    }

    (parse_query_plan(&plan_text), tokens)
}

/// Serialize a `ProcessEvent` into a single NDJSON line (trailing `\n`).
fn to_ndjson_line(event: &ProcessEvent) -> Option<String> {
    serde_json::to_string(event).ok().map(|mut s| {
        s.push('\n');
        s
    })
}

/// Result of executing a single agent's ReAct tool-loop.
struct AgentQueryResult {
    agent_id: String,
    response: String,
    tools_used: Vec<String>,
    token_usage: TokenUsage,
    /// `law_oldnew_comparison` 도구가 호출됐을 때 캡처된 신구법비교 데이터.
    old_new_comparison: Option<OldNewComparison>,
}

/// Routing decision produced by the smart router.
#[derive(Debug, Clone, Deserialize)]
struct RoutingDecision {
    primary_agent_id: String,
    agent_ids: Vec<String>,
    #[serde(default)]
    rationale: String,
}

const VALID_AGENT_IDS: [&str; 6] = [
    "ehs-permit-agent",
    "ehs-inspection-agent",
    "ehs-compliance-agent",
    "ehs-accident-agent",
    "ehs-education-agent",
    "ehs-health-environment-agent",
];

/// Sentinel agent id for questions unrelated to any of the 6 EHS domains.
/// Displayed as "기타" in the conversation history.
const OTHER_AGENT_ID: &str = "ehs-other";

/// Response for session summary
#[derive(Serialize, Deserialize)]
struct SummarizeResponse {
    title: String,
}

/// Health check response
#[derive(Serialize)]
struct HealthResponse {
    status: String,
    version: String,
    services: ServiceStatus,
}

/// Status of individual services
#[derive(Serialize)]
struct ServiceStatus {
    orchestrator: String,
    tools_registered: usize,
    active_agents: usize,
}

/// Error response
#[derive(Serialize)]
struct ErrorResponse {
    error: String,
    code: String,
}

/// Build the system prompt based on the selected agent.
///
/// When `identity` is `Some`, the logged-in employee's scope is injected so the LLM
/// filters results to that person (and their company). When `None` (anonymous), the
/// company-wide / 전체 조회 behaviour is kept.
fn build_system_prompt(agent_id: &str, identity: Option<&AuthIdentity>) -> String {
    let agent_desc = match agent_id {
        "ehs-permit-agent" => {
            "당신은 EHS(환경안전보건) 허가 관리 전문 AI 에이전트입니다.\n\
             역할: 위험작업 허가(PTW) 현황 조회/분석, 작업 허가 프로세스 안내, 위험작업 유형별 통계, 허가 승인/반려 현황 추적\n\
             주요 테이블: dangerousworkmanagement(위험작업허가), tbmmanagement(TBM), accidentfreemanagement(무재해)"
        }
        "ehs-inspection-agent" => {
            "당신은 EHS(환경안전보건) 점검 관리 전문 AI 에이전트입니다.\n\
             역할: 안전점검 일정/결과 조회, 점검 현황 분석/보고, 위험성 평가 관리, 아차사고 분석\n\
             주요 테이블: riskassessmentmanagement(위험성평가), nearmissmanagement(아차사고), safetyeducationmanagement(안전교육)"
        }
        "ehs-compliance-agent" => {
            "당신은 EHS(환경안전보건) 규정 준수 전문 AI 에이전트입니다.\n\
             역할: 법규/규정 준수 확인, MSDS 관리, 안전보건위원회 운영 현황, 컴플라이언스 보고서 지원\n\
             주요 테이블: msdsmanagement(MSDS), safetyhealthcommittee(안전보건위원회), employeeinfo(직원정보)\n\
             [법령 조회 도구] 법령 개정 내용, 최신 법령, 신구법 비교, 조문 변경을 묻는 질문에는\n\
             반드시 law_oldnew_comparison 도구를 법령명(예: 산업안전보건법)으로 호출해 국가법령정보센터의\n\
             실제 신구법비교 데이터를 가져오세요. 사내 DB(ehs_query)로 추측하지 마세요.\n\
             신구법비교 표는 화면에 자동으로 렌더링되므로, 텍스트 답변에는 전체 조문을 나열하지 말고\n\
             핵심 개정 사항(신설/삭제/주요 변경 조문)만 간결히 한국어로 요약하세요."
        }
        "ehs-accident-agent" => {
            "당신은 EHS(환경안전보건) 사고 관리 전문 AI 에이전트입니다.\n\
             역할: 사고/재해 보고 조회, 아차사고(근접사고) 분석, 사고 원인 분석 및 재발방지, 시정조치/후속조치 추적, 협력업체 사고 관리\n\
             주요 테이블: AccidentFreeManagement(사고/재해보고), NearMissManagement(아차사고), ReportHarmfulRiskFactors(유해위험요인보고), CorrectiveOrder(시정명령), ContractAccidentManagement(협력업체사고)"
        }
        "ehs-education-agent" => {
            "당신은 EHS(환경안전보건) 교육 관리 전문 AI 에이전트입니다.\n\
             역할: 안전교육 계획/현황 조회, 교육 이수/미이수 대상자 관리, 강사 배정 및 훈련 운영, 교육 알림/설문 현황 추적\n\
             주요 테이블: SafetyEducationManagement(안전교육), EducationPlanManagement(교육계획), EducationPlanTarget(교육대상자), SurveyQuestionnaireManagement(설문), Notification(알림)\n\
             참고: 교육 대상자는 educationyncode, educationcompletiondate, testpassyn, totalscore 컬럼으로 이수 여부를 판단하세요."
        }
        "ehs-health-environment-agent" => {
            "당신은 EHS(환경안전보건) 보건·환경 관리 전문 AI 에이전트입니다.\n\
             역할: 유해물질/화학물질 및 MSDS 관리, 작업환경측정 현황, 건강검진 관리, 비상대응/응급관리(비상연락망, 비상훈련, 구급함) 조회\n\
             주요 테이블: HazardousMaterialsManagement(유해물질), MSDSManagement(MSDS), WorkEnvironmentManagement(작업환경측정), RegularHealthCheckup(건강검진), EmergencyContactInformation(비상연락망), EmergencyTraining(비상훈련), FirstAidKitInfo(구급함)\n\
             참고: FirstAidKitInfo 테이블에는 companyCode 컬럼이 없으므로 회사코드 필터를 추가하지 마세요."
        }
        _ => {
            "당신은 EHS(환경안전보건) 통합 AI 에이전트입니다.\n\
             안전, 보건, 환경 관련 질문에 답변할 수 있습니다."
        }
    };

    // 현재 날짜를 동적으로 주입
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let year = chrono::Local::now().format("%Y").to_string();

    // 로그인 사용자 스코핑 지시문. 비로그인이면 전체 조회(기존 동작).
    let scope_section = match identity {
        Some(id) => format!(
            "\n[현재 로그인 사용자] employeeId={emp}, companyCode={cc}, 이름={name}, 권한={role}\n\
             [데이터 스코핑 규칙]\n\
             - 모든 조회는 반드시 companyCode='{cc}' 조건으로 회사를 한정하세요(해당 컬럼이 있는 경우).\n\
             - '내', '나의', '본인', '제가', '저한테' 같은 1인칭 표현이나 개인 담당 업무\n\
             (순회점검, 교육 이수, 배정된 작업/허가 등)를 물으면, 담당자/대상자 컬럼을\n\
             employeeId='{emp}'로 필터해 본인 관련 행만 조회하세요.\n\
             - 담당자 컬럼명을 모르면 추측하지 말고 먼저 information_schema로 컬럼을 확인한 뒤\n\
             (예: employeeid, chargerid, assignee, managerid, targetemployeeid 등) 적절한 컬럼으로 필터하세요.\n\
             - 회사 전체 통계처럼 명백히 전체 대상을 묻는 경우에만 개인 필터를 생략하세요.\n",
            emp = id.employee_id,
            cc = id.company_code,
            name = id.name,
            role = id.role,
        ),
        None => "\n[비로그인 상태] 특정 개인으로 한정하지 말고 회사/전체 기준으로 조회하세요.\n"
            .to_string(),
    };

    format!(
        "{agent_desc}\n{scope_section}\n\
         [현재 날짜] 오늘은 {today} 입니다. \
         날짜 관련 쿼리 시 이 날짜를 기준으로 하세요. \
         \"이번 달\", \"올해\", \"최근\" 등의 표현은 이 날짜 기준으로 해석하세요.\n\
         \n\
         [절대 규칙] 도구 사용:\n\
         당신은 ehs_query 도구를 통해 PostgreSQL 데이터베이스에 직접 접근할 수 있습니다.\n\
         - 데이터가 필요한 질문을 받으면 반드시 ehs_query 도구를 호출하여 실제 데이터를 조회하세요.\n\
         - 절대로 SQL 쿼리 예시를 사용자에게 보여주지 마세요. 직접 도구를 호출하세요.\n\
         - 도구 호출이 실패하면, 오류를 분석하고 수정된 쿼리로 즉시 재시도하세요.\n\
         - \"기능이 제한되어 있습니다\", \"직접 조회할 수 없습니다\" 같은 말은 절대 하지 마세요.\n\
         \n\
         [필수] PostgreSQL 문법 (MySQL 문법 사용 금지):\n\
         - 날짜 추출: EXTRACT(YEAR FROM col), EXTRACT(MONTH FROM col)\n\
         - 문자열 비교: col ILIKE '%검색어%'\n\
         - 조건부 집계: COUNT(*) FILTER (WHERE 조건)\n\
         - 날짜 캐스팅: col::date, CURRENT_DATE\n\
         - 현재 날짜: CURRENT_DATE (오늘), CURRENT_DATE - INTERVAL '30 days' (30일 전)\n\
         - 올해 시작: '{year}-01-01'::date\n\
         - YEAR(), MONTH(), IFNULL(), NOW() 등 MySQL 전용 함수는 사용 금지\n\
         \n\
         [필수] 테이블 스키마 탐색 절차:\n\
         처음 접하는 테이블은 반드시 먼저 컬럼명을 확인하세요:\n\
         SELECT column_name, data_type FROM information_schema.columns WHERE table_name = '테이블명' ORDER BY ordinal_position\n\
         컬럼명을 확인한 후, 정확한 컬럼명으로 데이터를 조회하세요. 컬럼명을 추측하지 마세요.\n\
         \n\
         [필수] 쿼리 작성 규칙:\n\
         - SELECT *는 피하고, 필요한 컬럼만 선택하세요.\n\
         - 큰 테이블은 반드시 LIMIT을 사용하세요.\n\
         - 통계가 필요하면 COUNT, SUM, AVG 등 집계 함수를 활용하세요.\n\
         - 쿼리 오류 시 오류 메시지를 분석하여 컬럼명/타입을 수정하고 재시도하세요.\n\
         \n\
         응답 규칙:\n\
         - 한국어로 답변하세요.\n\
         - 반드시 실제 데이터 기반으로 정확한 수치를 제공하세요.\n\
         - 필요시 개선 제안을 추가하세요.\n\
         \n\
         [필수] 응답 형식:\n\
         - 조회 결과가 여러 행이면 반드시 마크다운 테이블로 정리하세요.\n\
         - 테이블 형식: | 컬럼1 | 컬럼2 | ... |\\n|---|---|...|\\n| 값1 | 값2 | ... |\n\
         - 통계 요약도 테이블로 보여주세요.\n\
         - 테이블 앞뒤에 간단한 설명을 붙이세요.\n\
         - 숫자는 천 단위 쉼표를 사용하세요 (예: 1,234).\n\
         - 날짜는 YYYY-MM-DD 형식으로 표시하세요."
    )
}

/// Convert ToolDefinition to ClaudeTool format
fn tool_definitions_to_claude_tools(tools: &[pekko_agent_core::ToolDefinition]) -> Vec<ClaudeTool> {
    tools
        .iter()
        .map(|t| ClaudeTool {
            name: t.name.clone(),
            description: t.description.clone(),
            input_schema: t.input_schema.clone(),
        })
        .collect()
}

/// Maximum number of tool use rounds to prevent infinite loops
const MAX_TOOL_ROUNDS: usize = 10;

/// Maximum characters for tool result content to stay within Claude's context limits
const MAX_TOOL_RESULT_CHARS: usize = 30_000;

/// Truncate tool result content if it exceeds the limit
fn truncate_tool_result(content: &str) -> String {
    if content.len() <= MAX_TOOL_RESULT_CHARS {
        content.to_string()
    } else {
        let truncated = &content[..MAX_TOOL_RESULT_CHARS];
        format!(
            "{}...\n\n[결과가 너무 큽니다. {}자 중 {}자만 표시됩니다. 더 구체적인 SELECT 컬럼이나 WHERE 조건을 사용해주세요.]",
            truncated,
            content.len(),
            MAX_TOOL_RESULT_CHARS
        )
    }
}

/// GET /api/health - Health check endpoint
async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    let tools = state.tool_registry.read().await;
    let orch = state.orchestrator.read().await;

    Json(HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        services: ServiceStatus {
            orchestrator: "running".to_string(),
            tools_registered: tools.list_tools().len(),
            active_agents: orch.list_agents().len(),
        },
    })
}

/// GET /api/agents - List all available agents
async fn list_agents(State(state): State<AppState>) -> impl IntoResponse {
    let orch = state.orchestrator.read().await;
    let agents: Vec<_> = orch.list_agents().into_iter().cloned().collect();
    Json(agents)
}

/// POST /api/agents/:agent_id/query - Submit a query to a specific agent (buffered).
async fn query_agent(
    Path(agent_id): Path<String>,
    State(state): State<AppState>,
    OptionalAuth(auth): OptionalAuth,
    Json(req): Json<QueryRequest>,
) -> Result<Json<QueryResponse>, (StatusCode, Json<ErrorResponse>)> {
    let session_id = req.session_id.unwrap_or_else(Uuid::new_v4);
    let noop = |_: ProcessEvent| {};
    run_agent_query_pipeline(&state, &agent_id, &req, session_id, &noop, false, &auth)
        .await
        .map(Json)
}

/// POST /api/agents/:agent_id/query/stream - same as `query_agent` but streams
/// progress events as NDJSON so the frontend can show the live processing steps.
async fn query_agent_stream(
    Path(agent_id): Path<String>,
    State(state): State<AppState>,
    OptionalAuth(auth): OptionalAuth,
    Json(req): Json<QueryRequest>,
) -> axum::response::Response {
    use axum::body::{Body, Bytes};
    use tokio_stream::wrappers::UnboundedReceiverStream;

    let session_id = req.session_id.unwrap_or_else(Uuid::new_v4);
    let (tx, rx) =
        tokio::sync::mpsc::unbounded_channel::<Result<Bytes, std::convert::Infallible>>();

    tokio::spawn(async move {
        let emit = {
            let tx = tx.clone();
            move |event: ProcessEvent| {
                if let Some(line) = to_ndjson_line(&event) {
                    let _ = tx.send(Ok(Bytes::from(line)));
                }
            }
        };

        match run_agent_query_pipeline(&state, &agent_id, &req, session_id, &emit, true, &auth).await {
            Ok(resp) => emit(ProcessEvent::Final(resp)),
            Err((_status, Json(body))) => emit(ProcessEvent::Error { message: body.error }),
        }
    });

    let stream = UnboundedReceiverStream::new(rx);
    axum::response::Response::builder()
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(stream))
        .expect("failed to build streaming response")
}

/// Core direct-agent pipeline shared by the buffered and streaming endpoints.
/// `emit` receives progress events; pass a no-op closure for the buffered path.
async fn run_agent_query_pipeline(
    state: &AppState,
    agent_id: &str,
    req: &QueryRequest,
    session_id: Uuid,
    emit: &(impl Fn(ProcessEvent) + Send + Sync),
    stream: bool,
    auth: &Option<AuthIdentity>,
) -> Result<QueryResponse, (StatusCode, Json<ErrorResponse>)> {
    info!(
        agent_id = %agent_id,
        session_id = %session_id,
        content_len = req.content.len(),
        "Agent query received"
    );

    // Store the user message in conversation history
    let msg = Message::user(&req.content);
    let conv_store = state.conversation_store.clone();
    let _ = conv_store.append_message(&session_id, msg).await;

    // Publish task assigned event
    let event = pekko_agent_events::AgentEventEnvelope::new(
        "api-gateway",
        pekko_agent_events::event_types::TASK_ASSIGNED,
        &req.tenant_id,
        session_id,
        serde_json::json!({
            "agent_id": agent_id,
            "content": req.content,
        }),
    );
    let _ = state.event_publisher.publish(event).await;

    // Log audit entry
    state.audit_logger.log(pekko_agent_security::AuditEntry {
        id: Uuid::new_v4(),
        timestamp: chrono::Utc::now(),
        tenant_id: req.tenant_id.clone(),
        agent_id: agent_id.to_string(),
        action: "query".to_string(),
        resource: format!("agent/{}", agent_id),
        outcome: pekko_agent_security::AuditOutcome::Success,
        details: serde_json::json!({"session_id": session_id}),
    }).await;

    // A specific agent was chosen, so there is no routing classification — just
    // report the agent being called, then run the answer (which emits its own
    // fine-grained steps).
    emit(step_event(
        "routing",
        "담당 에이전트",
        Some(agent_display_name(agent_id).to_string()),
        StepPhase::Done,
    ));

    // Run the agent ReAct tool-loop (user message already stored above)
    let result = execute_agent_query(state, agent_id, req, session_id, emit, stream, auth).await?;

    Ok(QueryResponse {
        session_id,
        agent_id: result.agent_id,
        response: result.response,
        tools_used: result.tools_used,
        token_usage: result.token_usage,
        route_kind: "direct".to_string(),
        routed_agent_ids: vec![agent_id.to_string()],
        old_new_comparison: result.old_new_comparison,
    })
}

/// Execute a single agent's ReAct tool-loop for the given request.
///
/// Builds the conversation history for context, runs the OpenAI/Claude tool loop with
/// `build_system_prompt(agent_id)` + the registered tools, stores the assistant response
/// in the conversation store, and returns the aggregated result. This is the reusable
/// per-agent execution used by both `query_agent` and `query_home`.
async fn execute_agent_query(
    state: &AppState,
    agent_id: &str,
    req: &QueryRequest,
    session_id: Uuid,
    emit: &(impl Fn(ProcessEvent) + Send + Sync),
    enable_plan: bool,
    auth: &Option<AuthIdentity>,
) -> Result<AgentQueryResult, (StatusCode, Json<ErrorResponse>)> {
    let conv_store = state.conversation_store.clone();

    // Build conversation history for LLM context
    let history = conv_store
        .get_conversation(&session_id)
        .await
        .unwrap_or_default();

    let claude_messages: Vec<ClaudeMessage> = history
        .iter()
        .map(|m| ClaudeMessage {
            role: match m.role {
                pekko_agent_core::MessageRole::User => "user".to_string(),
                pekko_agent_core::MessageRole::Assistant => "assistant".to_string(),
                _ => "user".to_string(),
            },
            content: vec![ContentBlock::Text { text: m.content.clone() }],
        })
        .collect();

    // Get tool definitions from registry and convert to Claude format
    let tool_defs = {
        let registry = state.tool_registry.read().await;
        registry.list_tools()
    };
    let claude_tools = tool_definitions_to_claude_tools(&tool_defs);

    info!(tool_count = claude_tools.len(), "Sending tools to Claude");

    // Build system prompt
    let system_prompt = build_system_prompt(agent_id, auth.as_ref());

    // Tool use loop - keep calling Claude until it gives a final text response
    let mut messages = claude_messages;
    let mut all_tools_used: Vec<String> = Vec::new();
    let mut total_input_tokens: u32 = 0;
    let mut total_output_tokens: u32 = 0;
    let mut final_text = String::new();
    // 신구법비교 도구가 성공하면 그 구조화 결과를 캡처해 최종 응답에 실어 보낸다.
    let mut captured_comparison: Option<OldNewComparison> = None;
    // Tracks whether the previous round executed tools, so the next LLM round is
    // labelled as "result review" instead of "initial analysis".
    let mut used_tool_last_round = false;

    // Optional structured plan: extract conditions and normalized values up front
    // so the UI can show detailed processing steps.
    let mut showed_plan = false;
    if enable_plan {
        let (plan, plan_tokens) = generate_query_plan(state, &req.content).await;
        total_input_tokens += plan_tokens.input_tokens;
        total_output_tokens += plan_tokens.output_tokens;
        if let Some(plan) = plan {
            if !plan.conditions.is_empty() {
                emit(step_event(
                    "extract",
                    "조회 조건 추출",
                    Some(format_plan_items(&plan.conditions)),
                    StepPhase::Done,
                ));
                showed_plan = true;
            }
            if !plan.normalized.is_empty() {
                emit(step_event(
                    "normalize",
                    "기준값 변환",
                    Some(format_plan_items(&plan.normalized)),
                    StepPhase::Done,
                ));
                showed_plan = true;
            }
        }
    }

    // Round-0 reasoning label: when a plan was shown, this round writes the query;
    // otherwise it does the initial analysis.
    let round0_label = if showed_plan {
        "조회 쿼리 작성"
    } else {
        "질문 분석 · 조회 조건 구성"
    };

    for round in 0..MAX_TOOL_ROUNDS {
        // Announce the LLM reasoning step for this round.
        if round == 0 {
            emit(step_event("analyze", round0_label, None, StepPhase::Active));
        } else if used_tool_last_round {
            emit(step_event(
                "review",
                "조회 결과 분석",
                None,
                StepPhase::Active,
            ));
        }

        let llm_request = LlmRequest {
            system_prompt: system_prompt.clone(),
            messages: messages.clone(),
            tools: claude_tools.clone(),
            max_tokens: state.llm_config.max_tokens,
            temperature: Some(state.llm_config.temperature),
            cacheable: false,
        };

        // Call LLM based on provider
        let resp = if state.llm_provider == "openai" {
            if let Some(ref openai) = state.openai_client {
                match openai.send_message(&llm_request).await {
                    Ok(resp) => resp,
                    Err(e) => {
                        error!(error = %e, round = round, "OpenAI call failed");
                        final_text = format!("죄송합니다. AI 응답 생성 중 오류가 발생했습니다: {}", e);
                        break;
                    }
                }
            } else {
                error!("OpenAI client not configured");
                final_text = "OpenAI API 키가 설정되지 않았습니다.".to_string();
                break;
            }
        } else {
            match state.claude_client.send_message(&llm_request).await {
                Ok(resp) => resp,
                Err(e) => {
                    error!(error = %e, round = round, "Claude call failed");
                    final_text = format!("죄송합니다. AI 응답 생성 중 오류가 발생했습니다: {}", e);
                    break;
                }
            }
        };

        total_input_tokens += resp.usage.input_tokens;
        total_output_tokens += resp.usage.output_tokens;

        info!(
            round = round,
            stop_reason = %resp.stop_reason,
            content_blocks = resp.content.len(),
            "Claude response received"
        );

        // Check if Claude wants to use tools
        if resp.stop_reason == "tool_use" {
            // Extract text and tool_use blocks from response
            let mut assistant_content: Vec<ContentBlock> = Vec::new();
            let mut tool_uses: Vec<(String, String, serde_json::Value)> = Vec::new();

            for block in &resp.content {
                match block {
                    ContentBlock::Text { text } => {
                        assistant_content.push(ContentBlock::Text { text: text.clone() });
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        assistant_content.push(ContentBlock::ToolUse {
                            id: id.clone(),
                            name: name.clone(),
                            input: input.clone(),
                        });
                        tool_uses.push((id.clone(), name.clone(), input.clone()));
                    }
                    _ => {}
                }
            }

            // Add assistant message with tool_use to history
            messages.push(ClaudeMessage {
                role: "assistant".to_string(),
                content: assistant_content,
            });

            // Mark this round's reasoning step done and announce the DB query.
            let (analysis_id, analysis_label) = if round == 0 {
                ("analyze", round0_label)
            } else {
                ("review", "조회 결과 분석")
            };
            emit(step_event(analysis_id, analysis_label, None, StepPhase::Done));

            let mut tool_descs: Vec<String> = tool_uses
                .iter()
                .flat_map(|(_, name, input)| {
                    if name == "ehs_query" {
                        let tables = describe_query_tables(input);
                        if !tables.is_empty() {
                            // 예: ["위험작업허가(dangerousworkmanagement)", "직원정보(employeeinfo)"]
                            return tables;
                        }
                    }
                    // ehs_query가 아니거나 테이블 추출 실패 시 기존 친절한 이름
                    vec![tool_friendly_name(name).to_string()]
                })
                .collect();
            tool_descs.sort();
            tool_descs.dedup();
            let query_detail = if tool_descs.is_empty() {
                None
            } else {
                Some(tool_descs.join(", "))
            };
            emit(step_event(
                "query",
                "회사 DB 조회",
                query_detail.clone(),
                StepPhase::Active,
            ));

            // Execute each tool and collect results.
            // When logged in, carry the verified identity (from the JWT cookie, not the
            // request body) into the tool context so tools / future row-level guards can
            // scope to the employee.
            let mut tool_results: Vec<ContentBlock> = Vec::new();
            let mut credentials = HashMap::new();
            if let Some(id) = auth {
                credentials.insert("employee_id".to_string(), id.employee_id.clone());
                credentials.insert("company_code".to_string(), id.company_code.clone());
                credentials.insert("role".to_string(), id.role.clone());
            }
            let tool_ctx = ToolContext {
                tenant_id: req.tenant_id.clone(),
                user_id: auth
                    .as_ref()
                    .map(|i| i.employee_id.clone())
                    .unwrap_or_else(|| req.user_id.clone()),
                session_id,
                credentials,
                timeout: Duration::from_secs(10),
            };

            for (tool_use_id, tool_name, tool_input) in &tool_uses {
                info!(tool_name = %tool_name, tool_use_id = %tool_use_id, "Executing tool");
                all_tools_used.push(tool_name.clone());

                let result = {
                    let mut registry = state.tool_registry.write().await;
                    registry.execute(tool_name, tool_input.clone(), &tool_ctx).await
                };

                let (content_str, is_error) = match result {
                    Ok(output) => {
                        // 신구법비교 도구 결과는 구조화 데이터로 별도 캡처(첫 성공만).
                        // LLM에는 아래에서 동일 내용이 tool_result로도 전달돼 텍스트 요약에 쓰인다.
                        if tool_name == "law_oldnew_comparison"
                            && !output.is_error
                            && captured_comparison.is_none()
                        {
                            match serde_json::from_value::<OldNewComparison>(output.content.clone()) {
                                Ok(cmp) => captured_comparison = Some(cmp),
                                Err(e) => warn!(error = %e, "신구법비교 구조 파싱 실패"),
                            }
                        }
                        let content = serde_json::to_string(&output.content)
                            .unwrap_or_else(|_| "{}".to_string());
                        let content = truncate_tool_result(&content);
                        info!(tool_name = %tool_name, is_error = output.is_error, content_len = content.len(), "Tool executed");
                        (content, if output.is_error { Some(true) } else { None })
                    }
                    Err(e) => {
                        warn!(tool_name = %tool_name, error = %e, "Tool execution failed");
                        (format!("{{\"error\": \"{}\"}}", e), Some(true))
                    }
                };

                tool_results.push(ContentBlock::ToolResult {
                    tool_use_id: tool_use_id.clone(),
                    content: content_str,
                    is_error,
                });
            }

            // Mark the DB query step done; results are received.
            emit(step_event("query", "회사 DB 조회", query_detail, StepPhase::Done));
            used_tool_last_round = true;

            // Add tool results as user message
            messages.push(ClaudeMessage {
                role: "user".to_string(),
                content: tool_results,
            });

            // Continue loop to get Claude's next response
            continue;
        }

        // stop_reason is "end_turn" or other — mark the reasoning step done and
        // compose the final answer.
        let (analysis_id, analysis_label) = if round == 0 {
            ("analyze", round0_label)
        } else {
            ("review", "조회 결과 분석")
        };
        emit(step_event(analysis_id, analysis_label, None, StepPhase::Done));
        emit(step_event("compose", "최종 답변 생성", None, StepPhase::Active));

        for block in &resp.content {
            if let ContentBlock::Text { text } = block {
                if !final_text.is_empty() {
                    final_text.push('\n');
                }
                final_text.push_str(text);
            }
        }
        emit(step_event("compose", "최종 답변 생성", None, StepPhase::Done));
        break;
    }

    // The tool loop may exhaust MAX_TOOL_ROUNDS without ever producing a text
    // answer (the model kept calling tools). Force one final call with no tools
    // so it must synthesize an answer from the data already gathered.
    if final_text.trim().is_empty() {
        emit(step_event("compose", "최종 답변 생성", None, StepPhase::Active));
        // Nudge the model to stop calling tools and just answer, which avoids
        // leaking raw function-call syntax into the text channel.
        let mut synth_messages = messages.clone();
        synth_messages.push(ClaudeMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::Text {
                text: "지금까지 조회한 결과만으로 최종 답변을 작성하세요. 도구를 더 호출하지 말고, \
                       함수 호출 구문이나 JSON을 출력하지 말고, 한국어 마크다운으로 사용자에게 보여줄 \
                       최종 답변만 작성하세요."
                    .to_string(),
            }],
        });
        let llm_request = LlmRequest {
            system_prompt: system_prompt.clone(),
            messages: synth_messages,
            tools: vec![],
            max_tokens: state.llm_config.max_tokens,
            temperature: Some(state.llm_config.temperature),
            cacheable: false,
        };
        let resp = if state.llm_provider == "openai" {
            match &state.openai_client {
                Some(client) => client.send_message(&llm_request).await.ok(),
                None => None,
            }
        } else {
            state.claude_client.send_message(&llm_request).await.ok()
        };
        if let Some(resp) = resp {
            total_input_tokens += resp.usage.input_tokens;
            total_output_tokens += resp.usage.output_tokens;
            for block in &resp.content {
                if let ContentBlock::Text { text } = block {
                    if !final_text.is_empty() {
                        final_text.push('\n');
                    }
                    final_text.push_str(text);
                }
            }
        }
        emit(step_event("compose", "최종 답변 생성", None, StepPhase::Done));
    }

    // Strip any leaked tool-call artifacts from the answer text.
    final_text = sanitize_answer(&final_text);

    // Safety net: never return a completely empty answer.
    if final_text.trim().is_empty() {
        final_text =
            "죄송합니다. 요청하신 내용에 대한 답변을 생성하지 못했습니다. 질문을 더 구체적으로 작성해 주시면 다시 시도하겠습니다."
                .to_string();
    }

    // Store the assistant response
    let assistant_msg = Message::assistant(&final_text);
    let _ = conv_store.append_message(&session_id, assistant_msg).await;

    // Deduplicate tools_used
    all_tools_used.sort();
    all_tools_used.dedup();

    Ok(AgentQueryResult {
        agent_id: agent_id.to_string(),
        response: final_text,
        tools_used: all_tools_used,
        token_usage: TokenUsage {
            input_tokens: total_input_tokens,
            output_tokens: total_output_tokens,
        },
        old_new_comparison: captured_comparison,
    })
}

// ============================================================================
// Smart router: classify a generic /api/query request to one or more agents.
// ============================================================================

const KOREAN_TABLE_HEADER_RULE: &str = "마크다운 테이블을 출력할 때는 모든 헤더 셀을 한국어로 작성하세요. 영어나 스키마 형태의 컬럼명은 사용자가 원본 유지를 명시적으로 요청하지 않는 한 간결하고 자연스러운 한국어 라벨로 번역하세요.";

fn is_valid_agent_id(agent_id: &str) -> bool {
    VALID_AGENT_IDS.contains(&agent_id)
}

fn agent_display_name(agent_id: &str) -> &'static str {
    match agent_id {
        "ehs-permit-agent" => "허가관리",
        "ehs-inspection-agent" => "점검관리",
        "ehs-compliance-agent" => "규정 준수",
        "ehs-accident-agent" => "사고 관리",
        "ehs-education-agent" => "교육 관리",
        "ehs-health-environment-agent" => "보건·환경 관리",
        "ehs-other" => "기타",
        _ => "EHS AI",
    }
}

fn agent_routing_summary(agent_id: &str) -> &'static str {
    match agent_id {
        "ehs-permit-agent" => {
            "작업허가서, 위험작업허가, TBM, 작업 전 계획과 체크리스트, 화기 작업 등 허가 기반 업무"
        }
        "ehs-inspection-agent" => {
            "점검 일정, 순회점검, 위험성평가, 공동 안전점검, 점검 체크리스트와 개선조치"
        }
        "ehs-compliance-agent" => {
            "법령, 규정, SOP, 심사 대응, 준수 여부 판단, 산업안전보건법과 사내 규정 검토"
        }
        "ehs-accident-agent" => {
            "사고, 아차사고, 재해 보고, 원인 분석, 재발방지, 시정조치와 후속 조치"
        }
        "ehs-education-agent" => {
            "안전교육 계획, 교육 이수, 강사 배정, 훈련 운영, 미이수 대상자 관리"
        }
        "ehs-health-environment-agent" => {
            "보건, 환경, 유해물질, MSDS, 작업환경측정, 건강검진, 비상대응과 응급관리"
        }
        _ => "일반 EHS 업무",
    }
}

fn normalize_agent_ids(agent_ids: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();

    for agent_id in agent_ids {
        if is_valid_agent_id(&agent_id) && seen.insert(agent_id.clone()) {
            normalized.push(agent_id);
        }
    }

    normalized
}

fn add_token_usage(total: &mut TokenUsage, extra: &TokenUsage) {
    total.input_tokens = total.input_tokens.saturating_add(extra.input_tokens);
    total.output_tokens = total.output_tokens.saturating_add(extra.output_tokens);
}

fn truncate_for_prompt(content: &str, max_chars: usize) -> String {
    let char_count = content.chars().count();
    if char_count <= max_chars {
        return content.to_string();
    }

    let mut truncated: String = content.chars().take(max_chars).collect();
    truncated.push_str("\n...[truncated]");
    truncated
}

fn collect_text_blocks(blocks: &[ContentBlock]) -> String {
    let mut text = String::new();

    for block in blocks {
        if let ContentBlock::Text { text: block_text } = block {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(block_text);
        }
    }

    text
}

fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end < start {
        return None;
    }
    Some(&text[start..=end])
}

fn compact_lower(content: &str) -> String {
    content
        .split_whitespace()
        .collect::<String>()
        .to_lowercase()
}

fn is_prework_checklist_request(content: &str) -> bool {
    let compact = compact_lower(content);

    compact.contains("작업전체크리스트")
        || compact.contains("작업전점검")
        || compact.contains("preworkchecklist")
        || compact.contains("pre-workchecklist")
        || compact.contains("pre_work_checklist")
        || (compact.contains("pre-work") && compact.contains("checklist"))
        || (compact.contains("작업전") && compact.contains("체크리스트"))
}

fn is_round_inspection_request(content: &str) -> bool {
    let compact = compact_lower(content);

    compact.contains("순회점검")
        || compact.contains("정기순회점검")
        || compact.contains("roundinspectionmanagement")
        || compact.contains("roundinspection")
        || compact.contains("routineinspection")
        || compact.contains("periodicroutineinspection")
}

fn has_cross_domain_routing_hint(content: &str) -> bool {
    let normalized = content.to_lowercase();

    let has_other_domain_keyword = [
        "법령", "규정", "준수", "산업안전보건법", "사고", "재해", "교육", "보건", "환경", "msds",
    ]
    .iter()
    .any(|keyword| normalized.contains(keyword));

    let has_connector = [
        "동시에", "함께", "연계", "그리고", "및", "하면서", "같이", "요약하면서", "알려주고",
    ]
    .iter()
    .any(|keyword| normalized.contains(keyword));

    has_other_domain_keyword && has_connector
}

fn has_explicit_multi_intent(content: &str) -> bool {
    let normalized = content.to_lowercase();
    [
        "동시에", "함께", "연계", " 그리고 ", " 및 ", "하면서", "같이", "요약하면서", "알려주고",
    ]
    .iter()
    .any(|token| normalized.contains(token.trim()))
}

/// Keyword classification over the 6 agents. This is the routing safety net used
/// whenever the LLM classification fails or returns invalid JSON.
fn routing_fallback(content: &str) -> RoutingDecision {
    if is_prework_checklist_request(content) {
        return RoutingDecision {
            primary_agent_id: "ehs-permit-agent".to_string(),
            agent_ids: vec!["ehs-permit-agent".to_string()],
            rationale: "prework_checklist_is_permit_domain".to_string(),
        };
    }

    if is_round_inspection_request(content) && !has_cross_domain_routing_hint(content) {
        return RoutingDecision {
            primary_agent_id: "ehs-inspection-agent".to_string(),
            agent_ids: vec!["ehs-inspection-agent".to_string()],
            rationale: "round_inspection_is_inspection_domain".to_string(),
        };
    }

    let normalized = content.to_lowercase();

    let specs: [(&str, &[&str]); 6] = [
        (
            "ehs-permit-agent",
            &[
                "permit", "허가", "작업허가", "위험작업", "tbm", "작업전", "pre-work", "화기",
                "밀폐공간",
            ],
        ),
        (
            "ehs-inspection-agent",
            &[
                "inspection", "점검", "순회", "체크리스트", "위험성평가", "risk assessment",
                "near miss", "개선조치",
            ],
        ),
        (
            "ehs-compliance-agent",
            &[
                "compliance", "법령", "규정", "준수", "sop", "법", "감사", "심사", "위반",
                "산업안전보건법",
            ],
        ),
        (
            "ehs-accident-agent",
            &[
                "accident", "incident", "사고", "재해", "아차사고", "근접사고", "원인",
                "재발방지", "조사",
            ],
        ),
        (
            "ehs-education-agent",
            &[
                "education", "training", "교육", "훈련", "이수", "강사", "수강", "교육계획",
            ],
        ),
        (
            "ehs-health-environment-agent",
            &[
                "health", "environment", "msds", "보건", "환경", "유해물질", "화학물질",
                "작업환경", "건강검진", "응급", "비상",
            ],
        ),
    ];

    let mut ranked: Vec<(&str, usize)> = specs
        .iter()
        .map(|(agent_id, keywords)| {
            let score = keywords
                .iter()
                .filter(|keyword| normalized.contains(*keyword))
                .count();
            (*agent_id, score)
        })
        .collect();

    ranked.sort_by(|left, right| right.1.cmp(&left.1));

    let positives: Vec<(&str, usize)> = ranked
        .iter()
        .copied()
        .filter(|(_, score)| *score > 0)
        .collect();

    // No EHS keyword matched at all → treat as an unrelated ("기타") question.
    if positives.is_empty() {
        return RoutingDecision {
            primary_agent_id: OTHER_AGENT_ID.to_string(),
            agent_ids: vec![OTHER_AGENT_ID.to_string()],
            rationale: "no_ehs_keyword_match".to_string(),
        };
    }

    let multi_hint = [
        "동시에", "함께", "연계", "그리고", "및", "법령", "규정", "하면서", "같이", "요약하면서",
        "알려주고",
    ];

    let mut agent_ids: Vec<String> = if positives.is_empty() {
        vec!["ehs-permit-agent".to_string()]
    } else if positives.len() == 1 {
        vec![positives[0].0.to_string()]
    } else {
        let choose_multiple = positives[1].1 + 1 >= positives[0].1
            || multi_hint.iter().any(|hint| normalized.contains(hint));

        if choose_multiple {
            positives
                .iter()
                .take(3)
                .map(|(agent_id, _)| (*agent_id).to_string())
                .collect()
        } else {
            vec![positives[0].0.to_string()]
        }
    };

    agent_ids = normalize_agent_ids(agent_ids);

    if agent_ids.is_empty() {
        agent_ids.push("ehs-permit-agent".to_string());
    }

    let has_accident = agent_ids.iter().any(|agent_id| agent_id == "ehs-accident-agent");
    let has_compliance = agent_ids.iter().any(|agent_id| agent_id == "ehs-compliance-agent");

    if has_accident
        && has_compliance
        && (normalized.contains("사고")
            || normalized.contains("accident")
            || normalized.contains("incident"))
    {
        agent_ids.retain(|agent_id| agent_id != "ehs-accident-agent");
        agent_ids.insert(0, "ehs-accident-agent".to_string());
    }

    RoutingDecision {
        primary_agent_id: agent_ids[0].clone(),
        agent_ids,
        rationale: "keyword_fallback".to_string(),
    }
}

fn sanitize_routing_decision(content: &str, mut decision: RoutingDecision) -> RoutingDecision {
    let fallback = routing_fallback(content);

    // "기타" handling: if the decision points to the unrelated sentinel and no
    // real EHS agent is also selected, keep it as a clean 기타 decision.
    let mentions_other = decision.primary_agent_id == OTHER_AGENT_ID
        || decision.agent_ids.iter().any(|id| id == OTHER_AGENT_ID);
    if mentions_other {
        let has_real_agent = decision.agent_ids.iter().any(|id| is_valid_agent_id(id));
        if !has_real_agent {
            let rationale = if decision.rationale.trim().is_empty() {
                "other".to_string()
            } else {
                decision.rationale
            };
            return RoutingDecision {
                primary_agent_id: OTHER_AGENT_ID.to_string(),
                agent_ids: vec![OTHER_AGENT_ID.to_string()],
                rationale,
            };
        }
        // A real agent is present → this is a genuine EHS question; drop the
        // sentinel below via normalize and proceed normally.
    }

    decision.agent_ids = normalize_agent_ids(decision.agent_ids);

    if is_valid_agent_id(&decision.primary_agent_id)
        && !decision
            .agent_ids
            .iter()
            .any(|agent_id| agent_id == &decision.primary_agent_id)
    {
        decision.agent_ids.insert(0, decision.primary_agent_id.clone());
        decision.agent_ids = normalize_agent_ids(decision.agent_ids);
    }

    if decision.agent_ids.is_empty() {
        return fallback;
    }

    if !is_valid_agent_id(&decision.primary_agent_id)
        || !decision
            .agent_ids
            .iter()
            .any(|agent_id| agent_id == &decision.primary_agent_id)
    {
        decision.primary_agent_id = decision.agent_ids[0].clone();
    }

    if decision.agent_ids.len() > 3 {
        decision.agent_ids.truncate(3);
        if !decision
            .agent_ids
            .iter()
            .any(|agent_id| agent_id == &decision.primary_agent_id)
        {
            decision.primary_agent_id = decision.agent_ids[0].clone();
        }
    }

    if decision.rationale.trim().is_empty() {
        decision.rationale = fallback.rationale;
    }

    decision
}

fn merge_routing_decisions(
    content: &str,
    keyword_decision: RoutingDecision,
    mut llm_decision: RoutingDecision,
) -> RoutingDecision {
    if keyword_decision.agent_ids.len() > 1 {
        let missing_keyword_agents = keyword_decision
            .agent_ids
            .iter()
            .any(|agent_id| !llm_decision.agent_ids.contains(agent_id));

        if has_explicit_multi_intent(content)
            || (llm_decision.agent_ids.len() == 1 && missing_keyword_agents)
        {
            llm_decision.agent_ids = keyword_decision.agent_ids.clone();
            if !llm_decision
                .agent_ids
                .iter()
                .any(|agent_id| agent_id == &llm_decision.primary_agent_id)
            {
                llm_decision.primary_agent_id = keyword_decision.primary_agent_id.clone();
            }
            if llm_decision.rationale.trim().is_empty() {
                llm_decision.rationale = keyword_decision.rationale;
            }
        }
    }

    sanitize_routing_decision(content, llm_decision)
}

fn build_routing_prompt(content: &str) -> String {
    let mut catalog = String::new();
    for agent_id in VALID_AGENT_IDS {
        catalog.push_str(&format!(
            "- {agent_id}: {name}. {summary}\n",
            name = agent_display_name(agent_id),
            summary = agent_routing_summary(agent_id)
        ));
    }

    format!("You route EHS user questions to specialized agents. Answer with JSON only.\n\
Return a JSON object with exactly these keys: primary_agent_id, agent_ids, rationale.\n\
Rules:\n\
- Choose only from the listed agent ids.\n\
- If the question is clearly unrelated to all 6 EHS domains (e.g. greetings, small talk, weather, general knowledge, or topics with no EHS/safety/health/environment connection), set primary_agent_id and agent_ids to [\"ehs-other\"]. Do NOT force an EHS agent for unrelated questions.\n\
- agent_ids must contain 1 to 3 ids.\n\
- Choose multiple agents only when the user explicitly needs combined operational and compliance, accident and investigation, or other cross-domain synthesis.\n\
- primary_agent_id must be the best page for the UI to navigate to first.\n\
- Keep rationale short.\n\
- Queries containing `Pre-work checklist`, `Pre-work Checklist`, or `pre-work checklist` must be sent to `ehs-permit-agent`.\n\
- Queries containing `Routine Inspection`, `Periodic Routine Inspection`, or `RoundInspectionManagement` should be sent to `ehs-inspection-agent`.\n\
- Queries about education, training, education plan, completion, incomplete education, completed education, target trainees, test score, pass score, retest, certificate, survey, or education notifications must be sent to `ehs-education-agent`, not `ehs-inspection-agent`.\n\
- Korean queries containing `교육`, `교육계획`, `이수`, `미이수`, `수료`, `대상자`, `재시험`, `시험`, `점수`, `통과점수`, `설문`, or `교육 알림` must be sent to `ehs-education-agent`.\n\
- `pre-work` has priority over generic `checklist`: even if `checklist` appears, do not send it to `ehs-inspection-agent` when the query includes `pre-work`; send it to `ehs-permit-agent`.\n\
Available agents:\n\
{catalog}\n\
User question:\n\
{content}")
}

/// Run a single one-shot LLM text request through the active provider and return the
/// concatenated text response plus token usage. Returns an Err(String) on provider failure.
async fn run_oneshot_llm(state: &AppState, request: &LlmRequest, context: &str) -> Result<(String, TokenUsage), String> {
    let resp = if state.llm_provider == "openai" {
        if let Some(ref openai) = state.openai_client {
            openai.send_message(request).await.map_err(|e| {
                error!(error = %e, context = context, "OpenAI one-shot call failed");
                e.to_string()
            })?
        } else {
            return Err("OpenAI client not configured".to_string());
        }
    } else {
        state.claude_client.send_message(request).await.map_err(|e| {
            error!(error = %e, context = context, "Claude one-shot call failed");
            e.to_string()
        })?
    };

    let text = collect_text_blocks(&resp.content);
    let usage = TokenUsage {
        input_tokens: resp.usage.input_tokens,
        output_tokens: resp.usage.output_tokens,
    };
    Ok((text, usage))
}

/// 비-EHS 질문에 대한 고정 거부 문구 (LLM 호출 없음 = 0토큰).
const BLOCKED_MESSAGE: &str = "저는 EHS AI입니다. 요청하신 내용은 EHS 업무 범위와 \
직접 관련이 없어 답변을 제공하지 않습니다.\n\n허가·점검·규정준수·사고·교육·보건환경 \
관련 업무를 도와드릴 수 있습니다. 더 궁금한 점 있으신가요?";

/// 0토큰 고정 거부 응답을 구성한다.
fn blocked_query_response(session_id: Uuid) -> QueryResponse {
    QueryResponse {
        session_id,
        agent_id: OTHER_AGENT_ID.to_string(),
        response: BLOCKED_MESSAGE.to_string(),
        tools_used: vec![],
        token_usage: TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
        },
        route_kind: "blocked".to_string(),
        routed_agent_ids: vec![OTHER_AGENT_ID.to_string()],
        old_new_comparison: None,
    }
}

/// LLM classify + keyword fallback merge. On any LLM/parse failure, returns the
/// keyword fallback decision.
async fn determine_routing_decision(
    state: &AppState,
    req: &QueryRequest,
) -> (RoutingDecision, TokenUsage) {
    let keyword_decision = routing_fallback(&req.content);
    let llm_request = LlmRequest {
        system_prompt:
            "You classify EHS questions into the correct specialist agents. Return JSON only."
                .to_string(),
        messages: vec![ClaudeMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::Text {
                text: build_routing_prompt(&req.content),
            }],
        }],
        tools: vec![],
        // Reasoning models (gpt-5.x) spend budget on hidden reasoning before emitting
        // output, so a small cap (e.g. 250) can yield empty content. Give enough room
        // for reasoning + the small JSON object.
        max_tokens: 2000,
        temperature: Some(0.1),
        cacheable: false,
    };

    match run_oneshot_llm(state, &llm_request, "smart routing").await {
        Ok((raw, token_usage)) => {
            let decision = extract_json_object(&raw)
                .and_then(|json| serde_json::from_str::<RoutingDecision>(json).ok())
                .map(|decision| {
                    merge_routing_decisions(&req.content, keyword_decision.clone(), decision)
                })
                .unwrap_or_else(|| {
                    warn!(response = %raw, "Routing JSON parse failed, using fallback");
                    keyword_decision.clone()
                });
            (decision, token_usage)
        }
        Err(e) => {
            warn!(error = %e, "Routing call failed, using fallback");
            (
                keyword_decision,
                TokenUsage {
                    input_tokens: 0,
                    output_tokens: 0,
                },
            )
        }
    }
}

fn build_multi_agent_fallback(
    question: &str,
    decision: &RoutingDecision,
    results: &[AgentQueryResult],
) -> String {
    let mut sections = vec![format!(
        "질문을 종합 라우팅했습니다. 대표 에이전트는 {}이며, 관련 영역은 {} 입니다.",
        agent_display_name(&decision.primary_agent_id),
        decision
            .agent_ids
            .iter()
            .map(|agent_id| agent_display_name(agent_id))
            .collect::<Vec<_>>()
            .join(", "),
    )];
    sections.push(format!("원문 질문: {}", question));
    for result in results {
        sections.push(format!(
            "[{}]\n{}",
            agent_display_name(&result.agent_id),
            truncate_for_prompt(&result.response, 1200)
        ));
    }
    sections.join("\n\n")
}

fn build_multi_agent_summary_prompt(
    question: &str,
    decision: &RoutingDecision,
    results: &[AgentQueryResult],
) -> String {
    let mut specialist_outputs = String::new();
    for result in results {
        specialist_outputs.push_str(&format!(
            "## {} ({})\n{}\n\n",
            agent_display_name(&result.agent_id),
            result.agent_id,
            truncate_for_prompt(&result.response, 2500)
        ));
    }

    format!(
        "You are the lead coordinator for an EHS multi-agent system. Answer in Korean.\n\
{table_rule}\n\
Synthesize the specialist outputs into one final response grounded only in the provided evidence.\n\
If the specialist outputs disagree or one area needs more verification, say so clearly.\n\
Use short markdown headings when useful.\n\
Question:\n{question}\n\
Routing rationale: {rationale}\n\
Selected agents: {agents}\n\
Specialist outputs:\n{specialist_outputs}",
        table_rule = KOREAN_TABLE_HEADER_RULE,
        rationale = decision.rationale,
        agents = decision.agent_ids.iter().map(|agent_id| format!("{} ({})", agent_display_name(agent_id), agent_id)).collect::<Vec<_>>().join(", "),
    )
}

/// Merge multiple specialist outputs into one final Korean answer via the LLM. On LLM
/// failure, returns a deterministic concatenated fallback.
async fn compose_multi_agent_response(
    state: &AppState,
    question: &str,
    decision: &RoutingDecision,
    results: &[AgentQueryResult],
) -> (String, TokenUsage) {
    let llm_request = LlmRequest {
        system_prompt: format!(
            "You combine EHS specialist outputs into one final Korean answer. {}",
            KOREAN_TABLE_HEADER_RULE
        ),
        messages: vec![ClaudeMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::Text {
                text: build_multi_agent_summary_prompt(question, decision, results),
            }],
        }],
        tools: vec![],
        // Allow room for reasoning-model hidden tokens before the final synthesis text.
        max_tokens: 4000,
        temperature: Some(0.2),
        cacheable: false,
    };

    match run_oneshot_llm(state, &llm_request, "multi-agent synthesis").await {
        Ok((text, usage)) => (text, usage),
        Err(e) => {
            warn!(error = %e, "Multi-agent synthesis failed, using fallback text");
            (
                build_multi_agent_fallback(question, decision, results),
                TokenUsage {
                    input_tokens: 0,
                    output_tokens: 0,
                },
            )
        }
    }
}

/// POST /api/query - Smart-route a query to one or more agents when no agent is selected.
async fn query_home(
    State(state): State<AppState>,
    OptionalAuth(auth): OptionalAuth,
    Json(req): Json<QueryRequest>,
) -> Result<Json<QueryResponse>, (StatusCode, Json<ErrorResponse>)> {
    let session_id = req.session_id.unwrap_or_else(Uuid::new_v4);
    let noop = |_: ProcessEvent| {};
    run_query_pipeline(&state, &req, session_id, &noop, false, &auth)
        .await
        .map(Json)
}

/// POST /api/query/stream - same smart-router pipeline as `query_home`, but emits
/// progress events as NDJSON (one JSON object per line) so the frontend can show
/// the live processing steps. The terminal line is `{"type":"final", ...QueryResponse}`.
async fn query_home_stream(
    State(state): State<AppState>,
    OptionalAuth(auth): OptionalAuth,
    Json(req): Json<QueryRequest>,
) -> axum::response::Response {
    use axum::body::{Body, Bytes};
    use tokio_stream::wrappers::UnboundedReceiverStream;

    let session_id = req.session_id.unwrap_or_else(Uuid::new_v4);
    let (tx, rx) =
        tokio::sync::mpsc::unbounded_channel::<Result<Bytes, std::convert::Infallible>>();

    tokio::spawn(async move {
        let emit = {
            let tx = tx.clone();
            move |event: ProcessEvent| {
                if let Some(line) = to_ndjson_line(&event) {
                    let _ = tx.send(Ok(Bytes::from(line)));
                }
            }
        };

        match run_query_pipeline(&state, &req, session_id, &emit, true, &auth).await {
            Ok(resp) => emit(ProcessEvent::Final(resp)),
            Err((_status, Json(body))) => {
                emit(ProcessEvent::Error { message: body.error })
            }
        }
    });

    let stream = UnboundedReceiverStream::new(rx);
    axum::response::Response::builder()
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(stream))
        .expect("failed to build streaming response")
}

/// Core smart-router pipeline shared by the buffered (`query_home`) and streaming
/// (`query_home_stream`) endpoints. `emit` receives progress events as they occur;
/// pass a no-op closure for the non-streaming path.
async fn run_query_pipeline(
    state: &AppState,
    req: &QueryRequest,
    session_id: Uuid,
    emit: &(impl Fn(ProcessEvent) + Send + Sync),
    stream: bool,
    auth: &Option<AuthIdentity>,
) -> Result<QueryResponse, (StatusCode, Json<ErrorResponse>)> {
    let conv_store = state.conversation_store.clone();

    info!(
        session_id = %session_id,
        content_len = req.content.len(),
        "Smart-router query received"
    );

    // Load conversation history for context, then append the user message.
    let _history = conv_store
        .get_conversation(&session_id)
        .await
        .unwrap_or_default();

    let _ = conv_store
        .append_message(&session_id, Message::user(&req.content))
        .await;

    emit(step_event("intent", "질문 의도 파악", None, StepPhase::Active));

    // 하이브리드 1단계: 키워드로 명백히 비-EHS면 LLM 호출 없이 즉시 차단(0토큰).
    let keyword_decision = routing_fallback(&req.content);
    if keyword_decision.rationale == "no_ehs_keyword_match" {
        emit(step_event("intent", "질문 의도 파악", None, StepPhase::Done));
        emit(step_event(
            "routing",
            "담당 에이전트 선정",
            Some(agent_display_name(OTHER_AGENT_ID).to_string()),
            StepPhase::Done,
        ));
        let _ = conv_store
            .append_message(&session_id, Message::assistant(BLOCKED_MESSAGE))
            .await;
        info!(
            session_id = %session_id,
            route_kind = "blocked",
            "Non-EHS query blocked at keyword stage (0 tokens)"
        );
        return Ok(blocked_query_response(session_id));
    }

    let (decision, routing_tokens) = determine_routing_decision(state, req).await;
    emit(step_event("intent", "질문 의도 파악", None, StepPhase::Done));

    // Report the routed agents (by display name) before answer generation.
    let routed_display_ids = if decision.agent_ids.len() <= 1 {
        vec![decision.primary_agent_id.clone()]
    } else {
        decision.agent_ids.clone()
    };
    let routed_display_names = routed_display_ids
        .iter()
        .map(|id| agent_display_name(id).to_string())
        .collect::<Vec<_>>();
    emit(step_event(
        "routing",
        "담당 에이전트 선정",
        Some(routed_display_names.join(" + ")),
        StepPhase::Done,
    ));

    // "기타": question unrelated to all 6 EHS domains. Answer generally without
    // running an EHS specialist agent, and label the agent as 기타.
    if decision.primary_agent_id == OTHER_AGENT_ID {
        // 하이브리드 2단계: 키워드는 통과했으나 LLM 라우터가 최종적으로 "기타"로
        // 판단한 애매한 질문. 답변 생성 LLM 호출 없이 고정 거부 메시지 반환
        // (라우팅 토큰만 소비).
        let _ = conv_store
            .append_message(&session_id, Message::assistant(BLOCKED_MESSAGE))
            .await;

        info!(
            session_id = %session_id,
            route_kind = "blocked",
            "Query classified as non-EHS by LLM router"
        );

        return Ok(QueryResponse {
            session_id,
            agent_id: OTHER_AGENT_ID.to_string(),
            response: BLOCKED_MESSAGE.to_string(),
            tools_used: vec![],
            token_usage: routing_tokens,
            route_kind: "blocked".to_string(),
            routed_agent_ids: vec![OTHER_AGENT_ID.to_string()],
            old_new_comparison: None,
        });
    }

    // Single-agent smart route.
    if decision.agent_ids.len() <= 1 {
        let result =
            execute_agent_query(state, &decision.primary_agent_id, req, session_id, emit, stream, auth)
                .await?;
        // Persist the assistant response.
        let _ = conv_store
            .append_message(&session_id, Message::assistant(&result.response))
            .await;

        let mut token_usage = result.token_usage.clone();
        add_token_usage(&mut token_usage, &routing_tokens);

        let routed_agent_ids = normalize_agent_ids(decision.agent_ids.clone());
        let response_routed_agent_ids = if routed_agent_ids.is_empty() {
            vec![result.agent_id.clone()]
        } else {
            routed_agent_ids
        };

        info!(
            session_id = %session_id,
            route_kind = "smart_single",
            agent_id = %result.agent_id,
            "Smart route resolved to single agent"
        );

        return Ok(QueryResponse {
            session_id,
            agent_id: result.agent_id,
            response: result.response,
            tools_used: result.tools_used,
            token_usage,
            route_kind: "smart_single".to_string(),
            routed_agent_ids: response_routed_agent_ids,
            old_new_comparison: result.old_new_comparison,
        });
    }

    // Multi-agent route: run each agent, then synthesize. Per-agent steps are
    // coarse (one step per agent) to keep the timeline readable.
    let noop = |_: ProcessEvent| {};
    let mut total_tokens = routing_tokens;
    let mut all_tools_used = Vec::new();
    let mut results = Vec::new();

    for agent_id in &decision.agent_ids {
        let agent_name = agent_display_name(agent_id);
        let step_id = format!("agent-{agent_id}");
        emit(step_event(
            &step_id,
            &format!("{agent_name} 처리"),
            None,
            StepPhase::Active,
        ));
        let result = execute_agent_query(state, agent_id, req, session_id, &noop, false, auth).await?;
        emit(step_event(
            &step_id,
            &format!("{agent_name} 처리"),
            None,
            StepPhase::Done,
        ));
        add_token_usage(&mut total_tokens, &result.token_usage);
        all_tools_used.extend(result.tools_used.iter().cloned());
        results.push(result);
    }

    emit(step_event(
        "compose",
        "통합 답변 생성",
        None,
        StepPhase::Active,
    ));
    let (summary_text, summary_tokens) =
        compose_multi_agent_response(state, &req.content, &decision, &results).await;
    add_token_usage(&mut total_tokens, &summary_tokens);
    emit(step_event("compose", "통합 답변 생성", None, StepPhase::Done));

    let _ = conv_store
        .append_message(&session_id, Message::assistant(&summary_text))
        .await;

    all_tools_used.sort();
    all_tools_used.dedup();

    let routed_agent_ids = normalize_agent_ids(decision.agent_ids.clone());
    let primary_agent_id = if routed_agent_ids
        .iter()
        .any(|agent_id| agent_id == &decision.primary_agent_id)
    {
        decision.primary_agent_id.clone()
    } else {
        routed_agent_ids
            .first()
            .cloned()
            .unwrap_or_else(|| "ehs-permit-agent".to_string())
    };
    let response_routed_agent_ids = if routed_agent_ids.is_empty() {
        vec![primary_agent_id.clone()]
    } else {
        routed_agent_ids
    };

    info!(
        session_id = %session_id,
        route_kind = "multi_agent",
        primary_agent_id = %primary_agent_id,
        routed_agent_ids = ?response_routed_agent_ids,
        "Smart route resolved to multiple agents"
    );

    Ok(QueryResponse {
        session_id,
        agent_id: primary_agent_id,
        response: summary_text,
        tools_used: all_tools_used,
        token_usage: total_tokens,
        route_kind: "multi_agent".to_string(),
        routed_agent_ids: response_routed_agent_ids,
        old_new_comparison: results.iter().find_map(|r| r.old_new_comparison.clone()),
    })
}

/// GET /api/sessions/:session_id/history - Get conversation history for a session
async fn get_session_history(
    Path(session_id): Path<Uuid>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let messages = state.conversation_store
        .get_conversation(&session_id)
        .await
        .unwrap_or_default();
    Json(messages)
}

/// GET /api/sessions/:session_id/summarize - Summarize a session into a title
async fn summarize_session(
    Path(session_id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<SummarizeResponse>, (StatusCode, Json<ErrorResponse>)> {
    let history = state.conversation_store
        .get_conversation(&session_id)
        .await
        .unwrap_or_default();
    
    if history.is_empty() {
        return Ok(Json(SummarizeResponse { title: "새로운 대화".to_string() }));
    }

    // 첫 번째 사용자 질문을 찾아 요약 대상으로 삼음
    let first_msg = history.iter()
        .find(|m| m.role == pekko_agent_core::MessageRole::User)
        .map(|m| m.content.as_str())
        .unwrap_or("새로운 대화");

    let prompt = format!(
        "다음 질문의 내용을 분석하여 대화의 주제를 잘 나타내는 아주 명확한 제목(20자 이내)을 한글로 작성해줘. \
         '오늘'이나 '내일' 같은 단어가 포함되면 각각 '오늘 날짜', '내일 날짜'로 변환하고, \
         '도와줘', '해줘', '알려줘' 등의 구어체 종결 어미는 반드시 제거하고 '요청' 또는 명사형으로 자연스럽게 요약해줘. \
         오직 요약된 제목만 출력해:\n\n질문: {}",
        first_msg
    );

    let llm_request = LlmRequest {
        system_prompt: "당신은 대화 요약 전문가입니다.".to_string(),
        messages: vec![ClaudeMessage {
            role: "user".to_string(),
            content: vec![ContentBlock::Text { text: prompt }],
        }],
        tools: vec![],
        max_tokens: 100,
        temperature: Some(0.3),
        cacheable: false,
    };

    let resp = if state.llm_provider == "openai" {
        if let Some(ref openai) = state.openai_client {
            openai.send_message(&llm_request).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("LLM 요약 실패: {e}"),
                        code: "LLM_ERROR".into(),
                    }),
                )
            })?
        } else {
            return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse { error: "OpenAI client not configured".into(), code: "LLM_ERROR".into() })));
        }
    } else {
        state.claude_client.send_message(&llm_request).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("LLM 요약 실패: {e}"),
                    code: "LLM_ERROR".into(),
                }),
            )
        })?
    };

    let mut title = String::new();
    for block in resp.content {
        if let ContentBlock::Text { text } = block {
            title.push_str(&text);
        }
    }

    let title = title.trim().replace("\"", "").replace("'", "");
    let title = if title.chars().count() > 20 {
        title.chars().take(17).collect::<String>() + "..."
    } else {
        title
    };

    Ok(Json(SummarizeResponse { title }))
}

/// GET /api/tools - List all available tools
async fn list_tools(State(state): State<AppState>) -> impl IntoResponse {
    let registry = state.tool_registry.read().await;
    Json(registry.list_tools())
}

/// Build a CORS layer that supports credentialed requests from the web-chat frontend.
///
/// The frontend issues requests with `credentials: include`, so the browser rejects a
/// wildcard `Access-Control-Allow-Origin: *` and requires an explicit origin together with
/// `Access-Control-Allow-Credentials: true`. Allowed origins default to the local trunk
/// dev server and can be overridden via the `CORS_ALLOWED_ORIGINS` env var (comma-separated).
fn build_cors_layer() -> CorsLayer {
    let raw = std::env::var("CORS_ALLOWED_ORIGINS")
        .unwrap_or_else(|_| "http://127.0.0.1:8081,http://localhost:8081".to_string());

    let origins: Vec<HeaderValue> = raw
        .split(',')
        .map(|o| o.trim().trim_end_matches('/'))
        .filter(|o| !o.is_empty())
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods(AllowMethods::list([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ]))
        .allow_headers(AllowHeaders::list([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
        ]))
        .allow_credentials(true)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env file
    dotenvy::dotenv().ok();

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()),
        )
        .json()
        .init();

    info!("Starting pekko-agent API Gateway");

    // Build LLM config from environment.
    // CLAUDE_API_KEY is optional: when LLM_PROVIDER is "openai" the Claude
    // client is never used, so we fall back to an empty key instead of panicking.
    let llm_config = LlmConfig {
        api_key: std::env::var("CLAUDE_API_KEY").unwrap_or_default(),
        model: std::env::var("CLAUDE_MODEL")
            .unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string()),
        max_tokens: std::env::var("CLAUDE_MAX_TOKENS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4096),
        ..LlmConfig::default()
    };

    info!(model = %llm_config.model, "Claude client configured (used only when LLM_PROVIDER=claude)");

    // Create Claude client
    let claude_client = Arc::new(ClaudeClient::new(llm_config.clone()));

    // Connect to PostgreSQL database
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://paulyu@localhost:5432/astgroup".to_string());

    let pg_pool = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to PostgreSQL. Set DATABASE_URL env var.");

    info!("Connected to PostgreSQL database");
    let pg_pool = Arc::new(pg_pool);

    // 임베디드 coreDB: 사용자별 일일 토큰 사용량을 영구 저장한다(PostgreSQL 테이블 불필요).
    let coredb_dir = std::path::PathBuf::from(
        std::env::var("COREDB_DATA_DIR").unwrap_or_else(|_| "./data/coredb".to_string()),
    );
    let coredb = Arc::new(
        CoreDB::new(DatabaseConfig {
            data_directory: coredb_dir.join("data"),
            commitlog_directory: coredb_dir.join("commitlog"),
            ..Default::default()
        })
        .await
        .expect("Failed to initialize CoreDB for token usage"),
    );
    // 대화 기록을 coreDB에 영구 저장한다(서버 재시작/창 닫힘 후에도 답변을 이어받기 위함).
    let conversation_store = Arc::new(
        CoreDbConversationStore::new(coredb.clone(), 100)
            .await
            .expect("Failed to initialize conversation store"),
    );

    // RSA keypair for login password encryption. Use LOGIN_RSA_PRIVATE_KEY (PEM) when set
    // for cross-restart / multi-instance stability; otherwise generate an ephemeral key.
    let (rsa_private_key, rsa_public_key_pem) = {
        use rsa::pkcs8::{DecodePrivateKey, EncodePublicKey};
        let private_key = match std::env::var("LOGIN_RSA_PRIVATE_KEY") {
            Ok(pem) => rsa::RsaPrivateKey::from_pkcs8_pem(pem.trim())
                .expect("Invalid LOGIN_RSA_PRIVATE_KEY (expected PKCS#8 PEM)"),
            Err(_) => {
                info!("LOGIN_RSA_PRIVATE_KEY unset — generating an ephemeral RSA-2048 keypair");
                let mut rng = rand::rngs::OsRng;
                rsa::RsaPrivateKey::new(&mut rng, 2048).expect("RSA key generation failed")
            }
        };
        let public_pem = rsa::RsaPublicKey::from(&private_key)
            .to_public_key_pem(rsa::pkcs8::LineEnding::LF)
            .expect("Failed to encode RSA public key");
        (Arc::new(private_key), Arc::<str>::from(public_pem))
    };

    // Initialize tool registry with built-in tools
    let mut tool_registry = ToolRegistry::new();
    tool_registry.register(Arc::new(PermitSearchTool));
    tool_registry.register(Arc::new(ComplianceCheckTool));
    tool_registry.register(Arc::new(EhsQueryTool::new(pg_pool.clone())));
    tool_registry.register(Arc::new(LawCompareTool::new(
        std::env::var("LAW_API_OC").unwrap_or_else(|_| "odaseul312".to_string()),
    )));

    info!(tools = tool_registry.list_tools().len(), "Tool registry initialized");

    // Create orchestrator and register agents
    let orchestrator = OrchestratorActor::new();
    let orchestrator = Arc::new(RwLock::new(orchestrator));

    // Register EHS agents
    {
        let mut orch = orchestrator.write().await;
        orch.register_agent(AgentInfo {
            agent_id: "ehs-permit-agent".to_string(),
            agent_type: "ehs".to_string(),
            description: "위험작업 허가(PTW) 관리 에이전트".to_string(),
            capabilities: vec!["permit_search".to_string(), "ehs_query".to_string()],
            status: AgentStatus::Available,
        });
        orch.register_agent(AgentInfo {
            agent_id: "ehs-inspection-agent".to_string(),
            agent_type: "ehs".to_string(),
            description: "안전점검 및 위험성 평가 에이전트".to_string(),
            capabilities: vec!["ehs_query".to_string()],
            status: AgentStatus::Available,
        });
        orch.register_agent(AgentInfo {
            agent_id: "ehs-compliance-agent".to_string(),
            agent_type: "ehs".to_string(),
            description: "법규/규정 준수 확인 에이전트".to_string(),
            capabilities: vec!["compliance_check".to_string(), "ehs_query".to_string()],
            status: AgentStatus::Available,
        });
        orch.register_agent(AgentInfo {
            agent_id: "ehs-accident-agent".to_string(),
            agent_type: "ehs".to_string(),
            description: "사고/재해 분석 및 재발방지 에이전트".to_string(),
            capabilities: vec!["ehs_query".to_string()],
            status: AgentStatus::Available,
        });
        orch.register_agent(AgentInfo {
            agent_id: "ehs-education-agent".to_string(),
            agent_type: "ehs".to_string(),
            description: "안전교육 계획 및 이수 관리 에이전트".to_string(),
            capabilities: vec!["ehs_query".to_string()],
            status: AgentStatus::Available,
        });
        orch.register_agent(AgentInfo {
            agent_id: "ehs-health-environment-agent".to_string(),
            agent_type: "ehs".to_string(),
            description: "보건·환경 및 비상대응 관리 에이전트".to_string(),
            capabilities: vec!["ehs_query".to_string()],
            status: AgentStatus::Available,
        });
        info!("Registered {} EHS agents", orch.list_agents().len());
    }

    // Determine LLM provider
    let llm_provider = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "claude".to_string());

    // Resolve the model that will actually serve requests for the selected provider
    let active_model = match llm_provider.as_str() {
        "openai" => std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "(OPENAI_MODEL unset)".to_string()),
        _ => llm_config.model.clone(),
    };
    info!(provider = %llm_provider, model = %active_model, "LLM provider selected");

    // Create OpenAI client if API key available
    let openai_client = std::env::var("OPENAI_API_KEY").ok().map(|key| {
        let model = std::env::var("OPENAI_MODEL").ok();
        Arc::new(OpenAIClient::new(key, model))
    });

    // Create application state
    let state = AppState {
        tool_registry: Arc::new(RwLock::new(tool_registry)),
        conversation_store,
        vector_store: Arc::new(InMemoryVectorStore::new()),
        episodic_store: Arc::new(InMemoryEpisodicStore::new()),
        orchestrator,
        event_publisher: Arc::new(EventPublisher::new("pekko-agent", 1024)),
        rbac: Arc::new(RwLock::new(RbacManager::new())),
        tenant_manager: Arc::new(RwLock::new(TenantManager::new())),
        audit_logger: Arc::new(AuditLogger::new(10000)),
        llm_config,
        claude_client,
        openai_client,
        llm_provider,
        pg_pool: pg_pool.clone(),
        rsa_private_key,
        rsa_public_key_pem,
    };

    // Build the router
    let port: u16 = std::env::var("API_GATEWAY_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8082);

    let app = Router::new()
        .route("/api/health", get(health_check))
        .route("/auth/public-key", get(auth::public_key_handler))
        .route("/auth/me", get(auth::me_handler))
        .route("/auth/login", post(auth::login_handler))
        .route("/auth/logout", post(auth::logout_handler))
        .route("/api/agents", get(list_agents))
        .route("/api/query", post(query_home))
        .route("/api/query/stream", post(query_home_stream))
        .route("/api/agents/:agent_id/query", post(query_agent))
        .route("/api/agents/:agent_id/query/stream", post(query_agent_stream))
        .route("/api/sessions/:session_id/history", get(get_session_history))
        .route("/api/sessions/:session_id/summarize", get(summarize_session))
        .route("/api/tools", get(list_tools))
        .layer(build_cors_layer())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // Bind to port and start server
    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("API Gateway listening on {}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Signal handler for graceful shutdown
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for ctrl+c");
    info!("Shutdown signal received, gracefully shutting down");
}
