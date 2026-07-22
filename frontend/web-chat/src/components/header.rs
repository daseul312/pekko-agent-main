use yew::prelude::*;

use crate::types::{AgentMeta, AuthUser, NotificationItem};

#[derive(Properties, PartialEq)]
pub struct HeaderProps {
    pub agents: Vec<AgentMeta>,
    pub auth_user: Option<AuthUser>,
    pub on_login_click: Callback<()>,
    pub on_logout: Callback<()>,
    pub is_dark_mode: bool,
    /// 현재 대화가 답변 대기/생성 중이면 에이전트 라벨 대신 "답변 중..."을 표시한다.
    #[prop_or_default]
    pub is_answering: bool,
    pub on_theme_toggle: Callback<()>,
    #[prop_or_default]
    pub on_back_to_agents: Option<Callback<()>>,
    /// 인앱 알림 목록(헤더 종 드롭다운).
    #[prop_or_default]
    pub notifications: Vec<NotificationItem>,
    /// 안 읽은 알림 수(배지).
    #[prop_or_default]
    pub unread: i64,
    /// 알림 항목 클릭 콜백(읽음 처리 + 비교 페이지 이동).
    #[prop_or_default]
    pub on_notification_click: Callback<NotificationItem>,
}

#[function_component(ChatHeader)]
pub fn chat_header(props: &HeaderProps) -> Html {
    // 알림 드롭다운 펼침 상태.
    let notif_open = use_state(|| false);

    // 헤더 종 + 안읽음 배지 + 드롭다운(로그인 시에만).
    let notif_html = if props.auth_user.is_some() {
        let toggle = {
            let notif_open = notif_open.clone();
            Callback::from(move |_: MouseEvent| notif_open.set(!*notif_open))
        };
        let badge = if props.unread > 0 {
            let n = if props.unread > 9 { "9+".to_string() } else { props.unread.to_string() };
            html! { <span class="notif-badge">{ n }</span> }
        } else {
            html! {}
        };
        let dropdown = if *notif_open {
            let items = if props.notifications.is_empty() {
                html! { <div class="notif-empty">{ "알림이 없습니다" }</div> }
            } else {
                props
                    .notifications
                    .iter()
                    .map(|item| {
                        let date: String = item.created_at.chars().take(10).collect();
                        let on_click = {
                            let on_notification_click = props.on_notification_click.clone();
                            let notif_open = notif_open.clone();
                            let item = item.clone();
                            Callback::from(move |_: MouseEvent| {
                                notif_open.set(false);
                                on_notification_click.emit(item.clone());
                            })
                        };
                        html! {
                            <button class={classes!("notif-item", (!item.is_read).then_some("unread"))} onclick={on_click}>
                                <span class="notif-item-msg">{ &item.message }</span>
                                <span class="notif-item-date">{ date }</span>
                            </button>
                        }
                    })
                    .collect::<Html>()
            };
            html! { <div class="notif-dropdown">{ items }</div> }
        } else {
            html! {}
        };
        html! {
            <div class="notif-wrap">
                <button class="notif-bell" onclick={toggle} title="알림">
                    <svg xmlns="http://www.w3.org/2000/svg" width="19" height="19" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9"></path>
                        <path d="M13.73 21a2 2 0 0 1-3.46 0"></path>
                    </svg>
                    { badge }
                </button>
                { dropdown }
            </div>
        }
    } else {
        html! {}
    };

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
        // 피드백 통계 관리자만 보이는 전용 이동 버튼. 해시 라우팅은 main()에서만 분기되므로
        // 해시를 바꾼 뒤 reload해 전용 페이지(AdminFeedback)로 진입한다.
        let admin_btn = if user.is_feedback_admin {
            html! {
                <button class="admin-link-btn" title="답변 피드백 통계" onclick={move |_| {
                    if let Some(w) = web_sys::window() {
                        let _ = w.location().set_href("/?admin=1#/admin/feedback");
                    }
                }}>
                    <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                        <line x1="18" y1="20" x2="18" y2="10"></line>
                        <line x1="12" y1="20" x2="12" y2="4"></line>
                        <line x1="6" y1="20" x2="6" y2="14"></line>
                    </svg>
                    <span>{ "피드백 통계" }</span>
                </button>
            }
        } else {
            html! {}
        };
        html! {
            <div class="header-auth-info">
                { avatar }
                <span class="user-welcome">
                    <strong>{ &user.name }</strong>
                    { " 님" }
                </span>
                { admin_btn }
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

    let main_content = if props.is_answering {
        html! {
            <div class="chat-header-main">
                <div class="chat-header-row">
                    <span class="chat-header-title chat-header-answering">{ "답변 중..." }</span>
                </div>
            </div>
        }
    } else { match props.agents.as_slice() {
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
    } };

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
                { notif_html }
                { auth_section }
            </div>
        </div>
    }
}
