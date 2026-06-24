use gloo_net::http::{Request, RequestBuilder};
use uuid::Uuid;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Blob, File, FormData, RequestCredentials};

use crate::types::*;

const API_BASE_URL: Option<&str> = option_env!("WEB_CHAT_API_BASE_URL");
const API_PORT: &str = "8082";
const SESSION_OWNER_TOKEN_HEADER: &str = "x-session-owner-token";
pub const SESSION_EXPIRED_MESSAGE: &str = "로그인 세션이 만료되었습니다. 다시 로그인해주세요.";

pub fn is_auth_user_expired(user: &AuthUser) -> bool {
    user.expires_at
        .map(|expires_at| chrono::Utc::now().timestamp() >= expires_at)
        .unwrap_or(false)
}

pub fn is_session_expired_error(error: &str) -> bool {
    error == SESSION_EXPIRED_MESSAGE
}
const LOCAL_API_BASE_URL: &str = "http://127.0.0.1:8082";

fn configured_base_url() -> Option<String> {
    API_BASE_URL.and_then(|base_url| {
        let trimmed = base_url.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn runtime_base_url() -> Option<String> {
    let window = web_sys::window()?;
    let location = window.location();
    let hostname = location.hostname().ok()?;

    if matches!(
        hostname.as_str(),
        "localhost" | "127.0.0.1" | "0.0.0.0" | "::1" | "[::1]"
    ) {
        return Some(LOCAL_API_BASE_URL.to_string());
    }

    let protocol = location.protocol().ok().filter(|value| !value.is_empty())?;
    Some(format!("{protocol}//{hostname}:{API_PORT}"))
}

fn base_url() -> String {
    if let Some(runtime_base) = runtime_base_url() {
        if runtime_base == LOCAL_API_BASE_URL {
            return runtime_base;
        }

        if let Some(configured_base) = configured_base_url() {
            return configured_base;
        }

        return runtime_base;
    }

    configured_base_url().unwrap_or_else(|| LOCAL_API_BASE_URL.to_string())
}

fn api_url(path: &str) -> String {
    let base = base_url();
    format!("{base}{path}")
}

fn with_credentials(request: RequestBuilder) -> RequestBuilder {
    request.credentials(RequestCredentials::Include)
}

/// GET /api/health
pub async fn fetch_health() -> Result<HealthResponse, String> {
    let url = api_url("/api/health");
    let resp = with_credentials(Request::get(&url))
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if resp.ok() {
        resp.json::<HealthResponse>()
            .await
            .map_err(|e| format!("Parse error: {e}"))
    } else {
        let err = resp
            .json::<ErrorResponse>()
            .await
            .map(|e| e.error)
            .unwrap_or_else(|_| format!("HTTP {}", resp.status()));
        Err(err)
    }
}

/// GET /api/sessions/:session_id/summarize
pub async fn fetch_session_summary(
    session_id: Uuid,
    _token: Option<&str>,
    session_owner_token: Option<&str>,
) -> Result<String, String> {
    let url = api_url(&format!("/api/sessions/{}/summarize", session_id));
    let mut request = with_credentials(Request::get(&url));
    if let Some(owner_token) = session_owner_token {
        request = request.header(SESSION_OWNER_TOKEN_HEADER, owner_token);
    }
    let resp = request
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if resp.ok() {
        let res = resp
            .json::<SummarizeResponse>()
            .await
            .map_err(|e| format!("Parse error: {e}"))?;
        Ok(res.title)
    } else {
        let err = resp
            .json::<ErrorResponse>()
            .await
            .map(|e| e.error)
            .unwrap_or_else(|_| format!("HTTP {}", resp.status()));
        Err(err)
    }
}

/// POST /auth/login
pub async fn login(employee_id: String, password: String) -> Result<AuthUser, String> {
    let url = api_url("/auth/login");
    let body = LoginRequest {
        employee_id,
        company_code: "10".to_string(),
        password,
    };

    let resp = with_credentials(Request::post(&url))
        .header("Content-Type", "application/json")
        .json(&body)
        .map_err(|e| format!("Serialize error: {e}"))?
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if resp.ok() {
        let login = resp
            .json::<LoginResponse>()
            .await
            .map_err(|e| format!("Parse error: {e}"))?;

        let employee_name = login
            .employee_name
            .clone()
            .unwrap_or_else(|| login.employee_id.clone());

        Ok(AuthUser {
            id: login.employee_id,
            name: employee_name,
            company_code: login.company_code,
            role: login.role,
            expires_at: Some(login.expires_at),
        })
    } else {
        let err = resp
            .json::<ErrorResponse>()
            .await
            .map(|e| e.error)
            .unwrap_or_else(|_| format!("HTTP {}", resp.status()));
        Err(err)
    }
}

pub async fn logout() {
    let url = api_url("/auth/logout");
    let _ = with_credentials(Request::post(&url)).send().await;
}

/// POST /api/query or /api/agents/:agent_id/query (buffered, non-streaming).
/// Retained as a fallback; the live UI uses `send_query_stream`.
#[allow(dead_code)]
pub async fn send_query(
    agent_id: Option<&str>,
    content: &str,
    session_id: Option<Uuid>,
    _token: Option<&str>,
    session_owner_token: Option<&str>,
) -> Result<QueryResponse, String> {
    let url = match agent_id {
        Some(agent_id) => api_url(&format!("/api/agents/{}/query", agent_id)),
        None => api_url("/api/query"),
    };
    let body = QueryRequest {
        content: content.to_string(),
        session_id,
        tenant_id: "default".to_string(),
        user_id: "web-user".to_string(),
        company_code: "10".to_string(),
        session_owner_token: session_owner_token.map(str::to_string),
    };

    let request = with_credentials(Request::post(&url)).header("Content-Type", "application/json");

    let resp = request
        .json(&body)
        .map_err(|e| format!("Serialize error: {e}"))?
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if resp.ok() {
        resp.json::<QueryResponse>()
            .await
            .map_err(|e| format!("Parse error: {e}"))
    } else if resp.status() == 401 {
        Err(SESSION_EXPIRED_MESSAGE.to_string())
    } else {
        let err = resp
            .json::<ErrorResponse>()
            .await
            .map(|e| e.error)
            .unwrap_or_else(|_| format!("HTTP {}", resp.status()));
        Err(err)
    }
}

/// POST /api/query/stream — smart-router query with live NDJSON progress events.
///
/// Calls `on_event` for each `ProcessEvent` as it arrives from the backend and
/// returns the final `QueryResponse` carried by the terminal `Final` event.
/// Used only for the router path (no specific agent selected).
pub async fn send_query_stream(
    agent_id: Option<&str>,
    content: &str,
    session_id: Option<Uuid>,
    session_owner_token: Option<&str>,
    mut on_event: impl FnMut(ProcessEvent),
) -> Result<QueryResponse, String> {
    let url = match agent_id {
        Some(agent_id) => api_url(&format!("/api/agents/{}/query/stream", agent_id)),
        None => api_url("/api/query/stream"),
    };
    let body = QueryRequest {
        content: content.to_string(),
        session_id,
        tenant_id: "default".to_string(),
        user_id: "web-user".to_string(),
        company_code: "10".to_string(),
        session_owner_token: session_owner_token.map(str::to_string),
    };
    let body_json = serde_json::to_string(&body).map_err(|e| format!("Serialize error: {e}"))?;

    // Build the fetch request manually so we can read the streaming body.
    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_mode(web_sys::RequestMode::Cors);
    opts.set_credentials(RequestCredentials::Include);
    opts.set_body(&JsValue::from_str(&body_json));

    let headers = web_sys::Headers::new().map_err(js_err)?;
    let _ = headers.set("Content-Type", "application/json");
    if let Some(token) = session_owner_token {
        let _ = headers.set(SESSION_OWNER_TOKEN_HEADER, token);
    }
    opts.set_headers(&headers);

    let request =
        web_sys::Request::new_with_str_and_init(&url, &opts).map_err(js_err)?;

    let window = web_sys::window().ok_or_else(|| "No window".to_string())?;
    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("Network error: {}", js_err(e)))?;
    let resp: web_sys::Response = resp_value.dyn_into().map_err(js_err)?;

    if resp.status() == 401 {
        return Err(SESSION_EXPIRED_MESSAGE.to_string());
    }
    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let stream = resp.body().ok_or_else(|| "No response body".to_string())?;
    let reader: web_sys::ReadableStreamDefaultReader = stream
        .get_reader()
        .dyn_into()
        .map_err(|_| "스트림 리더를 가져오지 못했습니다.".to_string())?;

    let mut buffer: Vec<u8> = Vec::new();
    let mut final_response: Option<QueryResponse> = None;
    let mut error_message: Option<String> = None;

    loop {
        let result = JsFuture::from(reader.read())
            .await
            .map_err(|e| format!("Stream read error: {}", js_err(e)))?;

        let done = js_sys::Reflect::get(&result, &JsValue::from_str("done"))
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let value = js_sys::Reflect::get(&result, &JsValue::from_str("value")).map_err(js_err)?;

        if !value.is_undefined() && !value.is_null() {
            let chunk: js_sys::Uint8Array = value.dyn_into().map_err(js_err)?;
            let mut bytes = vec![0u8; chunk.length() as usize];
            chunk.copy_to(&mut bytes);
            buffer.extend_from_slice(&bytes);

            // Drain complete NDJSON lines. A 0x0A byte never occurs inside a
            // UTF-8 multibyte sequence, so splitting raw bytes on '\n' is safe.
            while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buffer.drain(..=pos).collect();
                let trimmed = &line[..line.len() - 1];
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(text) = std::str::from_utf8(trimmed) {
                    if let Ok(event) = serde_json::from_str::<ProcessEvent>(text) {
                        match &event {
                            ProcessEvent::Final(resp) => final_response = Some(resp.clone()),
                            ProcessEvent::Error { message } => {
                                error_message = Some(message.clone())
                            }
                            _ => {}
                        }
                        on_event(event);
                    }
                }
            }
        }

        if done {
            break;
        }
    }

    final_response.ok_or_else(|| {
        error_message.unwrap_or_else(|| "스트리밍 응답이 완료되지 않았습니다.".to_string())
    })
}

/// Render a `JsValue` error into a readable string.
fn js_err(value: JsValue) -> String {
    value
        .as_string()
        .or_else(|| {
            js_sys::Reflect::get(&value, &JsValue::from_str("message"))
                .ok()
                .and_then(|m| m.as_string())
        })
        .unwrap_or_else(|| "JS error".to_string())
}

pub async fn fetch_board_post_menus(_token: Option<&str>) -> Result<Vec<BoardPostMenu>, String> {
    let url = api_url("/api/board-post/menus");
    let request = with_credentials(Request::get(&url));

    let resp = request
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if resp.ok() {
        let res = resp
            .json::<BoardPostMenuResponse>()
            .await
            .map_err(|e| format!("Parse error: {e}"))?;
        Ok(res.menus)
    } else {
        let err = resp
            .json::<ErrorResponse>()
            .await
            .map(|e| e.error)
            .unwrap_or_else(|_| format!("HTTP {}", resp.status()));
        Err(err)
    }
}

#[allow(dead_code)]
pub async fn create_board_post(
    body: &BoardPostCreateRequest,
    _token: Option<&str>,
) -> Result<BoardPostCreateResponse, String> {
    let url = api_url("/api/board-post");
    let request = with_credentials(Request::post(&url)).header("Content-Type", "application/json");

    let resp = request
        .json(body)
        .map_err(|e| format!("Serialize error: {e}"))?
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if resp.ok() {
        resp.json::<BoardPostCreateResponse>()
            .await
            .map_err(|e| format!("Parse error: {e}"))
    } else {
        let err = resp
            .json::<ErrorResponse>()
            .await
            .map(|e| e.error)
            .unwrap_or_else(|_| format!("HTTP {}", resp.status()));
        Err(err)
    }
}

pub async fn create_board_post_form(
    body: &BoardPostCreateRequest,
    files: &[File],
    _token: Option<&str>,
) -> Result<BoardPostCreateResponse, String> {
    let url = api_url("/api/board-post/form");
    let form = FormData::new().map_err(js_error)?;

    form.append_with_str("menu_id", &body.menu_id.to_string())
        .map_err(js_error)?;
    form.append_with_str("title", &body.title)
        .map_err(js_error)?;
    form.append_with_str("contents", &body.contents)
        .map_err(js_error)?;
    form.append_with_str("is_fixed", if body.is_fixed { "true" } else { "false" })
        .map_err(js_error)?;

    if let Some(value) = &body.fixed_start_date {
        form.append_with_str("fixed_start_date", value)
            .map_err(js_error)?;
    }
    if let Some(value) = &body.fixed_end_date {
        form.append_with_str("fixed_end_date", value)
            .map_err(js_error)?;
    }

    for file in files {
        let blob: &Blob = file.as_ref();
        form.append_with_blob_and_filename("file", blob, &file.name())
            .map_err(js_error)?;
    }

    let request = with_credentials(Request::post(&url));

    let resp = request
        .body(form)
        .map_err(|e| format!("Serialize error: {e}"))?
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if resp.ok() {
        resp.json::<BoardPostCreateResponse>()
            .await
            .map_err(|e| format!("Parse error: {e}"))
    } else {
        let err = resp
            .json::<ErrorResponse>()
            .await
            .map(|e| e.error)
            .unwrap_or_else(|_| format!("HTTP {}", resp.status()));
        Err(err)
    }
}

pub async fn fetch_board_posts(
    menu_id: i32,
    _token: Option<&str>,
) -> Result<Vec<BoardPostListItem>, String> {
    let url = api_url(&format!("/api/board-post/{menu_id}/posts"));
    let request = with_credentials(Request::get(&url));

    let resp = request
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if resp.ok() {
        let res = resp
            .json::<BoardPostListResponse>()
            .await
            .map_err(|e| format!("Parse error: {e}"))?;
        Ok(res.posts)
    } else {
        let err = resp
            .json::<ErrorResponse>()
            .await
            .map(|e| e.error)
            .unwrap_or_else(|_| format!("HTTP {}", resp.status()));
        Err(err)
    }
}

pub async fn fetch_board_post(
    menu_id: i32,
    post_id: i32,
    _token: Option<&str>,
) -> Result<BoardPostDetail, String> {
    let url = api_url(&format!("/api/board-post/{menu_id}/posts/{post_id}"));
    let request = with_credentials(Request::get(&url));

    let resp = request
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if resp.ok() {
        resp.json::<BoardPostDetail>()
            .await
            .map_err(|e| format!("Parse error: {e}"))
    } else {
        let err = resp
            .json::<ErrorResponse>()
            .await
            .map(|e| e.error)
            .unwrap_or_else(|_| format!("HTTP {}", resp.status()));
        Err(err)
    }
}

pub async fn update_board_post(
    menu_id: i32,
    post_id: i32,
    body: &BoardPostUpdateRequest,
    _token: Option<&str>,
) -> Result<BoardPostUpdateResponse, String> {
    let url = api_url(&format!("/api/board-post/{menu_id}/posts/{post_id}"));
    let request = with_credentials(Request::put(&url)).header("Content-Type", "application/json");

    let resp = request
        .json(body)
        .map_err(|e| format!("Serialize error: {e}"))?
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if resp.ok() {
        resp.json::<BoardPostUpdateResponse>()
            .await
            .map_err(|e| format!("Parse error: {e}"))
    } else {
        let err = resp
            .json::<ErrorResponse>()
            .await
            .map(|e| e.error)
            .unwrap_or_else(|_| format!("HTTP {}", resp.status()));
        Err(err)
    }
}

pub async fn delete_board_post(
    menu_id: i32,
    post_id: i32,
    _token: Option<&str>,
) -> Result<BoardPostDeleteResponse, String> {
    let url = api_url(&format!("/api/board-post/{menu_id}/posts/{post_id}"));
    let request = with_credentials(Request::delete(&url));

    let resp = request
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if resp.ok() {
        resp.json::<BoardPostDeleteResponse>()
            .await
            .map_err(|e| format!("Parse error: {e}"))
    } else {
        let err = resp
            .json::<ErrorResponse>()
            .await
            .map(|e| e.error)
            .unwrap_or_else(|_| format!("HTTP {}", resp.status()));
        Err(err)
    }
}

fn js_error(value: JsValue) -> String {
    value
        .as_string()
        .unwrap_or_else(|| "Browser form-data error".to_string())
}
