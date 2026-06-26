//! CoreDB 기반 "조치 완료" 저장소.
//!
//! 관리자가 부정 피드백을 처리(개선 답변 재생성)하면 이전/새 답변 스냅샷과 상태를
//! 임베디드 CoreDB에 보관한다. 사용자가 비교 페이지에서 "정확/부정확"으로 재확인한 결과도
//! 같은 레코드에 기록한다. 운영 PostgreSQL을 건드리지 않기 위해 공유·피드백 저장소와
//! 동일하게 CoreDB `pekko_agent` keyspace를 쓴다. 키는 피드백과 동일한 `feedback_key`.

use coredb::{CassandraValue, CoreDB, QueryResult};
use pekko_agent_core::MemoryError;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// 조치 레코드(조회 결과).
pub struct ResolutionRecord {
    pub feedback_key: String,
    /// 알림 수신자(원 피드백 작성자).
    pub employee_id: String,
    pub company_code: String,
    pub agent_id: String,
    pub question: String,
    pub old_answer: String,
    pub old_chart: Option<String>,
    pub new_answer: String,
    pub new_chart: Option<String>,
    /// "reviewing"(재생성됨·미발송) | "sent"(알림 발송됨).
    pub status: String,
    pub resolved_by: String,
    pub resolved_at: String,
    /// ""(미확인) | "accurate" | "inaccurate".
    pub reverify_result: String,
    pub reverify_comment: Option<String>,
    pub reverify_at: String,
}

fn ignore_already_exists<T, E: std::fmt::Display>(
    result: Result<T, E>,
    what: &str,
) -> Result<(), MemoryError> {
    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if msg.to_lowercase().contains("already exists") {
                debug!(what, "schema object already exists; skipping");
                Ok(())
            } else {
                Err(MemoryError::StorageError(format!(
                    "Failed to create {}: {}",
                    what, msg
                )))
            }
        }
    }
}

fn esc(s: &str) -> String {
    s.replace('\'', "''")
}

/// `None`/빈값은 빈 문자열로 저장(CoreDB bareword `null` 문자열화 회피).
fn opt_lit(v: Option<&str>) -> String {
    v.map(|c| format!("'{}'", esc(c)))
        .unwrap_or_else(|| "''".to_string())
}

/// CoreDB 기반 조치 저장소. 피드백·공유와 같은 `pekko_agent` keyspace를 공유한다.
pub struct CoreDbResolutionStore {
    db: Arc<CoreDB>,
    keyspace: String,
}

impl CoreDbResolutionStore {
    pub async fn new(db: Arc<CoreDB>) -> Result<Self, MemoryError> {
        let keyspace = "pekko_agent".to_string();
        Self::init_schema(&db, &keyspace).await?;
        info!(keyspace = %keyspace, "CoreDB resolution store initialized");
        Ok(Self { db, keyspace })
    }

    async fn init_schema(db: &CoreDB, keyspace: &str) -> Result<(), MemoryError> {
        let cql = format!(
            "CREATE KEYSPACE IF NOT EXISTS {} WITH REPLICATION = {{'class': 'SimpleStrategy', 'replication_factor': 1}}",
            keyspace
        );
        ignore_already_exists(db.execute_cql(&cql).await, "keyspace")?;

        let cql = format!(
            "CREATE TABLE IF NOT EXISTS {}.feedback_resolutions (
                feedback_key TEXT,
                employee_id TEXT,
                company_code TEXT,
                agent_id TEXT,
                question TEXT,
                old_answer TEXT,
                old_chart TEXT,
                new_answer TEXT,
                new_chart TEXT,
                status TEXT,
                resolved_by TEXT,
                resolved_at TEXT,
                reverify_result TEXT,
                reverify_comment TEXT,
                reverify_at TEXT,
                PRIMARY KEY (feedback_key)
            )",
            keyspace
        );
        ignore_already_exists(db.execute_cql(&cql).await, "feedback_resolutions table")?;
        debug!("CoreDB schema initialized for feedback_resolutions");
        Ok(())
    }

    /// 재생성 결과를 draft(status="reviewing")로 저장(덮어쓰기). 재확인 필드는 초기화한다.
    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_draft(
        &self,
        feedback_key: &str,
        employee_id: &str,
        company_code: &str,
        agent_id: &str,
        question: &str,
        old_answer: &str,
        old_chart: Option<&str>,
        new_answer: &str,
        new_chart: Option<&str>,
        resolved_by: &str,
        resolved_at: &str,
    ) -> Result<(), MemoryError> {
        let cql = format!(
            "INSERT INTO {}.feedback_resolutions \
             (feedback_key, employee_id, company_code, agent_id, question, old_answer, old_chart, new_answer, new_chart, status, resolved_by, resolved_at, reverify_result, reverify_comment, reverify_at) \
             VALUES ('{}', '{}', '{}', '{}', '{}', '{}', {}, '{}', {}, 'reviewing', '{}', '{}', '', '', '')",
            self.keyspace,
            esc(feedback_key),
            esc(employee_id),
            esc(company_code),
            esc(agent_id),
            esc(question),
            esc(old_answer),
            opt_lit(old_chart),
            esc(new_answer),
            opt_lit(new_chart),
            esc(resolved_by),
            esc(resolved_at),
        );
        self.db
            .execute_cql(&cql)
            .await
            .map_err(|e| MemoryError::StorageError(format!("Failed to upsert resolution: {}", e)))?;
        debug!(feedback_key = %feedback_key, "Resolution draft saved to CoreDB");
        Ok(())
    }

    /// status="sent"으로 갱신(알림 발송 확정). 기존 레코드를 읽어 같은 값으로 다시 INSERT한다.
    pub async fn mark_sent(&self, feedback_key: &str) -> Result<(), MemoryError> {
        let Some(r) = self.get(feedback_key).await? else {
            return Err(MemoryError::NotFound(format!(
                "resolution {} not found",
                feedback_key
            )));
        };
        let cql = format!(
            "INSERT INTO {}.feedback_resolutions \
             (feedback_key, employee_id, company_code, agent_id, question, old_answer, old_chart, new_answer, new_chart, status, resolved_by, resolved_at, reverify_result, reverify_comment, reverify_at) \
             VALUES ('{}', '{}', '{}', '{}', '{}', '{}', {}, '{}', {}, 'sent', '{}', '{}', '{}', {}, '{}')",
            self.keyspace,
            esc(&r.feedback_key),
            esc(&r.employee_id),
            esc(&r.company_code),
            esc(&r.agent_id),
            esc(&r.question),
            esc(&r.old_answer),
            opt_lit(r.old_chart.as_deref()),
            esc(&r.new_answer),
            opt_lit(r.new_chart.as_deref()),
            esc(&r.resolved_by),
            esc(&r.resolved_at),
            esc(&r.reverify_result),
            opt_lit(r.reverify_comment.as_deref()),
            esc(&r.reverify_at),
        );
        self.db
            .execute_cql(&cql)
            .await
            .map_err(|e| MemoryError::StorageError(format!("Failed to mark resolution sent: {}", e)))?;
        Ok(())
    }

    /// 사용자 재확인 결과 기록. 기존 레코드 위에 reverify 필드만 갱신.
    pub async fn set_reverify(
        &self,
        feedback_key: &str,
        result: &str,
        comment: Option<&str>,
        at: &str,
    ) -> Result<(), MemoryError> {
        let Some(r) = self.get(feedback_key).await? else {
            return Err(MemoryError::NotFound(format!(
                "resolution {} not found",
                feedback_key
            )));
        };
        let cql = format!(
            "INSERT INTO {}.feedback_resolutions \
             (feedback_key, employee_id, company_code, agent_id, question, old_answer, old_chart, new_answer, new_chart, status, resolved_by, resolved_at, reverify_result, reverify_comment, reverify_at) \
             VALUES ('{}', '{}', '{}', '{}', '{}', '{}', {}, '{}', {}, '{}', '{}', '{}', '{}', {}, '{}')",
            self.keyspace,
            esc(&r.feedback_key),
            esc(&r.employee_id),
            esc(&r.company_code),
            esc(&r.agent_id),
            esc(&r.question),
            esc(&r.old_answer),
            opt_lit(r.old_chart.as_deref()),
            esc(&r.new_answer),
            opt_lit(r.new_chart.as_deref()),
            esc(&r.status),
            esc(&r.resolved_by),
            esc(&r.resolved_at),
            esc(result),
            opt_lit(comment),
            esc(at),
        );
        self.db
            .execute_cql(&cql)
            .await
            .map_err(|e| MemoryError::StorageError(format!("Failed to set reverify: {}", e)))?;
        Ok(())
    }

    /// feedback_key로 단건 조회.
    pub async fn get(&self, feedback_key: &str) -> Result<Option<ResolutionRecord>, MemoryError> {
        let cql = format!(
            "SELECT feedback_key, employee_id, company_code, agent_id, question, old_answer, old_chart, new_answer, new_chart, status, resolved_by, resolved_at, reverify_result, reverify_comment, reverify_at FROM {}.feedback_resolutions WHERE feedback_key = '{}'",
            self.keyspace,
            esc(feedback_key)
        );
        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => Ok(rows.first().map(Self::row_to_record)),
            Ok(_) => Ok(None),
            Err(e) => {
                warn!(error = %e, "Failed to get resolution from CoreDB");
                Err(MemoryError::StorageError(format!("Failed to get resolution: {}", e)))
            }
        }
    }

    /// 전체 조회(통계 집계용).
    pub async fn list_all(&self) -> Result<Vec<ResolutionRecord>, MemoryError> {
        let cql = format!(
            "SELECT feedback_key, employee_id, company_code, agent_id, question, old_answer, old_chart, new_answer, new_chart, status, resolved_by, resolved_at, reverify_result, reverify_comment, reverify_at FROM {}.feedback_resolutions",
            self.keyspace
        );
        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => Ok(rows.iter().map(Self::row_to_record).collect()),
            Ok(_) => Ok(Vec::new()),
            Err(e) => {
                warn!(error = %e, "Failed to list resolutions from CoreDB");
                Err(MemoryError::StorageError(format!("Failed to list resolutions: {}", e)))
            }
        }
    }

    fn row_to_record(row: &coredb::query::result::Row) -> ResolutionRecord {
        let text = |key: &str| match row.columns.get(key) {
            Some(CassandraValue::Text(s)) => s.clone(),
            _ => String::new(),
        };
        // 빈 문자열은 None으로 정규화(차트/코멘트).
        let opt = |key: &str| {
            let v = text(key);
            if v.is_empty() || v == "null" {
                None
            } else {
                Some(v)
            }
        };
        ResolutionRecord {
            feedback_key: text("feedback_key"),
            employee_id: text("employee_id"),
            company_code: text("company_code"),
            agent_id: text("agent_id"),
            question: text("question"),
            old_answer: text("old_answer"),
            old_chart: opt("old_chart"),
            new_answer: text("new_answer"),
            new_chart: opt("new_chart"),
            status: text("status"),
            resolved_by: text("resolved_by"),
            resolved_at: text("resolved_at"),
            reverify_result: text("reverify_result"),
            reverify_comment: opt("reverify_comment"),
            reverify_at: text("reverify_at"),
        }
    }
}

impl Clone for CoreDbResolutionStore {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            keyspace: self.keyspace.clone(),
        }
    }
}
