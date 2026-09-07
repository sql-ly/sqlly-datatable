//! The `ChartCanvas` GPUI widget: renders one [`ChartState`] as a
//! canvas-painted chart and routes clicks into the click-to-navigate
//! hit-testing path.

use crate::grid::paint::grid_font;
use crate::grid::selection::to_grid_relative;

use gpui::{
    canvas, div, point, px, App, Bounds, Context, Entity, FocusHandle, Focusable, Hsla,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Render, Styled, Window,
};

use super::paint::ChartPaint;
use super::state::ChartState;

/// Default width of the chart controls sidebar.
pub const DEFAULT_CHART_SIDEBAR_WIDTH: f32 = 240.0;

/// Canvas widget rendering one [`ChartState`].
pub struct ChartCanvas {
    /// The shared chart state. The sidebar mutates the same entity.
    pub state: Entity<ChartState>,
}

/// What the canvas paints this frame: a plot, or the honest empty-state
/// message with its (theme-resolved) text color. The plot snapshot is boxed
/// — it is far larger than the message variant.
enum CanvasData {
    /// The resolved plot snapshot (also cached on the state for hit testing).
    Paint(Box<ChartPaint>),
    /// Nothing to chart — render this message centered.
    Empty {
        /// Human-facing explanation.
        message: String,
        /// Muted text color from the host theme.
        text_color: Hsla,
    },
}

impl ChartCanvas {
    /// Wrap an existing chart state entity.
    #[must_use]
    pub fn new(state: Entity<ChartState>) -> Self {
        Self { state }
    }
}

impl Focusable for ChartCanvas {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.state.read(cx).focus_handle.clone()
    }
}

impl Render for ChartCanvas {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state_canvas = self.state.clone();
        let state_down = self.state.clone();
        let bg = self.state.read(cx).theme.bg;
        let focus_handle = self.state.read(cx).focus_handle.clone();
        let focus_down = focus_handle.clone();
        let navigate_on_click = self.state.read(cx).config.navigate_on_click;

        let element = div()
            .size_full()
            .relative()
            .track_focus(&focus_handle)
            .bg(bg)
            .child(
                canvas(
                    move |bounds, _window, cx| -> CanvasData {
                        state_canvas.update(cx, |s, _cx| {
                            if s.bounds != bounds {
                                s.bounds = bounds;
                            }
                        });
                        let s = state_canvas.read(cx);
                        match s.paint_for() {
                            Ok(paint) => {
                                state_canvas.update(cx, |s, _cx| {
                                    s.last_paint = Some(paint.clone());
                                });
                                CanvasData::Paint(Box::new(paint))
                            }
                            Err(message) => CanvasData::Empty {
                                message: message.to_string(),
                                text_color: s.theme.muted_text,
                            },
                        }
                    },
                    move |bounds, data, window, cx| match data {
                        CanvasData::Paint(paint) => paint.paint(bounds, window, cx),
                        CanvasData::Empty {
                            message,
                            text_color,
                        } => {
                            paint_empty_message(&message, text_color, bounds, window, cx);
                        }
                    },
                )
                .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                move |event: &gpui::MouseDownEvent, window, cx| {
                    window.focus(&focus_down, cx);
                    let rel = to_grid_relative(event.position, state_down.read(cx).bounds.origin);
                    state_down.update(cx, |s, cx| {
                        if s.request_navigate_at(f32::from(rel.x), f32::from(rel.y)) {
                            cx.notify();
                        }
                    });
                },
            );
        if navigate_on_click {
            element.cursor_pointer()
        } else {
            element
        }
    }
}

/// Paint the honest empty-state message centered in `bounds`.
fn paint_empty_message(
    message: &str,
    text_color: Hsla,
    bounds: Bounds<gpui::Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let font = grid_font();
    let run = gpui::TextRun {
        len: message.len(),
        color: text_color,
        font,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let text_system = window.text_system().clone();
    let line = text_system.shape_line(message.to_owned().into(), px(14.0), &[run], None);
    let line_w = f32::from(line.width);
    let center_x = f32::from(bounds.origin.x) + (f32::from(bounds.size.width) - line_w) / 2.0;
    let center_y = f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0 - 7.0;
    let _ = line.paint(
        point(px(center_x), px(center_y)),
        px(20.0),
        gpui::TextAlign::Left,
        None,
        window,
        cx,
    );
}
