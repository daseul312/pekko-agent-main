//! 읽기전용 대화 뷰어. 피드백 통계 페이지의 "해당 채팅 바로가기"가 이 페이지로 이동한다.
//! `main()`이 `/#/admin/session/{id}` 경로를 감지해 렌더하며, 마운트 시 서버에서 해당
//! 세션의 전체 대화를 조회해(다른 직원 세션도 서버 저장이라 열람 가능) 보여준다.

use uuid::Uuid;
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;

use crate::api;
use crate::components::chart::ChartBlock;
use crate::markdown;
use crate::types::{ChartData, HistoryMessage};

#[derive(Properties, PartialEq)]
pub struct SessionViewProps {
    pub session_id: String,
}

#[derive(Clone)]
enum ViewState {
    Loading,
    Loaded(Vec<HistoryMessage>),
    Error(String),
}

fn render_message(idx: usize, msg: &HistoryMessage) -> Html {
    let is_user = msg.role == "User";
    let role_class = if is_user { "user" } else { "assistant" };
    let avatar = if is_user { "나" } else { "AI" };
    // 사용자 메시지는 평문, 그 외(답변)는 채팅과 동일한 마크다운 렌더를 재사용한다.
    let content = if is_user {
        html! { <div class="message-bubble">{ &msg.content }</div> }
    } else {
        let rendered = markdown::markdown_to_html(&msg.content);
        Html::from_html_unchecked(AttrValue::from(format!(
            "<div class=\"message-bubble md-content\">{}</div>",
            rendered
        )))
    };
    // 차트 JSON이 있으면 파싱해 채팅과 동일한 ChartBlock으로 답변 아래에 그린다.
    let chart = msg
        .chart_data
        .as_ref()
        .and_then(|json| serde_json::from_str::<ChartData>(json).ok())
        .map(|data| html! { <ChartBlock message_id={format!("session-{idx}")} data={data} /> })
        .unwrap_or_default();
    html! {
        <div class={classes!("message-row", role_class)}>
            <div class="message-avatar">{ avatar }</div>
            <div>
                { content }
                { chart }
            </div>
        </div>
    }
}

#[function_component(SessionView)]
pub fn session_view(props: &SessionViewProps) -> Html {
    let state = use_state(|| ViewState::Loading);

    {
        let state = state.clone();
        let session_id = props.session_id.clone();
        use_effect_with((), move |_| {
            spawn_local(async move {
                match Uuid::parse_str(&session_id) {
                    Ok(sid) => match api::fetch_session_history(sid).await {
                        Ok(msgs) => state.set(ViewState::Loaded(msgs)),
                        Err(e) => state.set(ViewState::Error(e)),
                    },
                    Err(_) => state.set(ViewState::Error("잘못된 세션 주소입니다.".to_string())),
                }
            });
            || ()
        });
    }

    let body = match &*state {
        ViewState::Loading => html! { <div class="admin-fb-status">{ "불러오는 중..." }</div> },
        ViewState::Loaded(msgs) if msgs.is_empty() => html! {
            <div class="admin-fb-status">{ "대화 내용을 찾을 수 없습니다." }</div>
        },
        ViewState::Loaded(msgs) => html! {
            <div class="session-view-chat">
                { for msgs.iter().enumerate().map(|(i, m)| render_message(i, m)) }
            </div>
        },
        ViewState::Error(msg) => html! {
            <div class="admin-fb-status">{ format!("오류가 발생했습니다: {msg}") }</div>
        },
    };

    // 피드백 통계로 돌아가기.
    let on_back = Callback::from(move |_: MouseEvent| {
        if let Some(window) = web_sys::window() {
            let loc = window.location();
            let _ = loc.set_hash("#/admin/feedback");
            let _ = loc.reload();
        }
    });

    html! {
        <div class="admin-fb-root">
            <header class="admin-fb-header">
                <button class="admin-fb-back" onclick={on_back} title="피드백 통계로 돌아가기">
                    <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
                        <path d="m15 18-6-6 6-6"></path>
                    </svg>
                </button>
                <span class="admin-fb-logo">{ "EHS AI" }</span>
                <span class="admin-fb-subtitle">{ "피드백이 남겨진 대화" }</span>
            </header>
            <main class="admin-fb-main">
                { body }
            </main>
        </div>
    }
}
