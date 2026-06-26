//! CoreDB 기반 답변 공유(share) 저장소.
//!
//! AI 답변 1개를 공유 링크로 만들 때, 공유 시점의 본문 스냅샷과 생성자 회사코드를
//! 임베디드 CoreDB에 보관한다. 운영 PostgreSQL을 건드리지 않기 위해 대화기록 저장소
//! (`CoreDbConversationStore`)와 동일하게 CoreDB를 사용한다.

use coredb::{CassandraValue, CoreDB, QueryResult};
use pekko_agent_core::MemoryError;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// 공유 레코드(조회 결과).
pub struct SharedAnswerRecord {
    pub content: String,
    pub company_code: String,
    /// 차트 JSON 문자열(있을 때만).
    pub chart_data: Option<String>,
    /// 생성 시각(RFC3339 문자열).
    pub created_at: String,
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

/// CoreDB 기반 공유 저장소. 대화기록과 같은 `pekko_agent` keyspace를 공유한다.
pub struct CoreDbShareStore {
    db: Arc<CoreDB>,
    keyspace: String,
}

impl CoreDbShareStore {
    pub async fn new(db: Arc<CoreDB>) -> Result<Self, MemoryError> {
        let keyspace = "pekko_agent".to_string();
        Self::init_schema(&db, &keyspace).await?;
        info!(keyspace = %keyspace, "CoreDB share store initialized");
        Ok(Self { db, keyspace })
    }

    async fn init_schema(db: &CoreDB, keyspace: &str) -> Result<(), MemoryError> {
        let cql = format!(
            "CREATE KEYSPACE IF NOT EXISTS {} WITH REPLICATION = {{'class': 'SimpleStrategy', 'replication_factor': 1}}",
            keyspace
        );
        ignore_already_exists(db.execute_cql(&cql).await, "keyspace")?;

        // 공유 1건 = share_id(파티션 키) → 답변 스냅샷.
        let cql = format!(
            "CREATE TABLE IF NOT EXISTS {}.shares (
                share_id TEXT,
                content TEXT,
                company_code TEXT,
                chart_data TEXT,
                created_at TEXT,
                PRIMARY KEY (share_id)
            )",
            keyspace
        );
        ignore_already_exists(db.execute_cql(&cql).await, "shares table")?;
        debug!("CoreDB schema initialized for shares");
        Ok(())
    }

    /// 공유 레코드 저장. content/chart_data의 작은따옴표는 CQL 규칙대로 이스케이프한다.
    pub async fn create(
        &self,
        share_id: &str,
        content: &str,
        company_code: &str,
        chart_data: Option<&str>,
        created_at: &str,
    ) -> Result<(), MemoryError> {
        let esc = |s: &str| s.replace('\'', "''");
        let chart_lit = chart_data
            .map(|c| format!("'{}'", esc(c)))
            .unwrap_or_else(|| "null".to_string());

        let cql = format!(
            "INSERT INTO {}.shares (share_id, content, company_code, chart_data, created_at) \
             VALUES ('{}', '{}', '{}', {}, '{}')",
            self.keyspace,
            esc(share_id),
            esc(content),
            esc(company_code),
            chart_lit,
            esc(created_at)
        );

        self.db
            .execute_cql(&cql)
            .await
            .map_err(|e| MemoryError::StorageError(format!("Failed to create share: {}", e)))?;
        debug!(share_id = %share_id, "Share saved to CoreDB");
        Ok(())
    }

    /// share_id로 공유 레코드 조회. 없으면 `None`.
    pub async fn get(&self, share_id: &str) -> Result<Option<SharedAnswerRecord>, MemoryError> {
        let cql = format!(
            "SELECT content, company_code, chart_data, created_at FROM {}.shares WHERE share_id = '{}'",
            self.keyspace,
            share_id.replace('\'', "''")
        );

        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => {
                let Some(row) = rows.first() else {
                    return Ok(None);
                };
                let text = |key: &str| match row.columns.get(key) {
                    Some(CassandraValue::Text(s)) => Some(s.clone()),
                    _ => None,
                };
                let Some(content) = text("content") else {
                    return Ok(None);
                };
                Ok(Some(SharedAnswerRecord {
                    content,
                    company_code: text("company_code").unwrap_or_default(),
                    chart_data: text("chart_data"),
                    created_at: text("created_at").unwrap_or_default(),
                }))
            }
            Ok(_) => Ok(None),
            Err(e) => {
                warn!(error = %e, "Failed to get share from CoreDB");
                Err(MemoryError::StorageError(format!(
                    "Failed to get share: {}",
                    e
                )))
            }
        }
    }
}

impl Clone for CoreDbShareStore {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            keyspace: self.keyspace.clone(),
        }
    }
}
