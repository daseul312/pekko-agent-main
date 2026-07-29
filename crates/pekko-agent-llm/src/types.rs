use serde::{Deserialize, Serialize};

/// Claude API request
#[derive(Debug, Serialize)]
pub struct ClaudeRequest {
    pub model: String,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<ClaudeMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ClaudeTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ClaudeMessage {
    pub role: String,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Claude API response
#[derive(Debug, Deserialize)]
pub struct ClaudeResponse {
    pub id: String,
    pub content: Vec<ContentBlock>,
    pub model: String,
    pub stop_reason: String,
    pub usage: ClaudeUsage,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ClaudeUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl ClaudeUsage {
    pub fn total(&self) -> u32 { self.input_tokens + self.output_tokens }
}

/// LLM request/response
#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub system_prompt: String,
    pub messages: Vec<ClaudeMessage>,
    pub tools: Vec<ClaudeTool>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub cacheable: bool,
    /// 추론 모델(gpt-5.x, o-series)의 추론 예산. 허용값은 모델마다 다르다
    /// (gpt-5.5: `none|low|medium|high|xhigh`, 구세대 gpt-5: `minimal|low|medium|high`).
    /// 값 검증은 호출자(`/api/complete`)가 하고, 여기서는 문자열을 그대로 전달만 한다.
    /// `None` 이면 파라미터를 아예 보내지 않아 모델 기본값이 그대로 쓰인다.
    /// Claude 경로에서는 무시된다 — Anthropic 에 대응 파라미터가 없다.
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: Vec<ContentBlock>,
    pub stop_reason: String,
    pub usage: ClaudeUsage,
}

/// LLM configuration
#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub timeout_secs: u64,
    pub max_retries: u32,
    pub rate_limit_rpm: u32,
    pub token_budget_daily: u64,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: "https://api.anthropic.com".into(),
            model: "claude-sonnet-4-20250514".into(),
            max_tokens: 4096,
            temperature: 0.7,
            timeout_secs: 60,
            max_retries: 5,
            rate_limit_rpm: 1000,
            token_budget_daily: 1_000_000,
        }
    }
}
