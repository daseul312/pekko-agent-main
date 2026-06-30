//! CoreDB 기반 "질문별 개선 프롬프트(개선 지시)" 저장소.
//!
//! 사용자가 부정 피드백 → 관리자 조치(개선 답변 재생성) → 알림 → 재확인에서 "정확"으로
//! 승인하면, 그 답변을 만들어낸 개선 지시(directive)를 여기에 보관한다. 이후 같은 회사에서
//! 동일 질문을 다시 하면 이 지시를 system prompt에 주입해 첫 답변부터 개선이 적용되게 한다.
//!
//! 키는 `(company_code, agent_id, 정규화된 질문)`을 합친 `override_key`. 같은 질문을 다시
//! 승인하면 같은 행에 덮어쓴다. 운영 PostgreSQL을 건드리지 않기 위해 피드백·조치 저장소와
//! 동일하게 CoreDB `pekko_agent` keyspace를 쓴다.

use coredb::{CassandraValue, CoreDB, QueryResult};
use pekko_agent_core::MemoryError;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// 개선 프롬프트 1건(조회 결과).
pub struct PromptOverrideRecord {
    pub override_key: String,
    pub company_code: String,
    pub agent_id: String,
    /// 정규화된 질문(공백 제거+소문자). 매칭 키 구성 요소이자 디버그용.
    pub question_norm: String,
    /// system prompt에 주입할 개선 지시.
    pub directive: String,
    /// 이 개선을 만든 원 피드백 키(추적용).
    pub source_feedback_key: String,
    pub created_at: String,
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

/// `(company_code, agent_id, question_norm)`을 합친 결정적 키. 읽기/쓰기 양쪽에서 동일하게 쓴다.
pub fn override_key(company_code: &str, agent_id: &str, question_norm: &str) -> String {
    format!("{}|{}|{}", company_code, agent_id, question_norm)
}

/// CoreDB 기반 개선 프롬프트 저장소. 피드백·조치와 같은 `pekko_agent` keyspace를 공유한다.
pub struct CoreDbPromptOverrideStore {
    db: Arc<CoreDB>,
    keyspace: String,
}

impl CoreDbPromptOverrideStore {
    pub async fn new(db: Arc<CoreDB>) -> Result<Self, MemoryError> {
        let keyspace = "pekko_agent".to_string();
        Self::init_schema(&db, &keyspace).await?;
        info!(keyspace = %keyspace, "CoreDB prompt-override store initialized");
        Ok(Self { db, keyspace })
    }

    async fn init_schema(db: &CoreDB, keyspace: &str) -> Result<(), MemoryError> {
        let cql = format!(
            "CREATE KEYSPACE IF NOT EXISTS {} WITH REPLICATION = {{'class': 'SimpleStrategy', 'replication_factor': 1}}",
            keyspace
        );
        ignore_already_exists(db.execute_cql(&cql).await, "keyspace")?;

        let cql = format!(
            "CREATE TABLE IF NOT EXISTS {}.prompt_overrides (
                override_key TEXT,
                company_code TEXT,
                agent_id TEXT,
                question_norm TEXT,
                directive TEXT,
                source_feedback_key TEXT,
                created_at TEXT,
                PRIMARY KEY (override_key)
            )",
            keyspace
        );
        ignore_already_exists(db.execute_cql(&cql).await, "prompt_overrides table")?;
        debug!("CoreDB schema initialized for prompt_overrides");
        Ok(())
    }

    /// 승인된 개선 지시를 저장(덮어쓰기). 같은 (회사·에이전트·질문)이면 같은 행에 갱신.
    pub async fn upsert(
        &self,
        company_code: &str,
        agent_id: &str,
        question_norm: &str,
        directive: &str,
        source_feedback_key: &str,
        created_at: &str,
    ) -> Result<(), MemoryError> {
        let key = override_key(company_code, agent_id, question_norm);
        let cql = format!(
            "INSERT INTO {}.prompt_overrides \
             (override_key, company_code, agent_id, question_norm, directive, source_feedback_key, created_at) \
             VALUES ('{}', '{}', '{}', '{}', '{}', '{}', '{}')",
            self.keyspace,
            esc(&key),
            esc(company_code),
            esc(agent_id),
            esc(question_norm),
            esc(directive),
            esc(source_feedback_key),
            esc(created_at),
        );
        self.db
            .execute_cql(&cql)
            .await
            .map_err(|e| MemoryError::StorageError(format!("Failed to upsert prompt override: {}", e)))?;
        debug!(override_key = %key, "Prompt override saved to CoreDB");
        Ok(())
    }

    /// `(company_code, agent_id, question_norm)`로 단건 조회. 없으면 `None`.
    pub async fn get(
        &self,
        company_code: &str,
        agent_id: &str,
        question_norm: &str,
    ) -> Result<Option<PromptOverrideRecord>, MemoryError> {
        let key = override_key(company_code, agent_id, question_norm);
        let cql = format!(
            "SELECT override_key, company_code, agent_id, question_norm, directive, source_feedback_key, created_at FROM {}.prompt_overrides WHERE override_key = '{}'",
            self.keyspace,
            esc(&key)
        );
        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => Ok(rows.first().map(Self::row_to_record)),
            Ok(_) => Ok(None),
            Err(e) => {
                warn!(error = %e, "Failed to get prompt override from CoreDB");
                Err(MemoryError::StorageError(format!("Failed to get prompt override: {}", e)))
            }
        }
    }

    fn row_to_record(row: &coredb::query::result::Row) -> PromptOverrideRecord {
        let text = |key: &str| match row.columns.get(key) {
            Some(CassandraValue::Text(s)) => s.clone(),
            _ => String::new(),
        };
        PromptOverrideRecord {
            override_key: text("override_key"),
            company_code: text("company_code"),
            agent_id: text("agent_id"),
            question_norm: text("question_norm"),
            directive: text("directive"),
            source_feedback_key: text("source_feedback_key"),
            created_at: text("created_at"),
        }
    }
}

impl Clone for CoreDbPromptOverrideStore {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            keyspace: self.keyspace.clone(),
        }
    }
}
