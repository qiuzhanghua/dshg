use std::path::Path;

use async_channel::{Receiver, Sender};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::Disableable;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use gpui_kit::prelude::*;

use crate::backend;
use crate::config::Config;
use crate::connection::http::UnaryClient;
use crate::protocol::methods::{self, SessionSummary};

#[derive(Debug, Clone)]
pub enum AppEvent {
    Connected {
        authority: String,
        sessions: Vec<SessionSummary>,
    },
    Failed {
        code: String,
        message: String,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    NeedsAuth,
    Connecting,
    Ready,
}

struct ErrorView {
    code: String,
    message: String,
}

pub struct AppState {
    #[allow(dead_code)]
    config: Config,
    phase: Phase,
    url_input: Entity<InputState>,
    sessions: Vec<SessionSummary>,
    error: Option<ErrorView>,
    authority: Option<String>,
    events_tx: Sender<AppEvent>,
}

impl AppState {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config = Config::load_or_default(&Config::path());
        let url_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("粘贴 dsh web 打印的 URL（含 token）")
        });
        let (events_tx, events_rx): (Sender<AppEvent>, Receiver<AppEvent>) =
            async_channel::unbounded();
        let incoming = events_rx.clone();
        cx.spawn(async move |this, cx| {
            while let Ok(event) = incoming.recv().await {
                let _ = this.update(cx, |state, cx| {
                    state.apply(event);
                    cx.notify();
                });
            }
        })
        .detach();

        Self {
            config,
            phase: Phase::NeedsAuth,
            url_input,
            sessions: Vec::new(),
            error: None,
            authority: None,
            events_tx,
        }
    }

    fn apply(&mut self, event: AppEvent) {
        match event {
            AppEvent::Connected {
                authority,
                sessions,
            } => {
                self.phase = Phase::Ready;
                self.authority = Some(authority);
                self.sessions = sessions;
                self.error = None;
            }
            AppEvent::Failed { code, message } => {
                self.phase = Phase::NeedsAuth;
                self.error = Some(ErrorView { code, message });
            }
        }
    }

    fn connect(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.phase == Phase::Connecting {
            return;
        }
        let input = self.url_input.read(cx).value().to_string();
        let target = input.trim().to_string();
        if target.is_empty() {
            self.error = Some(ErrorView {
                code: "empty-url".to_string(),
                message: "请先粘贴 dsh web 打印的 URL".to_string(),
            });
            cx.notify();
            return;
        }
        self.phase = Phase::Connecting;
        self.error = None;
        cx.notify();

        let tx = self.events_tx.clone();
        backend::runtime().spawn(async move {
            let event = match connect_flow(&target).await {
                Ok((authority, sessions)) => AppEvent::Connected {
                    authority,
                    sessions,
                },
                Err(error) => AppEvent::Failed {
                    code: error.code().to_string(),
                    message: error.to_string(),
                },
            };
            tx.send(event).await.ok();
        });
    }

    fn render_onboarding(&self, cx: &mut Context<Self>, muted: Hsla, danger: Hsla) -> AnyElement {
        let connecting = self.phase == Phase::Connecting;
        let mut column = div()
            .flex()
            .flex_col()
            .w(px(620.))
            .gap(px(14.))
            .child(div().text_size(px(28.)).child("dshg"))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(muted)
                    .child("连接本机已运行的 DeepSeek Harness"),
            )
            .child(div().w_full().child(Input::new(&self.url_input).w_full()))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        Button::new("connect")
                            .primary()
                            .label(if connecting { "连接中…" } else { "连接" })
                            .disabled(connecting)
                            .on_click(cx.listener(Self::connect)),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(muted)
                            .child("启动：dsh --profile web"),
                    ),
            );

        if let Some(error) = &self.error {
            column = column.child(
                div()
                    .text_size(px(12.))
                    .text_color(danger)
                    .child(format!("[{}] {}", error.code, error.message)),
            );
        }

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(column)
            .into_any_element()
    }

    fn render_shell(
        &self,
        cx: &mut Context<Self>,
        muted: Hsla,
        border: Hsla,
        accent: Hsla,
    ) -> AnyElement {
        let authority = self.authority.clone().unwrap_or_default();

        let mut list = div().flex().flex_col().gap(px(6.));
        if self.sessions.is_empty() {
            list = list.child(
                div()
                    .text_size(px(12.))
                    .text_color(muted)
                    .child("暂无会话"),
            );
        }
        for session in &self.sessions {
            list = list.child(session_row(session, muted, border, accent));
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px(px(14.))
                    .py(px(10.))
                    .border_b(px(1.))
                    .border_color(border)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.))
                            .child(div().text_size(px(15.)).child("dshg"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(muted)
                                    .child(authority),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(muted)
                            .child(format!("{} 个会话", self.sessions.len())),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.))
                    .child(
                        div()
                            .w(px(280.))
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .p(px(12.))
                            .border_r(px(1.))
                            .border_color(border)
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(muted)
                                    .child("会话"),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_h(px(0.))
                                    .overflow_y_scrollbar()
                                    .child(list),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(muted)
                                    .child("选择会话后在此显示（M1）"),
                            ),
                    ),
            )
            .into_any_element()
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn session_row(session: &SessionSummary, muted: Hsla, border: Hsla, accent: Hsla) -> AnyElement {
    let label = session
        .cwd
        .as_deref()
        .and_then(|cwd| Path::new(cwd).file_name())
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| session.session_id.clone());

    let mut row = div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .px(px(10.))
        .py(px(8.))
        .rounded(px(6.))
        .border(px(1.))
        .border_color(border)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(13.))
                        .child(format!("#{}", short_id(&session.session_id))),
                )
                .child(if session.running {
                    div()
                        .text_size(px(11.))
                        .text_color(accent)
                        .child("running")
                        .into_any_element()
                } else {
                    div().into_any_element()
                }),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(muted)
                .child(label),
        );
    if session.blank {
        row = row.child(
            div()
                .text_size(px(11.))
                .text_color(muted)
                .child("空会话"),
        );
    }
    row.into_any_element()
}

async fn connect_flow(
    input: &str,
) -> Result<(String, Vec<SessionSummary>), crate::error::AppError> {
    let (target, cookie) = backend::authenticate(input).await?;
    let unary = UnaryClient::new(target.base.clone(), Some(cookie.clone()))?;
    let sessions = methods::session_list(&unary).await?;
    Ok((cookie.authority, sessions))
}

impl Render for AppState {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors;
        let background = colors.background;
        let foreground = colors.foreground;
        let muted = colors.muted_foreground;
        let border = colors.border;
        let accent = colors.accent;
        let danger = colors.danger;

        let body = if self.phase == Phase::Ready {
            self.render_shell(cx, muted, border, accent)
        } else {
            self.render_onboarding(cx, muted, danger)
        };

        div()
            .size_full()
            .bg(background)
            .text_color(foreground)
            .child(body)
    }
}
