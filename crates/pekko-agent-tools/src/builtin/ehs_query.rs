use async_trait::async_trait;
use pekko_agent_core::{Tool, ToolDefinition, ToolContext, ToolOutput, ToolError};
use serde::Deserialize;
use sqlx::PgPool;
use std::collections::HashSet;
use std::sync::{Arc, OnceLock};
use tokio::sync::OnceCell;
use tracing::{info, warn};

/// is_deleted 스키마 조회 실패 시 폴백으로 쓰는 빈 집합(가드 미적용).
static EMPTY_TABLE_SET: OnceLock<HashSet<String>> = OnceLock::new();

/// EHS 데이터베이스 조회 Tool
/// PostgreSQL의 EHS 테이블에 대해 읽기 전용 쿼리를 실행합니다.
pub struct EhsQueryTool {
    pool: Arc<PgPool>,
    /// `is_deleted` 컬럼을 가진 테이블 이름(소문자) 집합. 최초 1회만 조회 후 캐시.
    is_deleted_tables: OnceCell<HashSet<String>>,
    /// `use_yn` 컬럼을 가진 테이블 이름(소문자) 집합. 최초 1회만 조회 후 캐시.
    use_yn_tables: OnceCell<HashSet<String>>,
}

impl EhsQueryTool {
    pub fn new(pool: Arc<PgPool>) -> Self {
        Self {
            pool,
            is_deleted_tables: OnceCell::new(),
            use_yn_tables: OnceCell::new(),
        }
    }

    /// `is_deleted` 컬럼을 가진 테이블 집합을 반환한다(소문자). 최초 호출 시 한 번만
    /// information_schema에서 조회해 캐시한다. 조회 실패 시 빈 집합을 반환해 가드를
    /// 이번 호출에 한해 건너뛴다(전체 쿼리를 막지 않기 위한 fail-open). 실패는 캐시하지
    /// 않으므로 다음 호출에서 다시 시도한다.
    async fn is_deleted_tables(&self) -> &HashSet<String> {
        match self
            .is_deleted_tables
            .get_or_try_init(|| async {
                let rows = sqlx::query_scalar::<_, String>(
                    "SELECT DISTINCT lower(table_name) FROM information_schema.columns \
                     WHERE lower(column_name) = 'is_deleted'",
                )
                .fetch_all(self.pool.as_ref())
                .await?;
                Ok::<HashSet<String>, sqlx::Error>(rows.into_iter().collect())
            })
            .await
        {
            Ok(set) => set,
            Err(e) => {
                warn!(error = %e, "is_deleted 스키마 조회 실패 — 삭제 가드 이번 호출 건너뜀");
                EMPTY_TABLE_SET.get_or_init(HashSet::new)
            }
        }
    }

    /// `use_yn` 컬럼을 가진 테이블 집합을 반환한다(소문자). 최초 호출 시 한 번만
    /// information_schema에서 조회해 캐시한다. 조회 실패 시 빈 집합을 반환해 가드를
    /// 이번 호출에 한해 건너뛴다(전체 쿼리를 막지 않기 위한 fail-open).
    async fn use_yn_tables(&self) -> &HashSet<String> {
        match self
            .use_yn_tables
            .get_or_try_init(|| async {
                let rows = sqlx::query_scalar::<_, String>(
                    "SELECT DISTINCT lower(table_name) FROM information_schema.columns \
                     WHERE lower(column_name) = 'use_yn'",
                )
                .fetch_all(self.pool.as_ref())
                .await?;
                Ok::<HashSet<String>, sqlx::Error>(rows.into_iter().collect())
            })
            .await
        {
            Ok(set) => set,
            Err(e) => {
                warn!(error = %e, "use_yn 스키마 조회 실패 — 사용여부 가드 이번 호출 건너뜀");
                EMPTY_TABLE_SET.get_or_init(HashSet::new)
            }
        }
    }
}

/// SQL의 FROM/JOIN 뒤에 오는 테이블 이름을 소문자로 추출한다. 스키마 접두사(public.)와
/// 큰따옴표를 제거한다. 서브쿼리 내부 테이블도 함께 잡히므로 가드 범위에 포함된다.
fn extract_referenced_tables(sql: &str) -> Vec<String> {
    let lowered = sql.to_lowercase();
    let tokens: Vec<&str> = lowered
        .split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',')
        .filter(|t| !t.is_empty())
        .collect();

    let mut tables = Vec::new();
    for i in 0..tokens.len() {
        if tokens[i] == "from" || tokens[i] == "join" {
            if let Some(next) = tokens.get(i + 1) {
                let name = next.trim_matches('"').trim_end_matches(';');
                let name = name.rsplit('.').next().unwrap_or(name).trim_matches('"');
                if !name.is_empty() {
                    tables.push(name.to_string());
                }
            }
        }
    }
    tables
}

/// is_deleted 컬럼을 가진 테이블을 조회하면서, 삭제되지 않은 행(is_deleted='N')으로
/// 제한하는 조건이 있는지 검사한다. 공백 제거·소문자화한 사본에서 패턴을 본다.
/// 명시적으로 'Y'(삭제됨)를 조회하려는 쿼리는 거부 대상이다.
fn has_safe_deleted_filter(sql: &str) -> bool {
    let check: String = sql.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();

    // 삭제된 행을 명시적으로 노린 경우 → 안전하지 않음.
    if check.contains("is_deleted='y'") || check.contains("is_deletedin('y')") {
        return false;
    }

    check.contains("is_deleted='n'")
        || check.contains("is_deletedin('n')")
        || check.contains("is_deleted<>'y'")
        || check.contains("is_deleted!='y'")
        || check.contains("is_deletednotin('y')")
}

/// use_yn 컬럼을 가진 테이블을 조회하면서, 사용 중인 행(use_yn='Y')으로 제한하는
/// 조건이 있는지 검사한다. 명시적으로 'N'(미사용)을 조회하려는 쿼리는 거부 대상이다.
fn has_safe_use_yn_filter(sql: &str) -> bool {
    let check: String = sql.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();

    if check.contains("use_yn='n'") || check.contains("use_ynin('n')") {
        return false;
    }

    check.contains("use_yn='y'")
        || check.contains("use_ynin('y')")
        || check.contains("use_yn<>'n'")
        || check.contains("use_yn!='n'")
        || check.contains("use_ynnotin('n')")
}

#[derive(Deserialize)]
struct QueryParams {
    /// 실행할 SQL 쿼리 (SELECT만 허용)
    sql: String,
    /// 최대 반환 행 수 (기본: 50)
    limit: Option<i64>,
}

/// 허용된 EHS 테이블 목록 (읽기 전용)
const ALLOWED_TABLES: &[&str] = &[
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

/// SQL 안전성 검증
fn validate_sql(sql: &str) -> Result<(), String> {
    let sql_upper = sql.to_uppercase().trim().to_string();

    // SELECT만 허용
    if !sql_upper.starts_with("SELECT") {
        return Err("SELECT 쿼리만 허용됩니다. INSERT, UPDATE, DELETE 등은 사용할 수 없습니다.".into());
    }

    // 위험한 키워드 차단
    let forbidden = ["INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "CREATE",
                     "TRUNCATE", "GRANT", "REVOKE", "EXEC", "EXECUTE", "--", ";"];
    // ; 는 쿼리 끝에만 허용
    let sql_no_trailing = sql.trim().trim_end_matches(';');
    if sql_no_trailing.contains(';') {
        return Err("다중 쿼리(;)는 허용되지 않습니다.".into());
    }
    for kw in &forbidden[..forbidden.len() - 1] {
        // Check as a word boundary (not part of column names)
        if sql_upper.contains(&format!(" {kw} ")) || sql_upper.starts_with(&format!("{kw} ")) {
            return Err(format!("{kw} 문은 허용되지 않습니다."));
        }
    }
    if sql.contains("--") {
        return Err("SQL 주석(--)은 허용되지 않습니다.".into());
    }

    Ok(())
}

#[async_trait]
impl Tool for EhsQueryTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "ehs_query".to_string(),
            description: "EHS(환경안전보건) 데이터베이스에서 데이터를 조회합니다. \
                          SELECT 쿼리만 허용되며, 다음 테이블에 접근할 수 있습니다: \
                          dangerousworkmanagement(안전작업허가), tbmmanagement(TBM), \
                          accidentfreemanagement(무재해), riskassessmentmanagement(위험성평가), \
                          nearmissmanagement(아차사고), safetyeducationmanagement(안전교육), \
                          msdsmanagement(MSDS), safetyhealthcommittee(안전보건위원회), \
                          employeeinfo(직원정보)".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "sql": {
                        "type": "string",
                        "description": "실행할 SELECT SQL 쿼리. 예: SELECT count(*) FROM dangerousworkmanagement"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "최대 반환 행 수 (기본: 50, 최대: 200)",
                        "default": 50
                    }
                },
                "required": ["sql"]
            }),
            required_permissions: vec![],
            timeout_ms: 10_000,
            idempotent: true,
        }
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let params: QueryParams = serde_json::from_value(input)
            .map_err(|e| ToolError::ValidationFailed(format!("파라미터 오류: {e}")))?;

        // SQL 안전성 검증
        if let Err(e) = validate_sql(&params.sql) {
            return Ok(ToolOutput::error(e));
        }

        // [하드 가드] 삭제 데이터(is_deleted='Y') 노출 차단.
        // is_deleted 컬럼이 있는 테이블을 조회하면서 is_deleted='N' 조건이 없으면
        // 쿼리를 거부한다(에이전트가 조건을 추가해 재시도). 컬럼이 없는 테이블은
        // information_schema 기준으로 가드 대상에서 제외되므로 오탐으로 막지 않는다.
        // 프롬프트 지시만으론 LLM이 필터를 빠뜨리는 사례가 있어 코드로 강제한다.
        let deleted_tables = self.is_deleted_tables().await;
        if !deleted_tables.is_empty() {
            let touches_deleted = extract_referenced_tables(&params.sql)
                .iter()
                .any(|t| deleted_tables.contains(t));
            if touches_deleted && !has_safe_deleted_filter(&params.sql) {
                warn!(sql = %params.sql, "is_deleted 필터 누락 쿼리 거부");
                return Ok(ToolOutput::error(
                    "삭제된 데이터는 조회할 수 없습니다. 조회 대상 테이블에 is_deleted 컬럼이 \
                     있으므로 WHERE 절에 반드시 is_deleted = 'N' 조건을 추가해 다시 쿼리하세요 \
                     (삭제된 'Y' 행은 목록·집계 어디에도 포함 금지)."
                        .to_string(),
                ));
            }
        }

        // [하드 가드] 미사용 데이터(use_yn='N') 노출 차단.
        // use_yn 컬럼이 있는 테이블을 조회하면서 use_yn='Y' 조건이 없으면 쿼리를 거부한다.
        let use_yn_tables = self.use_yn_tables().await;
        if !use_yn_tables.is_empty() {
            let touches_use_yn = extract_referenced_tables(&params.sql)
                .iter()
                .any(|t| use_yn_tables.contains(t));
            if touches_use_yn && !has_safe_use_yn_filter(&params.sql) {
                warn!(sql = %params.sql, "use_yn 필터 누락 쿼리 거부");
                return Ok(ToolOutput::error(
                    "사용 중이 아닌 데이터는 조회할 수 없습니다. 조회 대상 테이블에 use_yn 컬럼이 \
                     있으므로 WHERE 절에 반드시 use_yn = 'Y' 조건을 추가해 다시 쿼리하세요 \
                     (미사용 'N' 행은 목록·집계 어디에도 포함 금지)."
                        .to_string(),
                ));
            }
        }

        let limit = params.limit.unwrap_or(50).min(200);

        // LIMIT이 없으면 자동으로 추가
        let sql = {
            let upper = params.sql.to_uppercase();
            if !upper.contains("LIMIT") {
                format!("{} LIMIT {}", params.sql.trim().trim_end_matches(';'), limit)
            } else {
                params.sql.trim().trim_end_matches(';').to_string()
            }
        };

        info!(sql = %sql, "EHS DB 쿼리 실행");

        // 쿼리 실행
        match sqlx::query(&sql).fetch_all(self.pool.as_ref()).await {
            Ok(rows) => {
                use sqlx::Row;
                use sqlx::Column;

                // 결과를 JSON 배열로 변환
                let mut result = Vec::new();
                for row in &rows {
                    let mut obj = serde_json::Map::new();
                    for (i, col) in row.columns().iter().enumerate() {
                        let name = col.name().to_string();
                        // Try to get as various types
                        let val: serde_json::Value = if let Ok(v) = row.try_get::<String, _>(i) {
                            serde_json::Value::String(v)
                        } else if let Ok(v) = row.try_get::<i64, _>(i) {
                            serde_json::Value::Number(v.into())
                        } else if let Ok(v) = row.try_get::<i32, _>(i) {
                            serde_json::Value::Number(v.into())
                        } else if let Ok(v) = row.try_get::<f64, _>(i) {
                            serde_json::json!(v)
                        } else if let Ok(v) = row.try_get::<bool, _>(i) {
                            serde_json::Value::Bool(v)
                        } else if let Ok(v) = row.try_get::<chrono::NaiveDateTime, _>(i) {
                            serde_json::Value::String(v.format("%Y-%m-%d %H:%M:%S").to_string())
                        } else if let Ok(v) = row.try_get::<chrono::NaiveDate, _>(i) {
                            serde_json::Value::String(v.format("%Y-%m-%d").to_string())
                        } else {
                            // null or unsupported type
                            serde_json::Value::Null
                        };
                        obj.insert(name, val);
                    }
                    result.push(serde_json::Value::Object(obj));
                }

                info!(row_count = result.len(), "EHS DB 쿼리 완료");

                Ok(ToolOutput::success(serde_json::json!({
                    "row_count": result.len(),
                    "rows": result
                })))
            }
            Err(e) => {
                warn!(error = %e, "EHS DB 쿼리 실패");
                Ok(ToolOutput::error(format!("쿼리 실행 오류: {e}")))
            }
        }
    }
}
