use serde::{Deserialize, Serialize};
use serde_json;
use wasm_bindgen::prelude::*;
use yew::prelude::*;

use crate::types::{AgentMeta, ChatSession, BOARD_POST_ANALYZING_AGENT_ID};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_name = getElementRect)]
    fn get_element_rect(id: &str) -> Option<String>;
}

#[derive(Deserialize, Serialize, Clone, Copy, Debug)]
struct ElementRect {
    top: f64,
    left: f64,
    width: f64,
    height: f64,
}

fn session_agent_label(agents: &[AgentMeta], session: &ChatSession) -> String {
    let candidate_ids: Vec<String> = if session.routed_agent_ids.is_empty() {
        vec![session.agent_id.clone()]
    } else {
        session.routed_agent_ids.clone()
    };

    let labels: Vec<String> = candidate_ids
        .iter()
        .filter_map(|id| {
            if id.as_str() == BOARD_POST_ANALYZING_AGENT_ID {
                Some("분석 중".to_string())
            } else {
                agents
                    .iter()
                    .find(|agent| agent.id == *id)
                    .map(|agent| agent.name.clone())
            }
        })
        .collect();

    if labels.is_empty() {
        session.agent_id.clone()
    } else {
        labels.join(", ")
    }
}

#[derive(Properties, PartialEq)]
pub struct SidebarProps {
    pub sessions: Vec<ChatSession>,
    pub agents: Vec<AgentMeta>,
    pub selected_session_id: Option<uuid::Uuid>,
    pub on_select_session: Callback<uuid::Uuid>,
    pub on_delete_session: Callback<uuid::Uuid>,
    pub on_delete_all_sessions: Callback<()>,
    pub on_new_chat: Callback<()>,
    pub on_search_open: Callback<()>,
    pub on_pin_session: Callback<uuid::Uuid>,
    pub on_rename_click: Callback<uuid::Uuid>,
    pub health_status: Option<String>,
}

#[function_component(Sidebar)]
pub fn sidebar(props: &SidebarProps) -> Html {
    let dropdown_open_id = use_state(|| None::<uuid::Uuid>);
    let dropdown_pos = use_state(|| (0.0, 0.0));

    // Handle outside click to close dropdown
    {
        let dropdown_open_id = dropdown_open_id.clone();
        use_effect_with((), move |_| {
            let listener =
                gloo_events::EventListener::new(&web_sys::window().unwrap(), "click", move |_| {
                    dropdown_open_id.set(None);
                });
            move || drop(listener)
        });
    }

    let mut sorted_sessions = props.sessions.clone();
    sorted_sessions.sort_by(|a, b| {
        if a.is_pinned != b.is_pinned {
            b.is_pinned.cmp(&a.is_pinned)
        } else {
            b.timestamp.cmp(&a.timestamp)
        }
    });

    let has_sessions = !sorted_sessions.is_empty();

    let health_class = match props.health_status.as_deref() {
        Some("healthy") => "healthy",
        Some(_) => "unhealthy",
        None => "unknown",
    };
    let health_text = match props.health_status.as_deref() {
        Some("healthy") => "서버 연결됨",
        Some(_) => "서버 오류",
        None => "연결 확인 중...",
    };

    let on_delete_all_sessions = {
        let cb = props.on_delete_all_sessions.clone();
        Callback::from(move |e: MouseEvent| {
            e.stop_propagation();
            if has_sessions {
                cb.emit(())
            }
        })
    };

    html! {
        <div class="sidebar">
            // ── Header ──
            <div class="sidebar-header" onclick={
                let cb = props.on_new_chat.clone();
                Callback::from(move |_: MouseEvent| cb.emit(()))
            }>
                <h1>
                    <span class="dot"></span>
                    { "Core Agentic AI" }
                </h1>
                <p>{ "EHS 안전보건환경 AI" }</p>
            </div>

            // ── New chat button ──
            <div class="sidebar-section">
                <div class="new-chat-btn" onclick={
                    let cb = props.on_new_chat.clone();
                    Callback::from(move |_: MouseEvent| cb.emit(()))
                }>
                    { "+ 새 대화" }
                </div>
            </div>

            // ── History ──
            <div class="sidebar-section" style="flex: 1; overflow-y: auto;">
                <div class="sidebar-section-title" style="display: flex; justify-content: space-between; align-items: center;">
                    { "대화 기록" }
                    <div class="history-actions">
                        <div class="search-trigger" onclick={
                            let cb = props.on_search_open.clone();
                            Callback::from(move |e: MouseEvent| {
                                e.stop_propagation();
                                cb.emit(())
                            })
                        }>
                            <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"></circle><line x1="21" y1="21" x2="16.65" y2="16.65"></line></svg>
                        </div>
                        <div
                            class={classes!("search-trigger", "delete-trigger", (!has_sessions).then_some("disabled"))}
                            onclick={on_delete_all_sessions}
                            title="대화 전체 삭제"
                        >
                            <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"></polyline><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path><line x1="10" y1="11" x2="10" y2="17"></line><line x1="14" y1="11" x2="14" y2="17"></line></svg>
                        </div>
                    </div>
                </div>
                { if sorted_sessions.is_empty() {
                    html! { <div style="padding: 20px; color: var(--muted); font-size: 12px; text-align: center;">{ "기록이 없습니다" }</div> }
                } else {
                    html! {
                        <div class="history-list">
                            { for sorted_sessions.iter().map(|session| {
                                let is_active = props.selected_session_id == Some(session.id);
                                let session_id = session.id;
                                let is_dropdown_open = *dropdown_open_id == Some(session_id);
                                let target_id = format!("more-btn-{}", session_id);

                                let on_click = {
                                    let cb = props.on_select_session.clone();
                                    Callback::from(move |_: MouseEvent| {
                                        cb.emit(session_id);
                                    })
                                };

                                let on_more_click = {
                                    let dropdown_open_id = dropdown_open_id.clone();
                                    let dropdown_pos = dropdown_pos.clone();
                                    let target_id = target_id.clone();
                                    Callback::from(move |e: MouseEvent| {
                                        e.stop_propagation();
                                        if *dropdown_open_id == Some(session_id) {
                                            dropdown_open_id.set(None);
                                        } else if let Some(json) = get_element_rect(&target_id) {
                                            if let Ok(rect) = serde_json::from_str::<ElementRect>(&json) {
                                                dropdown_pos.set((rect.top + rect.height + 4.0, rect.left));
                                                dropdown_open_id.set(Some(session_id));
                                            }
                                        }
                                    })
                                };

                                let on_pin = {
                                    let cb = props.on_pin_session.clone();
                                    Callback::from(move |e: MouseEvent| {
                                        e.stop_propagation();
                                        cb.emit(session_id);
                                    })
                                };

                                let on_rename = {
                                    let cb = props.on_rename_click.clone();
                                    Callback::from(move |e: MouseEvent| {
                                        e.stop_propagation();
                                        cb.emit(session_id);
                                    })
                                };

                                let on_delete = {
                                    let cb = props.on_delete_session.clone();
                                    Callback::from(move |e: MouseEvent| {
                                        e.stop_propagation();
                                        cb.emit(session_id);
                                    })
                                };

                                let agent_name = session_agent_label(&props.agents, session);

                                let time_str = session.timestamp.with_timezone(&chrono::Local).format("%m/%d %H:%M").to_string();

                                html! {
                                    <div
                                        class={classes!("history-item", is_active.then_some("active"))}
                                        onclick={on_click}
                                    >
                                        if session.is_pinned {
                                            <div class="pin-badge">
                                                <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="currentColor" stroke="none"><path d="M21 10h-8V2l-7 12h5v8l7-10z"/></svg>
                                            </div>
                                        }
                                        <div class="history-item-main">
                                            <div class="history-title">{ &session.title }</div>
                                            <div class="history-meta">
                                                <span>{ agent_name }</span>
                                                <span>{ time_str }</span>
                                            </div>
                                        </div>
                                        <div
                                            id={target_id}
                                            class={classes!("history-more-btn", is_dropdown_open.then_some("active"))}
                                            onclick={on_more_click}
                                        >
                                            <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="1"></circle><circle cx="12" cy="5" r="1"></circle><circle cx="12" cy="19" r="1"></circle></svg>
                                        </div>

                                        if is_dropdown_open {
                                            <div
                                                class="dropdown-menu visible"
                                                style={format!("top: {}px; left: {}px;", dropdown_pos.0, dropdown_pos.1)}
                                                onclick={Callback::from(|e: MouseEvent| e.stop_propagation())}
                                            >
                                                <div class="dropdown-item" onclick={on_pin}>
                                                    <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 10h-8V2l-7 12h5v8l7-10z"/></svg>
                                                    { if session.is_pinned { "고정 해제" } else { "고정" } }
                                                </div>
                                                <div class="dropdown-item" onclick={on_rename}>
                                                    <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"></path><path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"></path></svg>
                                                    { "이름 변경" }
                                                </div>
                                                <div class="dropdown-item delete" onclick={on_delete}>
                                                    <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 6 5 6 21 6"></polyline><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path><line x1="10" y1="11" x2="10" y2="17"></line><line x1="14" y1="11" x2="14" y2="17"></line></svg>
                                                    { "삭제" }
                                                </div>
                                            </div>
                                        }
                                    </div>
                                }
                            })}
                        </div>
                    }
                }}
            </div>

            // ── Footer ──
            <div class="sidebar-footer">
                <span class={classes!("status-dot", health_class)}></span>
                { health_text }
            </div>
        </div>
    }
}
