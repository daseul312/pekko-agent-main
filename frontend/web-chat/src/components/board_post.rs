use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{File, HtmlInputElement, HtmlSelectElement};
use yew::prelude::*;

use crate::api;
use crate::types::{
    BoardPostCompletion, BoardPostCreateRequest, BoardPostDetail, BoardPostExistingAttachment,
    BoardPostListItem, BoardPostMenu, BoardPostTarget, BoardPostUpdateRequest,
};

const MAX_ATTACHMENT_SIZE_BYTES: f64 = 20.0 * 1024.0 * 1024.0;
const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "avi", "wmv", "mkv", "mov", "webm", "flv", "asf", "3gp",
];

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = initBoardPostEditor)]
    fn init_board_post_editor(element_id: &str, initial_data: &str, on_change: &JsValue);

    #[wasm_bindgen(js_namespace = window, js_name = destroyBoardPostEditor)]
    fn destroy_board_post_editor(element_id: &str);

    #[wasm_bindgen(js_namespace = window, js_name = getBoardPostEditorData)]
    fn get_board_post_editor_data(element_id: &str) -> String;

    #[wasm_bindgen(js_namespace = window, js_name = setBoardPostEditorDisabled)]
    fn set_board_post_editor_disabled(element_id: &str, disabled: bool);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BoardPostFlow {
    Create,
    Edit,
    Delete,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BoardPostEntryMode {
    Create,
    Edit,
}

#[derive(Properties, PartialEq)]
pub struct BoardPostMenuFormProps {
    pub token: Option<String>,
    pub on_target_selected: Callback<BoardPostTarget>,
    pub on_board_post_complete: Callback<BoardPostCompletion>,
    #[prop_or_default]
    pub selected_target: Option<BoardPostTarget>,
    #[prop_or(BoardPostFlow::Create)]
    pub mode: BoardPostFlow,
}

#[function_component(BoardPostMenuForm)]
pub fn board_post_menu_form(props: &BoardPostMenuFormProps) -> Html {
    let menus = use_state(Vec::<BoardPostMenu>::new);
    let is_loading = use_state(|| true);
    let error = use_state(|| None::<String>);
    let selected_top_id = use_state(|| {
        props
            .selected_target
            .as_ref()
            .and_then(|target| target.top_menu_id)
    });
    let selected_middle_id = use_state(|| {
        props
            .selected_target
            .as_ref()
            .and_then(|target| target.middle_menu_id)
    });
    let selected_sub_id = use_state(|| {
        props
            .selected_target
            .as_ref()
            .and_then(|target| target.sub_menu_id)
    });
    let is_done = use_state(|| props.selected_target.is_some());
    let confirmed_target = use_state(|| props.selected_target.clone());

    {
        let selected_top_id = selected_top_id.clone();
        let selected_middle_id = selected_middle_id.clone();
        let selected_sub_id = selected_sub_id.clone();
        let is_done = is_done.clone();
        let confirmed_target = confirmed_target.clone();
        let menus = (*menus).clone();
        use_effect_with(props.selected_target.clone(), move |selected_target| {
            if let Some(target) = selected_target.as_ref() {
                let (top_id, middle_id, sub_id) = board_post_menu_path(&menus, target);
                selected_top_id.set(top_id);
                selected_middle_id.set(middle_id);
                selected_sub_id.set(sub_id);
                is_done.set(true);
                confirmed_target.set(Some(target.clone()));
            }
            || ()
        });
    }

    {
        let menus = menus.clone();
        let is_loading = is_loading.clone();
        let error = error.clone();
        let selected_top_id = selected_top_id.clone();
        let selected_middle_id = selected_middle_id.clone();
        let selected_sub_id = selected_sub_id.clone();
        let is_done = is_done.clone();
        let selected_target = props.selected_target.clone();
        let token = props.token.clone();
        use_effect_with(token, move |token| {
            let menus = menus.clone();
            let is_loading = is_loading.clone();
            let error = error.clone();
            let selected_top_id = selected_top_id.clone();
            let selected_middle_id = selected_middle_id.clone();
            let selected_sub_id = selected_sub_id.clone();
            let selected_target = selected_target.clone();
            let token = token.clone();
            spawn_local(async move {
                is_loading.set(true);
                match api::fetch_board_post_menus(token.as_deref()).await {
                    Ok(list) => {
                        error.set(None);
                        if let Some(target) = selected_target.as_ref() {
                            selected_top_id.set(target.top_menu_id);
                            selected_middle_id.set(target.middle_menu_id);
                            selected_sub_id.set(target.sub_menu_id);
                            is_done.set(true);
                        } else {
                            selected_top_id.set(None);
                            selected_middle_id.set(None);
                            selected_sub_id.set(None);
                        }
                        menus.set(list);
                    }
                    Err(err) => {
                        menus.set(Vec::new());
                        error.set(Some(err));
                        selected_top_id.set(None);
                        selected_middle_id.set(None);
                        selected_sub_id.set(None);
                    }
                }
                is_loading.set(false);
            });
            || ()
        });
    }

    let top_menus = child_menus(&menus, None);
    let selected_top_value = *selected_top_id;
    let selected_middle_value = *selected_middle_id;
    let selected_sub_value = *selected_sub_id;
    let middle_menus = selected_top_value
        .as_ref()
        .copied()
        .map(|parent_id| child_menus(&menus, Some(parent_id)))
        .unwrap_or_default();
    let sub_menus = selected_middle_value
        .as_ref()
        .copied()
        .map(|parent_id| child_menus(&menus, Some(parent_id)))
        .unwrap_or_default();
    let target_menu = selected_sub_value
        .as_ref()
        .copied()
        .or_else(|| selected_middle_value.as_ref().copied())
        .or_else(|| selected_top_value.as_ref().copied())
        .and_then(|id| find_menu(&menus, id))
        .filter(|menu| menu.selectable)
        .filter(|menu| {
            props.mode == BoardPostFlow::Create || is_editable_board_name(&menu.menu_name)
        });
    let can_confirm = target_menu.is_some() && !*is_done;

    let on_top_change = {
        let selected_top_id = selected_top_id.clone();
        let selected_middle_id = selected_middle_id.clone();
        let selected_sub_id = selected_sub_id.clone();
        Callback::from(move |e: Event| {
            let select = e.target_unchecked_into::<HtmlSelectElement>();
            selected_top_id.set(parse_select_value(&select.value()));
            selected_middle_id.set(None);
            selected_sub_id.set(None);
        })
    };

    let on_middle_change = {
        let selected_middle_id = selected_middle_id.clone();
        let selected_sub_id = selected_sub_id.clone();
        Callback::from(move |e: Event| {
            let select = e.target_unchecked_into::<HtmlSelectElement>();
            selected_middle_id.set(parse_select_value(&select.value()));
            selected_sub_id.set(None);
        })
    };

    let on_sub_change = {
        let selected_sub_id = selected_sub_id.clone();
        Callback::from(move |e: Event| {
            let select = e.target_unchecked_into::<HtmlSelectElement>();
            selected_sub_id.set(parse_select_value(&select.value()));
        })
    };

    let on_confirm = {
        let menus = menus.clone();
        let selected_top_id = selected_top_id.clone();
        let selected_middle_id = selected_middle_id.clone();
        let selected_sub_id = selected_sub_id.clone();
        let is_done = is_done.clone();
        let confirmed_target = confirmed_target.clone();
        let mode = props.mode;
        Callback::from(move |_: MouseEvent| {
            if *is_done {
                return;
            }

            let selected_top_value = *selected_top_id;
            let selected_middle_value = *selected_middle_id;
            let selected_sub_value = *selected_sub_id;
            let Some(menu) = selected_sub_value
                .as_ref()
                .copied()
                .or_else(|| selected_middle_value.as_ref().copied())
                .or_else(|| selected_top_value.as_ref().copied())
                .and_then(|id| find_menu(&menus, id))
                .filter(|menu| menu.selectable)
                .filter(|menu| {
                    mode == BoardPostFlow::Create || is_editable_board_name(&menu.menu_name)
                })
            else {
                return;
            };

            let top_menu_name = selected_top_value
                .as_ref()
                .copied()
                .and_then(|id| find_menu(&menus, id))
                .map(|menu| menu.menu_name);
            let middle_menu_name = selected_middle_value
                .as_ref()
                .copied()
                .and_then(|id| find_menu(&menus, id))
                .map(|menu| menu.menu_name);
            let sub_menu_name = selected_sub_value
                .as_ref()
                .copied()
                .and_then(|id| find_menu(&menus, id))
                .map(|menu| menu.menu_name);

            let target = BoardPostTarget {
                menu_id: menu.menu_id,
                board_name: menu.menu_name,
                top_menu_id: selected_top_value,
                middle_menu_id: selected_middle_value,
                sub_menu_id: selected_sub_value,
                top_menu_name,
                middle_menu_name,
                sub_menu_name,
            };

            is_done.set(true);
            confirmed_target.set(Some(target));
        })
    };

    if let Some(target) = (*confirmed_target).clone() {
        return match props.mode {
            BoardPostFlow::Create => html! {
                <BoardPostEntryForm
                    target={target}
                    token={props.token.clone()}
                    on_complete={props.on_board_post_complete.clone()}
                    mode={BoardPostEntryMode::Create}
                />
            },
            BoardPostFlow::Edit => html! {
                <BoardPostEditList
                    target={target}
                    token={props.token.clone()}
                    on_complete={props.on_board_post_complete.clone()}
                    mode={BoardPostFlow::Edit}
                />
            },
            BoardPostFlow::Delete => html! {
                <BoardPostEditList
                    target={target}
                    token={props.token.clone()}
                    on_complete={props.on_board_post_complete.clone()}
                    mode={BoardPostFlow::Delete}
                />
            },
        };
    }

    html! {
        <div class="board-post-card">
            <div class="board-post-title">{ board_post_menu_title(props.mode) }</div>
            if *is_loading {
                <div class="board-post-status">{ "메뉴를 불러오는 중입니다." }</div>
            } else if let Some(err) = &*error {
                <div class="board-post-error">{ err }</div>
            } else {
                if props.mode != BoardPostFlow::Create {
                    <div class="board-post-help">{ board_post_menu_help(props.mode) }</div>
                }
                <div class="board-post-menu-row">
                    <select class="board-post-select"
                        value={selected_top_value.map(|id| id.to_string()).unwrap_or_default()}
                        onchange={on_top_change}
                        disabled={*is_done}>
                        <option value="" selected={selected_top_value.is_none()}>{ "대메뉴 선택" }</option>
                        { for top_menus.iter().map(menu_option) }
                    </select>
                    <select class="board-post-select"
                        value={selected_middle_value.map(|id| id.to_string()).unwrap_or_default()}
                        onchange={on_middle_change}
                        disabled={*is_done || middle_menus.is_empty()}>
                        <option value="" selected={selected_middle_value.is_none()}>{ "중메뉴 선택" }</option>
                        { for middle_menus.iter().map(menu_option) }
                    </select>
                    <select class="board-post-select"
                        value={selected_sub_value.map(|id| id.to_string()).unwrap_or_default()}
                        onchange={on_sub_change}
                        disabled={*is_done || sub_menus.is_empty()}>
                        <option value="" selected={selected_sub_value.is_none()}>{ "소메뉴 선택" }</option>
                        { for sub_menus.iter().map(menu_option) }
                    </select>
                </div>
                <div class="board-post-actions">
                    <button class="board-post-confirm" onclick={on_confirm} disabled={!can_confirm}>{ "확인" }</button>
                </div>
            }
        </div>
    }
}

#[derive(Properties, PartialEq)]
struct BoardPostEditListProps {
    pub target: BoardPostTarget,
    pub token: Option<String>,
    pub on_complete: Callback<BoardPostCompletion>,
    pub mode: BoardPostFlow,
}

#[function_component(BoardPostEditList)]
fn board_post_edit_list(props: &BoardPostEditListProps) -> Html {
    let posts = use_state(Vec::<BoardPostListItem>::new);
    let is_loading = use_state(|| true);
    let is_loading_detail = use_state(|| false);
    let is_deleting = use_state(|| false);
    let is_confirming_delete = use_state(|| false);
    let is_done = use_state(|| false);
    let error = use_state(|| None::<String>);
    let selected_post_id = use_state(|| None::<i32>);
    let selected_detail = use_state(|| None::<BoardPostDetail>);

    {
        let posts = posts.clone();
        let is_loading = is_loading.clone();
        let error = error.clone();
        let selected_post_id = selected_post_id.clone();
        let selected_detail = selected_detail.clone();
        let target = props.target.clone();
        let token = props.token.clone();
        use_effect_with((target.menu_id, token), move |(menu_id, token)| {
            let posts = posts.clone();
            let is_loading = is_loading.clone();
            let error = error.clone();
            let selected_post_id = selected_post_id.clone();
            let selected_detail = selected_detail.clone();
            let token = token.clone();
            let menu_id = *menu_id;
            spawn_local(async move {
                is_loading.set(true);
                selected_post_id.set(None);
                selected_detail.set(None);
                match api::fetch_board_posts(menu_id, token.as_deref()).await {
                    Ok(list) => {
                        posts.set(list);
                        error.set(None);
                    }
                    Err(err) => {
                        posts.set(Vec::new());
                        error.set(Some(err));
                    }
                }
                is_loading.set(false);
            });
            || ()
        });
    }

    if props.mode == BoardPostFlow::Edit {
        if let Some(detail) = (*selected_detail).clone() {
            return html! {
                <BoardPostEntryForm
                    target={props.target.clone()}
                    token={props.token.clone()}
                    on_complete={props.on_complete.clone()}
                    mode={BoardPostEntryMode::Edit}
                    initial_post={Some(detail)}
                />
            };
        }
    }

    if *is_done {
        return html! {
            <div class="board-post-card">
                <div class="board-post-status">{ "삭제되었습니다." }</div>
            </div>
        };
    }

    let can_confirm = (*selected_post_id).is_some() && !*is_loading_detail && !*is_deleting;
    let on_confirm = {
        let target = props.target.clone();
        let token = props.token.clone();
        let selected_post_id = selected_post_id.clone();
        let selected_detail = selected_detail.clone();
        let is_loading_detail = is_loading_detail.clone();
        let is_confirming_delete = is_confirming_delete.clone();
        let error = error.clone();
        let mode = props.mode;
        Callback::from(move |_: MouseEvent| {
            if *is_loading_detail {
                return;
            }

            let Some(post_id) = *selected_post_id else {
                return;
            };

            if mode == BoardPostFlow::Delete {
                error.set(None);
                is_confirming_delete.set(true);
                return;
            }

            let target = target.clone();
            let token = token.clone();
            let selected_detail = selected_detail.clone();
            let is_loading_detail = is_loading_detail.clone();
            let error = error.clone();
            spawn_local(async move {
                is_loading_detail.set(true);
                match api::fetch_board_post(target.menu_id, post_id, token.as_deref()).await {
                    Ok(detail) => {
                        selected_detail.set(Some(detail));
                        error.set(None);
                    }
                    Err(err) => {
                        error.set(Some(err));
                    }
                }
                is_loading_detail.set(false);
            });
        })
    };
    let on_confirm_delete = {
        let target = props.target.clone();
        let token = props.token.clone();
        let selected_post_id = selected_post_id.clone();
        let is_deleting = is_deleting.clone();
        let is_confirming_delete = is_confirming_delete.clone();
        let is_done = is_done.clone();
        let error = error.clone();
        let on_complete = props.on_complete.clone();
        Callback::from(move |_: MouseEvent| {
            if *is_deleting {
                return;
            }

            let Some(post_id) = *selected_post_id else {
                is_confirming_delete.set(false);
                return;
            };

            is_deleting.set(true);
            error.set(None);
            let target = target.clone();
            let token = token.clone();
            let is_deleting = is_deleting.clone();
            let is_confirming_delete = is_confirming_delete.clone();
            let is_done = is_done.clone();
            let error = error.clone();
            let on_complete = on_complete.clone();
            spawn_local(async move {
                let completion =
                    match api::delete_board_post(target.menu_id, post_id, token.as_deref()).await {
                        Ok(resp) => BoardPostCompletion {
                            board_name: resp.board_name.clone(),
                            message: "삭제되었습니다.".to_string(),
                            success: true,
                        },
                        Err(err) => BoardPostCompletion {
                            board_name: target.board_name.clone(),
                            message: format!(
                                "[{}] 게시글 삭제가 다음과 같은 사유로 실패했습니다.\n{}",
                                target.board_name, err
                            ),
                            success: false,
                        },
                    };

                is_deleting.set(false);
                is_confirming_delete.set(false);
                if completion.success {
                    is_done.set(true);
                    on_complete.emit(completion);
                } else {
                    error.set(Some(completion.message.clone()));
                }
            });
        })
    };
    let on_cancel_delete = {
        let is_confirming_delete = is_confirming_delete.clone();
        Callback::from(move |_: MouseEvent| {
            is_confirming_delete.set(false);
        })
    };

    html! {
        <div class="board-post-card">
            <div class="board-post-title">{ board_post_list_title(props.mode, &props.target.board_name) }</div>
            if *is_loading {
                <div class="board-post-status">{ "게시글 목록을 불러오는 중입니다." }</div>
            } else if let Some(err) = &*error {
                <div class="board-post-error">{ err }</div>
            } else if posts.is_empty() {
                <div class="board-post-status">{ empty_post_list_message(props.mode) }</div>
            } else {
                <div class="board-post-list">
                    { for posts.iter().map(|post| {
                        let post_id = post.post_id;
                        let selected_post_id = selected_post_id.clone();
                        let checked = *selected_post_id == Some(post_id);
                        html! {
                            <label class="board-post-list-item">
                                <input
                                    type="radio"
                                    name={format!("board-post-{}-{}", props.mode.input_name(), props.target.menu_id)}
                                    checked={checked}
                                    onchange={Callback::from(move |_: Event| selected_post_id.set(Some(post_id)))}
                                />
                                <span class="board-post-list-main">
                                    <span class="board-post-list-title">{ &post.title }</span>
                                    <span class="board-post-list-meta">{ format!("#{} · {} · {}", post.bno, post.employeename, post.created_at) }</span>
                                </span>
                            </label>
                        }
                    }) }
                </div>
                if *is_confirming_delete {
                    <div class="board-post-confirm-panel">
                        <span>{ "삭제하시겠습니까?" }</span>
                        <div class="board-post-actions">
                            <button class="board-post-confirm" onclick={on_confirm_delete} disabled={*is_deleting}>{ if *is_deleting { "삭제 중..." } else { "확인" } }</button>
                            <button class="board-post-confirm board-post-cancel" onclick={on_cancel_delete} disabled={*is_deleting}>{ "아니오" }</button>
                        </div>
                    </div>
                } else {
                    <div class="board-post-actions">
                        <button class="board-post-confirm" onclick={on_confirm} disabled={!can_confirm}>
                            if *is_loading_detail {
                                { "불러오는 중..." }
                            } else {
                                { board_post_list_confirm_label(props.mode) }
                            }
                        </button>
                    </div>
                }
            }
        </div>
    }
}

#[derive(Properties, PartialEq)]
pub struct BoardPostEntryFormProps {
    pub target: BoardPostTarget,
    pub token: Option<String>,
    pub on_complete: Callback<BoardPostCompletion>,
    #[prop_or(BoardPostEntryMode::Create)]
    mode: BoardPostEntryMode,
    #[prop_or_default]
    initial_post: Option<BoardPostDetail>,
}

#[function_component(BoardPostEntryForm)]
pub fn board_post_entry_form(props: &BoardPostEntryFormProps) -> Html {
    let editor_id = use_state(|| format!("board-post-editor-{}", uuid::Uuid::new_v4().simple()));
    let initial_post = props.initial_post.clone();
    let initial_contents = initial_post
        .as_ref()
        .map(|post| post.contents.clone())
        .unwrap_or_default();
    let title = use_state(|| {
        initial_post
            .as_ref()
            .map(|post| post.title.clone())
            .unwrap_or_default()
    });
    let contents = use_state(|| initial_contents.clone());
    let is_fixed = use_state(|| {
        initial_post
            .as_ref()
            .map(|post| post.is_fixed)
            .unwrap_or(false)
    });
    let fixed_start_date = use_state(|| {
        initial_post
            .as_ref()
            .and_then(|post| post.fixed_start_date.clone())
    });
    let fixed_end_date = use_state(|| {
        initial_post
            .as_ref()
            .and_then(|post| post.fixed_end_date.clone())
    });
    let files = use_state(Vec::<File>::new);
    let error = use_state(|| None::<String>);
    let is_submitting = use_state(|| false);
    let is_done = use_state(|| false);
    let is_confirming = use_state(|| false);
    let is_cancelled = use_state(|| false);

    {
        let editor_id = (*editor_id).clone();
        let contents = contents.clone();
        let error = error.clone();
        let initial_contents = initial_contents.clone();
        use_effect_with(editor_id.clone(), move |editor_id| {
            let contents = contents.clone();
            let error = error.clone();
            let on_change = Closure::<dyn FnMut(String)>::wrap(Box::new(move |data: String| {
                contents.set(data);
            }));

            init_board_post_editor(editor_id, &initial_contents, on_change.as_ref());

            let editor_id = editor_id.clone();
            move || {
                destroy_board_post_editor(&editor_id);
                drop(on_change);
                error.set(None);
            }
        });
    }

    {
        let editor_id = (*editor_id).clone();
        let disabled = *is_submitting || *is_done || *is_confirming;
        use_effect_with(disabled, move |disabled| {
            set_board_post_editor_disabled(&editor_id, *disabled);
            || ()
        });
    }

    let on_title_input = {
        let title = title.clone();
        Callback::from(move |e: InputEvent| {
            let input = e.target_unchecked_into::<HtmlInputElement>();
            title.set(input.value().trim_start().to_string());
        })
    };

    let on_fixed_off = {
        let is_fixed = is_fixed.clone();
        let fixed_start_date = fixed_start_date.clone();
        let fixed_end_date = fixed_end_date.clone();
        Callback::from(move |_: Event| {
            is_fixed.set(false);
            fixed_start_date.set(None);
            fixed_end_date.set(None);
        })
    };

    let on_fixed_on = {
        let is_fixed = is_fixed.clone();
        Callback::from(move |_: Event| {
            is_fixed.set(true);
        })
    };

    let on_fixed_start_change = {
        let fixed_start_date = fixed_start_date.clone();
        Callback::from(move |e: InputEvent| {
            let input = e.target_unchecked_into::<HtmlInputElement>();
            fixed_start_date.set(non_empty_string(input.value()));
        })
    };

    let on_fixed_end_change = {
        let fixed_end_date = fixed_end_date.clone();
        Callback::from(move |e: InputEvent| {
            let input = e.target_unchecked_into::<HtmlInputElement>();
            fixed_end_date.set(non_empty_string(input.value()));
        })
    };

    let on_file_change = {
        let files = files.clone();
        let error = error.clone();
        Callback::from(move |e: Event| {
            let input = e.target_unchecked_into::<HtmlInputElement>();
            let selected = match validate_selected_files(&input) {
                Ok(selected) => selected,
                Err(err) => {
                    error.set(Some(err));
                    input.set_value("");
                    return;
                }
            };

            if !selected.is_empty() {
                let mut next = (*files).clone();
                next.extend(selected);
                files.set(next);
                error.set(None);
            }
            input.set_value("");
        })
    };

    let on_submit = {
        let editor_id = (*editor_id).clone();
        let title = title.clone();
        let contents = contents.clone();
        let is_fixed = is_fixed.clone();
        let fixed_start_date = fixed_start_date.clone();
        let fixed_end_date = fixed_end_date.clone();
        let files = files.clone();
        let error = error.clone();
        let is_confirming = is_confirming.clone();
        let is_submitting = is_submitting.clone();
        let is_done = is_done.clone();
        let target = props.target.clone();
        let token = props.token.clone();
        let on_complete = props.on_complete.clone();
        let mode = props.mode;
        let initial_post = props.initial_post.clone();
        Callback::from(move |_: MouseEvent| {
            if *is_submitting || *is_done {
                return;
            }

            let title_value = title.trim().to_string();
            let editor_value = get_board_post_editor_data(&editor_id);
            let contents_value = if editor_value.trim().is_empty() {
                contents.trim().to_string()
            } else {
                editor_value.trim().to_string()
            };

            if title_value.is_empty() || contents_value.is_empty() {
                error.set(Some("제목과 내용을 입력해주세요.".to_string()));
                return;
            }

            if mode == BoardPostEntryMode::Edit && initial_post.is_none() {
                error.set(Some("수정할 게시글 정보를 찾을 수 없습니다.".to_string()));
                return;
            }

            if !*is_confirming {
                error.set(None);
                is_confirming.set(true);
                return;
            }

            error.set(None);
            is_confirming.set(false);
            is_submitting.set(true);

            let is_submitting_done = is_submitting.clone();
            let is_done_done = is_done.clone();
            let on_complete_done = on_complete.clone();
            let board_name = target.board_name.clone();
            let token = token.clone();
            let fixed_start_date_value = (*fixed_start_date).clone();
            let fixed_end_date_value = (*fixed_end_date).clone();
            let is_fixed_value = *is_fixed;
            let target_menu_id = target.menu_id;

            match mode {
                BoardPostEntryMode::Create => {
                    let request = BoardPostCreateRequest {
                        menu_id: target_menu_id,
                        title: title_value,
                        contents: contents_value,
                        is_fixed: is_fixed_value,
                        fixed_start_date: fixed_start_date_value,
                        fixed_end_date: fixed_end_date_value,
                        attachments: Vec::new(),
                    };
                    let selected_files = (*files).clone();
                    spawn_local(async move {
                        let completion = match api::create_board_post_form(
                            &request,
                            &selected_files,
                            token.as_deref(),
                        )
                        .await
                        {
                            Ok(resp) => BoardPostCompletion {
                                board_name: resp.board_name.clone(),
                                message: "등록되었습니다.".to_string(),
                                success: true,
                            },
                            Err(err) => BoardPostCompletion {
                                board_name: board_name.clone(),
                                message: format!(
                                    "[{}] 게시글 등록이 다음과 같은 사유로 실패했습니다.\n{}",
                                    board_name, err
                                ),
                                success: false,
                            },
                        };
                        is_submitting_done.set(false);
                        if completion.success {
                            is_done_done.set(true);
                        }
                        on_complete_done.emit(completion);
                    });
                }
                BoardPostEntryMode::Edit => {
                    let Some(post) = initial_post.clone() else {
                        is_submitting.set(false);
                        error.set(Some("수정할 게시글 정보를 찾을 수 없습니다.".to_string()));
                        return;
                    };
                    let request = BoardPostUpdateRequest {
                        title: title_value,
                        contents: contents_value,
                        is_fixed: is_fixed_value,
                        fixed_start_date: fixed_start_date_value,
                        fixed_end_date: fixed_end_date_value,
                    };
                    spawn_local(async move {
                        let completion = match api::update_board_post(
                            target_menu_id,
                            post.post_id,
                            &request,
                            token.as_deref(),
                        )
                        .await
                        {
                            Ok(resp) => BoardPostCompletion {
                                board_name: resp.board_name.clone(),
                                message: "수정되었습니다.".to_string(),
                                success: true,
                            },
                            Err(err) => BoardPostCompletion {
                                board_name: board_name.clone(),
                                message: format!(
                                    "[{}] 게시글 수정이 다음과 같은 사유로 실패했습니다.\n{}",
                                    board_name, err
                                ),
                                success: false,
                            },
                        };
                        is_submitting_done.set(false);
                        if completion.success {
                            is_done_done.set(true);
                        }
                        on_complete_done.emit(completion);
                    });
                }
            }
        })
    };

    let on_confirm_no = {
        let is_confirming = is_confirming.clone();
        Callback::from(move |_: MouseEvent| {
            is_confirming.set(false);
        })
    };

    let on_cancel = {
        let is_done = is_done.clone();
        let is_cancelled = is_cancelled.clone();
        let target = props.target.clone();
        let on_complete = props.on_complete.clone();
        let mode = props.mode;
        Callback::from(move |_: MouseEvent| {
            if *is_done {
                return;
            }

            is_done.set(true);
            is_cancelled.set(true);
            on_complete.emit(BoardPostCompletion {
                board_name: target.board_name.clone(),
                message: cancel_message(mode).to_string(),
                success: false,
            });
        })
    };

    let disabled = *is_submitting || *is_done || *is_confirming;

    if *is_cancelled {
        return html! {
            <div class="board-post-card">
                <div class="board-post-status">{ cancel_message(props.mode) }</div>
            </div>
        };
    }

    html! {
        <div class="board-post-card">
            <div class="board-post-title">{ entry_title(props.mode, &props.target.board_name) }</div>
            <div class="board-post-field">
                <label>{ "제목" }</label>
                <input type="text" value={(*title).clone()} oninput={on_title_input} disabled={disabled} maxlength="200" />
            </div>
            <div class="board-post-field">
                <label>{ "내용" }</label>
                <div class="board-post-editor-shell">
                    <textarea id={(*editor_id).clone()} class="board-post-editor"></textarea>
                </div>
            </div>
            <div class="board-post-field">
                <label>{ "첨부파일" }</label>
                if props.mode == BoardPostEntryMode::Create {
                    <input type="file" multiple=true onchange={on_file_change} disabled={disabled} />
                    if !files.is_empty() {
                        <div class="board-post-file-list">
                            { for files.iter().enumerate().map(|(index, file)| file_chip(index, file, files.clone(), disabled)) }
                        </div>
                    }
                    <div class="board-post-help">{ "파일 업로드는 20MB까지 가능합니다." }</div>
                } else {
                    { render_existing_attachments(props.initial_post.as_ref()) }
                }
            </div>
            <div class="board-post-fixed">
                <span>{ "상단고정여부" }</span>
                <label>
                    <input type="radio" name={format!("fixed-{}-{}", props.mode.input_name(), props.target.menu_id)} checked={!*is_fixed} onchange={on_fixed_off} disabled={disabled} />
                    { "고정 안 함" }
                </label>
                <label>
                    <input type="radio" name={format!("fixed-{}-{}", props.mode.input_name(), props.target.menu_id)} checked={*is_fixed} onchange={on_fixed_on} disabled={disabled} />
                    { "상단 고정" }
                </label>
                if *is_fixed {
                    <div class="board-post-fixed-dates">
                        <span>{ "고정 기간" }</span>
                        <input type="date" value={(*fixed_start_date).clone().unwrap_or_default()} oninput={on_fixed_start_change} disabled={disabled} />
                        <span>{ "~" }</span>
                        <input type="date" value={(*fixed_end_date).clone().unwrap_or_default()} oninput={on_fixed_end_change} disabled={disabled} />
                    </div>
                }
            </div>
            if let Some(err) = &*error {
                <div class="board-post-error">{ err }</div>
            }
            if *is_confirming {
                <div class="board-post-confirm-panel">
                    <span>{ confirm_question(props.mode) }</span>
                    <div class="board-post-actions">
                        <button class="board-post-confirm" onclick={on_submit.clone()} disabled={*is_submitting || *is_done}>{ "네" }</button>
                        <button class="board-post-confirm board-post-cancel" onclick={on_confirm_no} disabled={*is_submitting || *is_done}>{ "아니요" }</button>
                    </div>
                </div>
            } else {
                <div class="board-post-actions">
                    <button class="board-post-confirm" onclick={on_submit} disabled={disabled}>
                        if *is_submitting {
                            { submitting_label(props.mode) }
                        } else {
                            { submit_label(props.mode) }
                        }
                    </button>
                    <button class="board-post-confirm board-post-cancel" onclick={on_cancel} disabled={disabled}>{ "취소" }</button>
                </div>
            }
        </div>
    }
}

impl BoardPostEntryMode {
    fn input_name(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Edit => "edit",
        }
    }
}

impl BoardPostFlow {
    fn input_name(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Edit => "edit",
            Self::Delete => "delete",
        }
    }
}

fn validate_selected_files(input: &HtmlInputElement) -> Result<Vec<File>, String> {
    let mut selected = Vec::new();
    let Some(file_list) = input.files() else {
        return Ok(selected);
    };

    for index in 0..file_list.length() {
        let Some(file) = file_list.get(index) else {
            continue;
        };
        let name = file.name();
        let extension = name
            .rsplit('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();

        if VIDEO_EXTENSIONS.contains(&extension.as_str()) {
            return Err("동영상 파일은 업로드할 수 없습니다.".to_string());
        }

        if file.size() > MAX_ATTACHMENT_SIZE_BYTES {
            return Err("파일 크기는 20MB를 초과할 수 없습니다.".to_string());
        }

        selected.push(file);
    }

    Ok(selected)
}

fn file_chip(index: usize, file: &File, files: UseStateHandle<Vec<File>>, disabled: bool) -> Html {
    let name = file.name();
    let on_remove = Callback::from(move |_: MouseEvent| {
        if disabled {
            return;
        }

        let mut next = (*files).clone();
        if index < next.len() {
            next.remove(index);
            files.set(next);
        }
    });

    html! {
        <span class="board-post-file-chip">
            <span>{ name }</span>
            <button type="button" class="board-post-file-remove" onclick={on_remove} disabled={disabled}>{ "삭제" }</button>
        </span>
    }
}

fn render_existing_attachments(post: Option<&BoardPostDetail>) -> Html {
    let attachments: &[BoardPostExistingAttachment] = match post {
        Some(post) => post.attachments.as_slice(),
        None => &[],
    };

    if attachments.is_empty() {
        return html! { <div class="board-post-help">{ "첨부파일 없음" }</div> };
    }

    html! {
        <div class="board-post-file-list">
            { for attachments.iter().map(existing_attachment_chip) }
        </div>
    }
}

fn existing_attachment_chip(attachment: &BoardPostExistingAttachment) -> Html {
    html! {
        <span class="board-post-file-chip">
            <span>{ existing_attachment_label(attachment) }</span>
        </span>
    }
}

fn existing_attachment_label(attachment: &BoardPostExistingAttachment) -> String {
    match (attachment.fno, attachment.uuid.as_deref()) {
        (Some(fno), Some(uuid)) if !uuid.trim().is_empty() => format!("fno {} · {}", fno, uuid),
        (Some(fno), _) => format!("fno {}", fno),
        (None, Some(uuid)) if !uuid.trim().is_empty() => uuid.to_string(),
        _ => "첨부파일".to_string(),
    }
}

fn non_empty_string(value: String) -> Option<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn child_menus(menus: &[BoardPostMenu], parent_id: Option<i32>) -> Vec<BoardPostMenu> {
    menus
        .iter()
        .filter(|menu| menu.parent_menu_id == parent_id)
        .cloned()
        .collect()
}

fn find_menu(menus: &[BoardPostMenu], menu_id: i32) -> Option<BoardPostMenu> {
    menus.iter().find(|menu| menu.menu_id == menu_id).cloned()
}

fn board_post_menu_path(
    menus: &[BoardPostMenu],
    target: &BoardPostTarget,
) -> (Option<i32>, Option<i32>, Option<i32>) {
    let Some(menu) = find_menu(menus, target.menu_id) else {
        return (
            target.top_menu_id,
            target.middle_menu_id,
            target.sub_menu_id,
        );
    };

    let Some(parent_id) = menu.parent_menu_id else {
        return (Some(menu.menu_id), None, None);
    };

    let Some(parent) = find_menu(menus, parent_id) else {
        return (
            target.top_menu_id,
            target.middle_menu_id,
            target.sub_menu_id,
        );
    };

    if parent.parent_menu_id.is_none() {
        return (Some(parent.menu_id), Some(menu.menu_id), None);
    }

    let Some(top_id) = parent.parent_menu_id else {
        return (
            target.top_menu_id,
            target.middle_menu_id,
            target.sub_menu_id,
        );
    };

    (Some(top_id), Some(parent.menu_id), Some(menu.menu_id))
}

fn parse_select_value(value: &str) -> Option<i32> {
    value.parse::<i32>().ok()
}

fn menu_option(menu: &BoardPostMenu) -> Html {
    html! {
        <option value={menu.menu_id.to_string()}>{ &menu.menu_name }</option>
    }
}

fn is_editable_board_name(name: &str) -> bool {
    matches!(
        name,
        "공지" | "소식" | "위험요인 POINT" | "위험요인 및 개선현황"
    )
}

fn board_post_menu_title(mode: BoardPostFlow) -> &'static str {
    match mode {
        BoardPostFlow::Create => "어떤 게시판에 등록하기를 원하시나요?",
        BoardPostFlow::Edit => "수정할 게시판을 선택해주세요.",
        BoardPostFlow::Delete => "삭제할 게시판을 선택해주세요.",
    }
}

fn board_post_menu_help(mode: BoardPostFlow) -> &'static str {
    match mode {
        BoardPostFlow::Create => "",
        BoardPostFlow::Edit => {
            "공지, 소식, 위험요인 POINT, 위험요인 및 개선현황 게시판을 수정할 수 있습니다."
        }
        BoardPostFlow::Delete => {
            "공지, 소식, 위험요인 POINT, 위험요인 및 개선현황 게시판에서 본인이 작성한 게시글을 삭제할 수 있습니다."
        }
    }
}

fn board_post_list_title(mode: BoardPostFlow, board_name: &str) -> String {
    match mode {
        BoardPostFlow::Create => format!("{board_name} 게시글 선택"),
        BoardPostFlow::Edit => format!("{board_name} 게시글 선택"),
        BoardPostFlow::Delete => format!("{board_name} 삭제할 게시글 선택"),
    }
}

fn empty_post_list_message(mode: BoardPostFlow) -> &'static str {
    match mode {
        BoardPostFlow::Create => "게시글이 없습니다.",
        BoardPostFlow::Edit => "수정 가능한 게시글이 없습니다.",
        BoardPostFlow::Delete => "삭제 가능한 게시글이 없습니다.",
    }
}

fn board_post_list_confirm_label(mode: BoardPostFlow) -> &'static str {
    match mode {
        BoardPostFlow::Create | BoardPostFlow::Edit => "확인",
        BoardPostFlow::Delete => "삭제",
    }
}

fn entry_title(mode: BoardPostEntryMode, board_name: &str) -> String {
    match mode {
        BoardPostEntryMode::Create => format!("{board_name} 게시글 등록"),
        BoardPostEntryMode::Edit => format!("{board_name} 게시글 수정"),
    }
}

fn submit_label(mode: BoardPostEntryMode) -> &'static str {
    match mode {
        BoardPostEntryMode::Create => "확인",
        BoardPostEntryMode::Edit => "수정",
    }
}

fn submitting_label(mode: BoardPostEntryMode) -> &'static str {
    match mode {
        BoardPostEntryMode::Create => "등록 중...",
        BoardPostEntryMode::Edit => "수정 중...",
    }
}

fn confirm_question(mode: BoardPostEntryMode) -> &'static str {
    match mode {
        BoardPostEntryMode::Create => "등록하시겠습니까?",
        BoardPostEntryMode::Edit => "수정하시겠습니까?",
    }
}

fn cancel_message(mode: BoardPostEntryMode) -> &'static str {
    match mode {
        BoardPostEntryMode::Create => "사용자가 게시글 등록을 취소하였습니다",
        BoardPostEntryMode::Edit => "사용자가 게시글 수정을 취소하였습니다",
    }
}
