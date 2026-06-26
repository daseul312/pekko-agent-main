use uuid::Uuid;
use web_sys::{Element, ScrollBehavior, ScrollIntoViewOptions};
use yew::prelude::*;

use crate::components::input::ChatInput;
use crate::components::message::{MessageBubble, ProcessSteps, TypingIndicator};
use crate::components::suggestions::Suggestions;
use crate::types::{
    AgentMeta, BoardPostCompletion, BoardPostTarget, ChatMessage, ChatMessageUi, FavoriteQuestion,
    FeedbackSubmission, MessageRole, ProcessStep,
};

#[derive(Properties, PartialEq)]
pub struct ChatProps {
    pub messages: Vec<ChatMessage>,
    pub is_loading: bool,
    #[prop_or_default]
    pub live_steps: Vec<ProcessStep>,
    pub on_send: Callback<String>,
    pub on_regenerate: Callback<String>,
    /// "다른 에이전트로 재시도": (assistant 메시지 id, 선택한 agent id).
    pub on_retry_with_agent: Callback<(String, String)>,
    /// "질문 수정": (user 메시지 id, 수정된 질문 내용).
    pub on_edit_message: Callback<(String, String)>,
    pub on_select_response_page: Callback<(String, usize)>,
    /// 답변 피드백(👍/👎) 제출.
    pub on_feedback: Callback<FeedbackSubmission>,
    pub on_board_post_target_selected: Callback<BoardPostTarget>,
    pub on_board_post_complete: Callback<BoardPostCompletion>,
    pub on_example_click: Callback<String>,
    pub on_back_to_agents: Callback<()>,
    /// 현재 에이전트로 이미 필터링된 즐겨찾기 목록(시작 화면 "내 즐겨찾기" 블록용).
    #[prop_or_default]
    pub favorites: Vec<FavoriteQuestion>,
    /// (agent_id, question) ⭐ 추가/해제 토글(예시질문용).
    pub on_favorite_toggle: Callback<(String, String)>,
    /// 즐겨찾기 원클릭 재질문.
    pub on_favorite_ask: Callback<FavoriteQuestion>,
    /// 내 질문 말풍선 ⭐ 토글(질문 내용만 전달, 에이전트는 상위에서 결합).
    pub on_message_favorite_toggle: Callback<String>,
    pub has_agent: bool,
    pub agent_meta: Option<AgentMeta>,
    pub is_authenticated: bool,
    pub on_login_click: Callback<()>,
    #[prop_or_default]
    pub token: Option<String>,
    /// 현재 세션 id(답변 공유 레코드 추적용 메타데이터).
    #[prop_or_default]
    pub session_id: Option<Uuid>,
}

/// 에이전트별 시작화면 예시질문 목록. welcome 분기 렌더와 "예시 출처 판정"(⭐ 노출 조건)에
/// 동일하게 재사용한다.
fn example_questions_for(agent_id: Option<&str>) -> Vec<&'static str> {
    match agent_id {
        Some("ehs-permit-agent") => vec![
            "오늘 확인 대기 중인 위험작업 허가 목록 보여줘",
            "이번 주 작업 전 계획서 제출 현황 요약해줘",
            "작업 전 체크리스트에서 미완료 항목 찾아줘",
            "오늘 TBM 서명 미완료자 현황 보여줘",
        ],
        Some("ehs-inspection-agent") => vec![
            "이번 주 순회점검 일정 요약해줘",
            "지연된 개선조치 현황 보여줘",
            "합동안전점검 결과 요약 초안 작성해줘",
            "이번 주 미점검 처리된 순회점검 찾아줘",
        ],
        Some("ehs-compliance-agent") => vec![
            "산업안전보건법 최신 법령 개정 내용 요약해줘",
            "중대재해 점검 항목 정리해줘",
            "최근 안전보건위원회 회의 안건과 참석자 현황 보여줘",
            "관리자 준수 책임을 설명해줘",
        ],
        Some("ehs-education-agent") => vec![
            "이번 달 예정된 교육 계획과 강사 배정 현황 보여줘",
            "미이수 교육 대상자와 재시험 대상자 확인해줘",
            "진행 중인 설문 목록과 응답 현황 요약해줘",
            "교육 관련 알림 발송과 완료 처리 현황 보여줘",
        ],
        Some("ehs-health-environment-agent") => vec![
            "유해화학물질 분석 완료 현황과 법규 대상 성분 요약해줘",
            "작업환경측정 결과에서 노출기준 초과 공정 찾아줘",
            "정기건강검진 미수검자와 사후관리 대상자 현황 보여줘",
            "비상연락망, 비상훈련, 구급함 점검 현황을 한 번에 요약해줘",
        ],
        Some("ehs-accident-agent") => vec![
            "최근 사고 보고와 첨부파일 현황 보여줘",
            "기한이 임박한 아차사고 조치 확인해줘",
            "유해위험요인 신고와 개선 현황 요약해줘",
            "수급업체 결재선과 대외문서 이력 검토해줘",
        ],
        _ => vec![
            "현재 위험작업 현황 보여줘",
            "이번 달 점검 계획 요약해줘",
            "등록된 MSDS 목록 보여줘",
            "최근 사고 추세 분석해줘",
        ],
    }
}

#[function_component(ChatArea)]
pub fn chat_area(props: &ChatProps) -> Html {
    let messages_end_ref = use_node_ref();
    let input_focus_trigger = use_state(|| 0u32);
    let input_forced_disabled = use_state(|| false);

    {
        let messages_end_ref = messages_end_ref.clone();
        let msg_count = props.messages.len();
        let is_loading = props.is_loading;
        use_effect_with((msg_count, is_loading), move |_| {
            if let Some(el) = messages_end_ref.cast::<Element>() {
                let opts = ScrollIntoViewOptions::new();
                opts.set_behavior(ScrollBehavior::Smooth);
                el.scroll_into_view_with_scroll_into_view_options(&opts);
            }
            || ()
        });
    }

    {
        let messages = props.messages.clone();
        let input_forced_disabled = input_forced_disabled.clone();
        use_effect_with(messages, move |msgs| {
            if let Some(last_msg) = msgs.last() {
                if last_msg.role == MessageRole::Assistant && last_msg.suggestions.is_some() {
                    input_forced_disabled.set(true);
                } else {
                    input_forced_disabled.set(false);
                }
            } else {
                input_forced_disabled.set(false);
            }
            || ()
        });
    }

    let on_suggestion_click = {
        let on_send = props.on_send.clone();
        let input_focus_trigger = input_focus_trigger.clone();
        let input_forced_disabled = input_forced_disabled.clone();
        Callback::from(move |suggested: String| {
            if suggested == "직접 입력" {
                input_forced_disabled.set(false);
                input_focus_trigger.set(*input_focus_trigger + 1);
            } else {
                input_forced_disabled.set(false);
                on_send.emit(suggested);
            }
        })
    };

    let show_welcome = props.messages.is_empty() && !props.is_loading;

    if show_welcome {
        let examples = example_questions_for(props.agent_meta.as_ref().map(|m| m.id.as_str()));
        let agent_class = props.agent_meta.as_ref().map(|m| m.css_class).unwrap_or("");
        let agent_name = props
            .agent_meta
            .as_ref()
            .map(|m| m.name.as_str())
            .unwrap_or("EHS AI Agent");
        // 현재 에이전트 id와, 이미 즐겨찾기에 담긴 질문 텍스트 집합(별 채움 판정용).
        let agent_id = props
            .agent_meta
            .as_ref()
            .map(|m| m.id.clone())
            .unwrap_or_default();
        let favorited_set: std::collections::HashSet<String> =
            props.favorites.iter().map(|f| f.question.clone()).collect();
        let back_to_agents = {
            let cb = props.on_back_to_agents.clone();
            Callback::from(move |_: MouseEvent| cb.emit(()))
        };

        html! {
            <>
                <div class="welcome">
                    <div class={classes!("welcome-agent-icon", agent_class)}></div>
                    <h2>{ format!("{}와 대화를 시작해보세요", agent_name) }</h2>
                    <p>{ "아래 예시 질문을 클릭하거나 궁금한 점을 직접 입력해 주세요." }</p>
                    { if props.favorites.is_empty() {
                        html! {}
                    } else {
                        html! {
                            <div class="welcome-favorites">
                                <div class="welcome-favorites-title">{ "⭐ 내 즐겨찾기" }</div>
                                <div class="welcome-favorites-chips">
                                    { for props.favorites.iter().map(|fav| {
                                        let fav_clone = fav.clone();
                                        let cb = props.on_favorite_ask.clone();
                                        html! {
                                            <button type="button" class="favorite-chip"
                                                onclick={Callback::from(move |_: MouseEvent| cb.emit(fav_clone.clone()))}
                                                title={fav.question.clone()}
                                            >
                                                <span class="favorite-chip-star">{ "⭐" }</span>
                                                <span class="favorite-chip-text">{ &fav.question }</span>
                                            </button>
                                        }
                                    })}
                                </div>
                            </div>
                        }
                    }}
                    <div class="welcome-examples">
                        { for examples.into_iter().map(|ex| {
                            let text = ex.to_string();
                            let cb = props.on_example_click.clone();
                            let emitted = text.clone();
                            let is_fav = favorited_set.contains(&text);
                            let on_star = {
                                let toggle = props.on_favorite_toggle.clone();
                                let agent_id = agent_id.clone();
                                let q = text.clone();
                                Callback::from(move |e: MouseEvent| {
                                    e.stop_propagation();
                                    toggle.emit((agent_id.clone(), q.clone()));
                                })
                            };
                            html! {
                                <div class="welcome-example"
                                    onclick={Callback::from(move |_: MouseEvent| cb.emit(emitted.clone()))}
                                >
                                    <span class="example-icon">{ "💡" }</span>
                                    <span class="example-text">{ text }</span>
                                    <button type="button"
                                        class={classes!("example-fav-star", is_fav.then_some("active"))}
                                        onclick={on_star}
                                        title={ if is_fav { "즐겨찾기 해제" } else { "즐겨찾기에 추가" } }
                                    >
                                        { if is_fav { "⭐" } else { "☆" } }
                                    </button>
                                </div>
                            }
                        })}
                    </div>
                    <button class="welcome-agent-switch-cta" type="button" onclick={back_to_agents}>
                        <span class="welcome-agent-switch-icon">
                            <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
                                <rect width="7" height="7" x="3" y="3" rx="1.5"></rect>
                                <rect width="7" height="7" x="14" y="3" rx="1.5"></rect>
                                <rect width="7" height="7" x="3" y="14" rx="1.5"></rect>
                                <rect width="7" height="7" x="14" y="14" rx="1.5"></rect>
                            </svg>
                        </span>
                        <span class="welcome-agent-switch-copy">
                            <span class="welcome-agent-switch-title">{ "다른 에이전트 선택" }</span>
                            <span class="welcome-agent-switch-subtitle">{ "업무 유형을 다시 고르기" }</span>
                        </span>
                        <span class="welcome-agent-switch-arrow">{ "→" }</span>
                    </button>
                </div>
                <ChatInput
                    on_send={props.on_send.clone()}
                    disabled={!props.has_agent || props.is_loading}
                    focus_trigger={*input_focus_trigger}
                />
            </>
        }
    } else {
        let favorited_set: std::collections::HashSet<String> =
            props.favorites.iter().map(|f| f.question.clone()).collect();
        // 현재 에이전트 예시질문 집합(예시 출처 메시지에는 ⭐ 미표시).
        let example_set: std::collections::HashSet<&str> =
            example_questions_for(props.agent_meta.as_ref().map(|m| m.id.as_str()))
                .into_iter()
                .collect();
        html! {
            <>
                <div class="messages">
                    { for props.messages.iter().enumerate().map(|(i, msg)| {
                        let is_user = matches!(msg.role, MessageRole::User);
                        let is_favorited = is_user && favorited_set.contains(&msg.content);
                        // 예시질문/종류 선택 칩으로 보낸 메시지에는 즐겨찾기 버튼을 숨긴다.
                        let from_example = is_user && example_set.contains(msg.content.as_str());
                        let from_suggestion = is_user
                            && i > 0
                            && matches!(props.messages[i - 1].role, MessageRole::Assistant)
                            && props.messages[i - 1]
                                .suggestions
                                .as_ref()
                                .map_or(false, |s| s.contains(&msg.content));
                        let can_favorite = is_user && !from_example && !from_suggestion;
                        let ui_key = match &msg.ui {
                            Some(ChatMessageUi::BoardPostMenu) => "menu",
                            Some(ChatMessageUi::BoardPostEditMenu) => "edit-menu",
                            Some(ChatMessageUi::BoardPostDeleteMenu) => "delete-menu",
                            Some(ChatMessageUi::BoardPostForm) => "form",
                            None => "none",
                        };
                        let key = format!("{}-{}", msg.id, ui_key);

                        html! {
                            <MessageBubble
                                key={key}
                                message={msg.clone()}
                                on_regenerate={props.on_regenerate.clone()}
                                on_retry_with_agent={props.on_retry_with_agent.clone()}
                                on_edit_message={props.on_edit_message.clone()}
                                on_select_response_page={props.on_select_response_page.clone()}
                                on_feedback={props.on_feedback.clone()}
                                on_board_post_target_selected={props.on_board_post_target_selected.clone()}
                                on_board_post_complete={props.on_board_post_complete.clone()}
                                on_favorite_toggle={props.on_message_favorite_toggle.clone()}
                                is_favorited={is_favorited}
                                can_favorite={can_favorite}
                                regenerate_disabled={props.is_loading}
                                token={props.token.clone()}
                                session_id={props.session_id}
                                is_authenticated={props.is_authenticated}
                            />
                        }
                    })}
                    if props.is_loading {
                        if !props.live_steps.is_empty() {
                            <div class="message-row assistant">
                                <div class="message-avatar">{ "AI" }</div>
                                <div class="process-steps-live">
                                    <ProcessSteps steps={props.live_steps.clone()} is_live=true />
                                    <div class="typing-indicator">
                                        <div class="typing-dot"></div>
                                        <div class="typing-dot"></div>
                                        <div class="typing-dot"></div>
                                    </div>
                                </div>
                            </div>
                        } else {
                            <TypingIndicator />
                        }
                    }
                    <div ref={messages_end_ref}></div>
                </div>
                { if !props.is_loading {
                    if let Some(msg) = props.messages.last() {
                        if msg.role == MessageRole::Assistant {
                            if let Some(suggestions) = &msg.suggestions {
                                html! { <Suggestions suggestions={suggestions.clone()} on_click={on_suggestion_click.clone()} /> }
                            } else {
                                html! {}
                            }
                        } else {
                            html! {}
                        }
                    } else {
                        html! {}
                    }
                } else {
                    html! {}
                }}
                <ChatInput
                    on_send={props.on_send.clone()}
                    disabled={!props.has_agent || props.is_loading || *input_forced_disabled}
                    focus_trigger={*input_focus_trigger}
                />
            </>
        }
    }
}
