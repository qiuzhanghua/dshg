use std::collections::HashSet;
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
use dshg::connection::events::{self, EventsMessage};
use dshg::connection::follow;
use dshg::connection::http::UnaryClient;
use dshg::protocol::methods::{self, SessionSummary};
use dshg::secrets::SessionCookie;
use dshg::transcript::{
    apply_events, diffs_from_meta, parse_frame, tool_summary, FileDiff, TranscriptEvent,
    TranscriptItem,
};

#[derive(Debug, Clone)]
pub enum AppEvent {
    Connected {
        authority: String,
        sessions: Vec<SessionSummary>,
        base: Url,
        cookie: SessionCookie,
    },
    Frame(Value),
    Events(EventsMessage),
    ApprovalDone {
        event_id: String,
        ok: bool,
        message: Option<String>,
    },
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

#[derive(Clone)]
struct ApprovalView {
    event_id: String,
    tool_name: String,
    reason: Option<String>,
}

pub struct AppState {
    #[allow(dead_code)]
    config: Config,
    phase: Phase,
    url_input: Entity<InputState>,
    sessions: Vec<SessionSummary>,
    transcript: Vec<TranscriptItem>,
    expanded: HashSet<String>,
    live_assistant: String,
    prompt_input: Entity<InputState>,
    sending: bool,
    notice: Option<String>,
    following: Option<String>,
    base: Option<Url>,
    cookie: Option<SessionCookie>,
    follow_task: Option<tokio::task::JoinHandle<()>>,
    events_task: Option<tokio::task::JoinHandle<()>>,
    client_id: Option<String>,
    approvals: Vec<ApprovalView>,
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

        let mut state = Self {
            config,
            phase: Phase::NeedsAuth,
            url_input,
            sessions: Vec::new(),
            transcript: Vec::new(),
            expanded: HashSet::new(),
            live_assistant: String::new(),
            prompt_input,
            sending: false,
            notice: None,
            following: None,
            base: None,
            cookie: None,
            follow_task: None,
            events_task: None,
            client_id: None,
            approvals: Vec::new(),
            error: None,
            authority: None,
            events_tx,
        };

        if std::env::var("DSHG_DEMO_TRANSCRIPT").is_ok() {
            state.phase = Phase::Ready;
            state.following = Some("demo".to_string());
            state.authority = Some("demo".to_string());
            state.transcript = demo_transcript();
        }
        state
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
                self.base = Some(base.clone());
                self.cookie = Some(cookie.clone());
                self.error = None;
                if let Some(task) = self.events_task.take() {
                    task.abort();
                }
                self.approvals.clear();
                self.client_id = None;
                let (event_tx, event_rx) = async_channel::unbounded::<EventsMessage>();
                let (event_base, event_cookie) = (base.clone(), cookie.clone());
                self.events_task = Some(backend::runtime().spawn(async move {
                    let _ = events::events_loop(&event_base, &event_cookie, event_tx).await;
                }));
                cx.spawn(async move |this, cx| {
                    while let Ok(message) = event_rx.recv().await {
                        let _ = this.update(cx, |state, cx| {
                            state.apply(AppEvent::Events(message), cx);
                            cx.notify();
                        });
                    }
                })
                .detach();
                if let Ok(session_id) = std::env::var("DSHG_OPEN_SESSION")
                    && self.sessions.iter().any(|s| s.session_id == session_id)
                {
                    self.open_session(session_id, cx);
                }
            }
            AppEvent::Frame(value) => {
                self.push_frame(value);
            }
            AppEvent::Events(message) => match message {
                EventsMessage::Ready { client_id } => {
                    self.client_id = Some(client_id);
                }
                EventsMessage::Approval {
                    event_id,
                    tool_name,
                    reason,
                } => {
                    if !self.approvals.iter().any(|item| item.event_id == event_id) {
                        self.approvals.push(ApprovalView {
                            event_id,
                            tool_name,
                            reason,
                        });
                    }
                }
                EventsMessage::Emit { .. } => {}
            },
            AppEvent::ApprovalDone {
                event_id,
                ok,
                message,
            } => {
                if ok {
                    self.approvals.retain(|item| item.event_id != event_id);
                } else {
                    self.notice = message;
                }
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
        let (is_snapshot, events) = parse_frame(&value);
        if is_snapshot {
            self.transcript.clear();
            self.live_assistant.clear();
        }
        if events
            .iter()
            .any(|event| matches!(event, TranscriptEvent::Assistant(_)))
        {
            self.live_assistant.clear();
        }
        apply_events(&mut self.transcript, events);
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

    fn respond(
        &mut self,
        event_id: String,
        outcome: &'static str,
        cx: &mut Context<Self>,
    ) {
        let (Some(base), Some(cookie), Some(client_id)) = (
            self.base.clone(),
            self.cookie.clone(),
            self.client_id.clone(),
        ) else {
            self.notice = Some("尚未收到 events ready，无法回应审批".to_string());
            cx.notify();
            return;
        };
        self.notice = None;
        let tx = self.events_tx.clone();
        backend::runtime().spawn(async move {
            let result = async {
                let client = UnaryClient::new(base, Some(cookie))?;
                methods::respond_approval(&client, &client_id, &event_id, outcome).await
            }
            .await;
            let event = match result {
                Ok(()) => AppEvent::ApprovalDone {
                    event_id,
                    ok: true,
                    message: None,
                },
                Err(error) => AppEvent::ApprovalDone {
                    event_id,
                    ok: false,
                    message: Some(error.to_string()),
                },
            };
            tx.send(event).await.ok();
        });
    }

    fn render_approvals(
        &self,
        cx: &mut Context<Self>,
        border: Hsla,
        accent: Hsla,
        muted: Hsla,
    ) -> AnyElement {
        if self.approvals.is_empty() {
            return div().into_any_element();
        }
        let mut column = div().flex().flex_col().gap(px(6.)).px(px(16.)).py(px(8.));
        for approval in &self.approvals {
            let event_id = approval.event_id.clone();
            let allow_id = event_id.clone();
            column = column.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .p(px(10.))
                    .rounded_md()
                    .border(px(1.))
                    .border_color(accent)
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(accent)
                            .child(format!("审批请求 · {}", approval.tool_name)),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(muted)
                            .child(approval.reason.clone().unwrap_or_default()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(8.))
                            .child(
                                Button::new(format!("allow-{allow_id}"))
                                    .primary()
                                    .label("允许一次")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.respond(allow_id.clone(), "allowed-once", cx);
                                    })),
                            )
                            .child(
                                Button::new(format!("reject-{event_id}"))
                                    .label("拒绝")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.respond(event_id.clone(), "rejected", cx);
                                    })),
                            ),
                    ),
            );
        }
        column.into_any_element()
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
        success: Hsla,
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
                            .child(self.render_transcript(cx, muted, accent, success, danger))
                        .child(self.render_approvals(cx, border, accent, muted))
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

    fn toggle_expanded(&mut self, call_id: String, cx: &mut Context<Self>) {
        if !self.expanded.remove(&call_id) {
            self.expanded.insert(call_id);
        }
        cx.notify();
    }

    fn render_item(
        &self,
        index: usize,
        item: &TranscriptItem,
        cx: &mut Context<Self>,
        muted: Hsla,
        accent: Hsla,
        success: Hsla,
        danger: Hsla,
    ) -> AnyElement {
        match item {
            TranscriptItem::User(text) => div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(div().text_size(px(11.)).text_color(muted).child("你"))
                .child(
                    TextView::markdown(
                        ElementId::Name(format!("user-{index}").into()),
                        text.clone(),
                    )
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
            TranscriptItem::Tool(call) => {
                let summary = tool_summary(&call.name, &call.arguments);
                let status = match &call.result {
                    Some(result) if result.error => "error",
                    Some(_) => "done",
                    None => "running",
                };
                let expanded = self.expanded.contains(&call.call_id);
                let text = call
                    .result
                    .as_ref()
                    .map(|result| result.text.clone())
                    .unwrap_or_default();
                let long = text.chars().count() > 600;
                let shown = if expanded {
                    truncate(&text, 6000)
                } else {
                    truncate(&text, 600)
                };
                let diffs = call
                    .result
                    .as_ref()
                    .and_then(|result| result.meta.as_deref())
                    .map(diffs_from_meta)
                    .unwrap_or_default();

                let toggle_id = call.call_id.clone();
                let copy_text = text.clone();

                let mut card = div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .p(px(8.))
                    .rounded_md()
                    .border(px(1.))
                    .border_color(muted.opacity(0.25))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(muted)
                                    .child(format!("tool · {}", call.name)),
                            )
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(if status == "error" { danger } else { muted })
                                    .child(status),
                            ),
                    );
                if !summary.is_empty() {
                    card = card.child(
                        div()
                            .text_size(px(11.))
                            .child(truncate(&summary, 300)),
                    );
                }
                if !text.is_empty() {
                    card = card.child(div().text_size(px(11.)).child(shown));
                }
                if !text.is_empty() {
                    let mut actions = div().flex().flex_row().gap(px(6.)).child(
                        Button::new(format!("copy-{}", call.call_id))
                            .compact()
                            .label("复制")
                            .on_click(cx.listener(move |_this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                            })),
                    );
                    if long {
                        actions = actions.child(
                            Button::new(format!("toggle-{}", call.call_id))
                                .compact()
                                .label(if expanded { "收起" } else { "展开" })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.toggle_expanded(toggle_id.clone(), cx);
                                })),
                        );
                    }
                    card = card.child(actions);
                }
                for diff in &diffs {
                    card = card.child(render_diff(diff, muted, success, danger));
                }
                card.into_any_element()
            }
        }
    }

    fn render_transcript(
        &self,
        cx: &mut Context<Self>,
        muted: Hsla,
        accent: Hsla,
        success: Hsla,
        danger: Hsla,
    ) -> AnyElement {
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
            column = column.child(self.render_item(index, item, cx, muted, accent, success, danger));
        }
        if !self.live_assistant.is_empty() {
            let live = TranscriptItem::Assistant(self.live_assistant.clone());
            column = column.child(self.render_item(self.transcript.len(), &live, cx, muted, accent, success, danger));
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

fn demo_transcript() -> Vec<TranscriptItem> {
    let meta = serde_json::json!({
        "diffs": [{
            "path": "src/demo.rs",
            "oldText": "fn main() {\n    old();\n}",
            "newText": "fn main() {\n    new();\n}",
        }]
    })
    .to_string();
    vec![
        TranscriptItem::User("改一下 demo".to_string()),
        TranscriptItem::Assistant("好的，我先看一下文件。".to_string()),
        TranscriptItem::Tool(dshg::transcript::ToolCall {
            call_id: "demo-call".to_string(),
            name: "edit".to_string(),
            arguments: r#"{"path":"src/demo.rs"}"#.to_string(),
            result: Some(dshg::transcript::ToolResult {
                text: "applied 1 hunk".to_string(),
                error: false,
                meta: Some(meta),
            }),
        }),
    ]
}

fn render_diff(diff: &FileDiff, muted: Hsla, success: Hsla, danger: Hsla) -> AnyElement {
    let mut block = div()
        .flex()
        .flex_col()
        .gap(px(1.))
        .p(px(6.))
        .rounded(px(4.))
        .border(px(1.))
        .border_color(muted.opacity(0.2))
        .child(
            div()
                .text_size(px(10.))
                .text_color(muted)
                .child(diff.path.clone()),
        );
    if let Some(old) = &diff.old_text {
        for line in old.lines().take(40) {
            block = block.child(
                div()
                    .text_size(px(10.))
                    .text_color(danger)
                    .child(format!("- {line}")),
            );
        }
    }
    for line in diff.new_text.lines().take(40) {
        block = block.child(
            div()
                .text_size(px(10.))
                .text_color(success)
                .child(format!("+ {line}")),
        );
    }
    block.into_any_element()
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
        let success = colors.success;

        let body = if self.phase == Phase::Ready {
            self.render_shell(cx, muted, border, accent, success, danger)
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

