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
use components::sidebar::Sidebar;
use types::{AgentMeta, ChatMessage, ChatSession, MessageRole};

const STORAGE_KEY: &str = "pekko_chat_sessions";

#[function_component(App)]
fn app() -> Html {
    // State
    let agents = use_state(|| AgentMeta::defaults());
    let selected_agent_id = use_state(|| Option::<String>::None);
    let messages = use_state(Vec::<ChatMessage>::new);
    let session_id = use_state(|| Option::<Uuid>::None);
    let sessions = use_state(|| {
        LocalStorage::get::<Vec<ChatSession>>(STORAGE_KEY).unwrap_or_default()
    });
    let is_loading = use_state(|| false);
    let health_status = use_state(|| Option::<String>::None);

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

    // Callbacks
    let on_select_agent = {
        let selected_agent_id = selected_agent_id.clone();
        let messages = messages.clone();
        let session_id = session_id.clone();
        Callback::from(move |id: String| {
            selected_agent_id.set(Some(id));
            messages.set(vec![]);
            session_id.set(None);
        })
    };

    let on_new_chat = {
        let selected_agent_id = selected_agent_id.clone();
        let messages = messages.clone();
        let session_id = session_id.clone();
        Callback::from(move |_: ()| {
            selected_agent_id.set(None);
            messages.set(vec![]);
            session_id.set(None);
        })
    };

    let on_select_session = {
        let selected_agent_id = selected_agent_id.clone();
        let messages = messages.clone();
        let session_id = session_id.clone();
        let sessions = sessions.clone();
        Callback::from(move |sid: Uuid| {
            if let Some(session) = sessions.iter().find(|s| s.id == sid) {
                selected_agent_id.set(Some(session.agent_id.clone()));
                messages.set(session.messages.clone());
                session_id.set(Some(session.id));
            }
        })
    };

    let delete_confirm_id = use_state(|| None::<Uuid>);

    let on_delete_session = {
        let delete_confirm_id = delete_confirm_id.clone();
        Callback::from(move |sid: Uuid| {
            delete_confirm_id.set(Some(sid));
        })
    };

    let on_confirm_delete = {
        let sessions = sessions.clone();
        let selected_session_id = session_id.clone();
        let messages = messages.clone();
        let selected_agent_id = selected_agent_id.clone();
        let delete_confirm_id = delete_confirm_id.clone();
        Callback::from(move |_: MouseEvent| {
            if let Some(sid) = *delete_confirm_id {
                let mut current_sessions = (*sessions).clone();
                current_sessions.retain(|s| s.id != sid);
                sessions.set(current_sessions);

                // Reset state when the active session is deleted
                if selected_session_id.as_ref() == Some(&sid) {
                    selected_session_id.set(None);
                    messages.set(vec![]);
                    selected_agent_id.set(None);
                }
            }
            delete_confirm_id.set(None);
        })
    };

    let on_cancel_delete = {
        let delete_confirm_id = delete_confirm_id.clone();
        Callback::from(move |_: MouseEvent| {
            delete_confirm_id.set(None);
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
        let selected_agent_id = selected_agent_id.clone();
        Callback::from(move |content: String| {
            let agent_id = match (*selected_agent_id).clone() {
                Some(id) => id,
                None => return,
            };

            // Add user message
            let user_msg = ChatMessage {
                id: Uuid::new_v4().to_string(),
                role: MessageRole::User,
                content: content.clone(),
                tools_used: vec![],
                token_usage: None,
                timestamp: chrono::Utc::now(),
            };
            let mut new_msgs = (*messages).clone();
            new_msgs.push(user_msg);
            messages.set(new_msgs.clone());
            is_loading.set(true);

            let messages_clone = messages.clone();
            let session_id_clone = session_id.clone();
            let sessions_clone = sessions.clone();
            let is_loading_clone = is_loading.clone();
            let sid = (*session_id).clone();
            let agent_id_clone = agent_id.clone();
            let current_msgs = new_msgs.clone();

            spawn_local(async move {
                match api::send_query(&agent_id, &content, sid).await {
                    Ok(resp) => {
                        let assistant_msg = ChatMessage {
                            id: Uuid::new_v4().to_string(),
                            role: MessageRole::Assistant,
                            content: resp.response,
                            tools_used: resp.tools_used,
                            token_usage: Some(resp.token_usage),
                            timestamp: chrono::Utc::now(),
                        };
                        let mut msgs = current_msgs.clone();
                        msgs.push(assistant_msg);
                        messages_clone.set(msgs.clone());

                        // Update or Create Session in History
                        let mut current_sessions = (*sessions_clone).clone();
                        let new_sid = resp.session_id;
                        session_id_clone.set(Some(new_sid));

                        if let Some(idx) = current_sessions.iter().position(|s| s.id == new_sid) {
                            current_sessions[idx].last_message = content.clone();
                            current_sessions[idx].messages = msgs;
                            current_sessions[idx].timestamp = chrono::Utc::now();
                        } else {
                            let mut clean_title = content.replace('?', "");
                            
                            if clean_title.contains("오늘") && !clean_title.contains("오늘 날짜") {
                                clean_title = clean_title.replace("오늘", "오늘 날짜");
                            }
                            if clean_title.contains("내일") && !clean_title.contains("내일 날짜") {
                                clean_title = clean_title.replace("내일", "내일 날짜");
                            }
                            
                            let endings_to_request = [
                                " 보여줘", " 알려줘", " 조회해줘", " 요약해줘", " 확인해줘", " 도와줘",
                                " 보여주세요", " 알려주세요", " 조회해주세요", " 요약해주세요", " 확인해주세요", " 도와주세요",
                                "보여줘", "알려줘", "조회해줘", "요약해줘", "확인해줘", "도와줘"
                            ];
                            let endings_to_remove = ["은", "는"];
                            
                            let mut handled = false;
                            for e in endings_to_request {
                                if clean_title.ends_with(e) {
                                    clean_title = clean_title.trim_end_matches(e).to_string();
                                    clean_title.push_str(" 요청");
                                    handled = true;
                                    break;
                                }
                            }
                            
                            if !handled {
                                for e in endings_to_remove {
                                    if clean_title.ends_with(e) {
                                        clean_title = clean_title.trim_end_matches(e).to_string();
                                        break;
                                    }
                                }
                            }
                            
                            clean_title = clean_title.trim().to_string();
                            
                            let initial_title = if clean_title.chars().count() > 25 {
                                clean_title.chars().take(22).collect::<String>() + "..."
                            } else {
                                clean_title
                            };

                            // Initial placeholder title based on first message
                            current_sessions.push(ChatSession {
                                id: new_sid,
                                agent_id: agent_id_clone.clone(),
                                title: initial_title,
                                last_message: content.clone(),
                                timestamp: chrono::Utc::now(),
                                messages: msgs,
                                is_pinned: false,
                            });
                            
                            // Trigger AI summarization in background
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
                        let err_msg = ChatMessage {
                            id: Uuid::new_v4().to_string(),
                            role: MessageRole::System,
                            content: format!("Error: {e}"),
                            tools_used: vec![],
                            token_usage: None,
                            timestamp: chrono::Utc::now(),
                        };
                        let mut msgs = current_msgs.clone();
                        msgs.push(err_msg);
                        messages_clone.set(msgs);
                    }
                }
                is_loading_clone.set(false);
            });
        })
    };

    let on_example_click = on_send.clone();

    // Resolve selected agent meta
    let selected_meta = (*selected_agent_id).as_ref().and_then(|id| {
        agents.iter().find(|a| a.id == *id).cloned()
    });

    html! {
        <>
                <Sidebar
                    sessions={(*sessions).clone()}
                    selected_session_id={(*session_id).clone()}
                    on_select_session={on_select_session.clone()}
                    on_delete_session={on_delete_session}
                    on_new_chat={on_new_chat}
                    on_search_open={on_search_open}
                    on_pin_session={on_pin_session}
                    on_rename_click={on_rename_click}
                    health_status={(*health_status).clone()}
                />
            <div class="main">
                if let Some(meta) = selected_meta {
                    <ChatHeader agent={Some(meta.clone())} />
                    <ChatArea
                        messages={(*messages).clone()}
                        is_loading={*is_loading}
                        on_send={on_send}
                        on_example_click={on_example_click}
                        has_agent=true
                        agent_meta={Some(meta)}
                    />
                } else {
                    <AgentSelector
                        agents={(*agents).clone()}
                        on_select={on_select_agent}
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
                                            let agent_name = match s.agent_id.as_str() {
                                                "ehs-permit-agent" => "허가 관리",
                                                "ehs-inspection-agent" => "점검 관리",
                                                "ehs-compliance-agent" => "규정 준수",
                                                _ => &s.agent_id,
                                            }.to_string();
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

            { if delete_confirm_id.is_some() {
                html! {
                    <div class="modal-backdrop">
                        <div class="confirm-modal">
                            <h3>{ "채팅 기록 삭제" }</h3>
                            <p>
                                { "정말로 이 채팅 기록을 삭제하시겠습니까?" }
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
        </>
    }
}

fn main() {
    wasm_logger::init(wasm_logger::Config::default());
    yew::Renderer::<App>::new().render();
}
