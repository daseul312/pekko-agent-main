//! 답변 내 데이터 시각화 컴포넌트.
//!
//! 백엔드 `visualize` 도구가 만든 `ChartData`를 받아, 마운트 후 JS `renderChart`
//! (index.html, Chart.js 기반)를 호출해 `<canvas>`에 차트를 그린다. AI가 짚어준
//! 이상 신호(`insight`)는 "확인 필요" 콜아웃으로 차트 아래 표시한다.
//!
//! 기존 표 다운로드(`downloadTableAsCSV`)와 동일하게 CDN JS 라이브러리를
//! `#[wasm_bindgen] extern`으로 호출하는 패턴이다.

use wasm_bindgen::prelude::*;
use yew::prelude::*;

use crate::types::ChartData;

#[wasm_bindgen]
extern "C" {
    /// index.html에 정의된 전역 함수. (canvas_id, ChartData JSON) → Chart.js 렌더.
    #[wasm_bindgen(js_namespace = window)]
    fn renderChart(canvas_id: &str, spec_json: &str);
}

#[derive(Properties, PartialEq)]
pub struct ChartBlockProps {
    /// 부모 메시지 id. canvas id를 `chart-{message_id}`로 만들어 충돌을 피한다.
    pub message_id: String,
    pub data: ChartData,
}

#[function_component(ChartBlock)]
pub fn chart_block(props: &ChartBlockProps) -> Html {
    let canvas_id = format!("chart-{}", props.message_id);

    // 마운트/데이터 변경 시 Chart.js로 (재)렌더. canvas가 DOM에 커밋된 뒤 실행된다.
    {
        let canvas_id = canvas_id.clone();
        let data = props.data.clone();
        use_effect_with((canvas_id.clone(), data.clone()), move |(id, data)| {
            match serde_json::to_string(data) {
                Ok(spec) => renderChart(id, &spec),
                Err(e) => log::error!("ChartBlock: 차트 직렬화 실패: {e}"),
            }
            || ()
        });
    }

    let insight_html = props.data.insight.as_ref().map_or_else(
        || html! {},
        |ins| {
            let (level_class, title) = match ins.level.as_str() {
                "alert" => ("level-alert", "확인 필요"),
                "warn" => ("level-warn", "확인 필요"),
                _ => ("level-info", "참고"),
            };
            html! {
                <div class={classes!("chart-insight", level_class)}>
                    <span class="chart-insight-icon">{ "⚠" }</span>
                    <div>
                        <div style="font-weight:600;">{ title }</div>
                        <div>{ &ins.text }</div>
                    </div>
                </div>
            }
        },
    );

    html! {
        <div class="chart-container">
            <div class="chart-title">{ &props.data.title }</div>
            <div class="chart-canvas-wrap">
                <canvas id={canvas_id}></canvas>
            </div>
            { insight_html }
        </div>
    }
}
