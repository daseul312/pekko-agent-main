use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct SuggestionsProps {
    pub suggestions: Vec<String>,
    pub on_click: Callback<String>,
}

#[function_component(Suggestions)]
pub fn suggestions(props: &SuggestionsProps) -> Html {
    let on_click = props.on_click.clone();

    html! {
        <div class="suggestions-container">
            { for props.suggestions.iter().map(|s| {
                let s = s.clone();
                let onclick = {
                    let on_click = on_click.clone();
                    let s = s.clone();
                    Callback::from(move |_| on_click.emit(s.clone()))
                };
                html! {
                    <button class="suggestion-chip" onclick={onclick}>
                        { s }
                    </button>
                }
            }) }
        </div>
    }
}
