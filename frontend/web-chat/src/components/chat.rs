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
/// 동일하게 재사용한다. 각 항목은 (카드 상단 태그 라벨, 질문 본문) 튜플.
fn example_questions_for(agent_id: Option<&str>) -> Vec<(&'static str, &'static str)> {
    match agent_id {
        Some("ehs-permit-agent") => vec![
            ("허가", "오늘 확인 대기 중인 안전작업 허가 목록 보여줘"),
            ("계획서", "승인된 작업계획서를 보고 싶어"),
            ("점검표", "작업전 점검표에서 미완료 항목 찾아줘"),
            ("TBM", "오늘 TBM 서명 미완료자 현황 보여줘"),
        ],
        Some("ehs-inspection-agent") => vec![
            ("일정", "이번 주 순회점검 일정 요약해줘"),
            ("개선조치", "지연된 개선조치 현황 보여줘"),
            ("합동점검", "합동안전점검 결과 요약 초안 작성해줘"),
            ("순회점검", "이번 주 미점검 처리된 순회점검 찾아줘"),
        ],
        Some("ehs-compliance-agent") => vec![
            ("법령", "산업안전보건법 최신 법령 개정 내용 요약해줘"),
            ("중대재해", "중대재해 점검 항목 정리해줘"),
            ("위원회", "최근 안전보건위원회 회의 안건과 참석자 현황 보여줘"),
            ("준수책임", "관리자 준수 책임을 설명해줘"),
        ],
        Some("ehs-education-agent") => vec![
            ("교육계획", "이번 달 예정된 교육 계획과 강사 배정 현황 보여줘"),
            ("미이수", "미이수 교육 대상자와 재시험 대상자 확인해줘"),
            ("설문", "진행 중인 설문 목록과 응답 현황 요약해줘"),
            ("알림", "교육 관련 알림 발송과 완료 처리 현황 보여줘"),
        ],
        Some("ehs-health-environment-agent") => vec![
            ("유해물질", "유해화학물질 분석 완료 현황과 법규 대상 성분 요약해줘"),
            ("작업환경", "작업환경측정 결과에서 노출기준 초과 공정 찾아줘"),
            ("건강검진", "정기건강검진 미수검자와 사후관리 대상자 현황 보여줘"),
            ("비상대응", "비상연락망, 비상훈련, 구급함 점검 현황을 한 번에 요약해줘"),
        ],
        Some("ehs-accident-agent") => vec![
            ("사고보고", "최근 사고 보고와 첨부파일 현황 보여줘"),
            ("아차사고", "기한이 임박한 아차사고 조치 확인해줘"),
            ("위험요인", "유해위험요인 신고와 개선 현황 요약해줘"),
            ("대외문서", "수급업체 결재선과 대외문서 이력 검토해줘"),
        ],
        _ => vec![
            ("현황", "현재 안전작업 현황 보여줘"),
            ("점검계획", "이번 달 점검 계획 요약해줘"),
            ("MSDS", "등록된 MSDS 목록 보여줘"),
            ("사고추세", "최근 사고 추세 분석해줘"),
        ],
    }
}

/// 에이전트 id로 시작화면 상단 통계 카드에 쓸 2글자 코드를 만든다(태그 라벨 접두어).
fn agent_short_code(agent_id: Option<&str>) -> &'static str {
    match agent_id {
        Some("ehs-permit-agent") => "PM",
        Some("ehs-inspection-agent") => "IN",
        Some("ehs-compliance-agent") => "CP",
        Some("ehs-education-agent") => "ED",
        Some("ehs-health-environment-agent") => "HE",
        Some("ehs-accident-agent") => "AC",
        _ => "SP",
    }
}

/// 시작화면 통계/예시질문 카드의 아이콘 배지용 라인 아이콘. 이모지 대신 앱 전역에서 쓰는
/// stroke 기반 SVG 스타일로 통일한다.
fn welcome_icon(kind: &str) -> Html {
    match kind {
        "clipboard" => html! {
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"></path>
                <rect x="8" y="2" width="8" height="4" rx="1" ry="1"></rect>
            </svg>
        },
        "document" => html! {
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"></path>
                <polyline points="14 2 14 8 20 8"></polyline>
                <line x1="16" y1="13" x2="8" y2="13"></line>
                <line x1="16" y1="17" x2="8" y2="17"></line>
            </svg>
        },
        "search" => html! {
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <circle cx="11" cy="11" r="8"></circle>
                <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
            </svg>
        },
        "people" => html! {
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"></path>
                <circle cx="9" cy="7" r="4"></circle>
                <path d="M23 21v-2a4 4 0 0 0-3-3.87"></path>
                <path d="M16 3.13a4 4 0 0 1 0 7.75"></path>
            </svg>
        },
        _ => html! {
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"></path>
                <polyline points="22 4 12 14.01 9 11.01"></polyline>
            </svg>
        },
    }
}

/// 시작화면 상단 통계 카드용 더미 데이터: (아이콘, 라벨, 값, 서브라벨, 하단 링크 문구) 3개.
/// 마지막 항목은 강조(accent) 카드로 렌더된다.
/// TODO: 실제 집계 API가 연결되면 더미 값을 대체한다.
type StatCard = (&'static str, &'static str, &'static str, &'static str, &'static str);
fn agent_stats_for(agent_id: Option<&str>) -> [StatCard; 3] {
    match agent_id {
        Some("ehs-permit-agent") => [
            ("clipboard", "확인 대기", "12건", "안전작업 허가", "목록 보기"),
            ("people", "서명 미완료", "5명", "오늘 TBM", "목록 보기"),
            ("check", "오늘 처리 필요", "17건", "허가·TBM 포함", "전체 업무 보기"),
        ],
        Some("ehs-inspection-agent") => [
            ("clipboard", "지연 조치", "7건", "개선조치", "목록 보기"),
            ("search", "미점검", "3건", "이번 주 순회점검", "목록 보기"),
            ("check", "오늘 처리 필요", "10건", "점검·조치 포함", "전체 업무 보기"),
        ],
        Some("ehs-compliance-agent") => [
            ("clipboard", "개정 법령", "4건", "이번 달", "목록 보기"),
            ("people", "미이행 안건", "2건", "위원회 회의", "목록 보기"),
            ("check", "오늘 처리 필요", "6건", "법령·안건 포함", "전체 업무 보기"),
        ],
        Some("ehs-education-agent") => [
            ("people", "미이수", "9명", "이번 달 교육", "목록 보기"),
            ("clipboard", "응답 대기", "3건", "진행 중 설문", "목록 보기"),
            ("check", "오늘 처리 필요", "12건", "교육·설문 포함", "전체 업무 보기"),
        ],
        Some("ehs-health-environment-agent") => [
            ("search", "노출기준 초과", "2건", "작업환경측정", "목록 보기"),
            ("people", "미수검", "6명", "정기건강검진", "목록 보기"),
            ("check", "오늘 처리 필요", "8건", "측정·검진 포함", "전체 업무 보기"),
        ],
        Some("ehs-accident-agent") => [
            ("clipboard", "처리 지연", "4건", "아차사고 조치", "목록 보기"),
            ("people", "미결재", "3건", "대외문서", "목록 보기"),
            ("check", "오늘 처리 필요", "7건", "사고·문서 포함", "전체 업무 보기"),
        ],
        _ => [
            ("clipboard", "확인 대기", "-", "-", "목록 보기"),
            ("people", "미완료", "-", "-", "목록 보기"),
            ("check", "오늘 처리 필요", "-", "-", "전체 업무 보기"),
        ],
    }
}

/// 예시질문 카드 왼쪽 아이콘 배지. 태그 의미와 무관하게 순서대로 4종을 순환시킨다.
fn example_icon_for(index: usize) -> &'static str {
    const ICONS: [&str; 4] = ["clipboard", "document", "search", "people"];
    ICONS[index % ICONS.len()]
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
        let agent_id = props.agent_meta.as_ref().map(|m| m.id.as_str());
        let examples = example_questions_for(agent_id);
        let short_code = agent_short_code(agent_id);
        let stats = agent_stats_for(agent_id);
        let agent_name = props
            .agent_meta
            .as_ref()
            .map(|m| m.name.as_str())
            .unwrap_or("EHS AI Agent");

        html! {
            <div class="welcome-shell">
                <div class="welcome">
                    <div class="welcome-hero">
                        <div class="welcome-hero-intro">
                            <div class="welcome-hero-eyebrow">{ "EHS AGENT" }</div>
                            <h2>{ format!("{}와", agent_name) }<br />{ "대화를 시작해보세요" }</h2>
                            <p>{ "아래 예시 질문을 클릭하거나 직접 입력하세요" }</p>
                        </div>
                        { for stats.iter().enumerate().map(|(i, (icon, label, value, sub, link_label))| {
                            let is_highlight = i == stats.len() - 1;
                            let card_class = classes!("welcome-stat-card", is_highlight.then_some("highlight"));
                            html! {
                                <div class={card_class}>
                                    <div class="welcome-stat-head">
                                        <span class="welcome-stat-icon">{ welcome_icon(icon) }</span>
                                        <span class="welcome-stat-label">{ *label }</span>
                                    </div>
                                    <div class="welcome-stat-value">{ *value }</div>
                                    <div class="welcome-stat-sub">{ *sub }</div>
                                    <div class="welcome-stat-link">
                                        <span>{ *link_label }</span>
                                        <span class="welcome-stat-link-arrow">{ "→" }</span>
                                    </div>
                                </div>
                            }
                        }) }
                    </div>
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
                    <div class="welcome-section-label">
                        <span class="welcome-section-label-icon">
                            <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"></path>
                            </svg>
                        </span>
                        <span>{ "예시 질문 · 클릭하면 바로 물어봐요" }</span>
                    </div>
                    <div class="welcome-examples">
                        { for examples.into_iter().enumerate().map(|(i, (tag, question))| {
                            let text = question.to_string();
                            let cb = props.on_example_click.clone();
                            let emitted = text.clone();
                            let tag_label = format!("{}-{:02} · {}", short_code, i + 1, tag);
                            let icon = example_icon_for(i);
                            html! {
                                <div class="welcome-example"
                                    onclick={Callback::from(move |_: MouseEvent| cb.emit(emitted.clone()))}
                                >
                                    <span class="example-icon-badge">{ welcome_icon(icon) }</span>
                                    <span class="example-text-col">
                                        <span class="example-tag">{ tag_label }</span>
                                        <span class="example-text">{ text }</span>
                                    </span>
                                    <span class="example-chevron">{ "›" }</span>
                                </div>
                            }
                        })}
                    </div>
                </div>
                <div class="welcome-bottom">
                    <ChatInput
                        on_send={props.on_send.clone()}
                        disabled={!props.has_agent || props.is_loading}
                        focus_trigger={*input_focus_trigger}
                    />
                    <div class="welcome-disclaimer">
                        { "AI가 안전보건 관련 정보를 기반으로 답변합니다. 중요한 의사결정은 담당자 확인이 필요합니다." }
                    </div>
                </div>
            </div>
        }
    } else {
        let favorited_set: std::collections::HashSet<String> =
            props.favorites.iter().map(|f| f.question.clone()).collect();
        // 현재 에이전트 예시질문 집합(예시 출처 메시지에는 ⭐ 미표시).
        let example_set: std::collections::HashSet<&str> =
            example_questions_for(props.agent_meta.as_ref().map(|m| m.id.as_str()))
                .into_iter()
                .map(|(_, question)| question)
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
