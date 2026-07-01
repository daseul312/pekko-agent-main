//! 개선 답변 비교 페이지. 이전 답변 vs 개선(재생성) 답변을 좌우로 보여준다.
//! `main()`이 해시 경로로 분기해 렌더한다:
//!  - `#/admin/resolution/{key}` → admin=true: 마운트 시 재생성(resolve) 후 검토, "알림 보내기".
//!  - `#/resolution/{key}` → admin=false: 발송된 조치를 사용자가 확인, 하단 "정확/부정확" 재확인.

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;

use crate::api;
use crate::components::chart::ChartBlock;
use crate::markdown;
use crate::types::{ChartData, FeedbackStatsError, ResolutionView as ResolutionData};

#[derive(Properties, PartialEq)]
pub struct ResolutionViewProps {
    pub feedback_key: String,
    pub admin: bool,
}

#[derive(Clone, PartialEq)]
enum ViewState {
    Loading,
    Loaded(ResolutionData),
    Error(FeedbackStatsError),
}

/// 답변 본문(마크다운) + 선택적 차트를 한 칼럼으로 렌더.
fn answer_panel(title: &str, content: &str, chart_json: &Option<String>, canvas_id: &str) -> Html {
    let rendered = markdown::markdown_to_html(content);
    let body = Html::from_html_unchecked(AttrValue::from(format!(
        "<div class=\"message-bubble md-content\">{}</div>",
        rendered
    )));
    let chart = chart_json
        .as_ref()
        .and_then(|j| serde_json::from_str::<ChartData>(j).ok())
        .map(|data| html! { <ChartBlock message_id={canvas_id.to_string()} data={data} /> })
        .unwrap_or_default();
    html! {
        <div class="resolution-col">
            <div class="resolution-col-title">{ title }</div>
            { body }
            { chart }
        </div>
    }
}

/// 로딩 중 실제 비교 레이아웃(질문 바 + 2단 답변) 형태의 스켈레톤을 렌더.
/// `title`/`sub`는 상단 상태 문구, 나머지는 shimmer 자리 표시.
fn loading_skeleton(title: &str, sub: Option<&str>) -> Html {
    // 각 칼럼의 줄 길이 패턴(true = 짧은 줄).
    let lines = |pattern: &[bool]| -> Html {
        pattern
            .iter()
            .map(|&short| html! { <div class={classes!("sk-line", short.then_some("short"))}></div> })
            .collect::<Html>()
    };
    let sub_html = sub
        .map(|s| html! { <div class="resolution-loading-sub">{ s }</div> })
        .unwrap_or_default();
    html! {
        <div class="resolution-skeleton">
            <div class="resolution-skeleton-status">
                <div class="resolution-spinner"></div>
                <div class="resolution-skeleton-text">
                    <div class="resolution-loading-title">{ title }</div>
                    { sub_html }
                </div>
            </div>
            <div class="sk-question"></div>
            <div class="sk-cols">
                <div class="sk-col">
                    <div class="sk-col-title"></div>
                    { lines(&[false, false, true, false, true]) }
                </div>
                <div class="sk-col">
                    <div class="sk-col-title"></div>
                    { lines(&[false, true, false, false, true]) }
                </div>
            </div>
        </div>
    }
}

#[function_component(ResolutionView)]
pub fn resolution_view(props: &ResolutionViewProps) -> Html {
    let state = use_state(|| ViewState::Loading);
    // 사용자 재확인: 부정확 코멘트 박스 펼침 + 값, 제출 완료 메시지.
    let reverify_open = use_state(|| false);
    let reverify_value = use_state(String::new);
    let reverify_done = use_state(|| Option::<String>::None);

    // 마운트 시 데이터 로드(admin=재생성, user=조회).
    {
        let state = state.clone();
        let feedback_key = props.feedback_key.clone();
        let admin = props.admin;
        use_effect_with((), move |_| {
            spawn_local(async move {
                let result = if admin {
                    api::resolve_feedback(&feedback_key).await
                } else {
                    api::fetch_resolution(&feedback_key).await
                };
                match result {
                    Ok(d) => state.set(ViewState::Loaded(d)),
                    Err(e) => state.set(ViewState::Error(e)),
                }
            });
            || ()
        });
    }

    let body = match &*state {
        ViewState::Loading => {
            if props.admin {
                loading_skeleton(
                    "개선 답변을 생성하고 있어요",
                    Some("이전 답변과 비교할 새 답변을 만드는 중입니다. 수 초 정도 걸려요."),
                )
            } else {
                loading_skeleton("불러오는 중...", None)
            }
        }
        ViewState::Error(FeedbackStatsError::Unauthorized) => {
            let key = props.feedback_key.clone();
            let admin = props.admin;
            let on_login = Callback::from(move |_: MouseEvent| {
                if let Some(window) = web_sys::window() {
                    let next = if admin {
                        format!("/#/admin/resolution/{key}")
                    } else {
                        format!("/#/resolution/{key}")
                    };
                    let encoded = String::from(js_sys::encode_uri_component(&next));
                    let _ = window.location().set_href(&format!("/?next={encoded}"));
                }
            });
            html! {
                <div class="admin-fb-status">
                    <p>{ "이 페이지를 보려면 로그인이 필요합니다." }</p>
                    <button class="modal-btn" onclick={on_login}>{ "로그인하기" }</button>
                </div>
            }
        }
        ViewState::Error(FeedbackStatsError::Forbidden) => html! {
            <div class="admin-fb-status">{ "이 페이지에 접근할 권한이 없습니다." }</div>
        },
        ViewState::Error(FeedbackStatsError::Other(msg)) => html! {
            <div class="admin-fb-status">{ format!("오류가 발생했습니다: {msg}") }</div>
        },
        ViewState::Loaded(d) => {
            let cols = html! {
                <div class="resolution-cols">
                    { answer_panel("이전 답변", &d.old_answer, &d.old_chart, "resolution-old") }
                    { answer_panel("개선된 답변", &d.new_answer, &d.new_chart, "resolution-new") }
                </div>
            };

            // 하단 액션.
            let actions = if props.admin {
                // 관리자: 알림 보내기 / 다시 생성.
                let key = props.feedback_key.clone();
                let on_send = {
                    let key = key.clone();
                    Callback::from(move |_: MouseEvent| {
                        let key = key.clone();
                        spawn_local(async move {
                            if api::send_resolution(&key).await.is_ok() {
                                if let Some(window) = web_sys::window() {
                                    let loc = window.location();
                                    let _ = loc.set_hash("#/admin/feedback");
                                    let _ = loc.reload();
                                }
                            }
                        });
                    })
                };
                let on_regen = {
                    let key = key.clone();
                    let state = state.clone();
                    Callback::from(move |_: MouseEvent| {
                        let key = key.clone();
                        let state = state.clone();
                        state.set(ViewState::Loading);
                        spawn_local(async move {
                            match api::resolve_feedback(&key).await {
                                Ok(d) => state.set(ViewState::Loaded(d)),
                                Err(e) => state.set(ViewState::Error(e)),
                            }
                        });
                    })
                };
                let sent = d.status == "sent";
                html! {
                    <div class="resolution-actions">
                        if sent {
                            <span class="resolution-note">{ "이미 사용자에게 알림이 발송된 조치입니다." }</span>
                        }
                        <button class="resolution-send-btn" onclick={on_send}>
                            { if sent { "다시 알림 보내기" } else { "📨 사용자에게 알림 보내기" } }
                        </button>
                        <button class="resolution-regen-btn" onclick={on_regen}>{ "🔄 다시 생성" }</button>
                    </div>
                }
            } else {
                // 사용자: 정확/부정확 재확인.
                if let Some(done_msg) = (*reverify_done).clone() {
                    html! { <div class="resolution-thanks">{ done_msg }</div> }
                } else if !d.reverify_result.is_empty() {
                    let label = if d.reverify_result == "accurate" { "정확함으로 확인됨" } else { "부정확으로 재피드백됨" };
                    html! { <div class="resolution-thanks">{ format!("이미 재확인하셨습니다: {label}") }</div> }
                } else {
                    let key = props.feedback_key.clone();
                    // 정확.
                    let on_accurate = {
                        let key = key.clone();
                        let reverify_done = reverify_done.clone();
                        Callback::from(move |_: MouseEvent| {
                            let key = key.clone();
                            let reverify_done = reverify_done.clone();
                            spawn_local(async move {
                                let _ = api::submit_reverify(&key, "accurate", None).await;
                                reverify_done.set(Some("소중한 확인 감사합니다! 정확한 답변으로 기록되었습니다.".to_string()));
                            });
                        })
                    };
                    // 부정확: 코멘트 박스 토글.
                    let on_inaccurate_open = {
                        let reverify_open = reverify_open.clone();
                        Callback::from(move |_: MouseEvent| reverify_open.set(true))
                    };
                    let comment_box = if *reverify_open {
                        let on_input = {
                            let reverify_value = reverify_value.clone();
                            Callback::from(move |e: InputEvent| {
                                if let Some(t) = e.target() {
                                    if let Ok(ta) = t.dyn_into::<web_sys::HtmlTextAreaElement>() {
                                        reverify_value.set(ta.value());
                                    }
                                }
                            })
                        };
                        let on_submit = {
                            let key = key.clone();
                            let reverify_value = reverify_value.clone();
                            let reverify_done = reverify_done.clone();
                            Callback::from(move |_: MouseEvent| {
                                let key = key.clone();
                                let comment = (*reverify_value).trim().to_string();
                                let reverify_done = reverify_done.clone();
                                spawn_local(async move {
                                    let c = if comment.is_empty() { None } else { Some(comment.as_str()) };
                                    let _ = api::submit_reverify(&key, "inaccurate", c).await;
                                    reverify_done.set(Some("재피드백이 접수되었습니다. 추가 개선에 반영하겠습니다.".to_string()));
                                });
                            })
                        };
                        html! {
                            <div class="resolution-reverify-box">
                                <textarea
                                    class="feedback-comment-input"
                                    placeholder="어떤 점이 여전히 부정확한가요? (선택)"
                                    value={(*reverify_value).clone()}
                                    oninput={on_input}
                                />
                                <button class="feedback-comment-submit" onclick={on_submit}>{ "제출" }</button>
                            </div>
                        }
                    } else {
                        html! {}
                    };
                    html! {
                        <div class="resolution-reverify">
                            <div class="resolution-reverify-q">{ "이 답변이 정확한가요?" }</div>
                            <div class="resolution-reverify-btns">
                                <button class="resolution-accurate" onclick={on_accurate}>{ "👍 정확해요" }</button>
                                <button class="resolution-inaccurate" onclick={on_inaccurate_open}>{ "👎 아직 부정확해요" }</button>
                            </div>
                            { comment_box }
                        </div>
                    }
                }
            };

            html! {
                <>
                    <div class="resolution-question">
                        <span class="resolution-question-label">{ "질문" }</span>
                        <span>{ &d.question }</span>
                    </div>
                    { cols }
                    { actions }
                </>
            }
        }
    };

    // 뒤로: admin이면 통계로, 사용자면 채팅으로.
    let admin = props.admin;
    let on_back = Callback::from(move |_: MouseEvent| {
        if let Some(window) = web_sys::window() {
            if admin {
                let loc = window.location();
                let _ = loc.set_hash("#/admin/feedback");
                let _ = loc.reload();
            } else {
                let _ = window.location().set_href("/");
            }
        }
    });

    html! {
        <div class="admin-fb-root">
            <header class="admin-fb-header">
                <button class="admin-fb-back" onclick={on_back} title="돌아가기">
                    <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
                        <path d="m15 18-6-6 6-6"></path>
                    </svg>
                </button>
                <span class="admin-fb-logo">{ "EHS AI" }</span>
                <span class="admin-fb-subtitle">{ "개선 답변 비교" }</span>
            </header>
            <main class="admin-fb-main">
                { body }
            </main>
        </div>
    }
}
