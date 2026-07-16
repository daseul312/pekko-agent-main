//! 에이전트별 요약 목록(현황판) 전체 페이지. 라우터가 없으므로 `main()`이
//! `/#/records/{agent_id}` 경로를 감지해 `App` 대신 이 컴포넌트를 렌더한다(엔트리 분기).
//!
//! 에이전트마다 표시할 레코드 종류가 다르므로 `records_config_for(agent_id)`로 분기한다:
//! - `ehs-permit-agent`: 작업계획서 목록(`/api/records/workplans`, 상태 필터 + 페이지네이션).
//! - 그 외: 로그인 사용자 본인의 피드백 목록(`/api/records/feedback`). admin 전용
//!   `/api/admin/feedback/*`(관리자 집계, `AdminFeedback` 페이지 전용)는 여기서 쓰지 않는다 —
//!   일반 로그인 사용자는 그 엔드포인트에 접근 권한이 없기 때문이다.
//!
//! 화면 하단에는 Next.js 관리자 대시보드 로그인 페이지로 새 탭 이동하는 버튼을 둔다.
//! 아이디(employeeId)만 쿼리로 프리필하고 비밀번호는 절대 전달하지 않는다.

use gloo_storage::{LocalStorage, Storage};
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;

use crate::api;
use crate::types::{
    AgentMeta, AuthUser, CommitteeResponse, CommitteeRow, EducationResponse, EducationRow,
    EnvironmentResponse, EnvironmentRow, FeedbackStatsError, InspectionsResponse, InspectionRow,
    MyFeedbackRow, NearMissResponse, NearMissRow, WorkPlanRow, WorkPlansResponse,
};

const PAGE_SIZE: i64 = 20;
/// 메인 앱(main.rs)/피드백 관리자 페이지와 동일한 키를 써서 다크모드 설정을 공유한다.
const THEME_STORAGE_KEY: &str = "pekko_theme_dark";

#[derive(Properties, PartialEq, Clone)]
pub struct RecordsViewProps {
    pub agent_id: String,
}

/// 이 화면이 다룰 레코드 종류. 새 에이전트가 추가되면 이 enum(또는 매핑)만 확장하면 된다.
#[derive(Clone, PartialEq)]
enum RecordsKind {
    /// 작업계획서(public."PreWorkPlan") 목록.
    WorkPlans,
    /// 순회점검(public."SafetyRulesInspect") 목록.
    Inspection,
    /// 교육이수(public."EducationPlanTarget") 목록.
    Education,
    /// 작업환경측정(public."WorkEnvironmentManagement") 목록.
    Environment,
    /// 아차사고(public."NearMissManagement") 목록.
    NearMiss,
    /// 산업안전보건위원회(public."SafetyHealthCommittee") 목록.
    Committee,
    /// 피드백 현황(해당 에이전트로 필터링). 미지원 agent_id의 폴백 전용.
    Feedback,
}

struct RecordsConfig {
    kind: RecordsKind,
    title: &'static str,
}

/// agent_id → 표시할 레코드 종류/제목 매핑.
fn records_config_for(agent_id: &str) -> RecordsConfig {
    match agent_id {
        "ehs-permit-agent" => RecordsConfig {
            kind: RecordsKind::WorkPlans,
            title: "작업계획서 목록",
        },
        "ehs-inspection-agent" => RecordsConfig {
            kind: RecordsKind::Inspection,
            title: "순회점검 목록",
        },
        "ehs-education-agent" => RecordsConfig {
            kind: RecordsKind::Education,
            title: "교육이수 목록",
        },
        "ehs-health-environment-agent" => RecordsConfig {
            kind: RecordsKind::Environment,
            title: "작업환경측정 목록",
        },
        "ehs-accident-agent" => RecordsConfig {
            kind: RecordsKind::NearMiss,
            title: "아차사고 목록",
        },
        "ehs-compliance-agent" => RecordsConfig {
            kind: RecordsKind::Committee,
            title: "산업안전보건위원회 목록",
        },
        _ => RecordsConfig {
            kind: RecordsKind::Feedback,
            title: "피드백 현황",
        },
    }
}

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

#[derive(Clone, PartialEq)]
enum WorkPlansView {
    Loading,
    Loaded(WorkPlansResponse),
    Error(FeedbackStatsError),
}

#[derive(Clone, PartialEq)]
enum FeedbackView {
    Loading,
    Loaded(Vec<MyFeedbackRow>),
    Error(FeedbackStatsError),
}

#[derive(Clone, PartialEq)]
enum InspectionView {
    Loading,
    Loaded(InspectionsResponse),
    Error(FeedbackStatsError),
}

#[derive(Clone, PartialEq)]
enum EducationView {
    Loading,
    Loaded(EducationResponse),
    Error(FeedbackStatsError),
}

#[derive(Clone, PartialEq)]
enum EnvironmentView {
    Loading,
    Loaded(EnvironmentResponse),
    Error(FeedbackStatsError),
}

#[derive(Clone, PartialEq)]
enum NearMissView {
    Loading,
    Loaded(NearMissResponse),
    Error(FeedbackStatsError),
}

#[derive(Clone, PartialEq)]
enum CommitteeView {
    Loading,
    Loaded(CommitteeResponse),
    Error(FeedbackStatsError),
}

fn status_error_html(err: &FeedbackStatsError) -> Html {
    match err {
        FeedbackStatsError::Unauthorized => {
            let on_login = Callback::from(move |_: MouseEvent| {
                if let Some(window) = web_sys::window() {
                    let next = window.location().hash().unwrap_or_default();
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
        FeedbackStatsError::Forbidden => html! {
            <div class="admin-fb-status">{ "이 목록을 볼 권한이 없어요." }</div>
        },
        FeedbackStatsError::Other(msg) => html! {
            <div class="admin-fb-status">{ format!("오류가 발생했습니다: {msg}") }</div>
        },
    }
}

/// 공백 없이 아주 긴 제목(예: 수천 자 연속 문자열)이 들어오면 `table-layout: auto`가
/// 컬럼 너비 계산을 망가뜨려 테이블 전체가 깨져 보인다. 표시용으로만 길이를 제한한다.
const TITLE_DISPLAY_MAX: usize = 80;
fn truncate_title(title: &str) -> String {
    let char_count = title.chars().count();
    if char_count <= TITLE_DISPLAY_MAX {
        title.to_string()
    } else {
        let head: String = title.chars().take(TITLE_DISPLAY_MAX).collect();
        format!("{head}…")
    }
}

fn workplan_row(row: &WorkPlanRow) -> Html {
    let title = truncate_title(&row.title);
    html! {
        <tr class="records-table-row">
            <td>{ &row.pwp_no }</td>
            <td class="records-table-title" title={row.title.clone()}>{ title }</td>
            <td>{ &row.work_type_name }</td>
            <td><span class="fb-badge">{ &row.apply_status }</span></td>
            <td>{ &row.apply_employee_name }</td>
            <td>{ &row.create_date }</td>
        </tr>
    }
}

fn inspection_row(row: &InspectionRow) -> Html {
    html! {
        <tr class="records-table-row">
            <td>{ &row.employee_name }</td>
            <td>{ &row.department_name }</td>
            <td><span class="fb-badge">{ &row.status }</span></td>
            <td>{ &row.repeat_cycle_date }</td>
            <td>{ &row.inspection_date }</td>
        </tr>
    }
}

fn education_row(row: &EducationRow) -> Html {
    html! {
        <tr class="records-table-row">
            <td>{ &row.employee_name }</td>
            <td>{ &row.department_name }</td>
            <td>{ &row.education_completion_date }</td>
            <td><span class="fb-badge">{ &row.test_pass_yn }</span></td>
        </tr>
    }
}

fn environment_row(row: &EnvironmentRow) -> Html {
    let title = truncate_title(&row.title);
    html! {
        <tr class="records-table-row">
            <td class="records-table-title" title={row.title.clone()}>{ title }</td>
            <td>{ &row.factory_name }</td>
            <td>{ &row.measure_date }</td>
            <td>{ &row.measure_type }</td>
            <td>{ row.exceed_count.to_string() }</td>
            <td><span class="fb-badge">{ &row.status }</span></td>
        </tr>
    }
}

fn nearmiss_row(row: &NearMissRow) -> Html {
    let content = truncate_title(&row.action_content);
    html! {
        <tr class="records-table-row">
            <td>{ &row.field_name }</td>
            <td>{ &row.declare_employee_name }</td>
            <td>{ &row.occurrence_date }</td>
            <td><span class="fb-badge">{ &row.action_status_name }</span></td>
            <td class="records-table-title" title={row.action_content.clone()}>{ content }</td>
        </tr>
    }
}

fn committee_row(row: &CommitteeRow) -> Html {
    let title = truncate_title(&row.title);
    html! {
        <tr class="records-table-row">
            <td class="records-table-title" title={row.title.clone()}>{ title }</td>
            <td>{ &row.location }</td>
            <td>{ &row.start_date }</td>
            <td><span class="fb-badge">{ &row.sign_status }</span></td>
            <td>{ &row.meeting_type }</td>
        </tr>
    }
}

fn feedback_comment_row(row: &MyFeedbackRow) -> Html {
    let date: String = row.created_at.chars().take(10).collect();
    let comment = row.comment.clone().unwrap_or_default();
    html! {
        <tr class="records-table-row">
            <td>{ date }</td>
            <td>{ &row.rating }</td>
            <td>{ if comment.is_empty() { "(코멘트 없음)".to_string() } else { comment } }</td>
        </tr>
    }
}

#[function_component(RecordsView)]
pub fn records_view(props: &RecordsViewProps) -> Html {
    let agent_id = props.agent_id.clone();
    let config = records_config_for(&agent_id);
    let is_dark_mode = use_state(|| LocalStorage::get::<bool>(THEME_STORAGE_KEY).unwrap_or(false));
    let status_filter = use_state(|| String::new());
    let page = use_state(|| 1i64);
    let workplans = use_state(|| WorkPlansView::Loading);
    let feedback = use_state(|| FeedbackView::Loading);
    let inspections = use_state(|| InspectionView::Loading);
    let education = use_state(|| EducationView::Loading);
    let environment = use_state(|| EnvironmentView::Loading);
    let nearmiss = use_state(|| NearMissView::Loading);
    let committee = use_state(|| CommitteeView::Loading);
    let auth_user = use_state(|| Option::<AuthUser>::None);

    // 세션 쿠키로 현재 로그인 사용자 복원(Next 관리자 로그인 링크의 employeeId 프리필용).
    {
        let auth_user = auth_user.clone();
        use_effect_with((), move |_| {
            spawn_local(async move {
                if let Ok(Some(user)) = api::fetch_me().await {
                    auth_user.set(Some(user));
                }
            });
            || ()
        });
    }

    {
        let is_dark_mode_state = is_dark_mode.clone();
        use_effect_with((*is_dark_mode_state).clone(), move |dark| {
            let _ = LocalStorage::set(THEME_STORAGE_KEY, dark);
            || ()
        });
    }

    // 작업계획서 목록 로드(상태 필터/페이지 변경 시 재조회).
    {
        let workplans = workplans.clone();
        let agent_id_dep = agent_id.clone();
        let kind_is_workplans = matches!(config.kind, RecordsKind::WorkPlans);
        use_effect_with(
            (agent_id_dep, (*status_filter).clone(), *page),
            move |(_, status, page)| {
                if !kind_is_workplans {
                    return;
                }
                let workplans = workplans.clone();
                let status = status.clone();
                let page = *page;
                workplans.set(WorkPlansView::Loading);
                spawn_local(async move {
                    let status_opt = if status.is_empty() { None } else { Some(status.as_str()) };
                    match api::fetch_workplans(status_opt, page, PAGE_SIZE).await {
                        Ok(r) => workplans.set(WorkPlansView::Loaded(r)),
                        Err(e) => workplans.set(WorkPlansView::Error(e)),
                    }
                });
            },
        );
    }

    // 피드백 현황 로드(로그인 사용자 본인 것만, 해당 에이전트로 필터링).
    {
        let feedback = feedback.clone();
        let agent_id_dep = agent_id.clone();
        let kind_is_feedback = matches!(config.kind, RecordsKind::Feedback);
        use_effect_with(agent_id_dep.clone(), move |agent_id| {
            if !kind_is_feedback {
                return;
            }
            let feedback = feedback.clone();
            let agent_id = agent_id.clone();
            feedback.set(FeedbackView::Loading);
            spawn_local(async move {
                match api::fetch_my_feedback(&agent_id, 50, 0).await {
                    Ok(resp) => feedback.set(FeedbackView::Loaded(resp.items)),
                    Err(e) => feedback.set(FeedbackView::Error(e)),
                }
            });
        });
    }

    // 순회점검 목록 로드.
    {
        let inspections = inspections.clone();
        let agent_id_dep = agent_id.clone();
        let kind_is_inspection = matches!(config.kind, RecordsKind::Inspection);
        use_effect_with(
            (agent_id_dep, (*status_filter).clone(), *page),
            move |(_, status, page)| {
                if !kind_is_inspection {
                    return;
                }
                let inspections = inspections.clone();
                let status = status.clone();
                let page = *page;
                inspections.set(InspectionView::Loading);
                spawn_local(async move {
                    let status_opt = if status.is_empty() { None } else { Some(status.as_str()) };
                    match api::fetch_inspections(status_opt, page, PAGE_SIZE).await {
                        Ok(r) => inspections.set(InspectionView::Loaded(r)),
                        Err(e) => inspections.set(InspectionView::Error(e)),
                    }
                });
            },
        );
    }

    // 교육이수 목록 로드.
    {
        let education = education.clone();
        let agent_id_dep = agent_id.clone();
        let kind_is_education = matches!(config.kind, RecordsKind::Education);
        use_effect_with(
            (agent_id_dep, (*status_filter).clone(), *page),
            move |(_, status, page)| {
                if !kind_is_education {
                    return;
                }
                let education = education.clone();
                let status = status.clone();
                let page = *page;
                education.set(EducationView::Loading);
                spawn_local(async move {
                    let status_opt = if status.is_empty() { None } else { Some(status.as_str()) };
                    match api::fetch_education(status_opt, page, PAGE_SIZE).await {
                        Ok(r) => education.set(EducationView::Loaded(r)),
                        Err(e) => education.set(EducationView::Error(e)),
                    }
                });
            },
        );
    }

    // 작업환경측정 목록 로드.
    {
        let environment = environment.clone();
        let agent_id_dep = agent_id.clone();
        let kind_is_environment = matches!(config.kind, RecordsKind::Environment);
        use_effect_with(
            (agent_id_dep, (*status_filter).clone(), *page),
            move |(_, status, page)| {
                if !kind_is_environment {
                    return;
                }
                let environment = environment.clone();
                let status = status.clone();
                let page = *page;
                environment.set(EnvironmentView::Loading);
                spawn_local(async move {
                    let status_opt = if status.is_empty() { None } else { Some(status.as_str()) };
                    match api::fetch_environment(status_opt, page, PAGE_SIZE).await {
                        Ok(r) => environment.set(EnvironmentView::Loaded(r)),
                        Err(e) => environment.set(EnvironmentView::Error(e)),
                    }
                });
            },
        );
    }

    // 아차사고 목록 로드.
    {
        let nearmiss = nearmiss.clone();
        let agent_id_dep = agent_id.clone();
        let kind_is_nearmiss = matches!(config.kind, RecordsKind::NearMiss);
        use_effect_with(
            (agent_id_dep, (*status_filter).clone(), *page),
            move |(_, status, page)| {
                if !kind_is_nearmiss {
                    return;
                }
                let nearmiss = nearmiss.clone();
                let status = status.clone();
                let page = *page;
                nearmiss.set(NearMissView::Loading);
                spawn_local(async move {
                    let status_opt = if status.is_empty() { None } else { Some(status.as_str()) };
                    match api::fetch_nearmiss(status_opt, page, PAGE_SIZE).await {
                        Ok(r) => nearmiss.set(NearMissView::Loaded(r)),
                        Err(e) => nearmiss.set(NearMissView::Error(e)),
                    }
                });
            },
        );
    }

    // 산업안전보건위원회 목록 로드.
    {
        let committee = committee.clone();
        let agent_id_dep = agent_id.clone();
        let kind_is_committee = matches!(config.kind, RecordsKind::Committee);
        use_effect_with(
            (agent_id_dep, (*status_filter).clone(), *page),
            move |(_, status, page)| {
                if !kind_is_committee {
                    return;
                }
                let committee = committee.clone();
                let status = status.clone();
                let page = *page;
                committee.set(CommitteeView::Loading);
                spawn_local(async move {
                    let status_opt = if status.is_empty() { None } else { Some(status.as_str()) };
                    match api::fetch_committee(status_opt, page, PAGE_SIZE).await {
                        Ok(r) => committee.set(CommitteeView::Loaded(r)),
                        Err(e) => committee.set(CommitteeView::Error(e)),
                    }
                });
            },
        );
    }

    let body = match config.kind {
        RecordsKind::WorkPlans => {
            let status_chips = {
                let cur = (*status_filter).clone();
                let make = |val: &'static str, label: &'static str| {
                    let status_filter = status_filter.clone();
                    let page = page.clone();
                    let active = cur == val;
                    let on_click = Callback::from(move |_: MouseEvent| {
                        status_filter.set(val.to_string());
                        page.set(1);
                    });
                    html! {
                        <button class={classes!("fb-period-chip", active.then_some("active"))} onclick={on_click}>
                            { label }
                        </button>
                    }
                };
                html! {
                    <div class="fb-period-bar">
                        { make("", "전체") }
                        { make("승인", "승인") }
                        { make("신청", "신청") }
                    </div>
                }
            };

            let table = match &*workplans {
                WorkPlansView::Loading => html! { <div class="admin-fb-status">{ "불러오는 중..." }</div> },
                WorkPlansView::Error(e) => status_error_html(e),
                WorkPlansView::Loaded(resp) => {
                    if resp.items.is_empty() {
                        html! { <div class="fb-empty">{ "작업계획서가 없습니다" }</div> }
                    } else {
                        let total_pages = ((resp.total.max(1) - 1) / PAGE_SIZE) + 1;
                        let cur_page = resp.page;
                        let page_state = page.clone();
                        let on_prev = {
                            let page_state = page_state.clone();
                            Callback::from(move |_: MouseEvent| {
                                if cur_page > 1 {
                                    page_state.set(cur_page - 1);
                                }
                            })
                        };
                        let on_next = {
                            let page_state = page_state.clone();
                            Callback::from(move |_: MouseEvent| {
                                if cur_page < total_pages {
                                    page_state.set(cur_page + 1);
                                }
                            })
                        };
                        html! {
                            <>
                                <table class="records-table">
                                    <thead>
                                        <tr>
                                            <th>{ "계획서번호" }</th>
                                            <th>{ "제목" }</th>
                                            <th>{ "작업종류" }</th>
                                            <th>{ "상태" }</th>
                                            <th>{ "신청자" }</th>
                                            <th>{ "작성일" }</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        { for resp.items.iter().map(workplan_row) }
                                    </tbody>
                                </table>
                                <div class="records-pagination">
                                    <button class="fb-more-btn" onclick={on_prev} disabled={cur_page <= 1}>{ "이전" }</button>
                                    <span>{ format!("{cur_page} / {total_pages}") }</span>
                                    <button class="fb-more-btn" onclick={on_next} disabled={cur_page >= total_pages}>{ "다음" }</button>
                                </div>
                            </>
                        }
                    }
                }
            };
            html! {
                <>
                    { status_chips }
                    { table }
                </>
            }
        }
        RecordsKind::Inspection => {
            let status_chips = {
                let cur = (*status_filter).clone();
                let make = |val: &'static str, label: &'static str| {
                    let status_filter = status_filter.clone();
                    let page = page.clone();
                    let active = cur == val;
                    let on_click = Callback::from(move |_: MouseEvent| {
                        status_filter.set(val.to_string());
                        page.set(1);
                    });
                    html! {
                        <button class={classes!("fb-period-chip", active.then_some("active"))} onclick={on_click}>
                            { label }
                        </button>
                    }
                };
                html! {
                    <div class="fb-period-bar">
                        { make("", "전체") }
                        { make("점검예정", "점검예정") }
                        { make("점검완료", "점검완료") }
                    </div>
                }
            };

            let table = match &*inspections {
                InspectionView::Loading => html! { <div class="admin-fb-status">{ "불러오는 중..." }</div> },
                InspectionView::Error(e) => status_error_html(e),
                InspectionView::Loaded(resp) => {
                    if resp.items.is_empty() {
                        html! { <div class="fb-empty">{ "순회점검 내역이 없습니다" }</div> }
                    } else {
                        let total_pages = ((resp.total.max(1) - 1) / PAGE_SIZE) + 1;
                        let cur_page = resp.page;
                        let page_state = page.clone();
                        let on_prev = {
                            let page_state = page_state.clone();
                            Callback::from(move |_: MouseEvent| {
                                if cur_page > 1 {
                                    page_state.set(cur_page - 1);
                                }
                            })
                        };
                        let on_next = {
                            let page_state = page_state.clone();
                            Callback::from(move |_: MouseEvent| {
                                if cur_page < total_pages {
                                    page_state.set(cur_page + 1);
                                }
                            })
                        };
                        html! {
                            <>
                                <table class="records-table">
                                    <thead>
                                        <tr>
                                            <th>{ "담당자" }</th>
                                            <th>{ "부서" }</th>
                                            <th>{ "상태" }</th>
                                            <th>{ "점검예정일" }</th>
                                            <th>{ "점검일" }</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        { for resp.items.iter().map(inspection_row) }
                                    </tbody>
                                </table>
                                <div class="records-pagination">
                                    <button class="fb-more-btn" onclick={on_prev} disabled={cur_page <= 1}>{ "이전" }</button>
                                    <span>{ format!("{cur_page} / {total_pages}") }</span>
                                    <button class="fb-more-btn" onclick={on_next} disabled={cur_page >= total_pages}>{ "다음" }</button>
                                </div>
                            </>
                        }
                    }
                }
            };
            html! {
                <>
                    { status_chips }
                    { table }
                </>
            }
        }
        RecordsKind::Education => {
            let status_chips = {
                let cur = (*status_filter).clone();
                let make = |val: &'static str, label: &'static str| {
                    let status_filter = status_filter.clone();
                    let page = page.clone();
                    let active = cur == val;
                    let on_click = Callback::from(move |_: MouseEvent| {
                        status_filter.set(val.to_string());
                        page.set(1);
                    });
                    html! {
                        <button class={classes!("fb-period-chip", active.then_some("active"))} onclick={on_click}>
                            { label }
                        </button>
                    }
                };
                html! {
                    <div class="fb-period-bar">
                        { make("", "전체") }
                        { make("incomplete", "미이수") }
                        { make("complete", "이수") }
                    </div>
                }
            };

            let table = match &*education {
                EducationView::Loading => html! { <div class="admin-fb-status">{ "불러오는 중..." }</div> },
                EducationView::Error(e) => status_error_html(e),
                EducationView::Loaded(resp) => {
                    if resp.items.is_empty() {
                        html! { <div class="fb-empty">{ "교육이수 내역이 없습니다" }</div> }
                    } else {
                        let total_pages = ((resp.total.max(1) - 1) / PAGE_SIZE) + 1;
                        let cur_page = resp.page;
                        let page_state = page.clone();
                        let on_prev = {
                            let page_state = page_state.clone();
                            Callback::from(move |_: MouseEvent| {
                                if cur_page > 1 {
                                    page_state.set(cur_page - 1);
                                }
                            })
                        };
                        let on_next = {
                            let page_state = page_state.clone();
                            Callback::from(move |_: MouseEvent| {
                                if cur_page < total_pages {
                                    page_state.set(cur_page + 1);
                                }
                            })
                        };
                        html! {
                            <>
                                <table class="records-table">
                                    <thead>
                                        <tr>
                                            <th>{ "이름" }</th>
                                            <th>{ "부서" }</th>
                                            <th>{ "이수완료일" }</th>
                                            <th>{ "시험합격여부" }</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        { for resp.items.iter().map(education_row) }
                                    </tbody>
                                </table>
                                <div class="records-pagination">
                                    <button class="fb-more-btn" onclick={on_prev} disabled={cur_page <= 1}>{ "이전" }</button>
                                    <span>{ format!("{cur_page} / {total_pages}") }</span>
                                    <button class="fb-more-btn" onclick={on_next} disabled={cur_page >= total_pages}>{ "다음" }</button>
                                </div>
                            </>
                        }
                    }
                }
            };
            html! {
                <>
                    { status_chips }
                    { table }
                </>
            }
        }
        RecordsKind::Environment => {
            let status_chips = {
                let cur = (*status_filter).clone();
                let make = |val: &'static str, label: &'static str| {
                    let status_filter = status_filter.clone();
                    let page = page.clone();
                    let active = cur == val;
                    let on_click = Callback::from(move |_: MouseEvent| {
                        status_filter.set(val.to_string());
                        page.set(1);
                    });
                    html! {
                        <button class={classes!("fb-period-chip", active.then_some("active"))} onclick={on_click}>
                            { label }
                        </button>
                    }
                };
                html! {
                    <div class="fb-period-bar">
                        { make("", "전체") }
                        { make("exceed", "초과") }
                    </div>
                }
            };

            let table = match &*environment {
                EnvironmentView::Loading => html! { <div class="admin-fb-status">{ "불러오는 중..." }</div> },
                EnvironmentView::Error(e) => status_error_html(e),
                EnvironmentView::Loaded(resp) => {
                    if resp.items.is_empty() {
                        html! { <div class="fb-empty">{ "작업환경측정 내역이 없습니다" }</div> }
                    } else {
                        let total_pages = ((resp.total.max(1) - 1) / PAGE_SIZE) + 1;
                        let cur_page = resp.page;
                        let page_state = page.clone();
                        let on_prev = {
                            let page_state = page_state.clone();
                            Callback::from(move |_: MouseEvent| {
                                if cur_page > 1 {
                                    page_state.set(cur_page - 1);
                                }
                            })
                        };
                        let on_next = {
                            let page_state = page_state.clone();
                            Callback::from(move |_: MouseEvent| {
                                if cur_page < total_pages {
                                    page_state.set(cur_page + 1);
                                }
                            })
                        };
                        html! {
                            <>
                                <table class="records-table">
                                    <thead>
                                        <tr>
                                            <th>{ "제목" }</th>
                                            <th>{ "사업장" }</th>
                                            <th>{ "측정일" }</th>
                                            <th>{ "측정종류" }</th>
                                            <th>{ "초과건수" }</th>
                                            <th>{ "상태" }</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        { for resp.items.iter().map(environment_row) }
                                    </tbody>
                                </table>
                                <div class="records-pagination">
                                    <button class="fb-more-btn" onclick={on_prev} disabled={cur_page <= 1}>{ "이전" }</button>
                                    <span>{ format!("{cur_page} / {total_pages}") }</span>
                                    <button class="fb-more-btn" onclick={on_next} disabled={cur_page >= total_pages}>{ "다음" }</button>
                                </div>
                            </>
                        }
                    }
                }
            };
            html! {
                <>
                    { status_chips }
                    { table }
                </>
            }
        }
        RecordsKind::NearMiss => {
            let status_chips = {
                let cur = (*status_filter).clone();
                let make = |val: &'static str, label: &'static str| {
                    let status_filter = status_filter.clone();
                    let page = page.clone();
                    let active = cur == val;
                    let on_click = Callback::from(move |_: MouseEvent| {
                        status_filter.set(val.to_string());
                        page.set(1);
                    });
                    html! {
                        <button class={classes!("fb-period-chip", active.then_some("active"))} onclick={on_click}>
                            { label }
                        </button>
                    }
                };
                html! {
                    <div class="fb-period-bar">
                        { make("", "전체") }
                        { make("delayed", "지연") }
                    </div>
                }
            };

            let table = match &*nearmiss {
                NearMissView::Loading => html! { <div class="admin-fb-status">{ "불러오는 중..." }</div> },
                NearMissView::Error(e) => status_error_html(e),
                NearMissView::Loaded(resp) => {
                    if resp.items.is_empty() {
                        html! { <div class="fb-empty">{ "아차사고 내역이 없습니다" }</div> }
                    } else {
                        let total_pages = ((resp.total.max(1) - 1) / PAGE_SIZE) + 1;
                        let cur_page = resp.page;
                        let page_state = page.clone();
                        let on_prev = {
                            let page_state = page_state.clone();
                            Callback::from(move |_: MouseEvent| {
                                if cur_page > 1 {
                                    page_state.set(cur_page - 1);
                                }
                            })
                        };
                        let on_next = {
                            let page_state = page_state.clone();
                            Callback::from(move |_: MouseEvent| {
                                if cur_page < total_pages {
                                    page_state.set(cur_page + 1);
                                }
                            })
                        };
                        html! {
                            <>
                                <table class="records-table">
                                    <thead>
                                        <tr>
                                            <th>{ "발생장소" }</th>
                                            <th>{ "신고자" }</th>
                                            <th>{ "발생일" }</th>
                                            <th>{ "조치상태" }</th>
                                            <th>{ "조치내용" }</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        { for resp.items.iter().map(nearmiss_row) }
                                    </tbody>
                                </table>
                                <div class="records-pagination">
                                    <button class="fb-more-btn" onclick={on_prev} disabled={cur_page <= 1}>{ "이전" }</button>
                                    <span>{ format!("{cur_page} / {total_pages}") }</span>
                                    <button class="fb-more-btn" onclick={on_next} disabled={cur_page >= total_pages}>{ "다음" }</button>
                                </div>
                            </>
                        }
                    }
                }
            };
            html! {
                <>
                    { status_chips }
                    { table }
                </>
            }
        }
        RecordsKind::Committee => {
            let status_chips = {
                let cur = (*status_filter).clone();
                let make = |val: &'static str, label: &'static str| {
                    let status_filter = status_filter.clone();
                    let page = page.clone();
                    let active = cur == val;
                    let on_click = Callback::from(move |_: MouseEvent| {
                        status_filter.set(val.to_string());
                        page.set(1);
                    });
                    html! {
                        <button class={classes!("fb-period-chip", active.then_some("active"))} onclick={on_click}>
                            { label }
                        </button>
                    }
                };
                html! {
                    <div class="fb-period-bar">
                        { make("", "전체") }
                        { make("pending", "미이행") }
                    </div>
                }
            };

            let table = match &*committee {
                CommitteeView::Loading => html! { <div class="admin-fb-status">{ "불러오는 중..." }</div> },
                CommitteeView::Error(e) => status_error_html(e),
                CommitteeView::Loaded(resp) => {
                    if resp.items.is_empty() {
                        html! { <div class="fb-empty">{ "위원회 안건이 없습니다" }</div> }
                    } else {
                        let total_pages = ((resp.total.max(1) - 1) / PAGE_SIZE) + 1;
                        let cur_page = resp.page;
                        let page_state = page.clone();
                        let on_prev = {
                            let page_state = page_state.clone();
                            Callback::from(move |_: MouseEvent| {
                                if cur_page > 1 {
                                    page_state.set(cur_page - 1);
                                }
                            })
                        };
                        let on_next = {
                            let page_state = page_state.clone();
                            Callback::from(move |_: MouseEvent| {
                                if cur_page < total_pages {
                                    page_state.set(cur_page + 1);
                                }
                            })
                        };
                        html! {
                            <>
                                <table class="records-table">
                                    <thead>
                                        <tr>
                                            <th>{ "안건" }</th>
                                            <th>{ "장소" }</th>
                                            <th>{ "개최일" }</th>
                                            <th>{ "서명상태" }</th>
                                            <th>{ "회의구분" }</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        { for resp.items.iter().map(committee_row) }
                                    </tbody>
                                </table>
                                <div class="records-pagination">
                                    <button class="fb-more-btn" onclick={on_prev} disabled={cur_page <= 1}>{ "이전" }</button>
                                    <span>{ format!("{cur_page} / {total_pages}") }</span>
                                    <button class="fb-more-btn" onclick={on_next} disabled={cur_page >= total_pages}>{ "다음" }</button>
                                </div>
                            </>
                        }
                    }
                }
            };
            html! {
                <>
                    { status_chips }
                    { table }
                </>
            }
        }
        RecordsKind::Feedback => match &*feedback {
            FeedbackView::Loading => html! { <div class="admin-fb-status">{ "불러오는 중..." }</div> },
            FeedbackView::Error(e) => status_error_html(e),
            FeedbackView::Loaded(items) => {
                let kpis = html! {
                    <div class="fb-kpi-grid">
                        <div class="fb-kpi-card">
                            <span class="fb-kpi-num">{ items.len().to_string() }</span>
                            <span class="fb-kpi-label">{ "내가 남긴 피드백 수" }</span>
                        </div>
                    </div>
                };
                let table = if items.is_empty() {
                    html! { <div class="fb-empty">{ "남긴 피드백이 없습니다" }</div> }
                } else {
                    html! {
                        <table class="records-table">
                            <thead>
                                <tr>
                                    <th>{ "날짜" }</th>
                                    <th>{ "평가" }</th>
                                    <th>{ "코멘트" }</th>
                                </tr>
                            </thead>
                            <tbody>
                                { for items.iter().map(feedback_comment_row) }
                            </tbody>
                        </table>
                    }
                };
                html! {
                    <>
                        { kpis }
                        <section class="fb-section">
                            <h3 class="fb-section-title">{ "내 피드백 내역" }</h3>
                            { table }
                        </section>
                    </>
                }
            }
        },
    };

    // 채팅으로 돌아가기.
    let on_back = Callback::from(move |_: MouseEvent| {
        if let Some(window) = web_sys::window() {
            let _ = window.location().set_href("/");
        }
    });

    let on_theme_toggle = {
        let is_dark_mode = is_dark_mode.clone();
        Callback::from(move |_: MouseEvent| is_dark_mode.set(!*is_dark_mode))
    };

    // Next 관리자 대시보드 로그인 페이지로 새 탭 이동. 비밀번호는 절대 포함하지 않고
    // employeeId만 쿼리로 프리필한다(로그인 화면이 프리필을 지원하지 않으면 그냥 무시됨).
    let admin_link = api::admin_dashboard_login_url((*auth_user).as_ref().map(|u| u.id.as_str()));
    let admin_btn = match admin_link {
        Some(url) => html! {
            <a class="modal-btn records-admin-link" href={url} target="_blank" rel="noopener noreferrer">
                { "Next 관리자 페이지에서 자세히 보기" }
            </a>
        },
        None => html! {},
    };

    let root_class = classes!("admin-fb-root", if *is_dark_mode { "dark" } else { "" });

    html! {
        <div class={root_class}>
            <header class="admin-fb-header">
                <button class="admin-fb-back" onclick={on_back} title="채팅으로 돌아가기">
                    <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
                        <path d="m15 18-6-6 6-6"></path>
                    </svg>
                </button>
                <span class="admin-fb-logo">{ "EHS AI" }</span>
                <span class="admin-fb-subtitle">{ format!("{} · {}", agent_display_name(&agent_id), config.title) }</span>
                <button class="theme-toggle-btn admin-fb-theme-toggle" onclick={on_theme_toggle} title={if *is_dark_mode { "라이트 모드로 변경" } else { "다크 모드로 변경" }}>
                    { if *is_dark_mode {
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
                    } }
                </button>
            </header>
            <main class="admin-fb-main">
                { body }
                <div class="records-footer">
                    <p class="records-footer-note">{ "자세한 내용은 관리자 페이지를 참조해주세요." }</p>
                    { admin_btn }
                </div>
            </main>
        </div>
    }
}
