use gloo_net::http::{Request, RequestBuilder};
use uuid::Uuid;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Blob, File, FormData, RequestCredentials};

use crate::types::*;

const API_BASE_URL: Option<&str> = option_env!("WEB_CHAT_API_BASE_URL");
const API_PORT: &str = "8082";
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
pub async fn fetch_session_summary(session_id: Uuid) -> Result<String, String> {
    let url = api_url(&format!("/api/sessions/{}/summarize", session_id));
    let resp = with_credentials(Request::get(&url))
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

/// GET /api/sessions/:session_id/history — 세션의 대화 메시지를 받아온다.
/// 창을 닫았다 다시 열었을 때, 백그라운드로 완료된 답변을 이어받기 위해 폴링에 쓴다.
pub async fn fetch_session_history(session_id: Uuid) -> Result<Vec<HistoryMessage>, String> {
    let url = api_url(&format!("/api/sessions/{}/history", session_id));
    let resp = with_credentials(Request::get(&url))
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if resp.ok() {
        resp.json::<Vec<HistoryMessage>>()
            .await
            .map_err(|e| format!("Parse error: {e}"))
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}

/// GET /auth/public-key — 비밀번호 암호화에 쓸 RSA 공개키(SPKI PEM)를 받아온다.
async fn fetch_public_key_pem() -> Result<String, String> {
    let url = api_url("/auth/public-key");
    let resp = with_credentials(Request::get(&url))
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if resp.ok() {
        resp.json::<PublicKeyResponse>()
            .await
            .map(|r| r.public_key_pem)
            .map_err(|e| format!("Parse error: {e}"))
    } else {
        Err(format!("공개키 조회 실패: HTTP {}", resp.status()))
    }
}

/// 비밀번호를 서버 공개키로 RSA-OAEP(SHA-256) 암호화 후 base64 인코딩한다.
fn encrypt_password(public_key_pem: &str, password: &str) -> Result<String, String> {
    use base64::Engine;
    use rsa::pkcs8::DecodePublicKey;

    let public_key = rsa::RsaPublicKey::from_public_key_pem(public_key_pem)
        .map_err(|e| format!("공개키 파싱 실패: {e}"))?;
    let mut rng = rand::rngs::OsRng;
    let ciphertext = public_key
        .encrypt(&mut rng, rsa::Oaep::new::<sha2::Sha256>(), password.as_bytes())
        .map_err(|e| format!("비밀번호 암호화 실패: {e}"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(ciphertext))
}

/// POST /auth/login — 비밀번호를 RSA 공개키로 암호화해 전송하면, 게이트웨이가 개인키로
/// 복호화 후 EmployeeInfo로 bcrypt 검증하고 HttpOnly 세션 쿠키를 발급한다. 쿠키는
/// `credentials: include` 덕분에 브라우저가 자동 저장하므로 여기서는 표시용 `AuthUser`만 돌려준다.
pub async fn login(
    employee_id: String,
    password: String,
    company_code: String,
) -> Result<AuthUser, String> {
    // 평문 비밀번호가 요청 payload에 노출되지 않도록 서버 공개키로 암호화해 보낸다.
    let public_key_pem = fetch_public_key_pem().await?;
    let password_encrypted = encrypt_password(&public_key_pem, &password)?;

    let url = api_url("/auth/login");
    let body = LoginRequest {
        employee_id,
        company_code,
        password_encrypted,
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

        Ok(AuthUser {
            id: login.employee_id,
            name: login.employee_name,
            company_code: login.company_code,
            role: login.role,
            profile_image_url: login.profile_image_url,
            is_feedback_admin: login.is_feedback_admin,
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

/// GET /auth/me — 세션 쿠키로 현재 로그인 사용자를 복원한다. 새로고침 후 호출해
/// 로그인 상태를 되살린다. 비로그인(401)이면 `Ok(None)`.
pub async fn fetch_me() -> Result<Option<AuthUser>, String> {
    let url = api_url("/auth/me");
    let resp = with_credentials(Request::get(&url))
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if resp.ok() {
        let login = resp
            .json::<LoginResponse>()
            .await
            .map_err(|e| format!("Parse error: {e}"))?;
        Ok(Some(AuthUser {
            id: login.employee_id,
            name: login.employee_name,
            company_code: login.company_code,
            role: login.role,
            profile_image_url: login.profile_image_url,
            is_feedback_admin: login.is_feedback_admin,
        }))
    } else if resp.status() == 401 {
        Ok(None)
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}

/// POST /api/query or /api/agents/:agent_id/query (buffered, non-streaming).
/// Retained as a fallback; the live UI uses `send_query_stream`.
#[allow(dead_code)]
pub async fn send_query(
    agent_id: Option<&str>,
    content: &str,
    session_id: Option<Uuid>,
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
    } else {
        let err = resp
            .json::<ErrorResponse>()
            .await
            .map(|e| e.error)
            .unwrap_or_else(|_| format!("HTTP {}", resp.status()));
        Err(err)
    }
}

/// 백엔드가 만든 공유 URL을, 프론트가 실제로 서빙되는 origin(scheme+host+**포트**)으로
/// 재작성한다. 백엔드 base가 무엇이든 현재 접속 주소(예: `http://host:8081`)를 따르므로
/// 8081 같은 포트가 항상 정확히 붙는다. share_id는 백엔드 URL의 마지막 경로 조각에서 취한다.
fn share_url_on_current_origin(backend_url: &str) -> String {
    let share_id = backend_url.trim_end_matches('/').rsplit('/').next().unwrap_or_default();
    let origin = web_sys::window()
        .and_then(|w| w.location().origin().ok())
        .filter(|o| !o.is_empty() && o != "null");
    match origin {
        Some(origin) if !share_id.is_empty() => {
            format!("{}/#/share/{}", origin.trim_end_matches('/'), share_id)
        }
        // origin을 못 구하면 백엔드 URL을 그대로 사용(서버 측 SHARE_BASE_URL).
        _ => backend_url.to_string(),
    }
}

/// POST /api/share — 답변 1개를 공유 링크로 만든다. 공유 시점의 답변 본문을
/// 스냅샷으로 보내고, 생성된 공유 URL을 돌려받는다(로그인 쿠키 필수).
pub async fn create_share(
    content: &str,
    session_id: Option<Uuid>,
    message_id: Option<String>,
    chart_data: Option<String>,
) -> Result<String, String> {
    let url = api_url("/api/share");
    let body = ShareCreateRequest {
        content: content.to_string(),
        session_id,
        message_id,
        chart_data,
    };

    let resp = with_credentials(Request::post(&url))
        .header("Content-Type", "application/json")
        .json(&body)
        .map_err(|e| format!("Serialize error: {e}"))?
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if resp.ok() {
        resp.json::<ShareCreateResponse>()
            .await
            .map(|r| share_url_on_current_origin(&r.share_url))
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

/// POST /api/feedback — 답변 1개(특정 버전)에 👍/👎 평가를 보낸다. `active=false`면
/// 토글 취소(평가 삭제). 로그인 쿠키 필수. 성공 시 `Ok(())`.
pub async fn submit_feedback(body: &FeedbackRequest) -> Result<(), String> {
    let url = api_url("/api/feedback");
    let resp = with_credentials(Request::post(&url))
        .header("Content-Type", "application/json")
        .json(body)
        .map_err(|e| format!("Serialize error: {e}"))?
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if resp.ok() {
        Ok(())
    } else {
        let err = resp
            .json::<ErrorResponse>()
            .await
            .map(|e| e.error)
            .unwrap_or_else(|_| format!("HTTP {}", resp.status()));
        Err(err)
    }
}

/// GET /api/share/:id — 공유된 답변을 조회한다. 상태코드별로 `ShareFetchError`를
/// 구분해 열람 페이지가 로그인 유도/접근 거부/없음 안내를 분기할 수 있게 한다.
pub async fn fetch_shared(share_id: &str) -> Result<SharedAnswer, ShareFetchError> {
    let url = api_url(&format!("/api/share/{share_id}"));
    let resp = with_credentials(Request::get(&url))
        .send()
        .await
        .map_err(|e| ShareFetchError::Other(format!("Network error: {e}")))?;

    match resp.status() {
        200 => resp
            .json::<SharedAnswer>()
            .await
            .map_err(|e| ShareFetchError::Other(format!("Parse error: {e}"))),
        401 => Err(ShareFetchError::Unauthorized),
        403 => Err(ShareFetchError::Forbidden),
        404 => Err(ShareFetchError::NotFound),
        status => Err(ShareFetchError::Other(format!("HTTP {status}"))),
    }
}

/// GET /api/admin/feedback/stats?period= — 회사+기간 단위 대시보드 집계(허용목록 사원만).
/// 상태코드별로 `FeedbackStatsError`를 구분해 관리자 페이지가 로그인/권한/오류를 분기한다.
pub async fn fetch_feedback_stats(
    period: &str,
) -> Result<FeedbackStatsResponse, FeedbackStatsError> {
    let url = api_url(&format!("/api/admin/feedback/stats?period={period}"));
    let resp = with_credentials(Request::get(&url))
        .send()
        .await
        .map_err(|e| FeedbackStatsError::Other(format!("Network error: {e}")))?;

    match resp.status() {
        200 => resp
            .json::<FeedbackStatsResponse>()
            .await
            .map_err(|e| FeedbackStatsError::Other(format!("Parse error: {e}"))),
        401 => Err(FeedbackStatsError::Unauthorized),
        403 => Err(FeedbackStatsError::Forbidden),
        status => Err(FeedbackStatsError::Other(format!("HTTP {status}"))),
    }
}

/// GET /api/admin/feedback/comments — 부정 코멘트 목록(페이지네이션). 같은 게이팅/에러 분기.
pub async fn fetch_feedback_comments(
    period: &str,
    limit: i64,
    offset: i64,
) -> Result<FeedbackCommentsResponse, FeedbackStatsError> {
    let url = api_url(&format!(
        "/api/admin/feedback/comments?period={period}&limit={limit}&offset={offset}"
    ));
    let resp = with_credentials(Request::get(&url))
        .send()
        .await
        .map_err(|e| FeedbackStatsError::Other(format!("Network error: {e}")))?;

    match resp.status() {
        200 => resp
            .json::<FeedbackCommentsResponse>()
            .await
            .map_err(|e| FeedbackStatsError::Other(format!("Parse error: {e}"))),
        401 => Err(FeedbackStatsError::Unauthorized),
        403 => Err(FeedbackStatsError::Forbidden),
        status => Err(FeedbackStatsError::Other(format!("HTTP {status}"))),
    }
}

/// GET /api/admin/feedback/reverify — 부정확 재확인 목록(admin).
pub async fn fetch_reverify_list(
    period: &str,
    limit: i64,
    offset: i64,
) -> Result<ReverifyListResponse, FeedbackStatsError> {
    let url = api_url(&format!(
        "/api/admin/feedback/reverify?period={period}&limit={limit}&offset={offset}"
    ));
    let resp = with_credentials(Request::get(&url))
        .send()
        .await
        .map_err(|e| FeedbackStatsError::Other(format!("Network error: {e}")))?;
    match resp.status() {
        200 => resp
            .json::<ReverifyListResponse>()
            .await
            .map_err(|e| FeedbackStatsError::Other(format!("Parse error: {e}"))),
        401 => Err(FeedbackStatsError::Unauthorized),
        403 => Err(FeedbackStatsError::Forbidden),
        status => Err(FeedbackStatsError::Other(format!("HTTP {status}"))),
    }
}

/// POST /api/admin/feedback/resolve — 개선 답변 재생성(admin). 느릴 수 있음(LLM 호출).
pub async fn resolve_feedback(feedback_key: &str) -> Result<ResolutionView, FeedbackStatsError> {
    let url = api_url("/api/admin/feedback/resolve");
    let body = serde_json::json!({ "feedback_key": feedback_key });
    let resp = with_credentials(Request::post(&url))
        .header("Content-Type", "application/json")
        .json(&body)
        .map_err(|e| FeedbackStatsError::Other(format!("Serialize error: {e}")))?
        .send()
        .await
        .map_err(|e| FeedbackStatsError::Other(format!("Network error: {e}")))?;
    match resp.status() {
        200 => resp
            .json::<ResolutionView>()
            .await
            .map_err(|e| FeedbackStatsError::Other(format!("Parse error: {e}"))),
        401 => Err(FeedbackStatsError::Unauthorized),
        403 => Err(FeedbackStatsError::Forbidden),
        status => Err(FeedbackStatsError::Other(format!("HTTP {status}"))),
    }
}

/// POST /api/admin/feedback/resolve/send — 조치 확정 + 사용자 알림(admin).
pub async fn send_resolution(feedback_key: &str) -> Result<(), String> {
    let url = api_url("/api/admin/feedback/resolve/send");
    let body = serde_json::json!({ "feedback_key": feedback_key });
    let resp = with_credentials(Request::post(&url))
        .header("Content-Type", "application/json")
        .json(&body)
        .map_err(|e| format!("Serialize error: {e}"))?
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;
    if resp.ok() {
        Ok(())
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}

/// GET /api/resolution/:feedback_key — 비교 페이지 데이터(수신자/admin).
pub async fn fetch_resolution(feedback_key: &str) -> Result<ResolutionView, FeedbackStatsError> {
    let url = api_url(&format!("/api/resolution/{feedback_key}"));
    let resp = with_credentials(Request::get(&url))
        .send()
        .await
        .map_err(|e| FeedbackStatsError::Other(format!("Network error: {e}")))?;
    match resp.status() {
        200 => resp
            .json::<ResolutionView>()
            .await
            .map_err(|e| FeedbackStatsError::Other(format!("Parse error: {e}"))),
        401 => Err(FeedbackStatsError::Unauthorized),
        403 => Err(FeedbackStatsError::Forbidden),
        status => Err(FeedbackStatsError::Other(format!("HTTP {status}"))),
    }
}

/// POST /api/resolution/:feedback_key/reverify — 사용자 재확인 제출.
pub async fn submit_reverify(
    feedback_key: &str,
    result: &str,
    comment: Option<&str>,
) -> Result<(), String> {
    let url = api_url(&format!("/api/resolution/{feedback_key}/reverify"));
    let body = serde_json::json!({ "result": result, "comment": comment });
    let resp = with_credentials(Request::post(&url))
        .header("Content-Type", "application/json")
        .json(&body)
        .map_err(|e| format!("Serialize error: {e}"))?
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;
    if resp.ok() {
        Ok(())
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}

/// GET /api/notifications — 본인 알림 목록 + 안읽음 수.
pub async fn fetch_notifications() -> Result<NotificationsResponse, String> {
    let url = api_url("/api/notifications");
    let resp = with_credentials(Request::get(&url))
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;
    if resp.ok() {
        resp.json::<NotificationsResponse>()
            .await
            .map_err(|e| format!("Parse error: {e}"))
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}

/// GET /api/permit/formal-work-types — 승인 작업계획서 작업종류 칩 목록.
pub async fn fetch_formal_work_types() -> Result<Vec<FormalWorkType>, String> {
    let url = api_url("/api/permit/formal-work-types");
    let resp = with_credentials(Request::get(&url))
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;
    if resp.ok() {
        resp.json::<FormalWorkTypesResponse>()
            .await
            .map(|r| r.work_types)
            .map_err(|e| format!("Parse error: {e}"))
    } else {
        Err(format!("HTTP {}", resp.status()))
    }
}

/// POST /api/notifications/:id/read — 알림 읽음.
pub async fn mark_notification_read(id: &str) -> Result<(), String> {
    let url = api_url(&format!("/api/notifications/{id}/read"));
    let resp = with_credentials(Request::post(&url))
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;
    if resp.ok() {
        Ok(())
    } else {
        Err(format!("HTTP {}", resp.status()))
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
    opts.set_headers(&headers);

    let request =
        web_sys::Request::new_with_str_and_init(&url, &opts).map_err(js_err)?;

    let window = web_sys::window().ok_or_else(|| "No window".to_string())?;
    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("Network error: {}", js_err(e)))?;
    let resp: web_sys::Response = resp_value.dyn_into().map_err(js_err)?;

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
