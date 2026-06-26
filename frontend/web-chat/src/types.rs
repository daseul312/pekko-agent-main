use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const BOARD_POST_ANALYZING_AGENT_ID: &str = "board-post-analyzing";
pub const COMPLIANCE_AGENT_ID: &str = "ehs-compliance-agent";
/// Sentinel agent id for questions unrelated to the 6 EHS domains; shown as "기타".
pub const OTHER_AGENT_ID: &str = "ehs-other";

#[derive(Clone, Debug, Serialize)]
pub struct QueryRequest {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,
    pub tenant_id: String,
    pub user_id: String,
    pub company_code: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct QueryResponse {
    pub session_id: Uuid,
    pub agent_id: String,
    pub response: String,
    pub tools_used: Vec<String>,
    pub token_usage: TokenUsage,
    #[serde(default)]
    pub route_kind: String,
    #[serde(default)]
    pub routed_agent_ids: Vec<String>,
    #[serde(default)]
    pub old_new_comparison: Option<OldNewComparison>,
    #[serde(default)]
    pub chart_data: Option<ChartData>,
}

/// Live progress event streamed from the `/stream` endpoints (NDJSON).
/// `Step` is a generic, upsert-by-label progress item.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProcessEvent {
    Step {
        label: String,
        #[serde(default)]
        detail: Option<String>,
        status: StepStatus,
    },
    Final(QueryResponse),
    Error { message: String },
}

/// Status of a single processing step shown in the "답변 과정 보기" panel.
/// `snake_case` to match the backend wire values ("active"/"done").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Active,
    Done,
}

/// A single processing step (e.g. "질문 의도 파악"). `detail` carries extra info
/// such as the routed agent names ("점검관리 + 교육 관리").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProcessStep {
    pub label: String,
    #[serde(default)]
    pub detail: Option<String>,
    pub status: StepStatus,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResponseVersion {
    pub content: String,
    pub tools_used: Vec<String>,
    pub token_usage: Option<TokenUsage>,
    #[serde(default)]
    pub suggestions: Option<Vec<String>>,
    #[serde(default)]
    pub old_new_comparison: Option<OldNewComparison>,
    #[serde(default)]
    pub chart_data: Option<ChartData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OldNewComparison {
    pub source_mode: String,
    pub law_name: String,
    pub old_law: OldNewLawInfo,
    pub new_law: OldNewLawInfo,
    #[serde(default)]
    pub article_comparisons: Vec<OldNewArticleComparison>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OldNewLawInfo {
    pub law_name: Option<String>,
    pub promulgation_date: Option<String>,
    pub promulgation_number: Option<String>,
    pub enforcement_date: Option<String>,
    pub amendment_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OldNewArticleComparison {
    pub article_no: Option<String>,
    pub title: Option<String>,
    pub change_type: Option<String>,
    pub old_text: Option<String>,
    pub new_text: Option<String>,
}

/// 백엔드 `ChartData`(api-gateway/src/main.rs)와 1:1로 대응하는 차트 구조.
/// `visualize` 도구가 호출됐을 때만 채워진다. ChartBlock 컴포넌트가 Chart.js로 렌더.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChartData {
    /// "bar" | "donut" | "line".
    pub chart_type: String,
    pub title: String,
    pub labels: Vec<String>,
    pub series: Vec<ChartSeries>,
    #[serde(default)]
    pub insight: Option<ChartInsight>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChartSeries {
    pub name: String,
    pub values: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChartInsight {
    /// "info" | "warn" | "alert".
    pub level: String,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SummarizeResponse {
    pub title: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HealthResponse {
    pub status: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ChatMessageUi {
    BoardPostMenu,
    BoardPostEditMenu,
    BoardPostDeleteMenu,
    BoardPostForm,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoardPostTarget {
    pub menu_id: i32,
    pub board_name: String,
    #[serde(default)]
    pub top_menu_id: Option<i32>,
    #[serde(default)]
    pub middle_menu_id: Option<i32>,
    #[serde(default)]
    pub sub_menu_id: Option<i32>,
    pub top_menu_name: Option<String>,
    pub middle_menu_name: Option<String>,
    pub sub_menu_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct BoardPostMenu {
    pub menu_id: i32,
    pub menu_name: String,
    pub parent_menu_id: Option<i32>,
    pub depth: Option<i32>,
    pub selectable: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BoardPostMenuResponse {
    pub menus: Vec<BoardPostMenu>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BoardPostCreateRequest {
    pub menu_id: i32,
    pub title: String,
    pub contents: String,
    pub is_fixed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_start_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_end_date: Option<String>,
    pub attachments: Vec<BoardPostAttachmentRequest>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BoardPostAttachmentRequest {
    pub fno: Option<i32>,
    pub uuid: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BoardPostCreateResponse {
    pub board_name: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub post_id: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct BoardPostListResponse {
    pub posts: Vec<BoardPostListItem>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct BoardPostListItem {
    pub post_id: i32,
    pub bno: i32,
    pub title: String,
    pub employeename: String,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct BoardPostDetail {
    pub board_name: String,
    pub post_id: i32,
    pub bno: i32,
    pub title: String,
    pub contents: String,
    pub is_fixed: bool,
    pub fixed_start_date: Option<String>,
    pub fixed_end_date: Option<String>,
    pub attachments: Vec<BoardPostExistingAttachment>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct BoardPostExistingAttachment {
    pub fno: Option<i32>,
    pub uuid: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BoardPostUpdateRequest {
    pub title: String,
    pub contents: String,
    pub is_fixed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_start_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_end_date: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BoardPostUpdateResponse {
    pub board_name: String,
    #[allow(dead_code)]
    pub post_id: i32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BoardPostDeleteResponse {
    pub board_name: String,
    #[allow(dead_code)]
    pub post_id: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoardPostCompletion {
    pub board_name: String,
    pub message: String,
    pub success: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    #[serde(default)]
    pub request_content: Option<String>,
    pub tools_used: Vec<String>,
    pub token_usage: Option<TokenUsage>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub suggestions: Option<Vec<String>>,
    #[serde(default)]
    pub old_new_comparison: Option<OldNewComparison>,
    #[serde(default)]
    pub chart_data: Option<ChartData>,
    #[serde(default)]
    pub response_versions: Vec<ResponseVersion>,
    #[serde(default)]
    pub current_response_index: usize,
    #[serde(default)]
    pub ui: Option<ChatMessageUi>,
    #[serde(default)]
    pub board_post_target: Option<BoardPostTarget>,
    /// Processing steps captured while this answer was generated (streaming).
    /// `#[serde(default)]` keeps older persisted sessions deserializable.
    #[serde(default)]
    pub process_steps: Vec<ProcessStep>,
    /// 자동 라우팅 근거 한 줄 안내(예: "🧭 자동 라우팅: 허가 관리 에이전트가 답변했어요.").
    /// 사용자가 직접 에이전트를 고른 경우(direct)나 차단(blocked)에는 `None`.
    #[serde(default)]
    pub routing_note: Option<String>,
    /// 이 답변을 생성한 에이전트 id(피드백 집계용). assistant 메시지에만 채워진다.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// 이 답변에 대한 내 피드백(👍/👎) 상태. 토글 하이라이트 표시용.
    #[serde(default)]
    pub feedback: Option<FeedbackRating>,
}

/// 답변 피드백 평가. 백엔드의 rating 문자열("up"/"down")과 대응한다.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedbackRating {
    Up,
    Down,
}

impl FeedbackRating {
    /// 백엔드로 보낼 rating 문자열.
    pub fn as_str(self) -> &'static str {
        match self {
            FeedbackRating::Up => "up",
            FeedbackRating::Down => "down",
        }
    }
}

/// UI에서 상위(main)로 올리는 피드백 액션 페이로드.
#[derive(Clone, Debug, PartialEq)]
pub struct FeedbackSubmission {
    /// 대상 assistant 메시지 id.
    pub message_id: String,
    /// 대상 답변 버전 인덱스(현재 표시 중인 버전).
    pub response_index: i64,
    /// 답변 에이전트 id(없으면 빈 문자열).
    pub agent_id: String,
    /// 답변 본문 앞부분 스냅샷(분석 맥락용).
    pub content_excerpt: String,
    /// 누른 평가.
    pub rating: FeedbackRating,
    /// 선택적 코멘트(주로 👎).
    pub comment: Option<String>,
    /// 부정 사유 태그(👎 다중 선택).
    pub tags: Vec<String>,
    /// true=평가 등록/변경, false=같은 평가 재클릭으로 토글 취소(삭제).
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

impl PartialEq for TokenUsage {
    fn eq(&self, other: &Self) -> bool {
        self.input_tokens == other.input_tokens && self.output_tokens == other.output_tokens
    }
}

impl ChatMessage {
    pub fn response_page_count(&self) -> usize {
        if matches!(self.role, MessageRole::Assistant | MessageRole::System) {
            self.response_versions.len().max(1)
        } else {
            1
        }
    }

    pub fn current_response_page(&self) -> usize {
        if self.response_versions.is_empty() {
            0
        } else {
            self.current_response_index
                .min(self.response_versions.len() - 1)
        }
    }

    pub fn ensure_response_versions(&mut self) {
        if !matches!(self.role, MessageRole::Assistant | MessageRole::System)
            || !self.response_versions.is_empty()
        {
            return;
        }

        self.response_versions.push(ResponseVersion {
            content: self.content.clone(),
            tools_used: self.tools_used.clone(),
            token_usage: self.token_usage.clone(),
            suggestions: self.suggestions.clone(),
            old_new_comparison: self.old_new_comparison.clone(),
            chart_data: self.chart_data.clone(),
        });
        self.current_response_index = 0;
    }

    pub fn sync_current_response(&mut self) {
        if self.response_versions.is_empty() {
            return;
        }

        let index = self.current_response_page();
        self.current_response_index = index;
        let version = self.response_versions[index].clone();
        self.content = version.content;
        self.tools_used = version.tools_used;
        self.token_usage = version.token_usage;
        self.suggestions = version.suggestions;
        self.old_new_comparison = version.old_new_comparison;
        self.chart_data = version.chart_data;
    }

    pub fn append_response_version(&mut self, version: ResponseVersion) {
        if !matches!(self.role, MessageRole::Assistant | MessageRole::System) {
            return;
        }

        self.ensure_response_versions();
        self.response_versions.push(version);
        self.current_response_index = self.response_versions.len().saturating_sub(1);
        self.sync_current_response();
    }

    pub fn set_response_page(&mut self, index: usize) {
        if !matches!(self.role, MessageRole::Assistant | MessageRole::System) {
            return;
        }

        self.ensure_response_versions();
        if self.response_versions.is_empty() {
            return;
        }

        self.current_response_index = index.min(self.response_versions.len() - 1);
        self.sync_current_response();
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentMeta {
    pub id: String,
    pub name: String,
    pub description: String,
    pub icon: &'static str,
    pub css_class: &'static str,
}

impl AgentMeta {
    pub fn defaults() -> Vec<Self> {
        vec![
            AgentMeta {
                id: "ehs-permit-agent".into(),
                name: "허가 관리".into(),
                description: "위험작업 허가와 TBM 관리".into(),
                icon: "",
                css_class: "permit",
            },
            AgentMeta {
                id: "ehs-inspection-agent".into(),
                name: "점검 관리".into(),
                description: "위험성평가와 순회점검 관리".into(),
                icon: "",
                css_class: "inspection",
            },
            AgentMeta {
                id: "ehs-compliance-agent".into(),
                name: "규정 준수".into(),
                description: "법령, SOP, 위원회, MSDS 점검".into(),
                icon: "",
                css_class: "compliance",
            },
            AgentMeta {
                id: "ehs-accident-agent".into(),
                name: "사고 관리".into(),
                description: "사고, 아차사고, 시정조치, 대외문서 관리".into(),
                icon: "",
                css_class: "accident",
            },
            AgentMeta {
                id: "ehs-education-agent".into(),
                name: "교육 관리".into(),
                description: "안전교육, 교육계획, 설문, 알림 관리".into(),
                icon: "",
                css_class: "education",
            },
            AgentMeta {
                id: "ehs-health-environment-agent".into(),
                name: "보건·환경 관리".into(),
                description: "유해물질, 작업환경, 건강검진, 비상대응 관리".into(),
                icon: "",
                css_class: "health-environment",
            },
        ]
    }
}

/// 표시용 로그인 사용자(실제 인증은 HttpOnly 쿠키가 보유). 새 쿠키 기반 로그인에서 사용.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AuthUser {
    pub id: String,
    pub name: String,
    pub company_code: String,
    pub role: String,
    /// 프로필 이미지 URL(S3 전체 URL, 없으면 None).
    pub profile_image_url: Option<String>,
    /// 피드백 통계(관리자 페이지) 열람 권한. 헤더의 관리자 버튼 노출 판정용.
    #[serde(default)]
    pub is_feedback_admin: bool,
}

/// `GET /api/sessions/:id/history` 응답의 단일 메시지(폴링으로 답변 이어받기용).
/// 백엔드 `MessageRole`은 PascalCase로 직렬화되므로 role은 "User"/"Assistant"/"System" 문자열.
#[derive(Clone, Debug, Deserialize)]
pub struct HistoryMessage {
    pub role: String,
    pub content: String,
    /// 답변에 그래프가 있을 때만 채워지는 차트 JSON 문자열(읽기전용 대화 뷰어가 파싱해 렌더).
    #[serde(default)]
    pub chart_data: Option<String>,
}

/// `POST /api/share` 요청. 답변 본문을 공유 시점 스냅샷으로 전송한다.
#[derive(Clone, Debug, Serialize)]
pub struct ShareCreateRequest {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    /// 차트(`ChartData`)를 직렬화한 JSON 문자열. 답변에 그래프가 있을 때만 채워진다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chart_data: Option<String>,
}

/// `POST /api/share` 응답.
#[derive(Clone, Debug, Deserialize)]
pub struct ShareCreateResponse {
    pub share_url: String,
}

/// `GET /api/admin/feedback/stats` 응답. 회사+기간 단위 대시보드 집계.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct FeedbackStatsResponse {
    pub total_up: i64,
    pub total_down: i64,
    pub comment_count: i64,
    pub agents: Vec<AgentRate>,
    pub tag_counts: Vec<TagCount>,
    pub trend: Vec<TrendPoint>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct AgentRate {
    pub agent_id: String,
    pub up: i64,
    pub down: i64,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct TagCount {
    pub tag: String,
    pub count: i64,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct TrendPoint {
    pub month: String,
    pub up: i64,
    pub down: i64,
}

/// `GET /api/admin/feedback/comments` 응답(페이지네이션).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct FeedbackCommentsResponse {
    pub items: Vec<CommentRow>,
    pub total: i64,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct CommentRow {
    pub agent_id: String,
    pub rating: String,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// 원본 대화 세션 id(빈 문자열이면 바로가기 불가).
    #[serde(default)]
    pub session_id: String,
    pub created_at: String,
}

/// `POST /api/feedback` 요청. 답변 1개(특정 버전)에 대한 👍/👎 평가.
#[derive(Clone, Debug, Serialize)]
pub struct FeedbackRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<Uuid>,
    pub message_id: String,
    pub response_index: i64,
    pub agent_id: String,
    /// "up" | "down".
    pub rating: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_excerpt: Option<String>,
    /// 부정 사유 태그(👎 다중 선택).
    #[serde(default)]
    pub tags: Vec<String>,
    /// false면 토글 취소(평가 삭제).
    pub active: bool,
}

/// `GET /api/share/:id` 응답(공유된 답변 본문).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct SharedAnswer {
    pub content: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// 차트 JSON 문자열(있을 때만). 열람 페이지가 파싱해 `ChartBlock`으로 렌더.
    #[serde(default)]
    pub chart_data: Option<String>,
}

/// 공유 답변 조회 실패 종류(열람 페이지가 상태별로 다른 안내를 띄우기 위함).
#[derive(Clone, Debug, PartialEq)]
pub enum ShareFetchError {
    /// 401 — 미로그인.
    Unauthorized,
    /// 403 — 다른 회사 계정.
    Forbidden,
    /// 404 — 없는 공유 링크.
    NotFound,
    /// 기타(네트워크/서버 오류 등). 사용자 표시용 메시지 포함.
    Other(String),
}

/// 피드백 통계 조회 실패 종류(관리자 페이지가 상태별로 다른 안내를 띄우기 위함).
#[derive(Clone, Debug, PartialEq)]
pub enum FeedbackStatsError {
    /// 401 — 미로그인.
    Unauthorized,
    /// 403 — 허용목록 외(관리자 아님).
    Forbidden,
    /// 기타(네트워크/서버 오류 등).
    Other(String),
}

#[derive(Clone, Debug, Serialize)]
pub struct LoginRequest {
    pub employee_id: String,
    pub company_code: String,
    /// RSA-OAEP(SHA-256)로 암호화한 비밀번호의 base64 문자열.
    pub password_encrypted: String,
}

/// 게이트웨이 `GET /auth/public-key` 응답(비밀번호 암호화용 RSA 공개키 PEM).
#[derive(Clone, Debug, Deserialize)]
pub struct PublicKeyResponse {
    pub public_key_pem: String,
}

/// 게이트웨이 `POST /auth/login` 응답(쿠키는 별도로 브라우저가 저장).
#[derive(Clone, Debug, Deserialize)]
pub struct LoginResponse {
    pub employee_id: String,
    pub employee_name: String,
    pub company_code: String,
    pub role: String,
    #[allow(dead_code)]
    pub expires_at: i64,
    #[serde(default)]
    pub profile_image_url: Option<String>,
    #[serde(default)]
    pub is_feedback_admin: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: Uuid,
    pub agent_id: String,
    pub title: String,
    pub last_message: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub is_pinned: bool,
    #[serde(default)]
    pub use_router: bool,
    #[serde(default)]
    pub routed_agent_ids: Vec<String>,
    /// 답변 대기 중(질문은 보냈으나 답변 미도착). 창을 닫아도 유지되어 재진입 시 폴링 대상이 된다.
    #[serde(default)]
    pub pending: bool,
    /// 사용자가 보지 않은 사이에 답변이 완료됨(재접속 시 '답변 완료' 배지 표시용).
    #[serde(default)]
    pub unread: bool,
}

/// 즐겨찾기 질문 템플릿. 사용자가 자주 쓰는 질문을 ⭐로 저장해 원클릭으로 재질문한다.
/// localStorage(`pekko_favorite_questions`)에 보관하며, 저장 당시 에이전트(`agent_id`)에
/// 고정 라우팅된다. 중복 판정은 `(agent_id, question)` 쌍 기준.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FavoriteQuestion {
    pub id: Uuid,
    pub agent_id: String,
    pub question: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
