use std::path::Path;

use async_channel::{Receiver, Sender};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::Disableable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::text::TextView;
use gpui_kit::prelude::*;
use gpui_kit::*;
use serde_json::Value;
use url::Url;

use crate::backend;
use dshg::config::Config;
use dshg::connection::follow;
use dshg::connection::http::UnaryClient;
use dshg::protocol::methods::{self, SessionSummary};
use dshg::secrets::SessionCookie;
use dshg::transcript::{parse_frame, TranscriptItem};

#[derive(Debug, Clone)]
pub enum AppEvent {
    Connected {
        authority: String,
        sessions: Vec<SessionSummary>,
        base: Url,
        cookie: SessionCookie,
    },
    Frame(Value),
    PromptDone {
        ok: bool,
        message: Option<String>,
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
    transcript: Vec<TranscriptItem>,
    live_assistant: String,
    prompt_input: Entity<InputState>,
    sending: bool,
    notice: Option<String>,
    following: Option<String>,
    base: Option<Url>,
    cookie: Option<SessionCookie>,
    follow_task: Option<tokio::task::JoinHandle<()>>,
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
        let prompt_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("输入消息，Enter 发送（M2）")
        });
        let (events_tx, events_rx): (Sender<AppEvent>, Receiver<AppEvent>) =
            async_channel::unbounded();
        let incoming = events_rx.clone();
        cx.spawn(async move |this, cx| {
            while let Ok(event) = incoming.recv().await {
                let _ = this.update(cx, |state, cx| {
                    state.apply(event, cx);
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
            transcript: Vec::new(),
            live_assistant: String::new(),
            prompt_input,
            sending: false,
            notice: None,
            following: None,
            base: None,
            cookie: None,
            follow_task: None,
            error: None,
            authority: None,
            events_tx,
        }
    }

    fn apply(&mut self, event: AppEvent, cx: &mut Context<Self>) {
        match event {
            AppEvent::Connected {
                authority,
                sessions,
                base,
                cookie,
            } => {
                self.phase = Phase::Ready;
                self.authority = Some(authority);
                self.sessions = sessions;
                self.base = Some(base);
                self.cookie = Some(cookie);
                self.error = None;
                if let Ok(session_id) = std::env::var("DSHG_OPEN_SESSION")
                    && self.sessions.iter().any(|s| s.session_id == session_id)
                {
                    self.open_session(session_id, cx);
                }
            }
            AppEvent::Frame(value) => {
                self.push_frame(value);
            }
            AppEvent::PromptDone { ok, message } => {
                self.sending = false;
                if !ok {
                    self.notice = message;
                }
            }
            AppEvent::Failed { code, message } => {
                self.phase = Phase::NeedsAuth;
                self.error = Some(ErrorView { code, message });
            }
        }
    }

    fn push_frame(&mut self, value: Value) {
        if value.get("type").and_then(Value::as_str) == Some("assistant-stream") {
            self.apply_assistant_stream(&value);
            return;
        }
        let (is_snapshot, items) = parse_frame(&value);
        if is_snapshot {
            self.transcript.clear();
            self.live_assistant.clear();
        }
        if items
            .iter()
            .any(|item| matches!(item, TranscriptItem::Assistant(_)))
        {
            self.live_assistant.clear();
        }
        self.transcript.extend(items);
    }

    fn apply_assistant_stream(&mut self, value: &Value) {
        let Some(frame) = value.get("frame") else {
            return;
        };
        match frame.get("type").and_then(Value::as_str) {
            Some("start") => self.live_assistant.clear(),
            Some("chunk") => {
                if let Some(chunk) = frame.get("chunk")
                    && chunk.get("type").and_then(Value::as_str) == Some("text-delta")
                    && let Some(text) = chunk.get("text").and_then(Value::as_str)
                {
                    self.live_assistant.push_str(text);
                }
            }
            Some("end") => {
                if let Some(outcome) = frame.get("outcome")
                    && outcome.get("kind").and_then(Value::as_str) == Some("abandoned")
                {
                    self.live_assistant.clear();
                }
            }
            _ => {}
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
        self.start_connect(target, cx);
    }

    pub(crate) fn start_connect(&mut self, target: String, cx: &mut Context<Self>) {
        if self.phase == Phase::Connecting {
            return;
        }
        self.phase = Phase::Connecting;
        self.error = None;
        cx.notify();

        let tx = self.events_tx.clone();
        backend::runtime().spawn(async move {
            let event = match connect_flow(&target).await {
                Ok((authority, sessions, base, cookie)) => AppEvent::Connected {
                    authority,
                    sessions,
                    base,
                    cookie,
                },
                Err(error) => AppEvent::Failed {
                    code: error.code().to_string(),
                    message: error.to_string(),
                },
            };
            tx.send(event).await.ok();
        });
    }

    fn send(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.sending {
            return;
        }
        let Some(session_id) = self.following.clone() else {
            return;
        };
        let (Some(base), Some(cookie)) = (self.base.clone(), self.cookie.clone()) else {
            return;
        };
        let text = self.prompt_input.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        self.sending = true;
        self.notice = None;
        cx.notify();
        self.prompt_input
            .update(cx, |state, cx| state.set_value("", window, cx));

        let tx = self.events_tx.clone();
        backend::runtime().spawn(async move {
            let result: Result<(), dshg::error::AppError> = async {
                let client = UnaryClient::new(base, Some(cookie))?;
                let request_id = uuid::Uuid::new_v4().to_string();
                methods::send_prompt(&client, &session_id, &request_id, &text).await
            }
            .await;
            let event = match result {
                Ok(()) => AppEvent::PromptDone {
                    ok: true,
                    message: None,
                },
                Err(error) => AppEvent::PromptDone {
                    ok: false,
                    message: Some(error.to_string()),
                },
            };
            tx.send(event).await.ok();
        });
    }

    fn open_session(&mut self, session_id: String, cx: &mut Context<Self>) {
        let (Some(base), Some(cookie)) = (self.base.clone(), self.cookie.clone()) else {
            return;
        };
        if let Some(task) = self.follow_task.take() {
            task.abort();
        }
        self.transcript.clear();
        self.following = Some(session_id.clone());
        cx.notify();

        let (frame_tx, frame_rx) = async_channel::unbounded::<Value>();
        self.follow_task = Some(backend::runtime().spawn(async move {
            if let Err(error) = follow::follow_session(&base, &cookie, &session_id, frame_tx).await {
                tracing::warn!(%error, "session follow ended");
            }
        }));

        cx.spawn(async move |this, cx| {
            while let Ok(value) = frame_rx.recv().await {
                let _ = this.update(cx, |state, cx| {
                    state.push_frame(value);
                    cx.notify();
                });
            }
        })
        .detach();
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
        danger: Hsla,
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
            list = list.child(self.render_session_row(session, cx, muted, border, accent));
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
                            .flex_col()
                            .min_h(px(0.))
                            .child(self.render_transcript(cx, muted, accent))
                        .child(self.render_composer(cx, muted, border, danger)),
                    ),
            )
            .into_any_element()
    }

    fn render_session_row(
        &self,
        session: &SessionSummary,
        cx: &mut Context<Self>,
        muted: Hsla,
        border: Hsla,
        accent: Hsla,
    ) -> AnyElement {
        let label = session
            .cwd
            .as_deref()
            .and_then(|cwd| Path::new(cwd).file_name())
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| session.session_id.clone());
        let selected = self.following.as_deref() == Some(session.session_id.as_str());
        let session_id = session.session_id.clone();

        let mut row = div()
            .id(ElementId::Name(format!("session-{}", session.session_id).into()))
            .flex()
            .flex_col()
            .gap(px(3.))
            .px(px(10.))
            .py(px(8.))
            .rounded_md()
            .border(px(1.))
            .border_color(if selected { accent } else { border })
            .cursor_pointer()
            .hover(|style| style.bg(muted.opacity(0.08)))
            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.open_session(session_id.clone(), cx);
            }))
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
            .child(div().text_size(px(11.)).text_color(muted).child(label));
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

    fn render_composer(
        &self,
        cx: &mut Context<Self>,
        muted: Hsla,
        border: Hsla,
        danger: Hsla,
    ) -> AnyElement {
        if self.following.is_none() {
            return div().into_any_element();
        }
        let mut column = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .px(px(16.))
            .py(px(10.))
            .border_t(px(1.))
            .border_color(border);
        if let Some(notice) = &self.notice {
            column = column.child(
                div()
                    .text_size(px(11.))
                    .text_color(danger)
                    .child(notice.clone()),
            );
        }
        column
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().child(Input::new(&self.prompt_input).w_full()))
                    .child(
                        Button::new("send")
                            .primary()
                            .label(if self.sending { "发送中…" } else { "发送" })
                            .disabled(self.sending)
                            .on_click(cx.listener(Self::send)),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(muted)
                    .child("Enter 发送暂未绑定（M2 待补）"),
            )
            .into_any_element()
    }

    fn render_transcript(&self, _cx: &mut Context<Self>, muted: Hsla, accent: Hsla) -> AnyElement {
        if self.following.is_none() {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(muted)
                        .child("选择左侧会话开始回放"),
                )
                .into_any_element();
        }

        let mut column = div().flex().flex_col().gap(px(12.)).p(px(16.));
        if self.transcript.is_empty() {
            column = column.child(
                div()
                    .text_size(px(12.))
                    .text_color(muted)
                    .child("载入会话…"),
            );
        }
        for (index, item) in self.transcript.iter().enumerate() {
            column = column.child(render_item(index, item, muted, accent));
        }
        if !self.live_assistant.is_empty() {
            let live = TranscriptItem::Assistant(self.live_assistant.clone());
            column = column.child(render_item(self.transcript.len(), &live, muted, accent));
        }

        div()
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scrollbar()
            .child(column)
            .into_any_element()
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn render_item(index: usize, item: &TranscriptItem, muted: Hsla, accent: Hsla) -> AnyElement {
    match item {
        TranscriptItem::User(text) => div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(div().text_size(px(11.)).text_color(muted).child("你"))
            .child(
                TextView::markdown(ElementId::Name(format!("user-{index}").into()), text.clone())
                    .w_full(),
            )
            .into_any_element(),
        TranscriptItem::Assistant(text) => div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(div().text_size(px(11.)).text_color(accent).child("dsh"))
            .child(
                TextView::markdown(
                    ElementId::Name(format!("assistant-{index}").into()),
                    text.clone(),
                )
                .w_full(),
            )
            .into_any_element(),
        TranscriptItem::ToolCall { name, arguments } => div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(8.))
            .rounded_md()
            .border(px(1.))
            .border_color(muted.opacity(0.25))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(muted)
                    .child(format!("tool · {name}")),
            )
            .child(div().text_size(px(11.)).child(truncate(arguments, 300)))
            .into_any_element(),
        TranscriptItem::ToolResult { text, error } => div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(8.))
            .rounded_md()
            .border(px(1.))
            .border_color(muted.opacity(0.25))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(if *error { accent } else { muted })
                    .child(if *error { "tool result · error" } else { "tool result" }),
            )
            .child(div().text_size(px(11.)).child(truncate(text, 600)))
            .into_any_element(),
    }
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(limit).collect();
        out.push('…');
        out
    }
}

async fn connect_flow(
    input: &str,
) -> Result<(String, Vec<SessionSummary>, Url, SessionCookie), dshg::error::AppError> {
    let (target, cookie) = backend::authenticate(input).await?;
    let unary = UnaryClient::new(target.base.clone(), Some(cookie.clone()))?;
    let sessions = methods::session_list(&unary).await?;
    Ok((cookie.authority.clone(), sessions, target.base, cookie))
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
            self.render_shell(cx, muted, border, accent, danger)
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

