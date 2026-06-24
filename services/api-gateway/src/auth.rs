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

    let row = sqlx::query_as::<_, EmployeeRow>(
        r#"SELECT "employeeId", "employeeName", "companyCode", "password", "role"::text AS role
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
    Ok((
        jar,
        Json(LoginResponse {
            employee_id: row.employee_id,
            employee_name: name,
            company_code: row.company_code,
            role,
            expires_at,
        }),
    ))
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
