//! 공유 링크 열람 페이지. 라우터가 없으므로 `main()`이 `/share/{id}` 경로를
//! 감지해 `App` 대신 이 컴포넌트를 렌더한다(엔트리 분기). 마운트 시 백엔드에서
//! 공유된 답변 1개를 조회해 읽기 전용으로 보여준다.

use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;

use crate::api;
use crate::components::chart::ChartBlock;
use crate::markdown;
use crate::types::{ChartData, ShareFetchError, SharedAnswer};

#[derive(Properties, PartialEq)]
pub struct ShareViewProps {
    pub share_id: String,
}

#[derive(Clone, PartialEq)]
enum ViewState {
    Loading,
    Loaded(SharedAnswer),
    Error(ShareFetchError),
}

#[function_component(ShareView)]
pub fn share_view(props: &ShareViewProps) -> Html {
    let state = use_state(|| ViewState::Loading);

    {
        let state = state.clone();
        let share_id = props.share_id.clone();
        use_effect_with((), move |_| {
            spawn_local(async move {
                match api::fetch_shared(&share_id).await {
                    Ok(answer) => state.set(ViewState::Loaded(answer)),
                    Err(e) => state.set(ViewState::Error(e)),
                }
            });
            || ()
        });
    }

    let body = match &*state {
        ViewState::Loading => html! {
            <div class="share-view-status">{ "불러오는 중..." }</div>
        },
        ViewState::Loaded(answer) => {
            // 메시지 버블과 동일한 마크다운 렌더 유틸을 재사용한다.
            let rendered = markdown::markdown_to_html(&answer.content);
            let content = Html::from_html_unchecked(AttrValue::from(format!(
                "<div class=\"message-bubble md-content\">{}</div>",
                rendered
            )));
            // 차트 JSON이 있으면 파싱해 채팅과 동일한 ChartBlock으로 그린다.
            let chart = answer
                .chart_data
                .as_ref()
                .and_then(|json| serde_json::from_str::<ChartData>(json).ok())
                .map(|data| html! { <ChartBlock message_id={"share".to_string()} data={data} /> })
                .unwrap_or_default();
            html! {
                <div class="share-view-card">
                    { content }
                    { chart }
                </div>
            }
        }
        ViewState::Error(ShareFetchError::Unauthorized) => {
            // 로그인 후 이 링크로 복귀하도록 `?next=`를 붙여 메인 앱으로 이동한다.
            let on_login = {
                let share_id = props.share_id.clone();
                Callback::from(move |_: MouseEvent| {
                    if let Some(window) = web_sys::window() {
                        // 복귀 경로는 해시 형태. 인코딩하면 `#`→`%23`으로 쿼리에 안전히 실린다.
                        let next = format!("/#/share/{share_id}");
                        let encoded = String::from(js_sys::encode_uri_component(&next));
                        let _ = window.location().set_href(&format!("/?next={encoded}"));
                    }
                })
            };
            html! {
                <div class="share-view-status">
                    <p>{ "이 답변을 보려면 로그인이 필요합니다." }</p>
                    <button class="modal-btn" onclick={on_login}>{ "로그인하기" }</button>
                </div>
            }
        }
        ViewState::Error(ShareFetchError::Forbidden) => html! {
            <div class="share-view-status">{ "같은 회사 계정으로 로그인해야 열람 가능합니다." }</div>
        },
        ViewState::Error(ShareFetchError::NotFound) => html! {
            <div class="share-view-status">{ "공유된 답변을 찾을 수 없습니다." }</div>
        },
        ViewState::Error(ShareFetchError::Other(msg)) => html! {
            <div class="share-view-status">{ format!("오류가 발생했습니다: {msg}") }</div>
        },
    };

    html! {
        <div class="share-view-root">
            <header class="share-view-header">
                <span class="share-view-logo">{ "EHS AI" }</span>
                <span class="share-view-subtitle">{ "공유된 답변" }</span>
            </header>
            <main class="share-view-main">
                { body }
            </main>
        </div>
    }
}
