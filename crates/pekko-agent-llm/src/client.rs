use crate::types::*;
use reqwest::Client;
use std::time::Duration;
use tracing::{info, warn};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("API error: status={status}, body={body}")]
    ApiError { status: u16, body: String },
    #[error("Network error: {0}")]
    NetworkError(String),
    #[error("Parse error: {0}")]
    ParseError(String),
    #[error("Timeout")]
    Timeout,
}

/// rate limit 응답 헤더에서 정수 값을 읽는다. 헤더가 없거나 정수가 아니면 None.
fn ratelimit_header(resp: &reqwest::Response, name: &str) -> Option<u64> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
}

/// Claude API Client
pub struct ClaudeClient {
    client: Client,
    config: LlmConfig,
}

impl ClaudeClient {
    pub fn new(config: LlmConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .expect("Failed to build HTTP client");
        Self { client, config }
    }

    pub async fn send_message(
        &self,
        request: &LlmRequest,
    ) -> Result<LlmResponse, ClientError> {
        let claude_req = ClaudeRequest {
            model: self.config.model.clone(),
            max_tokens: request.max_tokens,
            system: Some(request.system_prompt.clone()),
            messages: request.messages.clone(),
            tools: request.tools.clone(),
            temperature: request.temperature,
        };

        let mut retries = 0;
        loop {
            let resp = self
                .client
                .post(format!("{}/v1/messages", self.config.base_url))
                .header("x-api-key", &self.config.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&claude_req)
                .send()
                .await;

            match resp {
                Ok(response) if response.status().is_success() => {
                    let body: ClaudeResponse = response.json().await
                        .map_err(|e| ClientError::ParseError(e.to_string()))?;
                    
                    info!(
                        model = %body.model,
                        input_tokens = body.usage.input_tokens,
                        output_tokens = body.usage.output_tokens,
                        "Claude API call succeeded"
                    );

                    return Ok(LlmResponse {
                        content: body.content,
                        stop_reason: body.stop_reason,
                        usage: body.usage,
                    });
                }
                Ok(response) if response.status().as_u16() == 429 => {
                    // Anthropic은 정확히 몇 초 뒤에 재시도하면 되는지 `retry-after`
                    // 헤더(초)로 알려준다. 있으면 그만큼만 기다리고, 없으면 기존
                    // 지수 백오프로 폴백한다.
                    let retry_after = response
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.trim().parse::<u64>().ok());

                    // 관측용: 조직의 분당 토큰 rate limit 잔여/한도(있으면 로깅).
                    let tokens_remaining =
                        ratelimit_header(&response, "anthropic-ratelimit-tokens-remaining");
                    let tokens_limit =
                        ratelimit_header(&response, "anthropic-ratelimit-tokens-limit");

                    retries += 1;
                    if retries > self.config.max_retries {
                        let hint = match retry_after {
                            Some(s) => format!("retry after ~{}s", s),
                            None => "retry later".to_string(),
                        };
                        return Err(ClientError::NetworkError(format!(
                            "rate limited (429); {}",
                            hint
                        )));
                    }

                    // 서버가 알려준 대기 시간 우선, 없으면 2^retries 초.
                    let wait = retry_after.unwrap_or_else(|| 2u64.pow(retries));
                    warn!(
                        retry = retries,
                        wait_secs = wait,
                        retry_after = ?retry_after,
                        tokens_remaining = ?tokens_remaining,
                        tokens_limit = ?tokens_limit,
                        "Rate limited, waiting server-advised delay before retry"
                    );
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                }
                Ok(response) => {
                    let status = response.status();
                    let body = response.text().await.unwrap_or_default();
                    return Err(ClientError::ApiError { status: status.as_u16(), body });
                }
                Err(e) if retries < self.config.max_retries => {
                    retries += 1;
                    warn!(retry = retries, error = %e, "Request failed, retrying...");
                    tokio::time::sleep(Duration::from_secs(2u64.pow(retries))).await;
                }
                Err(e) => {
                    return Err(ClientError::NetworkError(e.to_string()));
                }
            }
        }
    }
}
