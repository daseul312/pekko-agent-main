use yew::prelude::*;

use crate::types::{AgentMeta, AuthUser};

#[derive(Properties, PartialEq)]
pub struct HeaderProps {
    pub agents: Vec<AgentMeta>,
    pub auth_user: Option<AuthUser>,
    pub on_login_click: Callback<()>,
    pub on_logout: Callback<()>,
    pub is_dark_mode: bool,
    pub on_theme_toggle: Callback<()>,
    #[prop_or_default]
    pub on_back_to_agents: Option<Callback<()>>,
}

#[function_component(ChatHeader)]
pub fn chat_header(props: &HeaderProps) -> Html {
    let auth_section = if let Some(user) = &props.auth_user {
        let avatar = match &user.profile_image_url {
            Some(url) if !url.is_empty() => html! {
                <img class="user-avatar" src={url.clone()} alt="" />
            },
            _ => html! {
                <span class="user-avatar user-avatar-fallback">
                    { user.name.chars().next().unwrap_or('?').to_string() }
                </span>
            },
        };
        html! {
            <div class="header-auth-info">
                { avatar }
                <span class="user-welcome">
                    <strong>{ &user.name }</strong>
                    { " 님" }
                </span>
                <button class="logout-btn" onclick={let on_logout = props.on_logout.clone(); move |_| on_logout.emit(())}>
                    <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"></path>
                        <polyline points="16 17 21 12 16 7"></polyline>
                        <line x1="21" y1="12" x2="9" y2="12"></line>
                    </svg>
                    <span>{ "로그아웃" }</span>
                </button>
            </div>
        }
    } else {
        html! {
            <div class="header-auth-action" onclick={let on_login = props.on_login_click.clone(); move |_| on_login.emit(())}>
                <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="login-icon">
                    <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"></path>
                    <circle cx="12" cy="7" r="4"></circle>
                </svg>
            </div>
        }
    };

    let main_content = match props.agents.as_slice() {
        [] => html! {
            <div class="chat-header-main">
                <div class="chat-header-row">
                    <span class="chat-header-title">{ "EHS AI Agent" }</span>
                    <span class="chat-header-badge">{ "질문을 분석해 적합한 에이전트를 선택합니다" }</span>
                </div>
            </div>
        },
        [agent] => html! {
            <div class="chat-header-main">
                <div class="chat-header-row">
                    <span style="font-size: 20px;">{ agent.icon }</span>
                    <span class="chat-header-title">{ &agent.name }</span>
                    <span class="chat-header-badge">{ &agent.description }</span>
                </div>
            </div>
        },
        agents => {
            let title = agents
                .iter()
                .map(|agent| agent.name.clone())
                .collect::<Vec<_>>()
                .join(" + ");
            let summary = agents
                .iter()
                .map(|agent| agent.description.clone())
                .collect::<Vec<_>>()
                .join(" / ");

            html! {
                <div class="chat-header-main">
                    <div class="chat-header-row">
                        <span class="chat-header-title">{ title }</span>
                        <span class={classes!("chat-header-badge", "multi")}>{ "멀티 에이전트 라우팅" }</span>
                    </div>
                    <div class="chat-header-agents">
                        { for agents.iter().map(|agent| html! {
                            <span class="chat-header-chip">{ format!("{} · {}", agent.name, agent.description) }</span>
                        })}
                    </div>
                    <div class="chat-header-summary">{ summary }</div>
                </div>
            }
        }
    };

    let theme_toggle = html! {
        <button class="theme-toggle-btn" onclick={let on_toggle = props.on_theme_toggle.clone(); move |_| on_toggle.emit(())} title={if props.is_dark_mode { "라이트 모드로 변경" } else { "다크 모드로 변경" }}>
            { if props.is_dark_mode {
                html! {
                    <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="theme-icon moon">
                        <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"></path>
                    </svg>
                }
            } else {
                html! {
                    <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="theme-icon sun">
                        <circle cx="12" cy="12" r="5"></circle>
                        <line x1="12" y1="1" x2="12" y2="3"></line>
                        <line x1="12" y1="21" x2="12" y2="23"></line>
                        <line x1="4.22" y1="4.22" x2="5.64" y2="5.64"></line>
                        <line x1="18.36" y1="18.36" x2="19.78" y2="19.78"></line>
                        <line x1="1" y1="12" x2="3" y2="12"></line>
                        <line x1="21" y1="12" x2="23" y2="12"></line>
                        <line x1="4.22" y1="19.78" x2="5.64" y2="18.36"></line>
                        <line x1="18.36" y1="5.64" x2="19.78" y2="4.22"></line>
                    </svg>
                }
            }}
        </button>
    };

    html! {
        <div class="chat-header">
            { if let Some(on_back) = &props.on_back_to_agents {
                html! {
                    <button
                        class="chat-header-back-btn"
                        type="button"
                        title="에이전트 선택으로 돌아가기"
                        aria-label="에이전트 선택으로 돌아가기"
                        data-tooltip="에이전트 선택 화면으로 돌아가기"
                        onclick={let on_back = on_back.clone(); move |_| on_back.emit(())}
                    >
                        <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
                            <path d="m15 18-6-6 6-6"></path>
                        </svg>
                    </button>
                }
            } else { html! {} } }
            { main_content }
            <div class={classes!("header-right-section", if props.auth_user.is_some() { "auth" } else { "" })}>
                { theme_toggle }
                { auth_section }
            </div>
        </div>
    }
}
