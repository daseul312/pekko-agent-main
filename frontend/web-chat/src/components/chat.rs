use web_sys::{Element, ScrollBehavior, ScrollIntoViewOptions};
use yew::prelude::*;

use crate::components::input::ChatInput;
use crate::components::message::{MessageBubble, TypingIndicator};
use crate::types::{ChatMessage, AgentMeta};

#[derive(Properties, PartialEq)]
pub struct ChatProps {
    pub messages: Vec<ChatMessage>,
    pub is_loading: bool,
    pub on_send: Callback<String>,
    pub on_example_click: Callback<String>,
    pub has_agent: bool,
    pub agent_meta: Option<AgentMeta>,
}

#[function_component(ChatArea)]
pub fn chat_area(props: &ChatProps) -> Html {
    let messages_end_ref = use_node_ref();

    // Auto-scroll to bottom when new messages arrive
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

    let show_welcome = props.messages.is_empty() && !props.is_loading;
    
    if show_welcome {
        let examples = match props.agent_meta.as_ref().map(|m| m.id.as_str()) {
            Some("ehs-permit-agent") => vec![
                "오늘 승인된 화기작업 목록 보여줘",
                "내일 예정된 밀폐공간 작업은?",
                "승인 대기 중인 허가서 확인해줘",
                "위험작업 승인 절차 알려줘",
            ],
            Some("ehs-inspection-agent") => vec![
                "이번 주 안전점검 일정 알려줘",
                "지난 점검 시 지적사항 요약해줘",
                "시설물 점검 보고서 작성 도와줘",
                "점검 누락된 구역이 어디야?",
            ],
            Some("ehs-compliance-agent") => vec![
                "산업안전보건법 최신 개정안 요약",
                "중대재해처벌법 대응 가이드",
                "우리 사업장 필수 안전 교육은?",
                "안전관리자 선임 기준 알려줘",
            ],
            _ => vec![
                "현재 진행 중인 위험작업 현황",
                "이번 달 안전점검 일정은?",
                "MSDS 등록 화학물질 목록",
                "최근 아차사고 현황 분석",
            ],
        };

        let agent_class = props.agent_meta.as_ref().map(|m| m.css_class).unwrap_or("");
        let agent_name = props.agent_meta.as_ref().map(|m| m.name.as_str()).unwrap_or("EHS AI Agent");

        html! {
            <>
                <div class="welcome">
                    <div class={classes!("welcome-agent-icon", agent_class)}></div>
                    <h2>{ format!("{}와 대화를 시작해보세요", agent_name) }</h2>
                    <p>
                        { "아래 예시 질문을 클릭하거나 궁금한 점을 직접 입력해 주세요." }
                    </p>
                    <div class="welcome-examples">
                        { for examples.into_iter().map(|ex| {
                            let text = ex.to_string();
                            let cb = props.on_example_click.clone();
                            let t = text.clone();
                            html! {
                                <div class="welcome-example"
                                    onclick={Callback::from(move |_: MouseEvent| cb.emit(t.clone()))}
                                >
                                    <span class="example-icon">{ "💡" }</span>
                                    <span class="example-text">{ text }</span>
                                </div>
                            }
                        })}
                    </div>
                </div>
                <ChatInput on_send={props.on_send.clone()} disabled={!props.has_agent || props.is_loading} />
            </>
        }
    } else {
        html! {
            <>
                <div class="messages">
                    { for props.messages.iter().map(|msg| html! {
                        <MessageBubble message={msg.clone()} />
                    })}
                    if props.is_loading {
                        <TypingIndicator />
                    }
                    <div ref={messages_end_ref}></div>
                </div>
                <ChatInput on_send={props.on_send.clone()} disabled={!props.has_agent || props.is_loading} />
            </>
        }
    }
}
