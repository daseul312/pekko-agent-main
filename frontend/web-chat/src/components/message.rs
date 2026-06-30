use uuid::Uuid;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;

use crate::components::board_post::{BoardPostFlow, BoardPostMenuForm};
use crate::components::chart::ChartBlock;
use crate::markdown;
use crate::types::{
    AgentMeta, BoardPostCompletion, BoardPostTarget, ChatMessage, ChatMessageUi, FeedbackRating,
    FeedbackSubmission, MessageRole, OldNewArticleComparison, OldNewComparison, OldNewLawInfo,
    ProcessStep, StepStatus,
};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window)]
    fn downloadTableAsCSV(message_id: &str);
    /// index.html이 정의한 카카오 공유 래퍼. Kakao SDK가 로드/초기화돼 있으면
    /// 공유창을 띄우고 true, 아니면 false를 돌려준다(미로드 시 안내용). 타이틀은 헬퍼 내부 고정.
    #[wasm_bindgen(js_namespace = window, js_name = shareKakao)]
    fn share_kakao(url: &str, description: &str) -> bool;
}

/// 부정 피드백(👎) 사유 고정 5종. 백엔드 `FEEDBACK_TAGS`와 동일 순서/문자열이어야 한다.
const FEEDBACK_REASON_TAGS: [&str; 5] = [
    "출처/근거 불명확",
    "내용 부정확",
    "답변이 너무 길다",
    "최신 정보 미반영",
    "기타",
];

/// 공유 모달의 세 가지 액션.
#[derive(Clone, Copy)]
enum ShareAction {
    CopyLink,
    Kakao,
}

/// 이메일 공유 대상. 사용자가 "이메일로 보내기" 하위 메뉴에서 고른다.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EmailProvider {
    /// OS 기본 메일 앱(데스크톱 Outlook 등) — mailto.
    Default,
    /// Gmail 웹 작성(본문 prefill 지원).
    Gmail,
    /// Outlook 웹 작성(office.com, 본문 prefill 지원).
    Outlook,
    /// 네이버 메일 — 본문 prefill URL이 없어 링크를 클립보드에 복사 후 작성창을 연다.
    Naver,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CopyStatus {
    Idle,
    Success,
    Failed,
}

#[derive(Properties, PartialEq)]
pub struct MessageProps {
    pub message: ChatMessage,
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
    /// 사용자 질문 ⭐ 토글: 질문 내용을 전달(에이전트 id는 상위에서 결합).
    pub on_favorite_toggle: Callback<String>,
    /// 이 질문이 현재 에이전트 즐겨찾기에 담겨 있는지(별 채움 판정).
    #[prop_or_default]
    pub is_favorited: bool,
    /// 즐겨찾기 버튼 노출 여부. 예시질문/종류 선택 칩으로 보낸 메시지는 false.
    #[prop_or_default]
    pub can_favorite: bool,
    pub regenerate_disabled: bool,
    #[prop_or_default]
    pub token: Option<String>,
    /// 현재 세션 id(공유 레코드 추적용 메타데이터). 없어도 공유는 동작한다.
    #[prop_or_default]
    pub session_id: Option<Uuid>,
    /// 로그인 여부. 비로그인 사용자에게는 [공유] 버튼을 숨긴다.
    #[prop_or_default]
    pub is_authenticated: bool,
}

#[function_component(MessageBubble)]
pub fn message_bubble(props: &MessageProps) -> Html {
    let msg = &props.message;
    let copy_status = use_state(|| CopyStatus::Idle);
    let retry_menu_open = use_state(|| false);
    let is_editing = use_state(|| false);
    let edit_value = use_state(|| msg.content.clone());
    // 공유 모달 상태. share_url은 첫 공유 액션 시 1회 생성 후 캐시(고아 레코드 방지).
    let is_share_modal_open = use_state(|| false);
    let share_url = use_state(|| Option::<String>::None);
    let share_loading = use_state(|| false);
    let share_error = use_state(|| Option::<String>::None);
    let share_copied = use_state(|| false);
    // 이메일 하위 메뉴(서비스 선택) 펼침 여부 + 안내 문구(예: 네이버 복사 안내).
    let email_submenu_open = use_state(|| false);
    let share_notice = use_state(|| Option::<String>::None);
    // 👎 클릭 시 펼쳐지는 코멘트 입력창 상태 + 입력값 + 선택된 사유 태그(다중).
    let feedback_comment_open = use_state(|| false);
    let feedback_comment_value = use_state(String::new);
    let feedback_tags = use_state(Vec::<String>::new);
    let is_user = matches!(msg.role, MessageRole::User);
    let is_assistant = matches!(msg.role, MessageRole::Assistant);
    let board_post_route_flow = board_post_route_flow(&msg.content);
    let is_rendered_markdown = matches!(msg.role, MessageRole::Assistant | MessageRole::System)
        && msg.ui.is_none()
        && board_post_route_flow.is_none();
    let current_page = msg.current_response_page();
    let page_count = msg.response_page_count();

    let role_class = match msg.role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "assistant",
    };

    let avatar = match msg.role {
        MessageRole::User => "나",
        MessageRole::Assistant => "AI",
        MessageRole::System => "AI",
    };

    let has_token_usage = msg.token_usage.is_some();
    let token_html = if let Some(ref usage) = msg.token_usage {
        html! {
            <div class="token-info">
                <span class="token-item">
                    <span class="token-label">{ "입력" }</span>
                    { usage.input_tokens }
                </span>
                <span class="token-divider">{ "/" }</span>
                <span class="token-item">
                    <span class="token-label">{ "출력" }</span>
                    { usage.output_tokens }
                </span>
            </div>
        }
    } else {
        html! {}
    };

    let has_table = msg.content.lines().any(|line| line.trim().starts_with('|'));
    let msg_id = msg.id.clone();

    let content_html = match msg.ui.as_ref() {
        Some(ChatMessageUi::BoardPostMenu) => {
            html! {
                <div class="message-bubble board-post-bubble">
                    <BoardPostMenuForm
                        token={props.token.clone()}
                        on_target_selected={props.on_board_post_target_selected.clone()}
                        on_board_post_complete={props.on_board_post_complete.clone()}
                        selected_target={msg.board_post_target.clone()}
                        mode={BoardPostFlow::Create}
                    />
                </div>
            }
        }
        Some(ChatMessageUi::BoardPostEditMenu) => {
            html! {
                <div class="message-bubble board-post-bubble">
                    <BoardPostMenuForm
                        token={props.token.clone()}
                        on_target_selected={props.on_board_post_target_selected.clone()}
                        on_board_post_complete={props.on_board_post_complete.clone()}
                        selected_target={msg.board_post_target.clone()}
                        mode={BoardPostFlow::Edit}
                    />
                </div>
            }
        }
        Some(ChatMessageUi::BoardPostDeleteMenu) => {
            html! {
                <div class="message-bubble board-post-bubble">
                    <BoardPostMenuForm
                        token={props.token.clone()}
                        on_target_selected={props.on_board_post_target_selected.clone()}
                        on_board_post_complete={props.on_board_post_complete.clone()}
                        selected_target={msg.board_post_target.clone()}
                        mode={BoardPostFlow::Delete}
                    />
                </div>
            }
        }
        None if board_post_route_flow == Some(BoardPostFlow::Create) => {
            html! {
                <div class="message-bubble board-post-bubble">
                    <BoardPostMenuForm
                        token={props.token.clone()}
                        on_target_selected={props.on_board_post_target_selected.clone()}
                        on_board_post_complete={props.on_board_post_complete.clone()}
                        selected_target={msg.board_post_target.clone()}
                        mode={BoardPostFlow::Create}
                    />
                </div>
            }
        }
        None if board_post_route_flow == Some(BoardPostFlow::Edit) => {
            html! {
                <div class="message-bubble board-post-bubble">
                    <BoardPostMenuForm
                        token={props.token.clone()}
                        on_target_selected={props.on_board_post_target_selected.clone()}
                        on_board_post_complete={props.on_board_post_complete.clone()}
                        selected_target={msg.board_post_target.clone()}
                        mode={BoardPostFlow::Edit}
                    />
                </div>
            }
        }
        None if board_post_route_flow == Some(BoardPostFlow::Delete) => {
            html! {
                <div class="message-bubble board-post-bubble">
                    <BoardPostMenuForm
                        token={props.token.clone()}
                        on_target_selected={props.on_board_post_target_selected.clone()}
                        on_board_post_complete={props.on_board_post_complete.clone()}
                        selected_target={msg.board_post_target.clone()}
                        mode={BoardPostFlow::Delete}
                    />
                </div>
            }
        }
        Some(ChatMessageUi::BoardPostForm) => html! {},
        None if is_rendered_markdown => {
            let rendered = markdown::markdown_to_html(&msg.content);
            Html::from_html_unchecked(AttrValue::from(format!(
                "<div class=\"message-bubble md-content\" id=\"msg-{}\">{}</div>",
                msg.id, rendered
            )))
        }
        None => html! { <div class="message-bubble">{ &msg.content }</div> },
    };

    let copy_btn = if is_rendered_markdown {
        let content = msg.content.clone();
        let on_copy = {
            let copy_status = copy_status.clone();
            Callback::from(move |_: MouseEvent| {
                let content = content.clone();
                let copy_status = copy_status.clone();
                spawn_local(async move {
                    let copied = copy_to_clipboard(content).await;
                    copy_status.set(if copied {
                        CopyStatus::Success
                    } else {
                        CopyStatus::Failed
                    });
                    gloo_timers::future::TimeoutFuture::new(2_000).await;
                    copy_status.set(CopyStatus::Idle);
                });
            })
        };

        html! {
            <button class="copy-btn" onclick={on_copy} title="복사">
                if *copy_status == CopyStatus::Success {
                    { "복사됨" }
                } else if *copy_status == CopyStatus::Failed {
                    { "복사 실패" }
                } else {
                    { "📋 복사" }
                }
            </button>
        }
    } else {
        html! {}
    };

    // 답변에 그래프가 있으면 차트 데이터를 JSON으로 직렬화해 함께 스냅샷한다(열람 페이지 재현용).
    let chart_json: Option<String> = msg
        .chart_data
        .as_ref()
        .and_then(|c| serde_json::to_string(c).ok());

    // 공유 액션 디스패처: 캐시된 share_url이 있으면 즉시 실행하고, 없으면
    // 이 순간(=실제 액션 클릭) 1회 생성 후 실행한다(모달 오픈만으론 생성 안 함).
    let on_share_action = {
        let share_url = share_url.clone();
        let share_loading = share_loading.clone();
        let share_error = share_error.clone();
        let share_copied = share_copied.clone();
        let content = msg.content.clone();
        let message_id = msg.id.clone();
        let chart_json = chart_json.clone();
        let session_id = props.session_id;
        Callback::from(move |action: ShareAction| {
            share_error.set(None);
            if let Some(url) = (*share_url).clone() {
                perform_share_action(action, url, &content, &share_copied, &share_error);
                return;
            }
            if *share_loading {
                return;
            }
            let share_url = share_url.clone();
            let share_loading = share_loading.clone();
            let share_error = share_error.clone();
            let share_copied = share_copied.clone();
            let content = content.clone();
            let message_id = message_id.clone();
            let chart_json = chart_json.clone();
            share_loading.set(true);
            spawn_local(async move {
                match crate::api::create_share(&content, session_id, Some(message_id), chart_json).await {
                    Ok(url) => {
                        share_url.set(Some(url.clone()));
                        share_loading.set(false);
                        perform_share_action(action, url, &content, &share_copied, &share_error);
                    }
                    Err(e) => {
                        share_loading.set(false);
                        share_error.set(Some(format!("공유 링크 생성 실패: {e}")));
                    }
                }
            });
        })
    };

    // 이메일 서비스별 디스패처. 웹 메일(Gmail/Outlook/Naver)은 팝업 차단을 피하려
    // 사용자 클릭(제스처) 시점에 빈 탭을 먼저 열고, 링크 준비 후 그 탭을 작성 URL로 이동시킨다.
    let on_email_provider = {
        let share_url = share_url.clone();
        let share_loading = share_loading.clone();
        let share_error = share_error.clone();
        let share_notice = share_notice.clone();
        let content = msg.content.clone();
        let message_id = msg.id.clone();
        let chart_json = chart_json.clone();
        let session_id = props.session_id;
        Callback::from(move |provider: EmailProvider| {
            share_error.set(None);
            share_notice.set(None);
            let preopened = match provider {
                EmailProvider::Default => None,
                _ => web_sys::window()
                    .and_then(|w| w.open_with_url_and_target("", "_blank").ok().flatten()),
            };
            if let Some(url) = (*share_url).clone() {
                open_email_provider(provider, &url, &content, preopened, &share_notice);
                return;
            }
            if *share_loading {
                if let Some(w) = preopened {
                    let _ = w.close();
                }
                return;
            }
            let share_url = share_url.clone();
            let share_loading = share_loading.clone();
            let share_error = share_error.clone();
            let share_notice = share_notice.clone();
            let content = content.clone();
            let message_id = message_id.clone();
            let chart_json = chart_json.clone();
            share_loading.set(true);
            spawn_local(async move {
                match crate::api::create_share(&content, session_id, Some(message_id), chart_json).await {
                    Ok(url) => {
                        share_url.set(Some(url.clone()));
                        share_loading.set(false);
                        open_email_provider(provider, &url, &content, preopened, &share_notice);
                    }
                    Err(e) => {
                        share_loading.set(false);
                        if let Some(w) = preopened {
                            let _ = w.close();
                        }
                        share_error.set(Some(format!("공유 링크 생성 실패: {e}")));
                    }
                }
            });
        })
    };

    let share_btn = if is_rendered_markdown && props.is_authenticated {
        let is_share_modal_open = is_share_modal_open.clone();
        let share_error = share_error.clone();
        let share_copied = share_copied.clone();
        let share_notice = share_notice.clone();
        let email_submenu_open = email_submenu_open.clone();
        let on_share = Callback::from(move |_: MouseEvent| {
            share_error.set(None);
            share_copied.set(false);
            share_notice.set(None);
            email_submenu_open.set(false);
            is_share_modal_open.set(true);
        });
        html! {
            <button class="share-btn" onclick={on_share} title="공유">
                <svg xmlns="http://www.w3.org/2000/svg" width="15" height="15" viewBox="0 0 24 24"
                     fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <circle cx="18" cy="5" r="3"></circle>
                    <circle cx="6" cy="12" r="3"></circle>
                    <circle cx="18" cy="19" r="3"></circle>
                    <line x1="8.59" y1="13.51" x2="15.42" y2="17.49"></line>
                    <line x1="15.41" y1="6.51" x2="8.59" y2="10.49"></line>
                </svg>
                { " 공유" }
            </button>
        }
    } else {
        html! {}
    };

    let regenerate_btn = if is_assistant && msg.ui.is_none() {
        let message_id = msg.id.clone();
        let on_regenerate = {
            let on_regenerate = props.on_regenerate.clone();
            Callback::from(move |_: MouseEvent| {
                on_regenerate.emit(message_id.clone());
            })
        };

        html! {
            <button
                class="regenerate-btn"
                onclick={on_regenerate}
                disabled={props.regenerate_disabled}
                title="다시 생성"
            >
                { "🔄 다시 생성" }
            </button>
        }
    } else {
        html! {}
    };

    // 답변 피드백(👍/👎). 로그인 사용자에게만 노출(공유 버튼과 동일 게이트).
    // 버튼은 다른 액션 버튼과 같은 줄(message-actions)에 통합되고, 👎 선택 시 코멘트창이
    // 액션 줄 아래에 펼쳐진다. 👍는 즉시 제출, 같은 평가 재클릭은 취소(토글).
    // 평가는 현재 표시 중인 버전(current_response_page)에 귀속된다.
    // 선택 버튼(suggestions)이 달린 "목록(선택 단계)" 메시지는 아직 평가할 답변이
    // 아니라 작업구분을 고르는 중간 단계이므로 피드백을 숨긴다. 작업구분 선택 후
    // 나오는 실제 답변(suggestions 없음)에만 노출한다.
    let is_selection_step = msg
        .suggestions
        .as_ref()
        .is_some_and(|s| !s.is_empty());
    let show_feedback =
        is_assistant && msg.ui.is_none() && !is_selection_step && props.is_authenticated;
    let (feedback_buttons_html, feedback_comment_html) = if show_feedback {
        let message_id = msg.id.clone();
        let agent_id = msg.agent_id.clone().unwrap_or_default();
        let response_index = msg.current_response_page() as i64;
        let excerpt: String = msg.content.chars().take(500).collect();
        let current = msg.feedback;

        // 👍: 즉시 제출(이미 👍면 취소).
        let on_up = {
            let on_feedback = props.on_feedback.clone();
            let message_id = message_id.clone();
            let agent_id = agent_id.clone();
            let excerpt = excerpt.clone();
            let feedback_comment_open = feedback_comment_open.clone();
            Callback::from(move |_: MouseEvent| {
                feedback_comment_open.set(false);
                let active = current != Some(FeedbackRating::Up);
                on_feedback.emit(FeedbackSubmission {
                    message_id: message_id.clone(),
                    response_index,
                    agent_id: agent_id.clone(),
                    content_excerpt: excerpt.clone(),
                    rating: FeedbackRating::Up,
                    comment: None,
                    tags: Vec::new(),
                    active,
                });
            })
        };

        // 👎: 이미 👎면 취소, 아니면 사유/코멘트 입력창 토글(제출/건너뛰기 버튼으로 확정).
        let on_down = {
            let on_feedback = props.on_feedback.clone();
            let message_id = message_id.clone();
            let agent_id = agent_id.clone();
            let excerpt = excerpt.clone();
            let feedback_comment_open = feedback_comment_open.clone();
            let feedback_comment_value = feedback_comment_value.clone();
            let feedback_tags = feedback_tags.clone();
            Callback::from(move |_: MouseEvent| {
                if current == Some(FeedbackRating::Down) {
                    feedback_comment_open.set(false);
                    on_feedback.emit(FeedbackSubmission {
                        message_id: message_id.clone(),
                        response_index,
                        agent_id: agent_id.clone(),
                        content_excerpt: excerpt.clone(),
                        rating: FeedbackRating::Down,
                        comment: None,
                        tags: Vec::new(),
                        active: false,
                    });
                } else {
                    let open = !*feedback_comment_open;
                    feedback_comment_open.set(open);
                    if open {
                        feedback_comment_value.set(String::new());
                        feedback_tags.set(Vec::new());
                    }
                }
            })
        };

        let up_active = current == Some(FeedbackRating::Up);
        let down_active = current == Some(FeedbackRating::Down);
        let buttons = html! {
            <div class="feedback-inline">
                <span class="feedback-label">{ "이 답변 어땠나요?" }</span>
                <button
                    class={classes!("feedback-btn", "feedback-up", up_active.then_some("active"))}
                    onclick={on_up}
                    title="도움이 됐어요"
                >{ "👍" }</button>
                <button
                    class={classes!("feedback-btn", "feedback-down", down_active.then_some("active"))}
                    onclick={on_down}
                    title="아쉬웠어요"
                >{ "👎" }</button>
            </div>
        };

        let comment = if *feedback_comment_open {
            // 사유 칩(다중 선택 토글).
            let chips = FEEDBACK_REASON_TAGS
                .iter()
                .map(|tag| {
                    let tag = (*tag).to_string();
                    let selected = feedback_tags.contains(&tag);
                    let on_toggle = {
                        let feedback_tags = feedback_tags.clone();
                        let tag = tag.clone();
                        Callback::from(move |_: MouseEvent| {
                            let mut next = (*feedback_tags).clone();
                            if let Some(pos) = next.iter().position(|t| t == &tag) {
                                next.remove(pos);
                            } else {
                                next.push(tag.clone());
                            }
                            feedback_tags.set(next);
                        })
                    };
                    html! {
                        <button
                            type="button"
                            class={classes!("feedback-tag-chip", selected.then_some("active"))}
                            onclick={on_toggle}
                        >{ tag }</button>
                    }
                })
                .collect::<Html>();

            let on_input = {
                let feedback_comment_value = feedback_comment_value.clone();
                Callback::from(move |e: InputEvent| {
                    if let Some(target) = e.target() {
                        if let Ok(textarea) = target.dyn_into::<web_sys::HtmlTextAreaElement>() {
                            feedback_comment_value.set(textarea.value());
                        }
                    }
                })
            };
            let on_submit = {
                let on_feedback = props.on_feedback.clone();
                let message_id = message_id.clone();
                let agent_id = agent_id.clone();
                let excerpt = excerpt.clone();
                let feedback_comment_open = feedback_comment_open.clone();
                let feedback_comment_value = feedback_comment_value.clone();
                let feedback_tags = feedback_tags.clone();
                Callback::from(move |_: MouseEvent| {
                    let comment = (*feedback_comment_value).trim().to_string();
                    let comment = if comment.is_empty() { None } else { Some(comment) };
                    let tags = (*feedback_tags).clone();
                    feedback_comment_open.set(false);
                    on_feedback.emit(FeedbackSubmission {
                        message_id: message_id.clone(),
                        response_index,
                        agent_id: agent_id.clone(),
                        content_excerpt: excerpt.clone(),
                        rating: FeedbackRating::Down,
                        comment,
                        tags,
                        active: true,
                    });
                })
            };
            // 건너뛰기: 사유·코멘트 없이 👎만 기록(평가가 항상 집계되도록).
            let on_skip = {
                let on_feedback = props.on_feedback.clone();
                let message_id = message_id.clone();
                let agent_id = agent_id.clone();
                let excerpt = excerpt.clone();
                let feedback_comment_open = feedback_comment_open.clone();
                Callback::from(move |_: MouseEvent| {
                    feedback_comment_open.set(false);
                    on_feedback.emit(FeedbackSubmission {
                        message_id: message_id.clone(),
                        response_index,
                        agent_id: agent_id.clone(),
                        content_excerpt: excerpt.clone(),
                        rating: FeedbackRating::Down,
                        comment: None,
                        tags: Vec::new(),
                        active: true,
                    });
                })
            };
            html! {
                <div class="feedback-comment-box">
                    <div class="feedback-tag-chips">{ chips }</div>
                    <textarea
                        class="feedback-comment-input"
                        placeholder="어떤 점이 아쉬웠나요? (선택)"
                        value={(*feedback_comment_value).clone()}
                        oninput={on_input}
                    />
                    <div class="feedback-comment-actions">
                        <button class="feedback-comment-submit" onclick={on_submit}>{ "제출" }</button>
                        <button class="feedback-comment-cancel" onclick={on_skip}>{ "건너뛰기" }</button>
                    </div>
                </div>
            }
        } else {
            html! {}
        };

        (buttons, comment)
    } else {
        (html! {}, html! {})
    };

    // "다른 에이전트로 재시도" 드롭다운: 같은 질문을 사용자가 고른 에이전트로
    // 다시 보내 새 답변 버블을 추가한다. 자동 라우팅이 틀렸을 때의 인라인 교정 수단.
    let retry_agent_btn = if is_assistant && msg.ui.is_none() {
        let is_open = *retry_menu_open;
        let toggle = {
            let retry_menu_open = retry_menu_open.clone();
            Callback::from(move |_: MouseEvent| retry_menu_open.set(!*retry_menu_open))
        };
        let menu = if is_open {
            let items = AgentMeta::defaults()
                .into_iter()
                .map(|agent| {
                    let on_pick = {
                        let on_retry_with_agent = props.on_retry_with_agent.clone();
                        let retry_menu_open = retry_menu_open.clone();
                        let message_id = msg.id.clone();
                        let agent_id = agent.id.clone();
                        Callback::from(move |_: MouseEvent| {
                            retry_menu_open.set(false);
                            on_retry_with_agent.emit((message_id.clone(), agent_id.clone()));
                        })
                    };
                    html! {
                        <button
                            class="retry-agent-item"
                            onclick={on_pick}
                            disabled={props.regenerate_disabled}
                        >
                            <span class="retry-agent-name">{ agent.name.clone() }</span>
                            <span class="retry-agent-desc">{ agent.description.clone() }</span>
                        </button>
                    }
                })
                .collect::<Html>();
            html! { <div class="retry-agent-menu">{ items }</div> }
        } else {
            html! {}
        };

        html! {
            <div class="retry-agent-wrap">
                <button
                    class="retry-agent-btn"
                    onclick={toggle}
                    disabled={props.regenerate_disabled}
                    title="다른 에이전트로 재시도"
                >
                    { "🔀 다른 에이전트로 재시도" }
                </button>
                { menu }
            </div>
        }
    } else {
        html! {}
    };

    let download_btn = if has_table && is_rendered_markdown {
        let id = msg_id.clone();
        html! {
            <button class="download-csv-btn" onclick={Callback::from(move |_| downloadTableAsCSV(&id))}>
                { "📥 표 다운로드" }
            </button>
        }
    } else {
        html! {}
    };

    // "질문 수정": 사용자 질문 버블을 인라인 편집으로 전환하는 버튼. 답변 처리 중
    // (regenerate_disabled = is_loading)에는 비활성화한다.
    let edit_btn = if is_user && msg.ui.is_none() && !*is_editing {
        let is_editing = is_editing.clone();
        let edit_value = edit_value.clone();
        let content = msg.content.clone();
        let on_click = Callback::from(move |_: MouseEvent| {
            edit_value.set(content.clone());
            is_editing.set(true);
        });
        html! {
            <button
                class="edit-btn"
                onclick={on_click}
                disabled={props.regenerate_disabled}
                title="질문 수정"
            >
                { "✏️ 수정" }
            </button>
        }
    } else {
        html! {}
    };

    // 내 질문 ⭐ 즐겨찾기 토글(사용자 메시지에만). 누르면 현재 에이전트 즐겨찾기에 추가/해제.
    let favorite_btn = if is_user && msg.ui.is_none() && !*is_editing && props.can_favorite {
        let on_favorite_toggle = props.on_favorite_toggle.clone();
        let content = msg.content.clone();
        let on_click = Callback::from(move |_: MouseEvent| {
            on_favorite_toggle.emit(content.clone());
        });
        html! {
            <button
                class={classes!("message-fav-btn", props.is_favorited.then_some("active"))}
                onclick={on_click}
                title={ if props.is_favorited { "즐겨찾기 해제" } else { "즐겨찾기에 추가" } }
            >
                { if props.is_favorited { "⭐ 저장됨" } else { "☆ 즐겨찾기" } }
            </button>
        }
    } else {
        html! {}
    };

    // 인라인 에디터: 수정된 질문을 전송하면 이후 대화를 지우고 새 답변을 생성한다.
    let editor_html = {
        let on_input = {
            let edit_value = edit_value.clone();
            Callback::from(move |e: InputEvent| {
                if let Some(target) = e.target() {
                    if let Ok(textarea) = target.dyn_into::<web_sys::HtmlTextAreaElement>() {
                        edit_value.set(textarea.value());
                    }
                }
            })
        };
        let on_cancel: Callback<()> = {
            let is_editing = is_editing.clone();
            let edit_value = edit_value.clone();
            let original = msg.content.clone();
            Callback::from(move |_| {
                edit_value.set(original.clone());
                is_editing.set(false);
            })
        };
        let on_save: Callback<()> = {
            let on_edit_message = props.on_edit_message.clone();
            let is_editing = is_editing.clone();
            let edit_value = edit_value.clone();
            let msg_id = msg.id.clone();
            let disabled = props.regenerate_disabled;
            Callback::from(move |_| {
                if disabled {
                    return;
                }
                let value = (*edit_value).trim().to_string();
                if value.is_empty() {
                    return;
                }
                is_editing.set(false);
                on_edit_message.emit((msg_id.clone(), value));
            })
        };
        let on_keydown = {
            let on_save = on_save.clone();
            let on_cancel = on_cancel.clone();
            Callback::from(move |e: web_sys::KeyboardEvent| {
                if e.key() == "Enter" && (e.ctrl_key() || e.meta_key()) && !e.is_composing() {
                    e.prevent_default();
                    on_save.emit(());
                } else if e.key() == "Escape" {
                    on_cancel.emit(());
                }
            })
        };
        let can_save = !(*edit_value).trim().is_empty() && !props.regenerate_disabled;

        html! {
            <div class="message-bubble edit-bubble">
                <textarea
                    class="edit-textarea"
                    value={(*edit_value).clone()}
                    oninput={on_input}
                    onkeydown={on_keydown}
                />
                <div class="edit-actions">
                    <button
                        class="edit-cancel-btn"
                        onclick={on_cancel.reform(|_: MouseEvent| ())}
                    >
                        { "취소" }
                    </button>
                    <button
                        class="edit-save-btn"
                        onclick={on_save.reform(|_: MouseEvent| ())}
                        disabled={!can_save}
                    >
                        { "전송" }
                    </button>
                </div>
            </div>
        }
    };

    let pagination_html = if is_assistant {
        let previous_page = current_page.saturating_sub(1);
        let next_page = (current_page + 1).min(page_count.saturating_sub(1));
        let prev_message_id = msg.id.clone();
        let next_message_id = msg.id.clone();
        let on_prev = {
            let on_select_response_page = props.on_select_response_page.clone();
            Callback::from(move |_: MouseEvent| {
                on_select_response_page.emit((prev_message_id.clone(), previous_page));
            })
        };
        let on_next = {
            let on_select_response_page = props.on_select_response_page.clone();
            Callback::from(move |_: MouseEvent| {
                on_select_response_page.emit((next_message_id.clone(), next_page));
            })
        };

        html! {
            <div class="response-pagination">
                <button class="page-btn" onclick={on_prev} disabled={current_page == 0}>{ "<" }</button>
                <span class="page-indicator">{ format!("{} / {}", current_page + 1, page_count) }</span>
                <button class="page-btn" onclick={on_next} disabled={current_page + 1 >= page_count}>{ ">" }</button>
            </div>
        }
    } else {
        html! {}
    };

    let show_meta = (is_assistant && msg.ui.is_none()) || has_token_usage;
    let has_law_compare = msg
        .old_new_comparison
        .as_ref()
        .map(|comparison| !comparison.article_comparisons.is_empty())
        .unwrap_or(false);
    let law_compare_html = msg
        .old_new_comparison
        .as_ref()
        .filter(|comparison| !comparison.article_comparisons.is_empty())
        .map(render_law_comparison)
        .unwrap_or_else(|| html! {});
    // `visualize` 도구가 데이터를 반환했을 때만 차트 블록을 답변 아래에 렌더한다.
    let chart_html = msg.chart_data.as_ref().map_or_else(
        || html! {},
        |data| html! { <ChartBlock message_id={msg.id.clone()} data={data.clone()} /> },
    );

    let share_modal = if *is_share_modal_open {
        let close = {
            let is_share_modal_open = is_share_modal_open.clone();
            let share_error = share_error.clone();
            let share_notice = share_notice.clone();
            let email_submenu_open = email_submenu_open.clone();
            Callback::from(move |_: MouseEvent| {
                is_share_modal_open.set(false);
                share_error.set(None);
                share_notice.set(None);
                email_submenu_open.set(false);
            })
        };
        let on_copy = {
            let on_share_action = on_share_action.clone();
            Callback::from(move |_: MouseEvent| on_share_action.emit(ShareAction::CopyLink))
        };
        let on_email_toggle = {
            let email_submenu_open = email_submenu_open.clone();
            let share_notice = share_notice.clone();
            Callback::from(move |_: MouseEvent| {
                share_notice.set(None);
                email_submenu_open.set(!*email_submenu_open);
            })
        };
        let email_btn = |provider: EmailProvider| {
            let on_email_provider = on_email_provider.clone();
            Callback::from(move |_: MouseEvent| on_email_provider.emit(provider))
        };
        let on_kakao = {
            let on_share_action = on_share_action.clone();
            Callback::from(move |_: MouseEvent| on_share_action.emit(ShareAction::Kakao))
        };
        html! {
            <div class="modal-backdrop" onclick={close.clone()}>
                <div class="share-modal" onclick={Callback::from(|e: MouseEvent| e.stop_propagation())}>
                    <h3>{ "답변 공유" }</h3>
                    <div class="share-options">
                        <button class="share-option" onclick={on_copy} disabled={*share_loading}>
                            { if *share_copied { "✓ 링크 복사됨" } else { "🔗 링크 복사" } }
                        </button>
                        <button class="share-option" onclick={on_email_toggle} disabled={*share_loading}>
                            <span>{ "✉️ 이메일로 보내기" }</span>
                            <span class="share-option-caret">{ if *email_submenu_open { "▴" } else { "▾" } }</span>
                        </button>
                        if *email_submenu_open {
                            <div class="share-suboptions">
                                <button class="share-suboption" onclick={email_btn(EmailProvider::Gmail)} disabled={*share_loading}>
                                    <svg class="mail-ico-svg" viewBox="0 0 48 48" width="18" height="18">
                                        <path fill="#4caf50" d="M45,16.2l-5,2.75l-5,4.75L35,40h7c1.657,0,3-1.343,3-3V16.2z"/>
                                        <path fill="#1e88e5" d="M3,16.2l3.614,1.71L13,23.7V40H6c-1.657,0-3-1.343-3-3V16.2z"/>
                                        <polygon fill="#e53935" points="35,11.2 24,19.45 13,11.2 12,17 13,23.7 24,31.95 35,23.7 36,17"/>
                                        <path fill="#c62828" d="M3,12.298V16.2l10,7.5V11.2L9.876,8.859C9.132,8.301,8.228,8,7.298,8 C4.924,8,3,9.924,3,12.298z"/>
                                        <path fill="#fbc02d" d="M45,12.298V16.2l-10,7.5V11.2l3.124-2.341C38.868,8.301,39.772,8,40.702,8 C43.076,8,45,9.924,45,12.298z"/>
                                    </svg>
                                    <span>{ "Gmail" }</span>
                                </button>
                                <button class="share-suboption" onclick={email_btn(EmailProvider::Outlook)} disabled={*share_loading}>
                                    <span class="mail-ico mail-ico-outlook">{ "O" }</span>
                                    <span>{ "Outlook" }</span>
                                </button>
                                <button class="share-suboption" onclick={email_btn(EmailProvider::Naver)} disabled={*share_loading}>
                                    <span class="mail-ico mail-ico-naver">{ "N" }</span>
                                    <span>{ "네이버 메일" }</span>
                                </button>
                                <button class="share-suboption" onclick={email_btn(EmailProvider::Default)} disabled={*share_loading}>
                                    <svg class="mail-ico-svg mail-ico-muted" viewBox="0 0 24 24" width="18" height="18"
                                         fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                        <rect x="3" y="5" width="18" height="14" rx="2"></rect>
                                        <path d="m3 7 9 6 9-6"></path>
                                    </svg>
                                    <span>{ "기본 메일 앱" }</span>
                                </button>
                            </div>
                        }
                        <button class="share-option" onclick={on_kakao} disabled={*share_loading}>
                            { "💬 카카오로 보내기" }
                        </button>
                    </div>
                    if *share_loading {
                        <div class="share-status">{ "공유 링크 생성 중..." }</div>
                    }
                    if let Some(note) = (*share_notice).clone() {
                        <div class="share-status">{ note }</div>
                    }
                    if let Some(err) = (*share_error).clone() {
                        <div class="share-error">{ err }</div>
                    }
                    <div class="modal-actions">
                        <button class="modal-btn cancel" onclick={close}>{ "닫기" }</button>
                    </div>
                </div>
            </div>
        }
    } else {
        html! {}
    };

    html! {
        <div class={classes!("message-row", role_class, has_law_compare.then_some("has-law-compare"))}>
            <div class="message-avatar">{ avatar }</div>
            <div>
                if is_assistant && !msg.process_steps.is_empty() {
                    <ProcessSteps steps={msg.process_steps.clone()} is_live=false />
                }
                if let Some(note) = msg.routing_note.as_ref() {
                    <div class="routing-note">{ note }</div>
                }
                if is_user && *is_editing {
                    { editor_html }
                } else {
                    { content_html }
                    { law_compare_html }
                    { chart_html }
                }
                <div class="message-actions">
                    { copy_btn }
                    { share_btn }
                    { edit_btn }
                    { favorite_btn }
                    { regenerate_btn }
                    { retry_agent_btn }
                    { download_btn }
                    { feedback_buttons_html }
                </div>
                { feedback_comment_html }
                if show_meta {
                    <div class="message-meta">
                        { pagination_html }
                        { token_html }
                    </div>
                }
            </div>
            { share_modal }
        </div>
    }
}

/// 캐시된(또는 방금 생성된) share_url로 실제 공유 동작을 수행한다.
fn perform_share_action(
    action: ShareAction,
    url: String,
    content: &str,
    share_copied: &UseStateHandle<bool>,
    share_error: &UseStateHandle<Option<String>>,
) {
    match action {
        ShareAction::CopyLink => {
            let share_copied = share_copied.clone();
            spawn_local(async move {
                if copy_to_clipboard(url).await {
                    share_copied.set(true);
                    gloo_timers::future::TimeoutFuture::new(2_000).await;
                    share_copied.set(false);
                }
            });
        }
        ShareAction::Kakao => {
            let description: String = content.chars().take(80).collect();
            if !share_kakao(&url, &description) {
                share_error.set(Some("카카오 공유를 사용할 수 없습니다.".to_string()));
            }
        }
    }
}

/// 링크 중심 이메일 본문을 만든다. 전체 답변(표 포함)은 공유 링크로 확인하도록 안내하고,
/// 미리보기는 표/제목 마크다운 전까지의 앞부분 산문만 깔끔하게 보여준다(`**`·`#` 제거).
fn build_email_body(url: &str, content: &str) -> String {
    const PREVIEW_LEN: usize = 200;
    // 제목(`#`)·표(`|`)·구분선(`---`)이 시작되면 미리보기를 끊어, 원본 마크다운이 새지 않게 한다.
    let mut prose = String::new();
    for line in content.lines() {
        let t = line.trim_start();
        if t.starts_with('#') || t.starts_with('|') || t.starts_with("---") {
            break;
        }
        if !prose.is_empty() {
            prose.push('\n');
        }
        prose.push_str(line);
    }
    // 앞부분이 곧바로 표/제목으로 시작하면(산문 없음) 전체 내용에서 잘라 쓴다.
    let source = if prose.trim().is_empty() { content } else { prose.as_str() };
    let cleaned = source.replace("**", "");
    let cleaned = cleaned.trim();
    let truncated = cleaned.chars().count() > PREVIEW_LEN;
    let mut preview: String = cleaned.chars().take(PREVIEW_LEN).collect();
    preview = preview.trim().to_string();
    if truncated {
        preview.push('…');
    }
    format!(
        "EHS AI 답변을 공유합니다.\n아래 링크에서 표를 포함한 전체 내용을 확인하세요.\n\n{url}\n\n[미리보기]\n{preview}"
    )
}

/// 선택한 이메일 서비스로 공유 메일 작성 화면을 연다. 서버 발송 없이 사용자의
/// 메일 클라이언트/웹메일 작성창을 띄운다(제목·본문은 URL 인코딩해 prefill).
///
/// `preopened`는 팝업 차단 회피용으로 클릭 시점에 미리 열어둔 빈 탭(웹메일에만 사용).
fn open_email_provider(
    provider: EmailProvider,
    url: &str,
    content: &str,
    preopened: Option<web_sys::Window>,
    share_notice: &UseStateHandle<Option<String>>,
) {
    // 링크 중심 본문: 전체 답변(표 포함)은 URL 길이 제한상 본문에 담을 수 없으므로
    // 공유 링크로 안내하고, 짧은 미리보기만 덧붙인다.
    let body_text = build_email_body(url, content);
    let subject = String::from(js_sys::encode_uri_component("EHS AI 답변 공유"));
    let body = String::from(js_sys::encode_uri_component(&body_text));

    match provider {
        EmailProvider::Default => {
            // OS 기본 메일 앱(데스크톱 Outlook 등). mailto는 프로토콜 핸들러라 팝업 차단 무관.
            let mailto = format!("mailto:?subject={subject}&body={body}");
            if let Some(window) = web_sys::window() {
                let _ = window.open_with_url_and_target(&mailto, "_blank");
            }
        }
        EmailProvider::Gmail => {
            let compose =
                format!("https://mail.google.com/mail/?view=cm&fs=1&su={subject}&body={body}");
            navigate_or_open(preopened, &compose);
        }
        EmailProvider::Outlook => {
            let compose = format!(
                "https://outlook.office.com/mail/deeplink/compose?subject={subject}&body={body}"
            );
            navigate_or_open(preopened, &compose);
        }
        EmailProvider::Naver => {
            // 네이버 메일은 본문 prefill URL이 없어, 링크 중심 본문을 클립보드에 복사하고
            // 새 메일 작성 화면을 연 뒤 사용자가 붙여넣도록 안내한다.
            let to_copy = body_text.clone();
            spawn_local(async move {
                let _ = copy_to_clipboard(to_copy).await;
            });
            navigate_or_open(preopened, "https://mail.naver.com/");
            share_notice.set(Some(
                "본문이 클립보드에 복사되었습니다. 네이버 메일에서 '메일쓰기' 후 붙여넣어 주세요.".to_string(),
            ));
        }
    }
}

/// 미리 열어둔 탭이 있으면 그 탭을 작성 URL로 이동시키고(팝업 차단 회피),
/// 없으면(차단됐거나 미리 열지 못한 경우) 새 탭으로 직접 연다.
fn navigate_or_open(preopened: Option<web_sys::Window>, url: &str) {
    if let Some(w) = preopened {
        let _ = w.location().set_href(url);
    } else if let Some(window) = web_sys::window() {
        let _ = window.open_with_url_and_target(url, "_blank");
    }
}

async fn copy_to_clipboard(content: String) -> bool {
    if let Some(window) = web_sys::window() {
        let clipboard = window.navigator().clipboard();
        if wasm_bindgen_futures::JsFuture::from(clipboard.write_text(&content))
            .await
            .is_ok()
        {
            return true;
        }
    }

    fallback_copy_to_clipboard(&content)
}

fn fallback_copy_to_clipboard(content: &str) -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let Some(document) = window.document() else {
        return false;
    };
    let Some(body) = document.body() else {
        return false;
    };
    let Ok(element) = document.create_element("textarea") else {
        return false;
    };
    let Ok(textarea) = element.dyn_into::<web_sys::HtmlTextAreaElement>() else {
        return false;
    };

    textarea.set_value(content);
    let style = textarea.style();
    let _ = style.set_property("position", "fixed");
    let _ = style.set_property("top", "-1000px");
    let _ = style.set_property("left", "-1000px");
    let _ = style.set_property("opacity", "0");

    if body.append_child(&textarea).is_err() {
        return false;
    }

    textarea.select();
    let copied = document
        .dyn_into::<web_sys::HtmlDocument>()
        .ok()
        .and_then(|document| document.exec_command("copy").ok())
        .unwrap_or(false);
    let _ = body.remove_child(&textarea);
    copied
}

fn board_post_route_flow(content: &str) -> Option<BoardPostFlow> {
    match content.trim() {
        "게시글 등록 요청입니다. 등록할 게시판을 선택해주세요." => {
            Some(BoardPostFlow::Create)
        }
        "게시글 수정 요청입니다. 수정할 게시판을 선택해주세요." => {
            Some(BoardPostFlow::Edit)
        }
        "게시글 삭제 요청입니다. 삭제할 게시판을 선택해주세요." => {
            Some(BoardPostFlow::Delete)
        }
        _ => None,
    }
}

fn render_law_comparison(comparison: &OldNewComparison) -> Html {
    html! {
        <div class="law-compare-shell">
            <div class="law-compare-grid law-compare-header">
                <div class="law-compare-nav">
                    <span class="law-compare-nav-icon">{ "‹" }</span>
                    <span>{ "이전" }</span>
                </div>
                <div class="law-compare-title-cell">
                    <div class="law-compare-law-name">{ comparison.old_law.law_name.as_deref().unwrap_or(&comparison.law_name) }</div>
                    <div class="law-compare-law-meta">{ law_info_text(&comparison.old_law) }</div>
                </div>
                <div class="law-compare-title-cell">
                    <div class="law-compare-law-name">{ comparison.new_law.law_name.as_deref().unwrap_or(&comparison.law_name) }</div>
                    <div class="law-compare-law-meta">{ law_info_text(&comparison.new_law) }</div>
                </div>
                <div class="law-compare-nav">
                    <span class="law-compare-nav-icon">{ "›" }</span>
                    <span>{ "다음" }</span>
                </div>
            </div>
            { for comparison.article_comparisons.iter().map(render_law_article_pair) }
        </div>
    }
}

fn render_law_article_pair(article: &OldNewArticleComparison) -> Html {
    let label = law_article_label(article);
    let old_label = label.clone();
    let old_text = article.old_text.as_deref().unwrap_or("");
    let new_text = article.new_text.as_deref().unwrap_or("");

    html! {
        <div class="law-compare-grid law-compare-row">
            <div class="law-compare-row-side"></div>
            <div class="law-compare-article">
                <div class="law-compare-article-title">{ old_label }</div>
                <div class="law-compare-article-text old">
                    { render_highlighted_law_text(old_text, new_text, "law-diff-old") }
                </div>
            </div>
            <div class="law-compare-article">
                <div class="law-compare-article-title">{ label }</div>
                <div class="law-compare-article-text new">
                    { render_highlighted_law_text(new_text, old_text, "law-diff-new") }
                </div>
            </div>
            <div class="law-compare-row-side"></div>
        </div>
    }
}

fn law_article_label(article: &OldNewArticleComparison) -> String {
    match (article.article_no.as_deref(), article.title.as_deref()) {
        (Some(no), Some(title)) => format!("{no}({title})"),
        (Some(no), None) => no.to_string(),
        (None, Some(title)) => title.to_string(),
        (None, None) => "변경 조문".to_string(),
    }
}

fn law_info_text(info: &OldNewLawInfo) -> String {
    let enforcement = info
        .enforcement_date
        .as_deref()
        .map(format_law_date)
        .map(|date| format!("[시행 {date}]"));
    let promulgation_date = info.promulgation_date.as_deref().map(format_law_date);
    let mut promulgation_parts = Vec::new();

    if let Some(number) = info.promulgation_number.as_deref() {
        promulgation_parts.push(format!("법률 {number}"));
    }
    if let Some(date) = promulgation_date {
        promulgation_parts.push(date);
    }
    if let Some(amendment_type) = info.amendment_type.as_deref() {
        promulgation_parts.push(amendment_type.to_string());
    }

    let promulgation = if promulgation_parts.is_empty() {
        None
    } else {
        Some(format!("[{}]", promulgation_parts.join(", ")))
    };

    [enforcement, promulgation]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_law_date(value: &str) -> String {
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() == 3 {
        let month = parts[1].trim_start_matches('0');
        let day = parts[2].trim_start_matches('0');
        format!("{}. {}. {}.", parts[0], month, day)
    } else {
        value.to_string()
    }
}

fn render_highlighted_law_text(text: &str, other: &str, highlight_class: &'static str) -> Html {
    if text.is_empty() {
        return html! { <span class="law-empty-text">{ "내용 없음" }</span> };
    }

    if other.is_empty() {
        return html! { <>{ text }</> };
    }

    let (prefix, changed, suffix) = changed_segment(text, other);
    let changed_html = if changed.is_empty() {
        html! {}
    } else {
        html! { <span class={highlight_class}>{ changed }</span> }
    };

    html! {
        <>
            { prefix }
            { changed_html }
            { suffix }
        </>
    }
}

fn changed_segment(text: &str, other: &str) -> (String, String, String) {
    let chars: Vec<char> = text.chars().collect();
    let other_chars: Vec<char> = other.chars().collect();
    let mut prefix_len = 0usize;
    let min_len = chars.len().min(other_chars.len());

    while prefix_len < min_len && chars[prefix_len] == other_chars[prefix_len] {
        prefix_len += 1;
    }

    let mut text_end = chars.len();
    let mut other_end = other_chars.len();
    while text_end > prefix_len
        && other_end > prefix_len
        && chars[text_end - 1] == other_chars[other_end - 1]
    {
        text_end -= 1;
        other_end -= 1;
    }

    (
        chars[..prefix_len].iter().collect(),
        chars[prefix_len..text_end].iter().collect(),
        chars[text_end..].iter().collect(),
    )
}

#[function_component(TypingIndicator)]
pub fn typing_indicator() -> Html {
    html! {
        <div class="message-row assistant">
            <div class="message-avatar">{ "AI" }</div>
            <div class="typing-indicator">
                <div class="typing-dot"></div>
                <div class="typing-dot"></div>
                <div class="typing-dot"></div>
            </div>
        </div>
    }
}

#[derive(Properties, PartialEq)]
pub struct ProcessStepsProps {
    pub steps: Vec<ProcessStep>,
    /// `true` while the answer is still streaming (default expanded look may differ).
    #[prop_or_default]
    pub is_live: bool,
}

/// Collapsible "답변 과정 보기" panel showing the live/captured processing steps.
#[function_component(ProcessSteps)]
pub fn process_steps(props: &ProcessStepsProps) -> Html {
    let expanded = use_state(|| false);

    let on_toggle = {
        let expanded = expanded.clone();
        Callback::from(move |_| expanded.set(!*expanded))
    };

    let arrow = if *expanded { "▲" } else { "▼" };

    let steps_html = props.steps.iter().map(|step| {
        let status_class = match step.status {
            StepStatus::Pending => "pending",
            StepStatus::Active => "active",
            StepStatus::Done => "done",
        };
        html! {
            <div class={classes!("timeline-step", status_class)}>
                <div class="timeline-marker">
                    <span class="timeline-dot"></span>
                </div>
                <div class="timeline-content">
                    <span class="timeline-label">{ &step.label }</span>
                    if let Some(detail) = &step.detail {
                        <span class="timeline-detail">{ detail }</span>
                    }
                </div>
            </div>
        }
    }).collect::<Html>();

    html! {
        <div class="process-steps">
            <button type="button" class="process-toggle" onclick={on_toggle}>
                { format!("답변 과정 보기 {arrow}") }
            </button>
            if *expanded {
                <div class="process-timeline">
                    { steps_html }
                </div>
            }
        </div>
    }
}
