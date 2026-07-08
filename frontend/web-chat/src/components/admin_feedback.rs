//! 피드백 통계(관리자) 대시보드. 라우터가 없으므로 `main()`이 `/#/admin/feedback`
//! 경로를 감지해 `App` 대신 이 컴포넌트를 렌더한다(엔트리 분기).
//!
//! 기간 필터(이번 달/지난 3개월/전체) → KPI 4종 → 에이전트별 긍정률 / 부정 사유 도넛 →
//! 월별 트렌드 → 최근 부정 코멘트 테이블(더 보기). 차트는 기존 `ChartBlock`을 재사용.
//! 접근은 백엔드의 `FEEDBACK_ADMIN_IDS` 허용목록으로 게이팅된다(401/403 분기).

use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;

use crate::api;
use crate::components::chart::ChartBlock;
use crate::types::{
    AgentMeta, AgentRate, ChartData, ChartSeries, CommentRow, FeedbackStatsError,
    FeedbackStatsResponse, ReverifyRow, TagCount, TrendPoint,
};

const COMMENTS_PAGE: i64 = 20;

#[derive(Clone, PartialEq)]
enum StatsView {
    Loading,
    Loaded(FeedbackStatsResponse),
    Error(FeedbackStatsError),
}

/// agent_id → 사람이 읽는 표시명(미매칭은 "(미지정)").
fn agent_display_name(agent_id: &str) -> String {
    if agent_id.is_empty() {
        return "(미지정)".to_string();
    }
    AgentMeta::defaults()
        .into_iter()
        .find(|a| a.id == agent_id)
        .map(|a| a.name)
        .unwrap_or_else(|| agent_id.to_string())
}

fn pct(part: i64, total: i64) -> i64 {
    if total <= 0 {
        0
    } else {
        (part as f64 / total as f64 * 100.0).round() as i64
    }
}

fn kpi_card(num: String, label: &str) -> Html {
    html! {
        <div class="fb-kpi-card">
            <span class="fb-kpi-num">{ num }</span>
            <span class="fb-kpi-label">{ label }</span>
        </div>
    }
}

/// 에이전트별 긍정률 바 + 상태 배지.
fn agent_rate_row(a: &AgentRate) -> Html {
    let total = a.up + a.down;
    let rate = pct(a.up, total);
    let (badge_class, badge_text) = if total == 0 {
        ("fb-badge-none", "데이터 없음")
    } else if rate >= 80 {
        ("fb-badge-good", "양호")
    } else if rate >= 70 {
        ("fb-badge-mid", "보통")
    } else {
        ("fb-badge-bad", "개선 필요")
    };
    html! {
        <div class="fb-agent-row">
            <div class="fb-agent-row-head">
                <span class="fb-agent-name">{ agent_display_name(&a.agent_id) }</span>
                <span class="fb-agent-meta">
                    <span class="fb-agent-rate">{ format!("{rate}%") }</span>
                    <span class={classes!("fb-badge", badge_class)}>{ badge_text }</span>
                </span>
            </div>
            <div class="fb-rate-track">
                <div class="fb-rate-fill" style={format!("width:{rate}%")}></div>
            </div>
            <div class="fb-agent-counts">{ format!("👍 {} · 👎 {}", a.up, a.down) }</div>
        </div>
    }
}

/// 부정 사유 도넛 ChartData(건수 0인 사유는 제외). 모두 0이면 None.
fn donut_data(tags: &[TagCount]) -> Option<ChartData> {
    let nonzero: Vec<&TagCount> = tags.iter().filter(|t| t.count > 0).collect();
    if nonzero.is_empty() {
        return None;
    }
    Some(ChartData {
        chart_type: "donut".to_string(),
        title: "부정 피드백 사유".to_string(),
        labels: nonzero.iter().map(|t| t.tag.clone()).collect(),
        series: vec![ChartSeries {
            name: "건수".to_string(),
            values: nonzero.iter().map(|t| t.count as f64).collect(),
        }],
        insight: None,
    })
}

/// 추세 라인 ChartData(긍정/개선필요 2계열). 비어있으면 None.
fn trend_data(trend: &[TrendPoint], granularity: &str) -> Option<ChartData> {
    if trend.is_empty() {
        return None;
    }
    let title = match granularity {
        "day" => "일별 추세",
        "week" => "주간 추세",
        _ => "월별 추세",
    };
    Some(ChartData {
        chart_type: "line".to_string(),
        title: title.to_string(),
        labels: trend.iter().map(|t| t.month.clone()).collect(),
        series: vec![
            ChartSeries {
                name: "긍정".to_string(),
                values: trend.iter().map(|t| t.up as f64).collect(),
            },
            ChartSeries {
                name: "개선 필요".to_string(),
                values: trend.iter().map(|t| t.down as f64).collect(),
            },
        ],
        insight: None,
    })
}

fn comment_row(row: &CommentRow) -> Html {
    let date: String = row.created_at.chars().take(10).collect();
    let comment = row.comment.clone().unwrap_or_default();
    let tags = if row.tags.is_empty() {
        html! {}
    } else {
        html! {
            <div class="fb-row-tags">
                { for row.tags.iter().map(|t| html! { <span class="fb-tag-chip sm">{ t }</span> }) }
            </div>
        }
    };
    let goto = if row.session_id.is_empty() {
        html! {}
    } else {
        let session_id = row.session_id.clone();
        let on_goto = Callback::from(move |_: MouseEvent| {
            if let Some(window) = web_sys::window() {
                let loc = window.location();
                let _ = loc.set_hash(&format!("#/admin/session/{session_id}"));
                let _ = loc.reload();
            }
        });
        html! { <button class="fb-goto-btn" onclick={on_goto} title="이 답변이 나온 대화 보기">{ "💬 바로가기" }</button> }
    };
    // 조치 완료 버튼 / 이미 조치된 건은 배지. feedback_key 없으면 미표시.
    let resolve = if row.feedback_key.is_empty() {
        html! {}
    } else if row.resolved {
        html! { <span class="fb-resolved-badge">{ "✅ 조치 완료됨" }</span> }
    } else {
        let key = row.feedback_key.clone();
        let on_resolve = Callback::from(move |_: MouseEvent| {
            if let Some(window) = web_sys::window() {
                let enc = String::from(js_sys::encode_uri_component(&key));
                let loc = window.location();
                let _ = loc.set_hash(&format!("#/admin/resolution/{enc}"));
                let _ = loc.reload();
            }
        });
        html! { <button class="fb-resolve-btn" onclick={on_resolve} title="개선 답변 생성 후 사용자에게 알림">{ "✅ 조치 완료" }</button> }
    };
    html! {
        <div class="fb-comment-row">
            <div class="fb-comment-agent">{ agent_display_name(&row.agent_id) }</div>
            <div class="fb-comment-body">
                <div class="fb-comment-text">{ comment }</div>
                { tags }
            </div>
            <div class="fb-comment-aside">
                <span class="fb-comment-date">{ date }</span>
                { goto }
                { resolve }
            </div>
        </div>
    }
}

/// 부정확 재확인 행(조치 후 재확인 섹션).
fn reverify_row(row: &ReverifyRow) -> Html {
    let date: String = row.created_at.chars().take(10).collect();
    let comment = row.comment.clone().unwrap_or_default();
    let key = row.feedback_key.clone();
    let on_goto = Callback::from(move |_: MouseEvent| {
        if let Some(window) = web_sys::window() {
            let enc = String::from(js_sys::encode_uri_component(&key));
            let loc = window.location();
            let _ = loc.set_hash(&format!("#/resolution/{enc}"));
            let _ = loc.reload();
        }
    });
    html! {
        <div class="fb-comment-row">
            <div class="fb-comment-agent">{ agent_display_name(&row.agent_id) }</div>
            <div class="fb-comment-body">
                <div class="fb-comment-text">{ if comment.is_empty() { "(코멘트 없음)".to_string() } else { comment } }</div>
            </div>
            <div class="fb-comment-aside">
                <span class="fb-comment-date">{ date }</span>
                <button class="fb-goto-btn" onclick={on_goto} title="개선 답변 비교 보기">{ "🔍 비교 보기" }</button>
            </div>
        </div>
    }
}

#[function_component(AdminFeedback)]
pub fn admin_feedback() -> Html {
    // "all" | "month" | "3m".
    let period = use_state(|| "all".to_string());
    // "day" | "week" | "month".
    let granularity = use_state(|| "month".to_string());
    let stats = use_state(|| StatsView::Loading);
    let comments = use_state(Vec::<CommentRow>::new);
    let comments_total = use_state(|| 0i64);
    let loading_more = use_state(|| false);
    let reverify = use_state(Vec::<ReverifyRow>::new);

    // 기간 또는 추세 집계 단위 변경 시 통계 재조회.
    {
        let stats = stats.clone();
        use_effect_with(((*period).clone(), (*granularity).clone()), move |(p, g)| {
            let p = p.clone();
            let g = g.clone();
            stats.set(StatsView::Loading);
            spawn_local(async move {
                match api::fetch_feedback_stats(&p, &g).await {
                    Ok(s) => stats.set(StatsView::Loaded(s)),
                    Err(e) => stats.set(StatsView::Error(e)),
                }
            });
            || ()
        });
    }

    // 기간 변경 시에만 코멘트 1페이지 + 부정확 재확인 목록 재조회(추세 단위 변경과 무관).
    {
        let comments = comments.clone();
        let comments_total = comments_total.clone();
        let reverify = reverify.clone();
        use_effect_with((*period).clone(), move |p| {
            let p = p.clone();
            comments.set(Vec::new());
            comments_total.set(0);
            reverify.set(Vec::new());
            spawn_local(async move {
                if let Ok(c) = api::fetch_feedback_comments(&p, COMMENTS_PAGE, 0).await {
                    comments.set(c.items);
                    comments_total.set(c.total);
                }
                if let Ok(rv) = api::fetch_reverify_list(&p, 50, 0).await {
                    reverify.set(rv.items);
                }
            });
            || ()
        });
    }

    let on_more = {
        let comments = comments.clone();
        let comments_total = comments_total.clone();
        let loading_more = loading_more.clone();
        let period = period.clone();
        Callback::from(move |_: MouseEvent| {
            let offset = (*comments).len() as i64;
            if offset >= *comments_total || *loading_more {
                return;
            }
            let comments = comments.clone();
            let loading_more = loading_more.clone();
            let p = (*period).clone();
            loading_more.set(true);
            spawn_local(async move {
                if let Ok(c) = api::fetch_feedback_comments(&p, COMMENTS_PAGE, offset).await {
                    let mut next = (*comments).clone();
                    next.extend(c.items);
                    comments.set(next);
                }
                loading_more.set(false);
            });
        })
    };

    // 기간 필터 칩.
    let period_chips = {
        let cur = (*period).clone();
        let make = |val: &'static str, label: &'static str| {
            let period = period.clone();
            let active = cur == val;
            let on_click = Callback::from(move |_: MouseEvent| period.set(val.to_string()));
            html! {
                <button class={classes!("fb-period-chip", active.then_some("active"))} onclick={on_click}>
                    { label }
                </button>
            }
        };
        html! {
            <div class="fb-period-bar">
                { make("month", "이번 달") }
                { make("3m", "지난 3개월") }
                { make("all", "전체 기간") }
            </div>
        }
    };

    // 추세 집계 단위 칩(일별/주간/월별).
    let granularity_chips = {
        let cur = (*granularity).clone();
        let make = |val: &'static str, label: &'static str| {
            let granularity = granularity.clone();
            let active = cur == val;
            let on_click = Callback::from(move |_: MouseEvent| granularity.set(val.to_string()));
            html! {
                <button class={classes!("fb-period-chip", "sm", active.then_some("active"))} onclick={on_click}>
                    { label }
                </button>
            }
        };
        html! {
            <div class="fb-period-bar fb-trend-granularity">
                { make("day", "일별") }
                { make("week", "주간") }
                { make("month", "월별") }
            </div>
        }
    };

    let body = match &*stats {
        StatsView::Loading => html! { <div class="admin-fb-status">{ "불러오는 중..." }</div> },
        StatsView::Error(FeedbackStatsError::Unauthorized) => {
            let on_login = Callback::from(move |_: MouseEvent| {
                if let Some(window) = web_sys::window() {
                    let next = "/#/admin/feedback";
                    let encoded = String::from(js_sys::encode_uri_component(next));
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
        StatsView::Error(FeedbackStatsError::Forbidden) => html! {
            <div class="admin-fb-status">{ "이 페이지는 관리자만 볼 수 있어요." }</div>
        },
        StatsView::Error(FeedbackStatsError::Other(msg)) => html! {
            <div class="admin-fb-status">{ format!("오류가 발생했습니다: {msg}") }</div>
        },
        StatsView::Loaded(s) => {
            let total = s.total_up + s.total_down;
            // KPI 카드.
            let kpis = html! {
                <div class="fb-kpi-grid">
                    { kpi_card(total.to_string(), "총 피드백 수") }
                    { kpi_card(format!("{}%", pct(s.total_up, total)), "긍정 응답률") }
                    { kpi_card(format!("{}%", pct(s.total_down, total)), "개선 필요 비율") }
                    { kpi_card(s.comment_count.to_string(), "코멘트 작성 수") }
                </div>
            };

            // 에이전트별 긍정률.
            let agents = if s.agents.is_empty() {
                html! { <div class="fb-empty">{ "데이터가 없습니다" }</div> }
            } else {
                html! { <>{ for s.agents.iter().map(agent_rate_row) }</> }
            };

            // 부정 사유 도넛 + 칩.
            let donut = match donut_data(&s.tag_counts) {
                Some(data) => html! { <ChartBlock message_id={"fb-donut".to_string()} data={data} /> },
                None => html! { <div class="fb-empty">{ "부정 사유 데이터가 없습니다" }</div> },
            };
            let tag_chips = html! {
                <div class="fb-tag-chip-list">
                    { for s.tag_counts.iter().filter(|t| t.count > 0).map(|t| html! {
                        <span class="fb-tag-chip">{ format!("{} {}", t.tag, t.count) }</span>
                    }) }
                </div>
            };

            // 추세(일별/주간/월별 선택 가능).
            let trend = match trend_data(&s.trend, &*granularity) {
                Some(data) => html! { <ChartBlock message_id={"fb-trend".to_string()} data={data} /> },
                None => html! { <div class="fb-empty">{ "데이터가 없습니다" }</div> },
            };

            // 코멘트 테이블 + 더 보기.
            let loaded = (*comments).len() as i64;
            let has_more = loaded < *comments_total;
            let comments_block = if comments.is_empty() {
                html! { <div class="fb-empty">{ "부정 코멘트가 없습니다" }</div> }
            } else {
                html! {
                    <>
                        <div class="fb-comments-table">
                            { for comments.iter().map(comment_row) }
                        </div>
                        if has_more {
                            <button class="fb-more-btn" onclick={on_more.clone()} disabled={*loading_more}>
                                { if *loading_more { "불러오는 중..." } else { "더 보기" } }
                            </button>
                        }
                    </>
                }
            };

            // 조치 후 재확인 섹션.
            let res = &s.resolution;
            let reverify_block = if reverify.is_empty() {
                html! { <div class="fb-empty">{ "부정확으로 재확인된 건이 없습니다" }</div> }
            } else {
                html! {
                    <div class="fb-comments-table">
                        { for reverify.iter().map(reverify_row) }
                    </div>
                }
            };

            html! {
                <>
                    { kpis }
                    <div class="fb-two-col">
                        <section class="fb-section">
                            <h3 class="fb-section-title">{ "에이전트별 긍정률" }</h3>
                            { agents }
                        </section>
                        <section class="fb-section">
                            <h3 class="fb-section-title">{ "부정 피드백 사유" }</h3>
                            { donut }
                            { tag_chips }
                        </section>
                    </div>
                    <section class="fb-section">
                        <div class="fb-section-head">
                            <h3 class="fb-section-title">{ "추세" }</h3>
                            { granularity_chips }
                        </div>
                        { trend }
                    </section>
                    <section class="fb-section">
                        <h3 class="fb-section-title">{ "최근 부정 코멘트" }</h3>
                        { comments_block }
                    </section>
                    <section class="fb-section">
                        <h3 class="fb-section-title">{ "조치 후 재확인" }</h3>
                        <div class="fb-reverify-kpis">
                            { kpi_card(res.resolved.to_string(), "조치 완료") }
                            { kpi_card(res.reverify_accurate.to_string(), "재확인 정확") }
                            { kpi_card(res.reverify_inaccurate.to_string(), "재확인 부정확") }
                        </div>
                        <h4 class="fb-subsection-title">{ "부정확 재피드백" }</h4>
                        { reverify_block }
                    </section>
                </>
            }
        }
    };

    // 채팅으로 돌아가기.
    let on_back = Callback::from(move |_: MouseEvent| {
        if let Some(window) = web_sys::window() {
            let _ = window.location().set_href("/");
        }
    });

    html! {
        <div class="admin-fb-root">
            <header class="admin-fb-header">
                <button class="admin-fb-back" onclick={on_back} title="채팅으로 돌아가기">
                    <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
                        <path d="m15 18-6-6 6-6"></path>
                    </svg>
                </button>
                <span class="admin-fb-logo">{ "EHS AI" }</span>
                <span class="admin-fb-subtitle">{ "답변 피드백 통계" }</span>
            </header>
            <main class="admin-fb-main">
                { period_chips }
                { body }
            </main>
        </div>
    }
}
