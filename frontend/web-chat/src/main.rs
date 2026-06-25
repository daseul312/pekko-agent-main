mod api;
mod components;
pub mod markdown;
mod types;

use gloo_storage::{LocalStorage, Storage};
use uuid::Uuid;
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;

use components::agent_selector::AgentSelector;
use components::chat::ChatArea;
use components::header::ChatHeader;
use components::input::ChatInput;
use components::sidebar::Sidebar;
use types::{
    AgentMeta, AuthUser, BoardPostCompletion, ChatMessage, ChatMessageUi, ChatSession, MessageRole,
    OldNewComparison, ProcessEvent, ProcessStep, ResponseVersion, TokenUsage,
    BOARD_POST_ANALYZING_AGENT_ID, COMPLIANCE_AGENT_ID, OTHER_AGENT_ID,
};

const THEME_STORAGE_KEY: &str = "pekko_theme_dark";

const STORAGE_KEY: &str = "pekko_chat_sessions";
const PENDING_PERMIT_WORK_TYPES: [&str; 10] = [
    "공통작업",
    "화기작업",
    "중장비 취급작업",
    "고소작업",
    "밀폐공간작업",
    "유해화학물질취급작업",
    "굴착작업",
    "전기작업",
    "방사선작업",
    "중량물 취급작업",
];
const CHECKLIST_WORK_TYPE_SUGGESTIONS: [&str; 10] = [
    "화기작업",
    "중장비 취급작업",
    "고소작업대작업",
    "밀폐공간작업",
    "유해화학물질취급작업",
    "굴착작업",
    "전기작업",
    "방사선작업",
    "중랑물취급작업",
    "기타",
];

fn normalize_for_matching(value: &str) -> String {
    value.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn is_board_post_registration_request(content: &str) -> bool {
    let normalized = normalize_for_matching(content);
    let asks_registration = normalized.contains("게시글등록")
        || normalized.contains("게시물등록")
        || normalized.contains("게시판등록")
        || normalized.contains("글등록")
        || (normalized.contains("게시글") && normalized.contains("등록"));
    let asks_lookup = ["현황", "조회", "목록", "리스트", "검색"]
        .iter()
        .any(|term| normalized.contains(term));

    asks_registration && !asks_lookup
}

fn is_board_post_edit_request(content: &str) -> bool {
    let normalized = normalize_for_matching(content);
    normalized.contains("게시글수정")
        || normalized.contains("게시물수정")
        || normalized.contains("게시판수정")
        || normalized.contains("글수정")
        || ((normalized.contains("게시글")
            || normalized.contains("게시물")
            || normalized.contains("게시판"))
            && normalized.contains("수정"))
}

fn is_board_post_delete_request(content: &str) -> bool {
    let normalized = normalize_for_matching(content);
    normalized.contains("게시글삭제")
        || normalized.contains("게시물삭제")
        || normalized.contains("게시판삭제")
        || normalized.contains("글삭제")
        || ((normalized.contains("게시글")
            || normalized.contains("게시물")
            || normalized.contains("게시판"))
            && normalized.contains("삭제"))
}

fn canonicalize_checklist_work_type(
    base_content: &str,
    selected_content: &str,
) -> Option<&'static str> {
    let normalized_base_content = normalize_for_matching(base_content);
    if !normalized_base_content.contains("작업전체크리스트")
        && !normalized_base_content.contains("체크리스트")
    {
        return None;
    }

    match normalize_for_matching(selected_content).as_str() {
        "고소작업대작업" => Some("고소작업"),
        "중랑물취급작업" | "중량물취급작업" => Some("중량물 취급작업"),
        _ => None,
    }
}

fn is_prework_plan_submission_request(content: &str) -> bool {
    let normalized = normalize_for_matching(content);
    normalized.contains("작업전계획서")
        && (normalized.contains("제출현황")
            || (normalized.contains("제출") && normalized.contains("현황")))
}

fn is_pending_permit_list_request(content: &str) -> bool {
    let normalized = normalize_for_matching(content);
    normalized.contains("오늘")
        && normalized.contains("위험작업허가")
        && (normalized.contains("확인대기") || normalized.contains("승인대기"))
        && (normalized.contains("목록") || normalized.contains("리스트"))
}

fn build_request_content(messages: &[ChatMessage], content: &str) -> String {
    let Some(last_assistant) = messages.last() else {
        return content.to_string();
    };

    if last_assistant.role != MessageRole::Assistant {
        return content.to_string();
    }

    let Some(suggestions) = &last_assistant.suggestions else {
        return content.to_string();
    };

    if !suggestions.iter().any(|suggestion| suggestion == content) {
        return content.to_string();
    }

    let Some(previous_user) = messages
        .iter()
        .rev()
        .skip(1)
        .find(|msg| msg.role == MessageRole::User)
    else {
        return content.to_string();
    };

    let base_content = previous_user
        .request_content
        .as_deref()
        .unwrap_or(&previous_user.content)
        .trim();
    let selected_content = content.trim();
    let normalized_selected_content = normalize_for_matching(selected_content);

    if is_prework_plan_submission_request(base_content)
        && PENDING_PERMIT_WORK_TYPES
            .iter()
            .any(|work_type| normalize_for_matching(work_type) == normalized_selected_content)
    {
        return format!(
            "{base_content}\n선택한 항목: {selected_content}\n조회 기준 작업 유형: {selected_content}\n위 선택을 반영해서 다시 답변해줘."
        );
    }

    if is_pending_permit_list_request(base_content)
        && PENDING_PERMIT_WORK_TYPES
            .iter()
            .any(|work_type| normalize_for_matching(work_type) == normalized_selected_content)
    {
        return format!(
            "{base_content}\n선택한 항목: {selected_content}\n조회 기준 작업 유형: {selected_content}\n위 선택을 반영해서 다시 답변해줘."
        );
    }

    if let Some(canonical_work_type) =
        canonicalize_checklist_work_type(base_content, selected_content)
    {
        return format!(
            "{base_content}\n선택한 항목: {selected_content}\n조회 기준 작업 유형: {canonical_work_type}\n위 선택을 반영해서 다시 답변해줘."
        );
    }

    format!("{base_content}\n선택한 항목: {selected_content}\n위 선택을 반영해서 다시 답변해줘.")
}

fn build_user_message(content: String, request_content: String) -> ChatMessage {
    ChatMessage {
        id: Uuid::new_v4().to_string(),
        role: MessageRole::User,
        content,
        request_content: Some(request_content),
        tools_used: vec![],
        token_usage: None,
        timestamp: chrono::Utc::now(),
        suggestions: None,
        old_new_comparison: None,
        response_versions: vec![],
        current_response_index: 0,
        ui: None,
        board_post_target: None,
        process_steps: Vec::new(),
    }
}

fn build_response_version(
    content: String,
    tools_used: Vec<String>,
    token_usage: Option<TokenUsage>,
    suggestions: Option<Vec<String>>,
    old_new_comparison: Option<OldNewComparison>,
) -> ResponseVersion {
    ResponseVersion {
        content,
        tools_used,
        token_usage,
        suggestions,
        old_new_comparison,
    }
}

fn build_assistant_message(version: ResponseVersion) -> ChatMessage {
    ChatMessage {
        id: Uuid::new_v4().to_string(),
        role: MessageRole::Assistant,
        content: version.content.clone(),
        request_content: None,
        tools_used: version.tools_used.clone(),
        token_usage: version.token_usage.clone(),
        timestamp: chrono::Utc::now(),
        suggestions: version.suggestions.clone(),
        old_new_comparison: version.old_new_comparison.clone(),
        response_versions: vec![version],
        current_response_index: 0,
        ui: None,
        board_post_target: None,
        process_steps: Vec::new(),
    }
}

fn build_board_post_menu_message() -> ChatMessage {
    ChatMessage {
        id: Uuid::new_v4().to_string(),
        role: MessageRole::Assistant,
        content: "어떤 게시판에 등록하기를 원하시나요?".to_string(),
        request_content: None,
        tools_used: vec![],
        token_usage: None,
        timestamp: chrono::Utc::now(),
        suggestions: None,
        old_new_comparison: None,
        response_versions: vec![],
        current_response_index: 0,
        ui: Some(ChatMessageUi::BoardPostMenu),
        board_post_target: None,
        process_steps: Vec::new(),
    }
}

fn build_board_post_edit_menu_message() -> ChatMessage {
    ChatMessage {
        id: Uuid::new_v4().to_string(),
        role: MessageRole::Assistant,
        content: "수정할 게시판을 선택해주세요.".to_string(),
        request_content: None,
        tools_used: vec![],
        token_usage: None,
        timestamp: chrono::Utc::now(),
        suggestions: None,
        old_new_comparison: None,
        response_versions: vec![],
        current_response_index: 0,
        ui: Some(ChatMessageUi::BoardPostEditMenu),
        board_post_target: None,
        process_steps: Vec::new(),
    }
}

fn build_board_post_delete_menu_message() -> ChatMessage {
    ChatMessage {
        id: Uuid::new_v4().to_string(),
        role: MessageRole::Assistant,
        content: "삭제할 게시판을 선택해주세요.".to_string(),
        request_content: None,
        tools_used: vec![],
        token_usage: None,
        timestamp: chrono::Utc::now(),
        suggestions: None,
        old_new_comparison: None,
        response_versions: vec![],
        current_response_index: 0,
        ui: Some(ChatMessageUi::BoardPostDeleteMenu),
        board_post_target: None,
        process_steps: Vec::new(),
    }
}

fn build_board_post_auth_required_message() -> ChatMessage {
    ChatMessage {
        id: Uuid::new_v4().to_string(),
        role: MessageRole::Assistant,
        content: "해당 요청은 로그인 상태에서만 가능합니다".to_string(),
        request_content: None,
        tools_used: vec![],
        token_usage: None,
        timestamp: chrono::Utc::now(),
        suggestions: None,
        old_new_comparison: None,
        response_versions: vec![],
        current_response_index: 0,
        ui: None,
        board_post_target: None,
        process_steps: Vec::new(),
    }
}

fn build_pending_permit_work_type_prompt_message() -> ChatMessage {
    ChatMessage {
        id: Uuid::new_v4().to_string(),
        role: MessageRole::Assistant,
        content: "어떤 작업구분의 위험작업 허가를 확인할까요? 아래에서 선택해 주세요.".to_string(),
        request_content: None,
        tools_used: vec![],
        token_usage: None,
        timestamp: chrono::Utc::now(),
        suggestions: Some(
            PENDING_PERMIT_WORK_TYPES
                .into_iter()
                .map(str::to_string)
                .collect(),
        ),
        old_new_comparison: None,
        response_versions: vec![],
        current_response_index: 0,
        ui: None,
        board_post_target: None,
        process_steps: Vec::new(),
    }
}

fn build_board_post_result_message(completion: BoardPostCompletion) -> ChatMessage {
    ChatMessage {
        id: Uuid::new_v4().to_string(),
        role: MessageRole::Assistant,
        content: completion.message,
        request_content: None,
        tools_used: vec![],
        token_usage: None,
        timestamp: chrono::Utc::now(),
        suggestions: None,
        old_new_comparison: None,
        response_versions: vec![],
        current_response_index: 0,
        ui: None,
        board_post_target: None,
        process_steps: Vec::new(),
    }
}

fn build_board_post_route_message(route_kind: &str, response: &str) -> Option<ChatMessage> {
    match response.trim() {
        "해당 요청은 로그인 상태에서만 가능합니다" => {
            return Some(build_board_post_auth_required_message());
        }
        "게시글 등록 요청입니다. 등록할 게시판을 선택해주세요." => {
            return Some(build_board_post_menu_message());
        }
        "게시글 수정 요청입니다. 수정할 게시판을 선택해주세요." => {
            return Some(build_board_post_edit_menu_message());
        }
        "게시글 삭제 요청입니다. 삭제할 게시판을 선택해주세요." => {
            return Some(build_board_post_delete_menu_message());
        }
        _ => {}
    }

    match route_kind {
        "board_post_create" => Some(build_board_post_menu_message()),
        "board_post_edit" => Some(build_board_post_edit_menu_message()),
        "board_post_delete" => Some(build_board_post_delete_menu_message()),
        _ => None,
    }
}

fn resolve_local_session_agents(
    messages: &[ChatMessage],
    selected_agent_id: Option<String>,
    routed_agent_ids: Vec<String>,
) -> (String, Vec<String>) {
    if messages
        .iter()
        .any(|msg| matches!(msg.ui, Some(ChatMessageUi::BoardPostForm)))
    {
        let agent_id = COMPLIANCE_AGENT_ID.to_string();
        return (agent_id.clone(), vec![agent_id]);
    }

    if messages.iter().any(|msg| {
        matches!(
            msg.ui,
            Some(
                ChatMessageUi::BoardPostMenu
                    | ChatMessageUi::BoardPostEditMenu
                    | ChatMessageUi::BoardPostDeleteMenu
            )
        )
    }) {
        let agent_id = BOARD_POST_ANALYZING_AGENT_ID.to_string();
        return (agent_id.clone(), vec![agent_id]);
    }

    let agent_id = selected_agent_id.unwrap_or_else(|| "board-post".to_string());
    (agent_id, routed_agent_ids)
}

fn find_regeneration_request(
    messages: &[ChatMessage],
    assistant_message_id: &str,
) -> Option<(String, String)> {
    let assistant_idx = messages
        .iter()
        .position(|msg| msg.id == assistant_message_id && msg.role == MessageRole::Assistant)?;

    let user_msg = messages[..assistant_idx]
        .iter()
        .rev()
        .find(|msg| msg.role == MessageRole::User)?;

    Some((
        user_msg.content.clone(),
        user_msg
            .request_content
            .clone()
            .unwrap_or_else(|| user_msg.content.clone()),
    ))
}

fn append_response_version_to_message(
    messages: &mut [ChatMessage],
    assistant_message_id: &str,
    version: ResponseVersion,
) -> bool {
    if let Some(message) = messages
        .iter_mut()
        .find(|msg| msg.id == assistant_message_id && msg.role == MessageRole::Assistant)
    {
        message.append_response_version(version);
        true
    } else {
        false
    }
}

fn set_message_response_page(
    messages: &mut [ChatMessage],
    assistant_message_id: &str,
    page_index: usize,
) -> bool {
    if let Some(message) = messages
        .iter_mut()
        .find(|msg| msg.id == assistant_message_id && msg.role == MessageRole::Assistant)
    {
        message.set_response_page(page_index);
        true
    } else {
        false
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_local_messages(
    messages: UseStateHandle<Vec<ChatMessage>>,
    session_id: UseStateHandle<Option<Uuid>>,
    sessions: UseStateHandle<Vec<ChatSession>>,
    selected_agent_id: UseStateHandle<Option<String>>,
    use_router: UseStateHandle<bool>,
    active_routed_agent_ids: UseStateHandle<Vec<String>>,
    updated_messages: Vec<ChatMessage>,
    last_message: String,
) {
    let sid = (*session_id).clone().unwrap_or_else(Uuid::new_v4);
    session_id.set(Some(sid.clone()));
    messages.set(updated_messages.clone());

    let mut current_sessions = (*sessions).clone();
    let (agent_id, routed_agent_ids) = resolve_local_session_agents(
        updated_messages.as_slice(),
        (*selected_agent_id).clone(),
        (*active_routed_agent_ids).clone(),
    );
    let use_router_value = *use_router;

    if let Some(idx) = current_sessions
        .iter()
        .position(|session| session.id == sid)
    {
        current_sessions[idx].last_message = last_message;
        current_sessions[idx].messages = updated_messages;
        current_sessions[idx].timestamp = chrono::Utc::now();
        current_sessions[idx].agent_id = agent_id;
        current_sessions[idx].use_router = use_router_value;
        current_sessions[idx].routed_agent_ids = routed_agent_ids;
    } else {
        current_sessions.push(ChatSession {
            id: sid,
            agent_id,
            title: build_initial_session_title(&last_message),
            last_message,
            timestamp: chrono::Utc::now(),
            messages: updated_messages,
            is_pinned: false,
            use_router: use_router_value,
            routed_agent_ids,
        });
    }

    sessions.set(current_sessions);
}

/// Apply a streamed `ProcessEvent` to the running list of processing steps,
/// upserting each step by its label so repeated events update in place.
fn apply_process_event(steps: &mut Vec<ProcessStep>, event: &ProcessEvent) {
    match event {
        ProcessEvent::Step {
            label,
            detail,
            status,
        } => {
            if let Some(step) = steps.iter_mut().find(|step| &step.label == label) {
                step.status = status.clone();
                if detail.is_some() {
                    step.detail = detail.clone();
                }
            } else {
                steps.push(ProcessStep {
                    label: label.clone(),
                    detail: detail.clone(),
                    status: status.clone(),
                });
            }
        }
        ProcessEvent::Final(_) | ProcessEvent::Error { .. } => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn submit_query(
    messages: UseStateHandle<Vec<ChatMessage>>,
    session_id: UseStateHandle<Option<Uuid>>,
    sessions: UseStateHandle<Vec<ChatSession>>,
    is_loading: UseStateHandle<bool>,
    live_steps: UseStateHandle<Vec<ProcessStep>>,
    selected_agent_id: UseStateHandle<Option<String>>,
    use_router: UseStateHandle<bool>,
    active_routed_agent_ids: UseStateHandle<Vec<String>>,
    content: String,
    request_content: String,
    current_msgs: Vec<ChatMessage>,
    regenerate_target_id: Option<String>,
) {
    let route_via_router = *use_router || (*selected_agent_id).is_none();
    let is_regeneration = regenerate_target_id.is_some();

    messages.set(current_msgs.clone());
    is_loading.set(true);
    live_steps.set(Vec::new());

    let messages_clone = messages.clone();
    let session_id_clone = session_id.clone();
    let sessions_clone = sessions.clone();
    let is_loading_clone = is_loading.clone();
    let live_steps_clone = live_steps.clone();
    let live_steps_event = live_steps.clone();
    let selected_agent_id_clone = selected_agent_id.clone();
    let use_router_clone = use_router.clone();
    let active_routed_agent_ids_clone = active_routed_agent_ids.clone();
    let sid = (*session_id).clone();
    let active_agent_id = (*selected_agent_id).clone();

    spawn_local(async move {
        // `final_steps` captures the processing steps from the streaming router
        // path so they can be persisted onto the completed assistant message.
        let mut final_steps: Vec<ProcessStep> = Vec::new();

        // Route via the smart router (no specific agent) or directly to the
        // selected agent. Both paths stream live processing steps.
        let agent_id_opt: Option<String> = if route_via_router {
            None
        } else {
            active_agent_id.clone()
        };

        let result = if route_via_router || agent_id_opt.is_some() {
            let steps_acc =
                std::rc::Rc::new(std::cell::RefCell::new(Vec::<ProcessStep>::new()));
            let on_event = {
                let steps_acc = steps_acc.clone();
                let live_steps_event = live_steps_event.clone();
                move |event: ProcessEvent| {
                    {
                        let mut acc = steps_acc.borrow_mut();
                        apply_process_event(&mut acc, &event);
                    }
                    live_steps_event.set(steps_acc.borrow().clone());
                }
            };
            let res = api::send_query_stream(
                agent_id_opt.as_deref(),
                &request_content,
                sid,
                on_event,
            )
            .await;
            final_steps = steps_acc.borrow().clone();
            // Fallback: if streaming failed before producing any step (e.g. the
            // stream endpoint is unavailable), retry with the buffered endpoint
            // so answers still work (without the live steps).
            if res.is_err() && final_steps.is_empty() {
                api::send_query(agent_id_opt.as_deref(), &request_content, sid).await
            } else {
                res
            }
        } else {
            Err("No agent selected".to_string())
        };

        match result {
            Ok(resp) => {
                let routed_agent_ids = if resp.routed_agent_ids.is_empty() {
                    vec![resp.agent_id.clone()]
                } else {
                    resp.routed_agent_ids.clone()
                };
                let mut msgs = current_msgs.clone();

                if let Some(board_post_message) =
                    build_board_post_route_message(&resp.route_kind, &resp.response)
                {
                    msgs.push(board_post_message);
                } else {
                    let assistant_suggestions = extract_suggestions(&content, &resp.response);
                    let response_version = build_response_version(
                        resp.response.clone(),
                        resp.tools_used.clone(),
                        Some(resp.token_usage.clone()),
                        assistant_suggestions,
                        resp.old_new_comparison.clone(),
                    );

                    if let Some(target_id) = regenerate_target_id.as_deref() {
                        if !append_response_version_to_message(
                            &mut msgs,
                            target_id,
                            response_version.clone(),
                        ) {
                            let mut assistant_msg = build_assistant_message(response_version);
                            assistant_msg.process_steps = final_steps.clone();
                            msgs.push(assistant_msg);
                        }
                    } else {
                        let mut assistant_msg = build_assistant_message(response_version);
                        assistant_msg.process_steps = final_steps.clone();
                        msgs.push(assistant_msg);
                    }
                }

                messages_clone.set(msgs.clone());

                selected_agent_id_clone.set(Some(resp.agent_id.clone()));
                use_router_clone.set(route_via_router);
                active_routed_agent_ids_clone.set(routed_agent_ids.clone());

                let mut current_sessions = (*sessions_clone).clone();
                let new_sid = resp.session_id;
                session_id_clone.set(Some(new_sid));

                if let Some(idx) = current_sessions.iter().position(|s| s.id == new_sid) {
                    current_sessions[idx].agent_id = resp.agent_id.clone();
                    if !is_regeneration {
                        current_sessions[idx].last_message = content.clone();
                    }
                    current_sessions[idx].messages = msgs;
                    current_sessions[idx].timestamp = chrono::Utc::now();
                    current_sessions[idx].use_router = route_via_router;
                    current_sessions[idx].routed_agent_ids = routed_agent_ids.clone();
                } else {
                    current_sessions.push(ChatSession {
                        id: new_sid,
                        agent_id: resp.agent_id.clone(),
                        title: build_initial_session_title(&content),
                        last_message: content.clone(),
                        timestamp: chrono::Utc::now(),
                        messages: msgs,
                        is_pinned: false,
                        use_router: route_via_router,
                        routed_agent_ids: routed_agent_ids.clone(),
                    });

                    let sessions_inner = sessions_clone.clone();
                    spawn_local(async move {
                        if let Ok(summary_title) = api::fetch_session_summary(new_sid).await {
                            let mut s_list = (*sessions_inner).clone();
                            if let Some(id) = s_list.iter().position(|s| s.id == new_sid) {
                                s_list[id].title = summary_title;
                                sessions_inner.set(s_list);
                            }
                        }
                    });
                }
                sessions_clone.set(current_sessions);
            }
            Err(e) => {
                use_router_clone.set(route_via_router);
                let err_msg = ChatMessage {
                    id: Uuid::new_v4().to_string(),
                    role: MessageRole::System,
                    content: format!("Error: {e}"),
                    request_content: None,
                    tools_used: vec![],
                    token_usage: None,
                    timestamp: chrono::Utc::now(),
                    suggestions: None,
                    old_new_comparison: None,
                    response_versions: vec![],
                    current_response_index: 0,
                    ui: None,
                    board_post_target: None,
                    process_steps: Vec::new(),
                };
                let mut msgs = current_msgs.clone();
                msgs.push(err_msg);
                messages_clone.set(msgs);
            }
        }
        is_loading_clone.set(false);
        live_steps_clone.set(Vec::new());
    });
}

fn build_initial_session_title(content: &str) -> String {
    let normalized = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let title = if normalized.is_empty() {
        "New chat".to_string()
    } else {
        normalized
    };

    if title.chars().count() > 25 {
        title.chars().take(22).collect::<String>() + "..."
    } else {
        title
    }
}

fn fallback_agent_ids(
    routed_agent_ids: &[String],
    primary_agent_id: Option<&String>,
) -> Vec<String> {
    if !routed_agent_ids.is_empty() {
        routed_agent_ids.to_vec()
    } else {
        primary_agent_id.into_iter().cloned().collect()
    }
}

fn resolve_agent_metas(agents: &[AgentMeta], agent_ids: &[String]) -> Vec<AgentMeta> {
    agent_ids
        .iter()
        .filter_map(|agent_id| agents.iter().find(|agent| agent.id == *agent_id).cloned())
        .collect()
}

fn summarize_agent_names(
    agents: &[AgentMeta],
    routed_agent_ids: &[String],
    primary_agent_id: &str,
) -> String {
    let ids = if routed_agent_ids.is_empty() {
        vec![primary_agent_id.to_string()]
    } else {
        routed_agent_ids.to_vec()
    };

    let names: Vec<String> = ids
        .iter()
        .filter_map(|agent_id| {
            if agent_id.as_str() == BOARD_POST_ANALYZING_AGENT_ID {
                Some("분석 중".to_string())
            } else if agent_id.as_str() == OTHER_AGENT_ID {
                Some("기타".to_string())
            } else {
                agents
                    .iter()
                    .find(|agent| agent.id == *agent_id)
                    .map(|agent| agent.name.clone())
            }
        })
        .collect();

    if names.is_empty() {
        primary_agent_id.to_string()
    } else {
        names.join(", ")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeleteTarget {
    Session(Uuid),
    All,
}

#[function_component(App)]
fn app() -> Html {
    // State
    let agents = use_state(|| AgentMeta::defaults());
    let selected_agent_id = use_state(|| Option::<String>::None);
    let messages = use_state(Vec::<ChatMessage>::new);
    let session_id = use_state(|| Option::<Uuid>::None);
    let sessions =
        use_state(|| LocalStorage::get::<Vec<ChatSession>>(STORAGE_KEY).unwrap_or_default());
    let is_loading = use_state(|| false);
    let live_steps = use_state(Vec::<ProcessStep>::new);
    let health_status = use_state(|| Option::<String>::None);
    let use_router = use_state(|| false);
    let active_routed_agent_ids = use_state(Vec::<String>::new);
    // 로그인 신원은 HttpOnly 쿠키가 보유하며, 표시용 상태만 메모리에 둔다(새로고침 시 초기화).
    let auth_user = use_state(|| Option::<AuthUser>::None);
    let is_login_modal_open = use_state(|| false);
    let login_id = use_state(String::new);
    let login_password = use_state(String::new);
    let login_error = use_state(|| Option::<String>::None);
    let is_logging_in = use_state(|| false);
    let show_password = use_state(|| false);
    let is_dark_mode = use_state(|| LocalStorage::get::<bool>(THEME_STORAGE_KEY).unwrap_or(false));
    let is_mobile_sidebar_open = use_state(|| false);

    // Persist theme
    {
        let is_dark_mode_state = is_dark_mode.clone();
        use_effect_with((*is_dark_mode_state).clone(), move |dark| {
            let _ = LocalStorage::set(THEME_STORAGE_KEY, dark);
            || ()
        });
    }

    // Persist sessions when history changes
    {
        let sessions_state = sessions.clone();
        use_effect_with((*sessions_state).clone(), move |s| {
            let _ = LocalStorage::set(STORAGE_KEY, s);
            || ()
        });
    }

    // Fetch health status on mount
    {
        let health_status = health_status.clone();
        use_effect_with((), move |_| {
            spawn_local(async move {
                match api::fetch_health().await {
                    Ok(h) => health_status.set(Some(h.status)),
                    Err(_) => health_status.set(Some("error".into())),
                }
            });
            || ()
        });
    }

    // 새로고침 후 세션 쿠키로 로그인 상태를 복원한다. auth_user는 메모리에만 있으므로,
    // 마운트 시 /auth/me를 호출해 로그인 사용자를 되살린다. 비로그인이면 아무 것도 하지 않는다.
    {
        let auth_user = auth_user.clone();
        use_effect_with((), move |_| {
            spawn_local(async move {
                if let Ok(Some(user)) = api::fetch_me().await {
                    auth_user.set(Some(user));
                }
            });
            || ()
        });
    }

    // Callbacks
    let on_login_click = {
        let is_login_modal_open = is_login_modal_open.clone();
        let login_error = login_error.clone();
        Callback::from(move |_: ()| {
            login_error.set(None);
            is_login_modal_open.set(true);
        })
    };

    let on_logout = {
        let auth_user = auth_user.clone();
        Callback::from(move |_: ()| {
            spawn_local(async move {
                api::logout().await;
            });
            auth_user.set(None);
        })
    };

    let on_theme_toggle = {
        let is_dark_mode = is_dark_mode.clone();
        Callback::from(move |_: ()| {
            is_dark_mode.set(!*is_dark_mode);
        })
    };

    let on_login_submit = {
        let auth_user = auth_user.clone();
        let is_login_modal_open = is_login_modal_open.clone();
        let login_id = login_id.clone();
        let login_password = login_password.clone();
        let login_error = login_error.clone();
        let is_logging_in = is_logging_in.clone();

        Callback::from(move |_: MouseEvent| {
            let id = (*login_id).clone();
            let password = (*login_password).clone();

            if id.is_empty() || password.is_empty() {
                login_error.set(Some("아이디와 비밀번호를 입력해주세요.".into()));
                return;
            }

            let auth_user = auth_user.clone();
            let is_login_modal_open = is_login_modal_open.clone();
            let login_error = login_error.clone();
            let is_logging_in = is_logging_in.clone();
            let login_password = login_password.clone();

            is_logging_in.set(true);
            spawn_local(async move {
                // 단일 회사 배포 기준 회사코드 "10" 고정. 다회사면 모달에 입력 필드 추가.
                match api::login(id, password, "10".to_string()).await {
                    Ok(user) => {
                        auth_user.set(Some(user));
                        login_error.set(None);
                        is_login_modal_open.set(false);
                        login_password.set(String::new());
                    }
                    Err(e) => {
                        login_error.set(Some(e));
                    }
                }
                is_logging_in.set(false);
            });
        })
    };

    // 모달을 닫고 비로그인(전체 조회) 상태로 계속 이용.
    let on_continue_without_login = {
        let is_login_modal_open = is_login_modal_open.clone();
        let login_error = login_error.clone();
        let login_password = login_password.clone();
        Callback::from(move |_: MouseEvent| {
            login_error.set(None);
            login_password.set(String::new());
            is_login_modal_open.set(false);
        })
    };

    let on_login_keydown = {
        let on_login_submit = on_login_submit.clone();
        Callback::from(move |e: KeyboardEvent| {
            if e.key() == "Enter" {
                on_login_submit.emit(MouseEvent::new("click").unwrap());
            }
        })
    };

    let on_select_agent = {
        let selected_agent_id = selected_agent_id.clone();
        let messages = messages.clone();
        let session_id = session_id.clone();
        let use_router = use_router.clone();
        let active_routed_agent_ids = active_routed_agent_ids.clone();
        Callback::from(move |id: String| {
            selected_agent_id.set(Some(id.clone()));
            messages.set(vec![]);
            session_id.set(None);
            use_router.set(false);
            active_routed_agent_ids.set(vec![id]);
        })
    };

    let on_back_to_agents = {
        let selected_agent_id = selected_agent_id.clone();
        let messages = messages.clone();
        let session_id = session_id.clone();
        let use_router = use_router.clone();
        let active_routed_agent_ids = active_routed_agent_ids.clone();
        Callback::from(move |_: ()| {
            selected_agent_id.set(None);
            messages.set(vec![]);
            session_id.set(None);
            use_router.set(false);
            active_routed_agent_ids.set(vec![]);
        })
    };

    let on_new_chat = {
        let selected_agent_id = selected_agent_id.clone();
        let messages = messages.clone();
        let session_id = session_id.clone();
        let use_router = use_router.clone();
        let active_routed_agent_ids = active_routed_agent_ids.clone();
        Callback::from(move |_: ()| {
            selected_agent_id.set(None);
            messages.set(vec![]);
            session_id.set(None);
            use_router.set(false);
            active_routed_agent_ids.set(vec![]);
        })
    };

    let on_select_session = {
        let selected_agent_id = selected_agent_id.clone();
        let messages = messages.clone();
        let session_id = session_id.clone();
        let sessions = sessions.clone();
        let use_router = use_router.clone();
        let active_routed_agent_ids = active_routed_agent_ids.clone();
        Callback::from(move |sid: Uuid| {
            if let Some(session) = sessions.iter().find(|s| s.id == sid) {
                selected_agent_id.set(Some(session.agent_id.clone()));
                messages.set(session.messages.clone());
                session_id.set(Some(session.id));
                use_router.set(session.use_router);
                active_routed_agent_ids.set(fallback_agent_ids(
                    &session.routed_agent_ids,
                    Some(&session.agent_id),
                ));
            }
        })
    };

    let on_mobile_sidebar_open = {
        let is_mobile_sidebar_open = is_mobile_sidebar_open.clone();
        Callback::from(move |_: MouseEvent| {
            is_mobile_sidebar_open.set(true);
        })
    };

    let on_mobile_sidebar_close = {
        let is_mobile_sidebar_open = is_mobile_sidebar_open.clone();
        Callback::from(move |_: MouseEvent| {
            is_mobile_sidebar_open.set(false);
        })
    };

    let on_sidebar_new_chat = {
        let on_new_chat = on_new_chat.clone();
        let is_mobile_sidebar_open = is_mobile_sidebar_open.clone();
        Callback::from(move |_: ()| {
            on_new_chat.emit(());
            is_mobile_sidebar_open.set(false);
        })
    };

    let on_sidebar_select_session = {
        let on_select_session = on_select_session.clone();
        let is_mobile_sidebar_open = is_mobile_sidebar_open.clone();
        Callback::from(move |sid: Uuid| {
            on_select_session.emit(sid);
            is_mobile_sidebar_open.set(false);
        })
    };

    let delete_target = use_state(|| None::<DeleteTarget>);

    let on_delete_session = {
        let delete_target = delete_target.clone();
        Callback::from(move |sid: Uuid| {
            delete_target.set(Some(DeleteTarget::Session(sid)));
        })
    };

    let on_delete_all_sessions = {
        let delete_target = delete_target.clone();
        Callback::from(move |_: ()| {
            delete_target.set(Some(DeleteTarget::All));
        })
    };

    let on_confirm_delete = {
        let sessions = sessions.clone();
        let selected_session_id = session_id.clone();
        let messages = messages.clone();
        let selected_agent_id = selected_agent_id.clone();
        let delete_target = delete_target.clone();
        let use_router = use_router.clone();
        let active_routed_agent_ids = active_routed_agent_ids.clone();
        Callback::from(move |_: MouseEvent| {
            match *delete_target {
                Some(DeleteTarget::Session(sid)) => {
                    let mut current_sessions = (*sessions).clone();
                    current_sessions.retain(|s| s.id != sid);
                    sessions.set(current_sessions);

                    if selected_session_id.as_ref() == Some(&sid) {
                        selected_session_id.set(None);
                        messages.set(vec![]);
                        selected_agent_id.set(None);
                        use_router.set(false);
                        active_routed_agent_ids.set(vec![]);
                    }
                }
                Some(DeleteTarget::All) => {
                    sessions.set(vec![]);
                    selected_session_id.set(None);
                    messages.set(vec![]);
                    selected_agent_id.set(None);
                    use_router.set(false);
                    active_routed_agent_ids.set(vec![]);
                }
                None => {}
            }
            delete_target.set(None);
        })
    };

    let on_cancel_delete = {
        let delete_target = delete_target.clone();
        Callback::from(move |_: MouseEvent| {
            delete_target.set(None);
        })
    };

    let is_search_open = use_state(|| false);
    let search_query = use_state(String::new);
    let rename_target_id = use_state(|| None::<Uuid>);
    let rename_input_value = use_state(String::new);

    let on_search_open = {
        let is_search_open = is_search_open.clone();
        let search_query = search_query.clone();
        Callback::from(move |_: ()| {
            search_query.set(String::new());
            is_search_open.set(true);
        })
    };

    let on_sidebar_search_open = {
        let on_search_open = on_search_open.clone();
        let is_mobile_sidebar_open = is_mobile_sidebar_open.clone();
        Callback::from(move |_: ()| {
            on_search_open.emit(());
            is_mobile_sidebar_open.set(false);
        })
    };

    let on_search_close = {
        let is_search_open = is_search_open.clone();
        Callback::from(move |_: MouseEvent| {
            is_search_open.set(false);
        })
    };

    let on_pin_session = {
        let sessions = sessions.clone();
        Callback::from(move |sid: Uuid| {
            let mut current_sessions = (*sessions).clone();
            if let Some(idx) = current_sessions.iter().position(|s| s.id == sid) {
                current_sessions[idx].is_pinned = !current_sessions[idx].is_pinned;
                sessions.set(current_sessions);
            }
        })
    };

    let on_rename_click = {
        let rename_target_id = rename_target_id.clone();
        let rename_input_value = rename_input_value.clone();
        let sessions = sessions.clone();
        Callback::from(move |sid: Uuid| {
            if let Some(session) = sessions.iter().find(|s| s.id == sid) {
                rename_input_value.set(session.title.clone());
                rename_target_id.set(Some(sid));
            }
        })
    };

    let on_confirm_rename = {
        let sessions = sessions.clone();
        let rename_target_id = rename_target_id.clone();
        let rename_input_value = rename_input_value.clone();
        Callback::from(move |_: MouseEvent| {
            if let Some(sid) = *rename_target_id {
                let mut current_sessions = (*sessions).clone();
                if let Some(idx) = current_sessions.iter().position(|s| s.id == sid) {
                    current_sessions[idx].title = (*rename_input_value).clone();
                    sessions.set(current_sessions);
                }
            }
            rename_target_id.set(None);
        })
    };

    let on_cancel_rename = {
        let rename_target_id = rename_target_id.clone();
        Callback::from(move |_: MouseEvent| {
            rename_target_id.set(None);
        })
    };

    let on_send = {
        let messages = messages.clone();
        let session_id = session_id.clone();
        let sessions = sessions.clone();
        let is_loading = is_loading.clone();
        let live_steps = live_steps.clone();
        let selected_agent_id = selected_agent_id.clone();
        let use_router = use_router.clone();
        let active_routed_agent_ids = active_routed_agent_ids.clone();
        let auth_user = auth_user.clone();
        Callback::from(move |content: String| {
            let request_content = build_request_content((*messages).as_slice(), &content);
            let mut new_msgs = (*messages).clone();
            new_msgs.push(build_user_message(content.clone(), request_content.clone()));

            if is_board_post_delete_request(&content) {
                if auth_user.is_none() {
                    new_msgs.push(build_board_post_auth_required_message());
                    apply_local_messages(
                        messages.clone(),
                        session_id.clone(),
                        sessions.clone(),
                        selected_agent_id.clone(),
                        use_router.clone(),
                        active_routed_agent_ids.clone(),
                        new_msgs,
                        content,
                    );
                    return;
                }

                new_msgs.push(build_board_post_delete_menu_message());
                apply_local_messages(
                    messages.clone(),
                    session_id.clone(),
                    sessions.clone(),
                    selected_agent_id.clone(),
                    use_router.clone(),
                    active_routed_agent_ids.clone(),
                    new_msgs,
                    content,
                );
                return;
            }

            if is_board_post_edit_request(&content) {
                if auth_user.is_none() {
                    new_msgs.push(build_board_post_auth_required_message());
                    apply_local_messages(
                        messages.clone(),
                        session_id.clone(),
                        sessions.clone(),
                        selected_agent_id.clone(),
                        use_router.clone(),
                        active_routed_agent_ids.clone(),
                        new_msgs,
                        content,
                    );
                    return;
                }

                new_msgs.push(build_board_post_edit_menu_message());
                apply_local_messages(
                    messages.clone(),
                    session_id.clone(),
                    sessions.clone(),
                    selected_agent_id.clone(),
                    use_router.clone(),
                    active_routed_agent_ids.clone(),
                    new_msgs,
                    content,
                );
                return;
            }

            if is_pending_permit_list_request(&content) {
                new_msgs.push(build_pending_permit_work_type_prompt_message());
                apply_local_messages(
                    messages.clone(),
                    session_id.clone(),
                    sessions.clone(),
                    selected_agent_id.clone(),
                    use_router.clone(),
                    active_routed_agent_ids.clone(),
                    new_msgs,
                    content,
                );
                return;
            }

            if is_board_post_registration_request(&content) {
                if auth_user.is_none() {
                    new_msgs.push(build_board_post_auth_required_message());
                    apply_local_messages(
                        messages.clone(),
                        session_id.clone(),
                        sessions.clone(),
                        selected_agent_id.clone(),
                        use_router.clone(),
                        active_routed_agent_ids.clone(),
                        new_msgs,
                        content,
                    );
                    return;
                }

                new_msgs.push(build_board_post_menu_message());
                apply_local_messages(
                    messages.clone(),
                    session_id.clone(),
                    sessions.clone(),
                    selected_agent_id.clone(),
                    use_router.clone(),
                    active_routed_agent_ids.clone(),
                    new_msgs,
                    content,
                );
                return;
            }

            submit_query(
                messages.clone(),
                session_id.clone(),
                sessions.clone(),
                is_loading.clone(),
                live_steps.clone(),
                selected_agent_id.clone(),
                use_router.clone(),
                active_routed_agent_ids.clone(),
                content,
                request_content,
                new_msgs,
                None,
            );
        })
    };

    let on_regenerate = {
        let messages = messages.clone();
        let session_id = session_id.clone();
        let sessions = sessions.clone();
        let is_loading = is_loading.clone();
        let live_steps = live_steps.clone();
        let selected_agent_id = selected_agent_id.clone();
        let use_router = use_router.clone();
        let active_routed_agent_ids = active_routed_agent_ids.clone();
        Callback::from(move |assistant_message_id: String| {
            if *is_loading {
                return;
            }

            let Some((content, request_content)) =
                find_regeneration_request((*messages).as_slice(), &assistant_message_id)
            else {
                return;
            };

            submit_query(
                messages.clone(),
                session_id.clone(),
                sessions.clone(),
                is_loading.clone(),
                live_steps.clone(),
                selected_agent_id.clone(),
                use_router.clone(),
                active_routed_agent_ids.clone(),
                content,
                request_content,
                (*messages).clone(),
                Some(assistant_message_id),
            );
        })
    };

    let on_select_response_page = {
        let messages = messages.clone();
        let sessions = sessions.clone();
        let session_id = session_id.clone();
        Callback::from(move |(assistant_message_id, page_index): (String, usize)| {
            let mut updated_messages = (*messages).clone();
            if !set_message_response_page(&mut updated_messages, &assistant_message_id, page_index)
            {
                return;
            }

            messages.set(updated_messages.clone());

            if let Some(current_session_id) = *session_id {
                let mut current_sessions = (*sessions).clone();
                if let Some(idx) = current_sessions
                    .iter()
                    .position(|s| s.id == current_session_id)
                {
                    current_sessions[idx].messages = updated_messages;
                    sessions.set(current_sessions);
                }
            }
        })
    };

    let on_board_post_target_selected = { Callback::noop() };

    let on_board_post_complete = {
        let messages = messages.clone();
        let session_id = session_id.clone();
        let sessions = sessions.clone();
        let selected_agent_id = selected_agent_id.clone();
        let use_router = use_router.clone();
        let active_routed_agent_ids = active_routed_agent_ids.clone();
        Callback::from(move |completion: BoardPostCompletion| {
            let mut updated_messages = (*messages).clone();
            let last_message = completion.message.clone();
            if matches!(
                completion.message.as_str(),
                "사용자가 게시글 등록을 취소하였습니다"
                    | "사용자가 게시글 수정을 취소하였습니다"
                    | "수정되었습니다."
                    | "삭제되었습니다."
            ) {
                if let Some(message) = updated_messages.iter_mut().rev().find(|message| {
                    matches!(
                        message.ui,
                        Some(
                            ChatMessageUi::BoardPostMenu
                                | ChatMessageUi::BoardPostEditMenu
                                | ChatMessageUi::BoardPostDeleteMenu
                                | ChatMessageUi::BoardPostForm
                        )
                    )
                }) {
                    message.content = last_message.clone();
                    message.ui = None;
                    message.board_post_target = None;
                } else {
                    updated_messages.push(build_board_post_result_message(completion));
                }
            } else {
                updated_messages.push(build_board_post_result_message(completion));
            }
            apply_local_messages(
                messages.clone(),
                session_id.clone(),
                sessions.clone(),
                selected_agent_id.clone(),
                use_router.clone(),
                active_routed_agent_ids.clone(),
                updated_messages,
                last_message,
            );
        })
    };

    let on_example_click = on_send.clone();

    // Resolve selected agent meta
    let selected_meta = (*selected_agent_id)
        .as_ref()
        .and_then(|id| agents.iter().find(|a| a.id == *id).cloned());
    let header_agent_ids = fallback_agent_ids(
        (*active_routed_agent_ids).as_slice(),
        (*selected_agent_id).as_ref(),
    );
    let header_agents = resolve_agent_metas((*agents).as_slice(), &header_agent_ids);
    let header_back_to_agents = ((*messages).is_empty() && !*is_loading && selected_meta.is_some())
        .then_some(on_back_to_agents.clone());

    let app_class = if *is_dark_mode { "dark" } else { "" };
    let mobile_sidebar_class = (*is_mobile_sidebar_open).then_some("mobile-sidebar-open");

    html! {
        <div class={classes!("app-container", app_class, mobile_sidebar_class)}>
                <Sidebar
                    sessions={(*sessions).clone()}
                    agents={(*agents).clone()}
                    selected_session_id={(*session_id).clone()}
                    on_select_session={on_sidebar_select_session}
                    on_delete_session={on_delete_session}
                    on_delete_all_sessions={on_delete_all_sessions}
                    on_new_chat={on_sidebar_new_chat}
                    on_search_open={on_sidebar_search_open}
                    on_pin_session={on_pin_session}
                    on_rename_click={on_rename_click}
                    health_status={(*health_status).clone()}
                />
            { if *is_mobile_sidebar_open {
                html! {
                    <button
                        type="button"
                        class="mobile-sidebar-backdrop"
                        aria-label="대화 기록 닫기"
                        onclick={on_mobile_sidebar_close.clone()}
                    ></button>
                }
            } else { html! {} } }
            <div class="main">
                <button
                    type="button"
                    class="mobile-sidebar-toggle"
                    aria-label="대화 기록 열기"
                    title="대화 기록"
                    onclick={on_mobile_sidebar_open}
                >
                    <span></span>
                    <span></span>
                    <span></span>
                </button>
                if let Some(meta) = selected_meta {
                    <ChatHeader
                        is_dark_mode={*is_dark_mode}
                        on_theme_toggle={on_theme_toggle.clone()}
                        agents={header_agents.clone()}
                        auth_user={(*auth_user).clone()}
                        on_login_click={on_login_click.clone()}
                        on_logout={on_logout.clone()}
                        on_back_to_agents={header_back_to_agents.clone()}
                    />
                    <ChatArea
                        messages={(*messages).clone()}
                        is_loading={*is_loading}
                        live_steps={(*live_steps).clone()}
                        on_send={on_send.clone()}
                        on_regenerate={on_regenerate.clone()}
                        on_select_response_page={on_select_response_page.clone()}
                        on_board_post_target_selected={on_board_post_target_selected.clone()}
                        on_board_post_complete={on_board_post_complete.clone()}
                        on_example_click={on_example_click.clone()}
                        on_back_to_agents={on_back_to_agents.clone()}
                        has_agent=true
                        agent_meta={Some(meta)}
                        is_authenticated={auth_user.is_some()}
                        on_login_click={on_login_click.clone()}
                        token={None::<String>}
                    />
                } else if *is_loading || !messages.is_empty() {
                    <ChatHeader
                        is_dark_mode={*is_dark_mode}
                        on_theme_toggle={on_theme_toggle.clone()}
                        agents={header_agents.clone()}
                        auth_user={(*auth_user).clone()}
                        on_login_click={on_login_click.clone()}
                        on_logout={on_logout.clone()}
                    />
                    <ChatArea
                        messages={(*messages).clone()}
                        is_loading={*is_loading}
                        live_steps={(*live_steps).clone()}
                        on_send={on_send.clone()}
                        on_regenerate={on_regenerate.clone()}
                        on_select_response_page={on_select_response_page.clone()}
                        on_board_post_target_selected={on_board_post_target_selected.clone()}
                        on_board_post_complete={on_board_post_complete.clone()}
                        on_example_click={on_example_click.clone()}
                        on_back_to_agents={on_back_to_agents.clone()}
                        has_agent=true
                        agent_meta={None}
                        is_authenticated={auth_user.is_some()}
                        on_login_click={on_login_click.clone()}
                        token={None::<String>}
                    />
                } else {
                    <ChatHeader
                        is_dark_mode={*is_dark_mode}
                        on_theme_toggle={on_theme_toggle.clone()}
                        agents={header_agents.clone()}
                        auth_user={(*auth_user).clone()}
                        on_login_click={on_login_click.clone()}
                        on_logout={on_logout.clone()}
                    />
                    <div class="home-main">
                        <AgentSelector
                            agents={(*agents).clone()}
                            on_select={on_select_agent}
                        />
                    </div>
                    <ChatInput
                        on_send={on_send.clone()}
                        disabled={*is_loading}
                    />
                }
            </div>
            { if *is_search_open {
                let filtered_sessions: Vec<_> = sessions.iter()
                    .filter(|s| s.title.to_lowercase().contains(&(*search_query).to_lowercase()))
                    .collect();

                let on_input = {
                    let search_query = search_query.clone();
                    Callback::from(move |e: InputEvent| {
                        if let Some(target) = e.target() {
                            use wasm_bindgen::JsCast;
                            if let Ok(input) = target.dyn_into::<web_sys::HtmlInputElement>() {
                                search_query.set(input.value());
                            }
                        }
                    })
                };

                html! {
                    <div class="modal-backdrop" onclick={on_search_close}>
                        <div class="search-modal" onclick={Callback::from(|e: MouseEvent| e.stop_propagation())}>
                            <div class="search-header">
                                <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="#94a3b8" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"></circle><line x1="21" y1="21" x2="16.65" y2="16.65"></line></svg>
                                <input
                                    class="search-input-box"
                                    type="text"
                                    placeholder="대화 제목으로 검색..."
                                    value={(*search_query).clone()}
                                    oninput={on_input}
                                    autofocus=true
                                />
                            </div>
                            <div class="search-results">
                                { if filtered_sessions.is_empty() {
                                    html! { <div class="no-results">{ "검색 결과가 없습니다." }</div> }
                                } else {
                                    html! {
                                        for filtered_sessions.into_iter().map(|s| {
                                            let sid = s.id;
                                            let title = s.title.clone();
                                            let agent_name = summarize_agent_names((*agents).as_slice(), &s.routed_agent_ids, &s.agent_id);
                                            let time_str = s.timestamp.with_timezone(&chrono::Local).format("%m/%d %H:%M").to_string();

                                            let on_click = {
                                                let on_select = on_select_session.clone();
                                                let is_search_open = is_search_open.clone();
                                                Callback::from(move |_: MouseEvent| {
                                                    on_select.emit(sid);
                                                    is_search_open.set(false);
                                                })
                                            };

                                            html! {
                                                <div class="search-result-item" onclick={on_click}>
                                                    <div class="result-title">{ title }</div>
                                                    <div class="result-meta">
                                                        <span>{ agent_name }</span>
                                                        <span>{ "·" }</span>
                                                        <span>{ time_str }</span>
                                                    </div>
                                                </div>
                                            }
                                        })
                                    }
                                }}
                            </div>
                        </div>
                    </div>
                }
            } else { html! {} } }

            { if rename_target_id.is_some() {
                let on_input = {
                    let rename_input_value = rename_input_value.clone();
                    Callback::from(move |e: InputEvent| {
                        if let Some(target) = e.target() {
                            use wasm_bindgen::JsCast;
                            if let Ok(input) = target.dyn_into::<web_sys::HtmlInputElement>() {
                                rename_input_value.set(input.value());
                            }
                        }
                    })
                };

                html! {
                    <div class="modal-backdrop" onclick={on_cancel_rename.clone()}>
                        <div class="confirm-modal" onclick={Callback::from(|e: MouseEvent| e.stop_propagation())}>
                            <h3>{ "대화 이름 변경" }</h3>
                            <input
                                class="rename-input"
                                type="text"
                                value={(*rename_input_value).clone()}
                                oninput={on_input}
                                autofocus=true
                            />
                            <div class="modal-actions">
                                <button class="modal-btn cancel" onclick={on_cancel_rename}>{ "취소" }</button>
                                <button class="modal-btn" style="background: var(--sidebar-active); color: white;" onclick={on_confirm_rename}>{ "저장" }</button>
                            </div>
                        </div>
                    </div>
                }
            } else { html! {} } }

            { if let Some(delete_target_value) = *delete_target {
                let (title, message) = match delete_target_value {
                    DeleteTarget::Session(_) => ("채팅 기록 삭제", "정말로 이 채팅 기록을 삭제하시겠습니까?"),
                    DeleteTarget::All => ("대화 전체 삭제", "대화내용 전체를 삭제하시겠습니까?"),
                };

                html! {
                    <div class="modal-backdrop">
                        <div class="confirm-modal">
                            <h3>{ title }</h3>
                            <p>
                                { message }
                                <br/>
                                { "삭제된 기록은 복구할 수 없습니다." }
                            </p>
                            <div class="modal-actions">
                                <button class="modal-btn cancel" onclick={on_cancel_delete}>{ "취소" }</button>
                                <button class="modal-btn delete" onclick={on_confirm_delete}>{ "삭제" }</button>
                            </div>
                        </div>
                    </div>
                }
            } else { html! {} } }
            { if *is_login_modal_open {
                let on_close = {
                    let is_login_modal_open = is_login_modal_open.clone();
                    Callback::from(move |_: MouseEvent| is_login_modal_open.set(false))
                };
                let on_id_input = {
                    let login_id = login_id.clone();
                    Callback::from(move |e: InputEvent| {
                        if let Some(target) = e.target() {
                            use wasm_bindgen::JsCast;
                            if let Ok(input) = target.dyn_into::<web_sys::HtmlInputElement>() {
                                login_id.set(input.value());
                            }
                        }
                    })
                };
                let on_password_input = {
                    let login_password = login_password.clone();
                    Callback::from(move |e: InputEvent| {
                        if let Some(target) = e.target() {
                            use wasm_bindgen::JsCast;
                            if let Ok(input) = target.dyn_into::<web_sys::HtmlInputElement>() {
                                login_password.set(input.value());
                            }
                        }
                    })
                };
                let on_continue_without_login = on_continue_without_login.clone();
                let on_toggle_password = {
                    let show_password = show_password.clone();
                    Callback::from(move |_: MouseEvent| show_password.set(!*show_password))
                };
                let password_input_type = if *show_password { "text" } else { "password" };

                html! {
                    <div class="modal-backdrop" onclick={on_close}>
                        <div class="login-modal" onclick={Callback::from(|e: MouseEvent| e.stop_propagation())}>
                            <div class="login-header">
                                <h3>{ "로그인" }</h3>
                            </div>
                            <div class="login-form">
                                <div class="form-group">
                                    <label>{ "아이디" }</label>
                                    <input
                                        type="text"
                                        value={(*login_id).clone()}
                                        oninput={on_id_input}
                                        onkeydown={on_login_keydown.clone()}
                                        autofocus=true
                                    />
                                </div>
                                <div class="form-group">
                                    <label>{ "비밀번호" }</label>
                                    <div class="password-field">
                                        <input
                                            type={password_input_type}
                                            value={(*login_password).clone()}
                                            oninput={on_password_input}
                                            onkeydown={on_login_keydown.clone()}
                                        />
                                        <button
                                            type="button"
                                            class="password-toggle"
                                            onclick={on_toggle_password}
                                            aria-label={ if *show_password { "비밀번호 숨기기" } else { "비밀번호 표시" } }
                                            title={ if *show_password { "비밀번호 숨기기" } else { "비밀번호 표시" } }
                                        >
                                            { if *show_password {
                                                html! {
                                                    <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                                        <path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19"></path>
                                                        <line x1="1" y1="1" x2="23" y2="23"></line>
                                                    </svg>
                                                }
                                            } else {
                                                html! {
                                                    <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                                        <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"></path>
                                                        <circle cx="12" cy="12" r="3"></circle>
                                                    </svg>
                                                }
                                            }}
                                        </button>
                                    </div>
                                </div>
                                if let Some(err) = &*login_error {
                                    <div class="login-error-msg">{ err }</div>
                                }
                                <button
                                    class="login-submit-btn"
                                    disabled={*is_logging_in}
                                    onclick={on_login_submit}
                                >
                                    if *is_logging_in {
                                        { "로그인 중..." }
                                    } else {
                                        { "로그인" }
                                    }
                                </button>
                                <button
                                    type="button"
                                    class="login-secondary-btn"
                                    onclick={on_continue_without_login}
                                >
                                    { "로그인하지 않고 이용하기" }
                                </button>
                            </div>
                        </div>
                    </div>
                }
            } else { html! {} } }
        </div>
    }
}

fn extract_suggestions(user_content: &str, response_content: &str) -> Option<Vec<String>> {
    let normalized_user_content = normalize_for_matching(user_content);
    let is_pending_permit_list_request = is_pending_permit_list_request(user_content);
    let is_prework_plan_submission_query = is_prework_plan_submission_request(user_content);
    let is_checklist_request = normalized_user_content.contains("작업전체크리스트")
        || normalized_user_content.contains("체크리스트");

    let is_permit_related =
        response_content.contains("허가서") || response_content.contains("작업허가");
    let is_checklist_related = response_content.contains("체크리스트") || is_checklist_request;
    let is_no_results = response_content.contains("없습니다")
        || response_content.contains("찾을 수")
        || response_content.contains("기록이");
    let is_asking_type = response_content.contains("어떤 종류")
        || response_content.contains("구체적인 종류")
        || response_content.contains("작업 종류")
        || response_content.contains("작업 유형")
        || response_content.contains("선택");
    let is_pending = response_content.contains("승인 대기")
        || response_content.contains("확인 대기")
        || response_content.contains("대기 중");
    let needs_suggestions = is_asking_type || is_no_results || is_pending;
    let is_pending_permit_work_type_selection = PENDING_PERMIT_WORK_TYPES
        .iter()
        .any(|work_type| normalize_for_matching(work_type) == normalized_user_content);
    let is_checklist_work_type_selection = CHECKLIST_WORK_TYPE_SUGGESTIONS
        .iter()
        .any(|work_type| normalize_for_matching(work_type) == normalized_user_content);

    if is_pending_permit_work_type_selection {
        return None;
    }

    if is_checklist_work_type_selection && is_no_results && !is_asking_type {
        return None;
    }

    if is_checklist_work_type_selection {
        return None;
    }

    if is_prework_plan_submission_query && is_asking_type {
        return Some(
            PENDING_PERMIT_WORK_TYPES
                .into_iter()
                .map(str::to_string)
                .collect(),
        );
    }

    if is_checklist_related && is_asking_type {
        return Some(
            CHECKLIST_WORK_TYPE_SUGGESTIONS
                .into_iter()
                .map(str::to_string)
                .collect(),
        );
    }

    if is_pending_permit_list_request && needs_suggestions {
        return Some(
            PENDING_PERMIT_WORK_TYPES
                .into_iter()
                .map(str::to_string)
                .collect(),
        );
    }

    if is_permit_related && needs_suggestions {
        return Some(
            ["위험작업 허가서", "화기작업 허가서", "직접 입력"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        );
    }

    None
}
fn main() {
    wasm_logger::init(wasm_logger::Config::default());
    yew::Renderer::<App>::new().render();
}
