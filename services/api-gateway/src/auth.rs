//! 직원 로그인 / 세션 인증.
//!
//! 게이트웨이가 `public."EmployeeInfo"` 테이블로 직접 ID/비밀번호(bcrypt)를 검증하고,
//! 성공 시 서명된 JWT를 HttpOnly 쿠키(`pekko_session`)로 발급한다. 이후 요청은
//! 이 쿠키로 신원을 복원하며, 쿠키가 없거나 무효하면 익명(전체 조회)으로 동작한다.

use axum::{
    extract::{FromRequestParts, State},
    http::{request::Parts, StatusCode},
    Json,
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use base64::Engine;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

/// 세션 쿠키 이름.
const COOKIE_NAME: &str = "pekko_session";
/// 세션(JWT) 유효 시간(시간 단위).
const SESSION_HOURS: i64 = 8;
/// 단일 회사 배포 기본 회사코드. 다회사면 로그인 요청에 명시.
const DEFAULT_COMPANY_CODE: &str = "10";

fn jwt_secret() -> String {
    std::env::var("JWT_SECRET").unwrap_or_else(|_| "dev-insecure-secret-change-me".to_string())
}

fn default_company_code() -> String {
    DEFAULT_COMPANY_CODE.to_string()
}

/// JWT 페이로드.
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    /// employeeId
    pub sub: String,
    pub company_code: String,
    pub role: String,
    pub name: String,
    /// 만료 시각(Unix epoch 초)
    pub exp: i64,
}

/// 쿠키 JWT로부터 복원된, 검증된 로그인 신원.
#[derive(Clone, Debug)]
pub struct AuthIdentity {
    pub employee_id: String,
    pub company_code: String,
    pub role: String,
    pub name: String,
    /// 세션 만료 시각(Unix epoch 초). `/auth/me`가 그대로 돌려준다.
    pub exp: i64,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub employee_id: String,
    #[serde(default = "default_company_code")]
    pub company_code: String,
    /// RSA-OAEP(SHA-256)로 암호화한 비밀번호의 base64 문자열.
    pub password_encrypted: String,
}

#[derive(Debug, Serialize)]
pub struct PublicKeyResponse {
    pub public_key_pem: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub employee_id: String,
    pub employee_name: String,
    pub company_code: String,
    pub role: String,
    pub expires_at: i64,
    /// 프로필 이미지 S3 URL(없으면 null).
    pub profile_image_url: Option<String>,
    /// 이 사용자가 피드백 통계(관리자 페이지)를 볼 수 있는지(`FEEDBACK_ADMIN_IDS` 허용목록).
    /// 프론트가 헤더의 관리자 버튼 노출 여부를 판단하는 데 쓴다.
    pub is_feedback_admin: bool,
}

#[derive(Debug, Serialize)]
pub struct LoginError {
    pub error: String,
}

/// `EmployeeInfo`에서 로그인 검증에 필요한 컬럼만 읽는다.
/// 컬럼명이 카멜케이스라 따옴표가 필요하며, `role`은 Postgres enum이라 `::text`로 캐스팅한다.
#[derive(sqlx::FromRow)]
struct EmployeeRow {
    #[sqlx(rename = "employeeId")]
    employee_id: String,
    #[sqlx(rename = "employeeName")]
    employee_name: Option<String>,
    #[sqlx(rename = "companyCode")]
    company_code: String,
    password: Option<String>,
    role: Option<String>,
    /// 사원정보번호. Profile/FileUpload 조인으로 프로필 이미지를 찾는 키.
    eino: i32,
}

/// 로그인 사용자의 현재 프로필 이미지 URL(S3 전체 URL)을 조회한다.
/// `EmployeeInfo.eino → Profile.eino → Profile.uuid → FileUpload.uuid → fileUploadUrl`.
/// 베스트-에포트: 조회 실패는 로그인에 영향을 주지 않고 `None`으로 처리한다.
async fn fetch_profile_image_url(pool: &sqlx::PgPool, eino: i32) -> Option<String> {
    let result = sqlx::query_scalar::<_, String>(
        r#"SELECT f."fileUploadUrl"
           FROM public."Profile" p
           JOIN public."FileUpload" f ON f."uuid" = p."uuid"
           WHERE p.eino = $1
           ORDER BY p.createdate DESC NULLS LAST
           LIMIT 1"#,
    )
    .bind(eino)
    .fetch_optional(pool)
    .await;

    match result {
        Ok(url) => url,
        Err(e) => {
            tracing::warn!(error = %e, eino, "프로필 이미지 조회 실패(무시)");
            None
        }
    }
}

fn unauthorized() -> (StatusCode, Json<LoginError>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(LoginError {
            error: "아이디 또는 비밀번호가 올바르지 않습니다.".to_string(),
        }),
    )
}

fn internal(msg: String) -> (StatusCode, Json<LoginError>) {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(LoginError { error: msg }))
}

/// 서명된 JWT를 HttpOnly 세션 쿠키로 감싼다.
fn build_session_cookie(token: String) -> Cookie<'static> {
    let secure = std::env::var("COOKIE_SECURE")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    let mut cookie = Cookie::new(COOKIE_NAME, token);
    cookie.set_http_only(true);
    cookie.set_path("/");
    cookie.set_same_site(SameSite::Lax);
    cookie.set_secure(secure);
    cookie
}

/// GET /auth/public-key — 프론트가 비밀번호를 암호화할 때 쓰는 RSA 공개키(SPKI PEM).
pub async fn public_key_handler(
    State(state): State<crate::AppState>,
) -> Json<PublicKeyResponse> {
    Json(PublicKeyResponse {
        public_key_pem: state.rsa_public_key_pem.to_string(),
    })
}

/// `password_encrypted`(base64 RSA-OAEP ciphertext)를 개인키로 복호화해 평문 비밀번호를 얻는다.
/// base64/복호화/UTF-8 실패는 모두 인증 실패로 처리(정보 비노출).
fn decrypt_password(
    private_key: &rsa::RsaPrivateKey,
    password_encrypted: &str,
) -> Result<String, (StatusCode, Json<LoginError>)> {
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(password_encrypted.trim())
        .map_err(|_| unauthorized())?;
    let plaintext = private_key
        .decrypt(rsa::Oaep::new::<sha2::Sha256>(), &ciphertext)
        .map_err(|_| unauthorized())?;
    String::from_utf8(plaintext).map_err(|_| unauthorized())
}

/// POST /auth/login — EmployeeInfo로 bcrypt 검증 후 세션 쿠키 발급.
pub async fn login_handler(
    State(state): State<crate::AppState>,
    jar: CookieJar,
    Json(req): Json<LoginRequest>,
) -> Result<(CookieJar, Json<LoginResponse>), (StatusCode, Json<LoginError>)> {
    // 프론트가 RSA 공개키로 암호화한 비밀번호를 개인키로 복호화.
    let password = decrypt_password(state.rsa_private_key.as_ref(), &req.password_encrypted)?;

    // 회사코드 자체가 유효한지 먼저 확인해, 존재하지 않는 회사는 계정 검증과 구분된 메시지로 안내한다.
    let company_exists = sqlx::query_scalar::<_, i64>(
        r#"SELECT COUNT(*) FROM public."CorporateInformationManagement" WHERE "companyCode" = $1"#,
    )
    .bind(&req.company_code)
    .fetch_one(state.pg_pool.as_ref())
    .await
    .map_err(|e| internal(format!("DB 오류: {e}")))?;

    if company_exists == 0 {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(LoginError {
                error: "존재하지 않는 회사입니다.".to_string(),
            }),
        ));
    }

    let row = sqlx::query_as::<_, EmployeeRow>(
        r#"SELECT "employeeId", "employeeName", "companyCode", "password", "role"::text AS role, eino
           FROM public."EmployeeInfo"
           WHERE "employeeId" = $1 AND "companyCode" = $2"#,
    )
    .bind(&req.employee_id)
    .bind(&req.company_code)
    .fetch_optional(state.pg_pool.as_ref())
    .await
    .map_err(|e| internal(format!("DB 오류: {e}")))?;

    // 존재하지 않는 계정과 비밀번호 불일치를 동일 메시지로 처리(계정 존재 여부 노출 방지).
    let row = row.ok_or_else(unauthorized)?;
    let hash = row.password.clone().ok_or_else(unauthorized)?;
    let verified = bcrypt::verify(&password, &hash).unwrap_or(false);
    if !verified {
        return Err(unauthorized());
    }

    let expires_at = (chrono::Utc::now() + chrono::Duration::hours(SESSION_HOURS)).timestamp();
    let name = row
        .employee_name
        .clone()
        .unwrap_or_else(|| row.employee_id.clone());
    let role = row.role.clone().unwrap_or_else(|| "USER".to_string());

    // 프로필 이미지(베스트-에포트): 실패해도 로그인은 계속 진행.
    let profile_image_url = fetch_profile_image_url(state.pg_pool.as_ref(), row.eino).await;

    let claims = Claims {
        sub: row.employee_id.clone(),
        company_code: row.company_code.clone(),
        role: role.clone(),
        name: name.clone(),
        exp: expires_at,
    };
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(jwt_secret().as_bytes()),
    )
    .map_err(|e| internal(format!("토큰 생성 실패: {e}")))?;

    let jar = jar.add(build_session_cookie(token));
    let is_feedback_admin = crate::is_feedback_admin(&row.employee_id);
    Ok((
        jar,
        Json(LoginResponse {
            employee_id: row.employee_id,
            employee_name: name,
            company_code: row.company_code,
            role,
            expires_at,
            profile_image_url,
            is_feedback_admin,
        }),
    ))
}

/// GET /auth/me — 세션 쿠키로 현재 로그인 사용자를 복원한다.
///
/// 세션 쿠키는 HttpOnly라 프론트(WASM)가 직접 읽지 못하므로, 새로고침 후 프론트가
/// 이 엔드포인트를 호출해 로그인 상태를 되살린다. 쿠키가 없거나 무효하면 401.
/// JWT에는 프로필 이미지가 없으므로 EmployeeInfo를 다시 조회해 동일한 정보를 채운다.
pub async fn me_handler(
    State(state): State<crate::AppState>,
    OptionalAuth(auth): OptionalAuth,
) -> Result<Json<LoginResponse>, (StatusCode, Json<LoginError>)> {
    let id = auth.ok_or_else(unauthorized)?;

    // 프로필 이미지 복원에는 eino가 필요하므로 먼저 EmployeeInfo에서 조회한다.
    // 베스트-에포트: 조회 실패 시 프로필 이미지는 None으로 둔다.
    let profile_image_url = match sqlx::query_scalar::<_, i32>(
        r#"SELECT eino FROM public."EmployeeInfo" WHERE "employeeId" = $1 AND "companyCode" = $2"#,
    )
    .bind(&id.employee_id)
    .bind(&id.company_code)
    .fetch_optional(state.pg_pool.as_ref())
    .await
    {
        Ok(Some(eino)) => fetch_profile_image_url(state.pg_pool.as_ref(), eino).await,
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(error = %e, "auth/me: EmployeeInfo 조회 실패(무시)");
            None
        }
    };

    let is_feedback_admin = crate::is_feedback_admin(&id.employee_id);
    Ok(Json(LoginResponse {
        employee_id: id.employee_id,
        employee_name: id.name,
        company_code: id.company_code,
        role: id.role,
        expires_at: id.exp,
        profile_image_url,
        is_feedback_admin,
    }))
}

/// POST /auth/logout — 세션 쿠키 제거.
pub async fn logout_handler(jar: CookieJar) -> (CookieJar, StatusCode) {
    let mut removal = Cookie::from(COOKIE_NAME);
    removal.set_path("/");
    (jar.remove(removal), StatusCode::NO_CONTENT)
}

/// 토큰 문자열을 검증해 신원으로 복원한다(만료/서명 검증 포함).
fn verify_token(token: &str) -> Option<AuthIdentity> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(jwt_secret().as_bytes()),
        &Validation::default(),
    )
    .ok()?;
    let c = data.claims;
    Some(AuthIdentity {
        employee_id: c.sub,
        company_code: c.company_code,
        role: c.role,
        name: c.name,
        exp: c.exp,
    })
}

/// 요청 쿠키에서 로그인 신원을 추출하는 추출기.
/// 쿠키가 없거나 토큰이 무효하면 `None`(익명) — 실패하지 않는다.
pub struct OptionalAuth(pub Option<AuthIdentity>);

#[axum::async_trait]
impl<S: Send + Sync> FromRequestParts<S> for OptionalAuth {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_request_parts(parts, state)
            .await
            .unwrap_or_default();
        let identity = jar.get(COOKIE_NAME).and_then(|c| verify_token(c.value()));
        Ok(OptionalAuth(identity))
    }
}
