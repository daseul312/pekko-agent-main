pub mod conversation;
pub mod vector_store;
pub mod episodic;

// CoreDB-backed implementations (replaces Redis/PostgreSQL)
pub mod coredb_conversation;
pub mod coredb_episodic;
pub mod coredb_journal;
pub mod coredb_share;
pub mod coredb_feedback;
pub mod coredb_resolution;
pub mod coredb_notification;
pub mod coredb_prompt_override;

pub use conversation::*;
pub use vector_store::*;
pub use episodic::*;

// CoreDB exports
pub use coredb_conversation::{CoreDbConversationStore, ConversationTurn};
pub use coredb_episodic::CoreDbEpisodicStore;
pub use coredb_journal::{CoreDbJournal, CoreDbSnapshotStore};
pub use coredb_share::{CoreDbShareStore, SharedAnswerRecord};
pub use coredb_feedback::{CoreDbFeedbackStore, FeedbackRecord};
pub use coredb_resolution::{CoreDbResolutionStore, ResolutionRecord};
pub use coredb_notification::{CoreDbNotificationStore, NotificationRecord};
pub use coredb_prompt_override::{CoreDbPromptOverrideStore, PromptOverrideRecord};
