use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;

use crate::components::board_post::{BoardPostFlow, BoardPostMenuForm};
use crate::markdown;
use crate::types::{
    BoardPostCompletion, BoardPostTarget, ChatMessage, ChatMessageUi, MessageRole,
    OldNewArticleComparison, OldNewComparison, OldNewLawInfo, ProcessStep, StepStatus,
};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window)]
    fn downloadTableAsCSV(message_id: &str);
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
    pub on_select_response_page: Callback<(String, usize)>,
    pub on_board_post_target_selected: Callback<BoardPostTarget>,
    pub on_board_post_complete: Callback<BoardPostCompletion>,
    pub regenerate_disabled: bool,
    #[prop_or_default]
    pub token: Option<String>,
}

#[function_component(MessageBubble)]
pub fn message_bubble(props: &MessageProps) -> Html {
    let msg = &props.message;
    let copy_status = use_state(|| CopyStatus::Idle);
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

    html! {
        <div class={classes!("message-row", role_class, has_law_compare.then_some("has-law-compare"))}>
            <div class="message-avatar">{ avatar }</div>
            <div>
                if is_assistant && !msg.process_steps.is_empty() {
                    <ProcessSteps steps={msg.process_steps.clone()} is_live=false />
                }
                { content_html }
                { law_compare_html }
                <div class="message-actions">
                    { copy_btn }
                    { regenerate_btn }
                    { download_btn }
                </div>
                if show_meta {
                    <div class="message-meta">
                        { pagination_html }
                        { token_html }
                    </div>
                }
            </div>
        </div>
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
