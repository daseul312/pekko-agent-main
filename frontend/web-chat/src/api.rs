use gloo_net::http::{Request, RequestBuilder};
use uuid::Uuid;
use wasm_bindgen::JsValue;
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

/// POST /api/query or /api/agents/:agent_id/query
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
