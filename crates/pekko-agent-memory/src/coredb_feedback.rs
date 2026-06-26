//! CoreDB 기반 답변 피드백(👍/👎) 저장소.
//!
//! 사용자가 AI 답변 1개에 대해 도움 여부를 평가(👍/👎)하고 선택적 코멘트를 남길 때,
//! 그 평가를 임베디드 CoreDB에 보관한다. 운영 PostgreSQL을 건드리지 않기 위해
//! 공유(`CoreDbShareStore`)·대화기록 저장소와 동일하게 CoreDB `pekko_agent` keyspace를 쓴다.
//!
//! 한 평가는 `(session_id, message_id, response_index)`로 유일하게 식별되며, 같은 키로
//! 다시 저장하면 덮어쓴다(재투표·👍↔👎 변경). 토글 취소는 `delete`로 처리한다.

use coredb::{CassandraValue, CoreDB, QueryResult};
use pekko_agent_core::MemoryError;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// 피드백 1건(조회 결과). 통계 집계 입력으로 쓴다.
pub struct FeedbackRecord {
    pub agent_id: String,
    /// "up" | "down".
    pub rating: String,
    pub comment: Option<String>,
    pub employee_id: String,
    pub company_code: String,
    pub content_excerpt: Option<String>,
    /// 생성 시각(RFC3339 문자열).
    pub created_at: String,
    pub session_id: String,
    pub message_id: String,
    pub response_index: i64,
    /// 부정 사유 태그(👎일 때만 채워짐). 병렬 `feedback_tags` 테이블에서 결합.
    pub tags: Vec<String>,
}

/// CoreDB의 `CREATE ... IF NOT EXISTS`가 이미 있으면 에러를 낼 수 있어, "already exists"는
/// 정상으로 보고 무시한다(영구 저장이라 재시작 시 이미 존재).
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

/// CoreDB 기반 피드백 저장소. 대화기록·공유와 같은 `pekko_agent` keyspace를 공유한다.
pub struct CoreDbFeedbackStore {
    db: Arc<CoreDB>,
    keyspace: String,
}

impl CoreDbFeedbackStore {
    pub async fn new(db: Arc<CoreDB>) -> Result<Self, MemoryError> {
        let keyspace = "pekko_agent".to_string();
        Self::init_schema(&db, &keyspace).await?;
        info!(keyspace = %keyspace, "CoreDB feedback store initialized");
        Ok(Self { db, keyspace })
    }

    async fn init_schema(db: &CoreDB, keyspace: &str) -> Result<(), MemoryError> {
        let cql = format!(
            "CREATE KEYSPACE IF NOT EXISTS {} WITH REPLICATION = {{'class': 'SimpleStrategy', 'replication_factor': 1}}",
            keyspace
        );
        ignore_already_exists(db.execute_cql(&cql).await, "keyspace")?;

        // 평가 1건 = feedback_key(파티션 키) → 평가 내용.
        // feedback_key = "{session_id}|{message_id}|{response_index}" 로 동일 답변(버전)에 대한
        // 재투표를 같은 행에 덮어쓴다.
        let cql = format!(
            "CREATE TABLE IF NOT EXISTS {}.feedback (
                feedback_key TEXT,
                session_id TEXT,
                message_id TEXT,
                response_index BIGINT,
                agent_id TEXT,
                rating TEXT,
                comment TEXT,
                employee_id TEXT,
                company_code TEXT,
                content_excerpt TEXT,
                created_at TEXT,
                PRIMARY KEY (feedback_key)
            )",
            keyspace
        );
        ignore_already_exists(db.execute_cql(&cql).await, "feedback table")?;

        // 부정 사유 태그(👎)는 기존 feedback 테이블 ALTER 위험을 피해 병렬 테이블에 저장한다.
        // tags = '|'로 이은 고정 사유 문자열(예: "내용 부정확|최신 정보 미반영").
        let cql = format!(
            "CREATE TABLE IF NOT EXISTS {}.feedback_tags (
                feedback_key TEXT,
                tags TEXT,
                PRIMARY KEY (feedback_key)
            )",
            keyspace
        );
        ignore_already_exists(db.execute_cql(&cql).await, "feedback_tags table")?;

        debug!("CoreDB schema initialized for feedback");
        Ok(())
    }

    /// 피드백 저장(upsert). 동일 `feedback_key` INSERT는 CoreDB에서 덮어쓰기로 동작한다.
    /// 문자열의 작은따옴표는 CQL 규칙대로 이스케이프한다.
    #[allow(clippy::too_many_arguments)]
    pub async fn upsert(
        &self,
        feedback_key: &str,
        session_id: &str,
        message_id: &str,
        response_index: i64,
        agent_id: &str,
        rating: &str,
        comment: Option<&str>,
        employee_id: &str,
        company_code: &str,
        content_excerpt: Option<&str>,
        created_at: &str,
        tags: &[String],
    ) -> Result<(), MemoryError> {
        let esc = |s: &str| s.replace('\'', "''");
        // 값이 없으면 빈 문자열로 저장한다. CoreDB는 bareword `null`을 SQL NULL이 아니라
        // 문자열 "null"로 저장하는 문제가 있어, 읽을 때 가짜 코멘트로 보이기 때문이다.
        let opt_lit = |v: Option<&str>| {
            v.map(|c| format!("'{}'", esc(c)))
                .unwrap_or_else(|| "''".to_string())
        };

        let cql = format!(
            "INSERT INTO {}.feedback \
             (feedback_key, session_id, message_id, response_index, agent_id, rating, comment, employee_id, company_code, content_excerpt, created_at) \
             VALUES ('{}', '{}', '{}', {}, '{}', '{}', {}, '{}', '{}', {}, '{}')",
            self.keyspace,
            esc(feedback_key),
            esc(session_id),
            esc(message_id),
            response_index,
            esc(agent_id),
            esc(rating),
            opt_lit(comment),
            esc(employee_id),
            esc(company_code),
            opt_lit(content_excerpt),
            esc(created_at),
        );

        self.db
            .execute_cql(&cql)
            .await
            .map_err(|e| MemoryError::StorageError(format!("Failed to upsert feedback: {}", e)))?;

        // 태그는 병렬 테이블에. 비어있으면(재투표로 태그 제거 등) 기존 행을 지운다.
        let clean: Vec<&str> = tags
            .iter()
            .map(|t| t.trim())
            .filter(|t| !t.is_empty() && !t.contains('|'))
            .collect();
        if clean.is_empty() {
            let _ = self
                .db
                .execute_cql(&format!(
                    "DELETE FROM {}.feedback_tags WHERE feedback_key = '{}'",
                    self.keyspace,
                    esc(feedback_key)
                ))
                .await;
        } else {
            let joined = clean.join("|");
            let cql = format!(
                "INSERT INTO {}.feedback_tags (feedback_key, tags) VALUES ('{}', '{}')",
                self.keyspace,
                esc(feedback_key),
                esc(&joined)
            );
            self.db.execute_cql(&cql).await.map_err(|e| {
                MemoryError::StorageError(format!("Failed to upsert feedback tags: {}", e))
            })?;
        }

        debug!(feedback_key = %feedback_key, rating = %rating, "Feedback saved to CoreDB");
        Ok(())
    }

    /// 피드백 삭제(토글 취소). 없는 키여도 에러 없이 통과한다. 태그도 함께 삭제.
    pub async fn delete(&self, feedback_key: &str) -> Result<(), MemoryError> {
        let key = feedback_key.replace('\'', "''");
        let cql = format!(
            "DELETE FROM {}.feedback WHERE feedback_key = '{}'",
            self.keyspace, key
        );
        self.db
            .execute_cql(&cql)
            .await
            .map_err(|e| MemoryError::StorageError(format!("Failed to delete feedback: {}", e)))?;
        let _ = self
            .db
            .execute_cql(&format!(
                "DELETE FROM {}.feedback_tags WHERE feedback_key = '{}'",
                self.keyspace, key
            ))
            .await;
        debug!(feedback_key = %feedback_key, "Feedback deleted from CoreDB");
        Ok(())
    }

    /// feedback_key → 태그 목록 맵을 읽는다(통계 결합용).
    async fn load_tags_map(&self) -> std::collections::HashMap<String, Vec<String>> {
        let mut map = std::collections::HashMap::new();
        let cql = format!("SELECT feedback_key, tags FROM {}.feedback_tags", self.keyspace);
        if let Ok(QueryResult::Rows(rows)) = self.db.execute_cql(&cql).await {
            for row in &rows {
                let key = match row.columns.get("feedback_key") {
                    Some(CassandraValue::Text(s)) => s.clone(),
                    _ => continue,
                };
                if let Some(CassandraValue::Text(joined)) = row.columns.get("tags") {
                    let tags: Vec<String> = joined
                        .split('|')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty() && t != "null")
                        .collect();
                    if !tags.is_empty() {
                        map.insert(key, tags);
                    }
                }
            }
        }
        map
    }

    /// 전체 피드백 행을 읽어 반환한다(통계 집계용). CoreDB는 WHERE 없는 풀스캔을 지원한다.
    /// 회사/에이전트 필터·집계는 호출부에서 수행한다.
    pub async fn list_all(&self) -> Result<Vec<FeedbackRecord>, MemoryError> {
        let tags_map = self.load_tags_map().await;
        let cql = format!(
            "SELECT feedback_key, agent_id, rating, comment, employee_id, company_code, content_excerpt, created_at, session_id, message_id, response_index FROM {}.feedback",
            self.keyspace
        );

        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => {
                let records = rows
                    .iter()
                    .filter_map(|row| {
                        let text = |key: &str| match row.columns.get(key) {
                            Some(CassandraValue::Text(s)) => Some(s.clone()),
                            _ => None,
                        };
                        let response_index = match row.columns.get("response_index") {
                            Some(CassandraValue::BigInt(n)) => *n,
                            Some(CassandraValue::Int(n)) => *n as i64,
                            _ => 0,
                        };
                        // rating/agent_id가 없는 손상 행은 건너뛴다.
                        let rating = text("rating")?;
                        let tags = text("feedback_key")
                            .and_then(|k| tags_map.get(&k).cloned())
                            .unwrap_or_default();
                        Some(FeedbackRecord {
                            agent_id: text("agent_id").unwrap_or_default(),
                            rating,
                            comment: text("comment"),
                            employee_id: text("employee_id").unwrap_or_default(),
                            company_code: text("company_code").unwrap_or_default(),
                            content_excerpt: text("content_excerpt"),
                            created_at: text("created_at").unwrap_or_default(),
                            session_id: text("session_id").unwrap_or_default(),
                            message_id: text("message_id").unwrap_or_default(),
                            response_index,
                            tags,
                        })
                    })
                    .collect();
                Ok(records)
            }
            // 빈 테이블 등 Rows가 아닌 결과는 빈 목록으로 처리.
            Ok(_) => Ok(Vec::new()),
            Err(e) => {
                warn!(error = %e, "Failed to list feedback from CoreDB");
                Err(MemoryError::StorageError(format!(
                    "Failed to list feedback: {}",
                    e
                )))
            }
        }
    }
}

impl Clone for CoreDbFeedbackStore {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            keyspace: self.keyspace.clone(),
        }
    }
}
