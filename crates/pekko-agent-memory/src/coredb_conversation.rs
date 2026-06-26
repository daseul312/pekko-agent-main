use async_trait::async_trait;
use pekko_agent_core::{ShortTermMemory, Message, MessageRole, MemoryError};
use coredb::{CoreDB, DatabaseConfig, QueryResult};
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;
use tracing::{info, debug, warn};

/// CoreDB-backed conversation store
///
/// Uses embedded coreDB (Cassandra-style NoSQL) for persistent conversation storage.
/// Replaces Redis with a single embedded database — no external dependencies needed.
pub struct CoreDbConversationStore {
    db: Arc<CoreDB>,
    max_messages: usize,
    keyspace: String,
}

/// coreDB의 `CREATE ... IF NOT EXISTS`는 멱등이 아니라 이미 있으면 에러를 반환한다.
/// 영구 저장이라 서버 재시작 시 keyspace/table이 이미 존재하므로, "already exists" 류
/// 에러는 정상으로 보고 무시하고 그 외 에러만 전파한다.
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

impl CoreDbConversationStore {
    /// Create a new CoreDB-backed conversation store
    ///
    /// # Arguments
    /// * `db` - Shared CoreDB instance
    /// * `max_messages` - Maximum messages to keep per conversation
    pub async fn new(db: Arc<CoreDB>, max_messages: usize) -> Result<Self, MemoryError> {
        let keyspace = "pekko_agent".to_string();

        // Initialize schema
        Self::init_schema(&db, &keyspace).await?;

        info!(keyspace = %keyspace, "CoreDB conversation store initialized");

        Ok(Self {
            db,
            max_messages,
            keyspace,
        })
    }

    /// Create with a new embedded CoreDB instance
    pub async fn with_embedded(
        data_dir: PathBuf,
        max_messages: usize,
    ) -> Result<Self, MemoryError> {
        let config = DatabaseConfig {
            data_directory: data_dir.join("data"),
            commitlog_directory: data_dir.join("commitlog"),
            ..Default::default()
        };

        let db = CoreDB::new(config).await.map_err(|e| {
            MemoryError::StorageError(format!("Failed to initialize CoreDB: {}", e))
        })?;

        Self::new(Arc::new(db), max_messages).await
    }

    /// Initialize keyspace and table schema
    async fn init_schema(db: &CoreDB, keyspace: &str) -> Result<(), MemoryError> {
        // Create keyspace
        let cql = format!(
            "CREATE KEYSPACE IF NOT EXISTS {} WITH REPLICATION = {{'class': 'SimpleStrategy', 'replication_factor': 1}}",
            keyspace
        );
        ignore_already_exists(db.execute_cql(&cql).await, "keyspace")?;

        // Create conversations table
        // partition key: session_id, clustering key: seq_num (for ordering)
        let cql = format!(
            "CREATE TABLE IF NOT EXISTS {}.conversations (
                session_id TEXT,
                seq_num BIGINT,
                role TEXT,
                content TEXT,
                timestamp TEXT,
                PRIMARY KEY (session_id, seq_num)
            )",
            keyspace
        );
        ignore_already_exists(db.execute_cql(&cql).await, "conversations table")?;

        // 메시지별 차트 데이터(불투명 JSON). 대화 본문(conversations)은 LLM 컨텍스트로도
        // 쓰여 가볍게 유지해야 하므로, 차트는 별도 테이블에 (session_id, seq_num)로 분리 저장한다.
        // 읽기전용 대화 뷰어가 답변 아래 그래프를 복원하는 데 쓴다.
        let cql = format!(
            "CREATE TABLE IF NOT EXISTS {}.message_charts (
                session_id TEXT,
                seq_num BIGINT,
                chart_data TEXT,
                PRIMARY KEY (session_id, seq_num)
            )",
            keyspace
        );
        ignore_already_exists(db.execute_cql(&cql).await, "message_charts table")?;

        debug!("CoreDB schema initialized for conversations");
        Ok(())
    }

    /// Get current max sequence number for a session
    async fn get_max_seq(&self, session_id: &Uuid) -> Result<i64, MemoryError> {
        let cql = format!(
            "SELECT MAX(seq_num) FROM {}.conversations WHERE session_id = '{}'",
            self.keyspace, session_id
        );

        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => {
                if let Some(row) = rows.first() {
                    if let Some(coredb::CassandraValue::BigInt(n)) = row.columns.get("max_seq_num") {
                        return Ok(*n);
                    }
                    // Try system_max(seq_num) column name variant
                    for (_, val) in &row.columns {
                        if let coredb::CassandraValue::BigInt(n) = val {
                            return Ok(*n);
                        }
                    }
                }
                Ok(0)
            }
            Ok(_) => Ok(0),
            Err(_) => Ok(0),
        }
    }

    /// Get the number of active conversations
    pub async fn conversation_count(&self) -> usize {
        let cql = format!(
            "SELECT DISTINCT session_id FROM {}.conversations",
            self.keyspace
        );
        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => rows.len(),
            _ => 0,
        }
    }

    /// Get the number of messages in a conversation
    pub async fn message_count(&self, session_id: &Uuid) -> usize {
        let cql = format!(
            "SELECT COUNT(*) FROM {}.conversations WHERE session_id = '{}'",
            self.keyspace, session_id
        );
        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => {
                if let Some(row) = rows.first() {
                    for (_, val) in &row.columns {
                        if let coredb::CassandraValue::BigInt(n) = val {
                            return *n as usize;
                        }
                        if let coredb::CassandraValue::Int(n) = val {
                            return *n as usize;
                        }
                    }
                }
                0
            }
            _ => 0,
        }
    }

    /// Delete a conversation
    pub async fn delete_conversation(&self, session_id: &Uuid) -> Result<(), MemoryError> {
        let cql = format!(
            "DELETE FROM {}.conversations WHERE session_id = '{}'",
            self.keyspace, session_id
        );
        self.db.execute_cql(&cql).await.map_err(|e| {
            MemoryError::StorageError(format!("Failed to delete conversation: {}", e))
        })?;
        info!(session_id = %session_id, "Conversation deleted from CoreDB");
        Ok(())
    }

    /// Convert CQL row to Message
    fn row_to_message(row: &coredb::query::result::Row) -> Option<Message> {
        let role_str = match row.columns.get("role") {
            Some(coredb::CassandraValue::Text(s)) => s.clone(),
            _ => return None,
        };
        let content = match row.columns.get("content") {
            Some(coredb::CassandraValue::Text(s)) => s.clone(),
            _ => return None,
        };
        let timestamp_str = match row.columns.get("timestamp") {
            Some(coredb::CassandraValue::Text(s)) => s.clone(),
            _ => return None,
        };

        let role = match role_str.as_str() {
            "user" => MessageRole::User,
            "assistant" => MessageRole::Assistant,
            "system" => MessageRole::System,
            "tool" => MessageRole::Tool,
            _ => MessageRole::User,
        };

        let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp_str)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now());

        Some(Message {
            role,
            content,
            timestamp,
        })
    }
}

#[async_trait]
impl ShortTermMemory for CoreDbConversationStore {
    async fn get_conversation(&self, session_id: &Uuid) -> Result<Vec<Message>, MemoryError> {
        let cql = format!(
            "SELECT role, content, timestamp FROM {}.conversations WHERE session_id = '{}' ORDER BY seq_num",
            self.keyspace, session_id
        );

        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => {
                let messages: Vec<Message> = rows
                    .iter()
                    .filter_map(|row| Self::row_to_message(row))
                    .collect();
                Ok(messages)
            }
            Ok(_) => Ok(vec![]),
            Err(e) => {
                warn!(error = %e, "Failed to get conversation from CoreDB");
                Ok(vec![])
            }
        }
    }

    async fn append_message(&self, session_id: &Uuid, msg: Message) -> Result<(), MemoryError> {
        self.append_returning_seq(session_id, msg).await.map(|_| ())
    }

    async fn summarize(&self, session_id: &Uuid) -> Result<String, MemoryError> {
        let messages = self.get_conversation(session_id).await?;

        if messages.is_empty() {
            return Err(MemoryError::NotFound(format!("Session {} not found", session_id)));
        }

        let summary = messages
            .iter()
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n---\n");

        Ok(summary)
    }

    async fn clear(&self, session_id: &Uuid) -> Result<(), MemoryError> {
        self.delete_conversation(session_id).await?;
        info!(session_id = %session_id, "Conversation cleared in CoreDB");
        Ok(())
    }
}

impl CoreDbConversationStore {
    /// 메시지를 저장하고 부여된 seq_num을 돌려준다(차트를 같은 메시지에 연결하기 위함).
    /// 트레잇 `append_message`는 이 메서드에 위임한다.
    pub async fn append_returning_seq(
        &self,
        session_id: &Uuid,
        msg: Message,
    ) -> Result<i64, MemoryError> {
        let seq = self.get_max_seq(session_id).await? + 1;

        let role_str = match msg.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::System => "system",
            MessageRole::Tool => "tool",
        };

        let timestamp_str = msg.timestamp.to_rfc3339();
        // Escape single quotes in content
        let escaped_content = msg.content.replace('\'', "''");

        let cql = format!(
            "INSERT INTO {}.conversations (session_id, seq_num, role, content, timestamp) VALUES ('{}', {}, '{}', '{}', '{}')",
            self.keyspace, session_id, seq, role_str, escaped_content, timestamp_str
        );

        self.db.execute_cql(&cql).await.map_err(|e| {
            MemoryError::StorageError(format!("Failed to append message: {}", e))
        })?;

        debug!(session_id = %session_id, role = %role_str, seq = seq, "Message appended to CoreDB");

        // Enforce max_messages limit — delete oldest messages if over limit
        let count = self.message_count(session_id).await;
        if count > self.max_messages {
            let overflow = count - self.max_messages;
            // Get the seq_nums of oldest messages to delete
            let cql = format!(
                "SELECT seq_num FROM {}.conversations WHERE session_id = '{}' ORDER BY seq_num LIMIT {}",
                self.keyspace, session_id, overflow
            );
            if let Ok(QueryResult::Rows(rows)) = self.db.execute_cql(&cql).await {
                for row in &rows {
                    if let Some(coredb::CassandraValue::BigInt(seq_to_delete)) = row.columns.get("seq_num") {
                        let delete_cql = format!(
                            "DELETE FROM {}.conversations WHERE session_id = '{}' AND seq_num = {}",
                            self.keyspace, session_id, seq_to_delete
                        );
                        let _ = self.db.execute_cql(&delete_cql).await;
                    }
                }
                debug!(session_id = %session_id, removed = overflow, "Trimmed conversation history in CoreDB");
            }
        }

        Ok(seq)
    }

    /// 특정 메시지(seq_num)에 차트 JSON을 연결 저장한다(불투명 문자열).
    pub async fn set_message_chart(
        &self,
        session_id: &Uuid,
        seq_num: i64,
        chart_json: &str,
    ) -> Result<(), MemoryError> {
        let escaped = chart_json.replace('\'', "''");
        let cql = format!(
            "INSERT INTO {}.message_charts (session_id, seq_num, chart_data) VALUES ('{}', {}, '{}')",
            self.keyspace, session_id, seq_num, escaped
        );
        self.db.execute_cql(&cql).await.map_err(|e| {
            MemoryError::StorageError(format!("Failed to set message chart: {}", e))
        })?;
        debug!(session_id = %session_id, seq = seq_num, "Message chart saved to CoreDB");
        Ok(())
    }

    /// 세션의 대화를 seq 순서로 (role, content, chart_data?) 형태로 돌려준다.
    /// `role`은 트레잇 `Message`와 동일하게 PascalCase("User"/"Assistant"/"System"/"Tool")로
    /// 직렬화해, 읽기전용 뷰어가 차트까지 복원할 수 있게 한다.
    pub async fn get_conversation_with_charts(
        &self,
        session_id: &Uuid,
    ) -> Vec<ConversationTurn> {
        // 차트 맵(seq_num → chart_json)을 먼저 읽는다.
        let mut charts: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
        let chart_cql = format!(
            "SELECT seq_num, chart_data FROM {}.message_charts WHERE session_id = '{}'",
            self.keyspace, session_id
        );
        if let Ok(QueryResult::Rows(rows)) = self.db.execute_cql(&chart_cql).await {
            for row in &rows {
                let seq = match row.columns.get("seq_num") {
                    Some(coredb::CassandraValue::BigInt(n)) => *n,
                    Some(coredb::CassandraValue::Int(n)) => *n as i64,
                    _ => continue,
                };
                if let Some(coredb::CassandraValue::Text(json)) = row.columns.get("chart_data") {
                    charts.insert(seq, json.clone());
                }
            }
        }

        let cql = format!(
            "SELECT role, content, seq_num FROM {}.conversations WHERE session_id = '{}' ORDER BY seq_num",
            self.keyspace, session_id
        );
        match self.db.execute_cql(&cql).await {
            Ok(QueryResult::Rows(rows)) => rows
                .iter()
                .filter_map(|row| {
                    let role = match row.columns.get("role") {
                        Some(coredb::CassandraValue::Text(s)) => match s.as_str() {
                            "user" => "User",
                            "assistant" => "Assistant",
                            "system" => "System",
                            "tool" => "Tool",
                            _ => "Assistant",
                        },
                        _ => return None,
                    };
                    let content = match row.columns.get("content") {
                        Some(coredb::CassandraValue::Text(s)) => s.clone(),
                        _ => return None,
                    };
                    let seq = match row.columns.get("seq_num") {
                        Some(coredb::CassandraValue::BigInt(n)) => *n,
                        Some(coredb::CassandraValue::Int(n)) => *n as i64,
                        _ => -1,
                    };
                    Some(ConversationTurn {
                        role: role.to_string(),
                        content,
                        chart_data: charts.get(&seq).cloned(),
                    })
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// 읽기전용 대화 뷰어용 턴(역할·본문·선택적 차트 JSON).
pub struct ConversationTurn {
    pub role: String,
    pub content: String,
    pub chart_data: Option<String>,
}

impl Clone for CoreDbConversationStore {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            max_messages: self.max_messages,
            keyspace: self.keyspace.clone(),
        }
    }
}
