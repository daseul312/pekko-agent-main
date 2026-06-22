use crate::types::AgentMeta;
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct AgentSelectorProps {
    pub agents: Vec<AgentMeta>,
    pub on_select: Callback<String>,
}

#[function_component(AgentSelector)]
pub fn agent_selector(props: &AgentSelectorProps) -> Html {
    html! {
        <div class="agent-selector-container">
            <div class="agent-selector-header">
                <h2>{ "에이전트를 선택하세요" }</h2>
                <p>
                    {"EHS 업무를 도와드릴 전문 AI 에이전트입니다."}
                    <br/>
                    {"대화를 시작할 에이전트를 골라주세요."}
                </p>
            </div>

            <div class="agent-grid">
                { for props.agents.iter().map(|agent| {
                    let agent_id = agent.id.clone();
                    let on_click = {
                        let cb = props.on_select.clone();
                        Callback::from(move |_: MouseEvent| cb.emit(agent_id.clone()))
                    };

                    html! {
                        <div class="agent-card" onclick={on_click}>
                            <div class={classes!("agent-card-icon", agent.css_class.to_string())}>
                                { agent.icon }
                            </div>
                            <div class="agent-card-content">
                                <h3 class="agent-card-name">{ &agent.name }</h3>
                                <p class="agent-card-desc">{ &agent.description }</p>
                                <div class="agent-card-footer">
                                    <span class="start-chat">{ "대화 시작하기 →" }</span>
                                </div>
                            </div>
                        </div>
                    }
                })}
            </div>
        </div>
    }
}
