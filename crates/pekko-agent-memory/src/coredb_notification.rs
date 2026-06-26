//! CoreDB 기반 인앱 알림 저장소.
//!
//! 관리자가 부정 피드백을 "조치 완료"로 발송하면 해당 피드백을 남긴 사용자에게 알림 1건을
//! 만든다. 사용자는 헤더 종 아이콘으로 폴링 조회한다. 운영 PostgreSQL을 건드리지 않기 위해
//! CoreDB `pekko_agent` keyspace를 쓴다. 파티션 키는 알림 id이고, 수신자별 조회는 풀스캔
//! 후 employee_id로 필터한다(알림량이 적어 충분).

use coredb::{CassandraValue, CoreDB, QueryResult};
use pekko_agent_core::MemoryError;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// 알림 레코드.
pub struct NotificationRecord {
    pub notification_id: String,
    pub employee_id: String,
    pub company_code: String,
    pub feedback_key: String,
    pub kind: String,
    pub message: String,
    pub is_read: bool,
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

/// CoreDB 기반 알림 저장소. 피드백·공유와 같은 `pekko_agent` keyspace를 공유한다.
pub struct CoreDbNotificationStore {
    db: Arc<CoreDB>,
    keyspace: String,
}

impl CoreDbNotificationStore {
    pub async fn new(db: Arc<CoreDB>) -> Result<Self, MemoryError> {
        let keyspace = "pekko_agent".to_string();
        Self::init_schema(&db, &keyspace).await?;
        info!(keyspace = %keyspace, "CoreDB notification store initialized");
        Ok(Self { db, keyspace })
    }

    async fn init_schema(db: &CoreDB, keyspace: &str) -> Result<(), MemoryError> {
        let cql = format!(
            "CREATE KEYSPACE IF NOT EXISTS {} WITH REPLICATION = {{'class': 'SimpleStrategy', 'replication_factor': 1}}",
            keyspace
        );
        ignore_already_exists(db.execute_cql(&cql).await, "keyspace")?;

        let cql = format!(
            "CREATE TABLE IF NOT EXISTS {}.notifications (
                notification_id TEXT,
                employee_id TEXT,
                company_code TEXT,
                feedback_key TEXT,
                kind TEXT,
                message TEXT,
                is_read INT,
                created_at TEXT,
                PRIMARY KEY (notification_id)
            )",
            keyspace
        );
        ignore_already_exists(db.execute_cql(&cql).await, "notifications table")?;
        debug!("CoreDB schema initialized for notifications");
        Ok(())
    }

    /// 알림 생성.
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        &self,
        notification_id: &str,
        employee_id: &str,
        company_code: &str,
        feedback_key: &str,
        kind: &str,
        message: &str,
        created_at: &str,
    ) -> Result<(), MemoryError> {
        let cql = format!(
            "INSERT INTO {}.notifications \
             (notification_id, employee_id, company_code, feedback_key, kind, message, is_read, created_at) \
             VALUES ('{}', '{}', '{}', '{}', '{}', '{}', 0, '{}')",
            self.keyspace,
            esc(notification_id),
            esc(employee_id),
            esc(company_code),
            esc(feedback_key),
            esc(kind),
            esc(message),
            esc(created_at),
        );
        self.db
            .execute_cql(&cql)
            .await
            .map_err(|e| MemoryError::StorageError(format!("Failed to create notification: {}", e)))?;
        debug!(employee_id = %employee_id, "Notification created in CoreDB");
        Ok(())
    }

    /// 수신자의 알림을 최신순으로 반환.
    pub async fn list_for(&self, employee_id: &str) -> Result<Vec<NotificationRecord>, MemoryError> {
        let cql = format!(
            "SELECT notification_id, employee_id, company_code, feedback_key, kind, message, is_read, created_at FROM {}.notifications",
            self.keyspace
        );
        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => {
                let mut list: Vec<NotificationRecord> = rows
                    .iter()
                    .map(Self::row_to_record)
                    .filter(|n| n.employee_id == employee_id)
                    .collect();
                list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
                Ok(list)
            }
            Ok(_) => Ok(Vec::new()),
            Err(e) => {
                warn!(error = %e, "Failed to list notifications from CoreDB");
                Err(MemoryError::StorageError(format!("Failed to list notifications: {}", e)))
            }
        }
    }

    /// 단건 조회(소유자 검증용).
    pub async fn get(&self, notification_id: &str) -> Result<Option<NotificationRecord>, MemoryError> {
        let cql = format!(
            "SELECT notification_id, employee_id, company_code, feedback_key, kind, message, is_read, created_at FROM {}.notifications WHERE notification_id = '{}'",
            self.keyspace,
            esc(notification_id)
        );
        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => Ok(rows.first().map(Self::row_to_record)),
            Ok(_) => Ok(None),
            Err(e) => Err(MemoryError::StorageError(format!("Failed to get notification: {}", e))),
        }
    }

    /// 읽음 처리(소유자만). 권한 밖이면 조용히 무시.
    pub async fn mark_read(&self, notification_id: &str, employee_id: &str) -> Result<(), MemoryError> {
        let Some(n) = self.get(notification_id).await? else {
            return Ok(());
        };
        if n.employee_id != employee_id {
            return Ok(());
        }
        let cql = format!(
            "INSERT INTO {}.notifications \
             (notification_id, employee_id, company_code, feedback_key, kind, message, is_read, created_at) \
             VALUES ('{}', '{}', '{}', '{}', '{}', '{}', 1, '{}')",
            self.keyspace,
            esc(&n.notification_id),
            esc(&n.employee_id),
            esc(&n.company_code),
            esc(&n.feedback_key),
            esc(&n.kind),
            esc(&n.message),
            esc(&n.created_at),
        );
        self.db
            .execute_cql(&cql)
            .await
            .map_err(|e| MemoryError::StorageError(format!("Failed to mark notification read: {}", e)))?;
        Ok(())
    }

    fn row_to_record(row: &coredb::query::result::Row) -> NotificationRecord {
        let text = |key: &str| match row.columns.get(key) {
            Some(CassandraValue::Text(s)) => s.clone(),
            _ => String::new(),
        };
        let is_read = match row.columns.get("is_read") {
            Some(CassandraValue::Int(n)) => *n != 0,
            Some(CassandraValue::BigInt(n)) => *n != 0,
            _ => false,
        };
        NotificationRecord {
            notification_id: text("notification_id"),
            employee_id: text("employee_id"),
            company_code: text("company_code"),
            feedback_key: text("feedback_key"),
            kind: text("kind"),
            message: text("message"),
            is_read,
            created_at: text("created_at"),
        }
    }
}

impl Clone for CoreDbNotificationStore {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            keyspace: self.keyspace.clone(),
        }
    }
}
