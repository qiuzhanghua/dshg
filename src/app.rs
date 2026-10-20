use std::collections::HashSet;
use std::rc::Rc;
use std::path::Path;

use async_channel::{Receiver, Sender};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::Disableable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{VirtualListScrollHandle, v_virtual_list};
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
use dshg::protocol::plugins::{self, ChangeResult, Inventory};
use dshg::protocol::credentials;
use dshg::protocol::files::{self, DirectoryEntry, DirectoryListing, FileText};
use dshg::protocol::settings::{self, NamespaceView, SettingsDescribe};
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
    SettingsLoaded(SettingsDescribe),
    SettingsFailed(String),
    SettingsSaved(NamespaceView),
    SettingsSaveFailed(String),
    PluginsLoaded(Inventory),
    PluginsFailed(String),
    PluginChanged {
        entry_id: String,
        enabled: bool,
        result: ChangeResult,
    },
    CredentialDone(String),
    CredentialFailed(String),
    FilesLoaded(DirectoryListing),
    FileLoaded(FileText),
    FilesFailed(String),
    FileChanged(String),
    HistoryLoaded(dshg::protocol::methods::SessionPage),
    HistoryFailed(String),
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
    show_settings: bool,
    settings: Option<SettingsDescribe>,
    settings_selected: usize,
    settings_loading: bool,
    settings_saving: bool,
    settings_error: Option<String>,
    settings_notice: Option<String>,
    settings_editor: Entity<TextareaState>,
    files_open: bool,
    files_dir: Vec<String>,
    files_listing: Option<DirectoryListing>,
    file_preview: Option<FileText>,
    files_error: Option<String>,
    files_loading: bool,
    following_cwd: Option<String>,
    preview_absolute: Option<String>,
    watch_task: Option<tokio::task::JoinHandle<()>>,
    transcript_scroll: VirtualListScrollHandle,
    history_cursor: Option<i64>,
    oldest_seq: Option<i64>,
    history_has_more: bool,
    history_loading: bool,
    plugins_open: bool,
    plugins: Option<Inventory>,
    plugins_loading: bool,
    plugins_error: Option<String>,
    plugins_notice: Option<String>,
    credentials_open: bool,
    cred_ref: Entity<InputState>,
    cred_value: Entity<InputState>,
    cred_status: Option<String>,
    cred_error: Option<String>,
    cred_busy: bool,
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
        let cred_ref = cx.new(|cx| {
            InputState::new(window, cx).placeholder("凭据引用，如 DEEPSEEK_API_KEY")
        });
        let cred_value = cx.new(|cx| {
            InputState::new(window, cx).placeholder("写入值（不会回显）")
        });
        let settings_editor = cx.new(|cx| {
            TextareaState::new(window, cx).placeholder("选择命名空间后点「载入当前值」再编辑 JSON")
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
            show_settings: false,
            settings: None,
            settings_selected: 0,
            settings_loading: false,
            settings_saving: false,
            settings_error: None,
            settings_notice: None,
            settings_editor,
            files_open: false,
            files_dir: Vec::new(),
            files_listing: None,
            file_preview: None,
            files_error: None,
            files_loading: false,
            following_cwd: None,
            preview_absolute: None,
            watch_task: None,
            transcript_scroll: VirtualListScrollHandle::new(),
            history_cursor: None,
            oldest_seq: None,
            history_has_more: false,
            history_loading: false,
            plugins_open: false,
            plugins: None,
            plugins_loading: false,
            plugins_error: None,
            plugins_notice: None,
            credentials_open: false,
            cred_ref,
            cred_value,
            cred_status: None,
            cred_error: None,
            cred_busy: false,
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

        match std::env::var("DSHG_DEMO_TRANSCRIPT").as_deref() {
            Ok("bulk") => {
                state.phase = Phase::Ready;
                state.following = Some("demo".to_string());
                state.authority = Some("demo".to_string());
                state.transcript = bulk_transcript(500);
            }
            Ok(_) => {
                state.phase = Phase::Ready;
                state.following = Some("demo".to_string());
                state.authority = Some("demo".to_string());
                state.transcript = demo_transcript();
            }
            Err(_) => {}
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
                if std::env::var("DSHG_OPEN_CREDENTIALS").is_ok() {
                    self.credentials_open = true;
                }
                if std::env::var("DSHG_OPEN_PLUGINS").is_ok() {
                    self.plugins_open = true;
                    self.load_plugins(cx);
                }
                if std::env::var("DSHG_OPEN_SETTINGS").is_ok() {
                    self.show_settings = true;
                    self.load_settings(cx);
                }
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
            AppEvent::SettingsLoaded(settings) => {
                self.settings_loading = false;
                self.settings_error = None;
                self.settings = Some(settings);
                self.settings_selected = 0;
            }
            AppEvent::SettingsFailed(message) => {
                self.settings_loading = false;
                self.settings_error = Some(message);
            }
            AppEvent::SettingsSaved(view) => {
                self.settings_saving = false;
                self.settings_notice = Some("已保存".to_string());
                self.settings_error = None;
                if let Some(settings) = &mut self.settings
                    && let Some(index) = settings
                        .namespaces
                        .iter()
                        .position(|item| item.ns == view.ns)
                {
                    settings.namespaces[index] = view;
                }
            }
            AppEvent::SettingsSaveFailed(message) => {
                self.settings_saving = false;
                let hint = if message.contains("settings/conflict") {
                    "（设置已被其他客户端修改，请先刷新）"
                } else {
                    ""
                };
                self.settings_error = Some(format!("{message}{hint}"));
            }
            AppEvent::PluginsLoaded(inventory) => {
                self.plugins_loading = false;
                self.plugins_error = None;
                self.plugins = Some(inventory);
            }
            AppEvent::PluginsFailed(message) => {
                self.plugins_loading = false;
                self.plugins_error = Some(message);
            }
            AppEvent::PluginChanged {
                entry_id,
                enabled,
                result,
            } => {
                self.plugins_loading = false;
                let application = result.application.clone().unwrap_or_default();
                if let Some(inventory) = &mut self.plugins
                    && let Some(entry) = inventory
                        .entries
                        .iter_mut()
                        .find(|entry| entry.entry_id.as_deref() == Some(entry_id.as_str()))
                {
                    entry.enabled = enabled;
                }
                match application.as_str() {
                    "applied" => self.plugins_notice = Some("已生效".to_string()),
                    "restart-required" => {
                        self.plugins_notice = Some("已保存，需要重启 dsh 生效".to_string())
                    }
                    "overridden" => {
                        self.plugins_notice =
                            Some("被更高优先级配置覆盖，未生效".to_string())
                    }
                    "failed" => {
                        self.plugins_error = Some(format!("操作失败：{:?}", result.error))
                    }
                    "cancelled" => self.plugins_notice = Some("已取消".to_string()),
                    other => {
                        self.plugins_notice = Some(format!("结果：{other}"));
                    }
                }
            }
            AppEvent::FilesLoaded(listing) => {
                self.files_loading = false;
                self.files_error = None;
                self.files_listing = Some(listing);
            }
            AppEvent::FileLoaded(file) => {
                self.files_loading = false;
                self.files_error = None;
                self.file_preview = Some(file);
            }
            AppEvent::FilesFailed(message) => {
                self.files_loading = false;
                self.files_error = Some(message);
            }
            AppEvent::HistoryLoaded(page) => {
                self.history_loading = false;
                let mut items = Vec::new();
                let mut oldest: Option<i64> = None;
                for record in &page.records {
                    if let Some(seq) = record
                        .get("event")
                        .and_then(|event| event.get("seq"))
                        .and_then(Value::as_i64)
                    {
                        oldest = Some(oldest.map_or(seq, |current| current.min(seq)));
                    }
                    if let Some(event) =
                        record.get("event").and_then(dshg::transcript::parse_event)
                    {
                        dshg::transcript::apply_events(&mut items, vec![event]);
                    }
                }
                if let Some(oldest) = oldest {
                    self.oldest_seq = Some(oldest);
                }
                items.append(&mut self.transcript);
                self.transcript = items;
                self.history_has_more = page.has_more;
            }
            AppEvent::HistoryFailed(message) => {
                self.history_loading = false;
                self.files_error = Some(message);
            }
            AppEvent::FileChanged(path) => {
                if self.preview_absolute.as_deref() == Some(path.as_str()) {
                    self.read_file(path, cx);
                }
            }
            AppEvent::CredentialDone(status) => {
                self.cred_busy = false;
                self.cred_error = None;
                self.cred_status = Some(status);
            }
            AppEvent::CredentialFailed(message) => {
                self.cred_busy = false;
                self.cred_error = Some(message);
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
            self.transcript_scroll.scroll_to_bottom();
            return;
        }
        let (is_snapshot, events) = parse_frame(&value);
        if is_snapshot {
            self.transcript.clear();
            self.live_assistant.clear();
            self.history_cursor = value.get("cursor").and_then(Value::as_i64);
            self.oldest_seq = value
                .get("records")
                .and_then(Value::as_array)
                .and_then(|records| {
                    records
                        .iter()
                        .filter_map(|record| record.get("event")?.get("seq")?.as_i64())
                        .min()
                });
            self.history_has_more = value
                .get("hasMore")
                .and_then(Value::as_bool)
                .unwrap_or(false);
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

    fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.show_settings = true;
        self.settings_error = None;
        cx.notify();
        if self.settings.is_none() && !self.settings_loading {
            self.load_settings(cx);
        }
    }

    fn load_settings(&mut self, cx: &mut Context<Self>) {
        let (Some(base), Some(cookie)) = (self.base.clone(), self.cookie.clone()) else {
            self.settings_error = Some("尚未连接引擎".to_string());
            cx.notify();
            return;
        };
        self.settings_loading = true;
        self.settings_error = None;
        cx.notify();
        let tx = self.events_tx.clone();
        backend::runtime().spawn(async move {
            let result = async {
                let client = UnaryClient::new(base, Some(cookie))?;
                settings::describe(&client).await
            }
            .await;
            let event = match result {
                Ok(settings) => AppEvent::SettingsLoaded(settings),
                Err(error) => AppEvent::SettingsFailed(error.to_string()),
            };
            tx.send(event).await.ok();
        });
    }

    fn open_settings_document(&mut self, cx: &mut Context<Self>) {
        let (Some(base), Some(cookie)) = (self.base.clone(), self.cookie.clone()) else {
            return;
        };
        let tx = self.events_tx.clone();
        backend::runtime().spawn(async move {
            let result = async {
                let client = UnaryClient::new(base, Some(cookie))?;
                settings::open_document(&client).await
            }
            .await;
            if let Err(error) = result {
                tx.send(AppEvent::SettingsFailed(error.to_string())).await.ok();
            }
        });
    }

    fn open_credentials(&mut self, cx: &mut Context<Self>) {
        self.credentials_open = true;
        self.cred_error = None;
        cx.notify();
    }

    fn check_credential(&mut self, cx: &mut Context<Self>) {
        let reference = self.cred_ref.read(cx).value().to_string();
        if reference.trim().is_empty() {
            self.cred_error = Some("请填写凭据引用".to_string());
            cx.notify();
            return;
        }
        let (Some(base), Some(cookie)) = (self.base.clone(), self.cookie.clone()) else {
            return;
        };
        self.cred_busy = true;
        self.cred_error = None;
        cx.notify();
        let tx = self.events_tx.clone();
        backend::runtime().spawn(async move {
            let result = async {
                let client = UnaryClient::new(base, Some(cookie))?;
                credentials::describe(&client, vec![reference.clone()]).await
            }
            .await;
            let event = match result {
                Ok(map) => {
                    let status = map
                        .get(&reference)
                        .map(|info| {
                            if info.configured {
                                format!(
                                    "已配置（来源：{}）",
                                    info.source.clone().unwrap_or_else(|| "未知".to_string())
                                )
                            } else {
                                "未配置".to_string()
                            }
                        })
                        .unwrap_or_else(|| "未配置".to_string());
                    AppEvent::CredentialDone(status)
                }
                Err(error) => AppEvent::CredentialFailed(error.to_string()),
            };
            tx.send(event).await.ok();
        });
    }

    fn write_credential(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reference = self.cred_ref.read(cx).value().to_string();
        let secret = self.cred_value.read(cx).value().to_string();
        if reference.trim().is_empty() || secret.is_empty() {
            self.cred_error = Some("引用和值都不能为空".to_string());
            cx.notify();
            return;
        }
        let (Some(base), Some(cookie)) = (self.base.clone(), self.cookie.clone()) else {
            return;
        };
        self.cred_busy = true;
        self.cred_error = None;
        cx.notify();
        self.cred_value
            .update(cx, |state, cx| state.set_value("", window, cx));
        let tx = self.events_tx.clone();
        backend::runtime().spawn(async move {
            let result = async {
                let client = UnaryClient::new(base, Some(cookie))?;
                credentials::set(&client, &reference, &secret).await
            }
            .await;
            let event = match result {
                Ok(()) => AppEvent::CredentialDone("已写入".to_string()),
                Err(error) => AppEvent::CredentialFailed(error.to_string()),
            };
            tx.send(event).await.ok();
        });
    }

    fn clear_credential(&mut self, cx: &mut Context<Self>) {
        let reference = self.cred_ref.read(cx).value().to_string();
        if reference.trim().is_empty() {
            self.cred_error = Some("请填写凭据引用".to_string());
            cx.notify();
            return;
        }
        let (Some(base), Some(cookie)) = (self.base.clone(), self.cookie.clone()) else {
            return;
        };
        self.cred_busy = true;
        self.cred_error = None;
        cx.notify();
        let tx = self.events_tx.clone();
        backend::runtime().spawn(async move {
            let result = async {
                let client = UnaryClient::new(base, Some(cookie))?;
                credentials::unset(&client, &reference).await
            }
            .await;
            let event = match result {
                Ok(()) => AppEvent::CredentialDone("已清除".to_string()),
                Err(error) => AppEvent::CredentialFailed(error.to_string()),
            };
            tx.send(event).await.ok();
        });
    }

    fn render_credentials(
        &self,
        cx: &mut Context<Self>,
        muted: Hsla,
        border: Hsla,
        accent: Hsla,
        danger: Hsla,
    ) -> AnyElement {
        let mut column = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .w(px(620.))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(muted)
                    .child("凭据引用由插件配置决定（如 DEEPSEEK_API_KEY / <ROUTE>_API_KEY）；值写入后不会回显。"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(div().text_size(px(11.)).child("引用"))
                    .child(Input::new(&self.cred_ref).w_full()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(div().text_size(px(11.)).child("值"))
                    .child(Input::new(&self.cred_value).w_full()),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(6.))
                    .child(
                        Button::new("cred-check")
                            .compact()
                            .label("查询")
                            .disabled(self.cred_busy)
                            .on_click(cx.listener(|this, _, _, cx| this.check_credential(cx))),
                    )
                    .child(
                        Button::new("cred-save")
                            .primary()
                            .compact()
                            .label("写入")
                            .disabled(self.cred_busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.write_credential(window, cx);
                            })),
                    )
                    .child(
                        Button::new("cred-unset")
                            .compact()
                            .label("清除")
                            .disabled(self.cred_busy)
                            .on_click(cx.listener(|this, _, _, cx| this.clear_credential(cx))),
                    ),
            );
        if let Some(status) = &self.cred_status {
            column = column.child(
                div()
                    .text_size(px(11.))
                    .text_color(accent)
                    .child(status.clone()),
            );
        }
        if let Some(error) = &self.cred_error {
            column = column.child(
                div()
                    .text_size(px(11.))
                    .text_color(danger)
                    .child(error.clone()),
            );
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
                    .child(div().text_size(px(15.)).child("凭据"))
                    .child(
                        Button::new("cred-close")
                            .compact()
                            .label("返回")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.credentials_open = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .p(px(16.))
                    .child(column),
            )
            .into_any_element()
    }

    fn open_plugins(&mut self, cx: &mut Context<Self>) {
        self.plugins_open = true;
        self.plugins_error = None;
        cx.notify();
        if self.plugins.is_none() && !self.plugins_loading {
            self.load_plugins(cx);
        }
    }

    fn load_plugins(&mut self, cx: &mut Context<Self>) {
        let (Some(base), Some(cookie)) = (self.base.clone(), self.cookie.clone()) else {
            self.plugins_error = Some("尚未连接引擎".to_string());
            cx.notify();
            return;
        };
        self.plugins_loading = true;
        self.plugins_error = None;
        cx.notify();
        let tx = self.events_tx.clone();
        backend::runtime().spawn(async move {
            let result = async {
                let client = UnaryClient::new(base, Some(cookie))?;
                plugins::inventory(&client).await
            }
            .await;
            let event = match result {
                Ok(inventory) => AppEvent::PluginsLoaded(inventory),
                Err(error) => AppEvent::PluginsFailed(error.to_string()),
            };
            tx.send(event).await.ok();
        });
    }

    fn toggle_plugin(&mut self, entry_id: String, enabled: bool, cx: &mut Context<Self>) {
        let (Some(base), Some(cookie)) = (self.base.clone(), self.cookie.clone()) else {
            return;
        };
        self.plugins_loading = true;
        self.plugins_error = None;
        self.plugins_notice = None;
        cx.notify();
        let tx = self.events_tx.clone();
        backend::runtime().spawn(async move {
            let result = async {
                let client = UnaryClient::new(base, Some(cookie))?;
                plugins::set_enabled(&client, &entry_id, enabled).await
            }
            .await;
            let event = match result {
                Ok(result) => AppEvent::PluginChanged {
                    entry_id,
                    enabled,
                    result,
                },
                Err(error) => AppEvent::PluginsFailed(error.to_string()),
            };
            tx.send(event).await.ok();
        });
    }

    fn render_plugins(
        &self,
        cx: &mut Context<Self>,
        muted: Hsla,
        border: Hsla,
        accent: Hsla,
        danger: Hsla,
    ) -> AnyElement {
        let mut rows = div().flex().flex_col().gap(px(4.));
        if let Some(inventory) = &self.plugins {
            if self.plugins_loading {
                rows = rows.child(
                    div()
                        .text_size(px(11.))
                        .text_color(muted)
                        .child("处理中…"),
                );
            }
            for entry in &inventory.entries {
                let Some(entry_id) = entry.entry_id.clone() else {
                    continue;
                };
                let enabled = entry.enabled;
                let name = entry.display_name();
                let phase = entry.fiber_phase.clone().unwrap_or_default();
                let target = entry_id.clone();
                rows = rows.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .px(px(10.))
                        .py(px(6.))
                        .rounded_md()
                        .border(px(1.))
                        .border_color(border)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.))
                                .child(div().text_size(px(12.)).child(name))
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(muted)
                                        .child(format!(
                                            "{entry_id} · {phase} · {}",
                                            if enabled { "启用" } else { "禁用" }
                                        )),
                                ),
                        )
                        .child(
                            Button::new(format!("plugin-{entry_id}"))
                                .compact()
                                .label(if enabled { "禁用" } else { "启用" })
                                .disabled(self.plugins_loading)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.toggle_plugin(target.clone(), !enabled, cx);
                                })),
                        ),
                );
            }
        }
        if let Some(notice) = &self.plugins_notice {
            rows = rows.child(
                div()
                    .text_size(px(11.))
                    .text_color(accent)
                    .child(notice.clone()),
            );
        }
        if let Some(error) = &self.plugins_error {
            rows = rows.child(
                div()
                    .text_size(px(11.))
                    .text_color(danger)
                    .child(error.clone()),
            );
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
                    .child(div().text_size(px(15.)).child("插件"))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(6.))
                            .child(
                                Button::new("plugins-refresh")
                                    .compact()
                                    .label("刷新")
                                    .disabled(self.plugins_loading)
                                    .on_click(cx.listener(|this, _, _, cx| this.load_plugins(cx))),
                            )
                            .child(
                                Button::new("plugins-close")
                                    .compact()
                                    .label("返回")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.plugins_open = false;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scrollbar()
                    .p(px(12.))
                    .child(rows),
            )
            .into_any_element()
    }

    fn load_older(&mut self, cx: &mut Context<Self>) {
        let (Some(session_id), Some(through), Some(before), Some(base), Some(cookie)) = (
            self.following.clone(),
            self.history_cursor,
            self.oldest_seq,
            self.base.clone(),
            self.cookie.clone(),
        ) else {
            return;
        };
        if self.history_loading {
            return;
        }
        self.history_loading = true;
        cx.notify();
        let tx = self.events_tx.clone();
        backend::runtime().spawn(async move {
            let result = async {
                let client = UnaryClient::new(base, Some(cookie))?;
                methods::page(&client, &session_id, through, before, 50).await
            }
            .await;
            let event = match result {
                Ok(page) => AppEvent::HistoryLoaded(page),
                Err(error) => AppEvent::HistoryFailed(error.to_string()),
            };
            tx.send(event).await.ok();
        });
    }

    fn toggle_files(&mut self, cx: &mut Context<Self>) {
        self.files_open = !self.files_open;
        if !self.files_open {
            if let Some(task) = self.watch_task.take() {
                task.abort();
            }
            self.preview_absolute = None;
        }
        if self.files_open && self.following.is_some() && self.files_listing.is_none() {
            self.load_files(cx);
        }
        cx.notify();
    }

    fn load_files(&mut self, cx: &mut Context<Self>) {
        let (Some(session_id), Some(base), Some(cookie)) = (
            self.following.clone(),
            self.base.clone(),
            self.cookie.clone(),
        ) else {
            return;
        };
        let path = if self.files_dir.is_empty() {
            ".".to_string()
        } else {
            self.files_dir.join("/")
        };
        self.files_loading = true;
        self.files_error = None;
        cx.notify();
        let tx = self.events_tx.clone();
        backend::runtime().spawn(async move {
            let result = async {
                let client = UnaryClient::new(base, Some(cookie))?;
                files::list(&client, &session_id, &path).await
            }
            .await;
            let event = match result {
                Ok(listing) => AppEvent::FilesLoaded(listing),
                Err(error) => AppEvent::FilesFailed(error.to_string()),
            };
            tx.send(event).await.ok();
        });
    }

    fn open_entry(&mut self, entry: &DirectoryEntry, cx: &mut Context<Self>) {
        if entry.kind == "directory" {
            self.files_dir.push(entry.name.clone());
            self.file_preview = None;
            self.load_files(cx);
            return;
        }
        if entry.kind != "file" {
            return;
        }
        let (Some(session_id), Some(base), Some(cookie), Some(cwd)) = (
            self.following.clone(),
            self.base.clone(),
            self.cookie.clone(),
            self.following_cwd.clone(),
        ) else {
            return;
        };
        let mut relative = self.files_dir.join("/");
        if !relative.is_empty() {
            relative.push('/');
        }
        relative.push_str(&entry.name);
        let absolute = format!("{}/{}", cwd.trim_end_matches('/'), relative);
        self.preview_absolute = Some(absolute.clone());
        self.start_watch(absolute.clone(), cx);
        self.read_file(absolute, cx);
    }

    fn read_file(&mut self, absolute: String, cx: &mut Context<Self>) {
        let (Some(session_id), Some(base), Some(cookie)) =
            (self.following.clone(), self.base.clone(), self.cookie.clone())
        else {
            return;
        };
        self.files_loading = true;
        self.files_error = None;
        cx.notify();
        let tx = self.events_tx.clone();
        backend::runtime().spawn(async move {
            let result = async {
                let client = UnaryClient::new(base, Some(cookie))?;
                files::read(&client, &session_id, &absolute, Some(1), Some(600)).await
            }
            .await;
            let event = match result {
                Ok(file) => AppEvent::FileLoaded(file),
                Err(error) => AppEvent::FilesFailed(error.to_string()),
            };
            tx.send(event).await.ok();
        });
    }

    fn start_watch(&mut self, absolute: String, cx: &mut Context<Self>) {
        if let Some(task) = self.watch_task.take() {
            task.abort();
        }
        let (Some(session_id), Some(base), Some(cookie)) =
            (self.following.clone(), self.base.clone(), self.cookie.clone())
        else {
            return;
        };
        let (frame_tx, frame_rx) = async_channel::unbounded::<Value>();
        self.watch_task = Some(backend::runtime().spawn(async move {
            loop {
                let tx = frame_tx.clone();
                let base = base.clone();
                let cookie = cookie.clone();
                let session_id = session_id.clone();
                let absolute = absolute.clone();
                let _ = files::watch(&base, &cookie, &session_id, &absolute, tx).await;
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }));
        cx.spawn(async move |this, cx| {
            while let Ok(value) = frame_rx.recv().await {
                if value.get("kind").and_then(Value::as_str) == Some("change")
                    && let Some(path) = value
                        .get("change")
                        .and_then(|change| change.get("absolutePath"))
                        .and_then(Value::as_str)
                {
                    let path = path.to_string();
                    let _ = this.update(cx, |state, cx| {
                        state.apply(AppEvent::FileChanged(path), cx);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    fn load_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(settings) = &self.settings else {
            return;
        };
        let Some(namespace) = settings.namespaces.get(self.settings_selected) else {
            return;
        };
        let text = serde_json::to_string_pretty(&namespace.value)
            .unwrap_or_else(|_| "{}".to_string());
        self.settings_editor
            .update(cx, |state, cx| state.set_value(text, window, cx));
        self.settings_notice = None;
        cx.notify();
    }

    fn save_settings(&mut self, cx: &mut Context<Self>) {
        let Some(settings) = &self.settings else {
            return;
        };
        let Some(namespace) = settings.namespaces.get(self.settings_selected) else {
            return;
        };
        if !settings.writable {
            self.settings_error = Some("当前 profile 不可写".to_string());
            cx.notify();
            return;
        }
        let ns = namespace.ns.clone();
        let revision = namespace.revision;
        let text = self.settings_editor.read(cx).value().to_string();
        let patch: Value = match serde_json::from_str(&text) {
            Ok(value @ Value::Object(_)) => value,
            Ok(_) => {
                self.settings_error = Some("顶层必须是 JSON 对象".to_string());
                cx.notify();
                return;
            }
            Err(error) => {
                self.settings_error = Some(format!("JSON 解析失败：{error}"));
                cx.notify();
                return;
            }
        };
        let (Some(base), Some(cookie)) = (self.base.clone(), self.cookie.clone()) else {
            return;
        };
        self.settings_saving = true;
        self.settings_notice = None;
        self.settings_error = None;
        cx.notify();
        let tx = self.events_tx.clone();
        backend::runtime().spawn(async move {
            let result = async {
                let client = UnaryClient::new(base, Some(cookie))?;
                settings::update(&client, &ns, patch, revision).await
            }
            .await;
            let event = match result {
                Ok(view) => AppEvent::SettingsSaved(view),
                Err(error) => AppEvent::SettingsSaveFailed(error.to_string()),
            };
            tx.send(event).await.ok();
        });
    }

    fn render_files(
        &self,
        cx: &mut Context<Self>,
        muted: Hsla,
        border: Hsla,
        accent: Hsla,
        danger: Hsla,
    ) -> AnyElement {
        let mut entries = div().flex().flex_col().gap(px(1.));
        if let Some(listing) = &self.files_listing {
            for entry in &listing.entries {
                let prefix = if entry.kind == "directory" { "▸" } else { "·" };
                let entry = entry.clone();
                entries = entries.child(
                    div()
                        .id(ElementId::Name(format!("file-{}", entry.name).into()))
                        .px(px(6.))
                        .py(px(3.))
                        .rounded(px(3.))
                        .cursor_pointer()
                        .text_size(px(11.))
                        .child(format!("{prefix} {}", entry.name))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_entry(&entry, cx);
                        })),
                );
            }
            if listing.truncated {
                entries = entries.child(
                    div()
                        .text_size(px(10.))
                        .text_color(muted)
                        .child("（已截断）"),
                );
            }
            if self.files_loading {
                entries = entries.child(
                    div()
                        .text_size(px(11.))
                        .text_color(muted)
                        .child("加载中…"),
                );
            }
        }

        let breadcrumb = if self.files_dir.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", self.files_dir.join("/"))
        };

        let mut preview = div().flex().flex_col().gap(px(4.));
        if let Some(file) = &self.file_preview {
            preview = preview.child(
                div()
                    .text_size(px(10.))
                    .text_color(muted)
                    .child(file.absolute_path.clone()),
            );
            preview = preview.child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scrollbar()
                    .child(div().text_size(px(10.)).child(file.text.clone())),
            );
        }

        div()
            .w(px(320.))
            .flex()
            .flex_col()
            .min_h(px(0.))
            .border_l(px(1.))
            .border_color(border)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px(px(8.))
                    .py(px(6.))
                    .border_b(px(1.))
                    .border_color(border)
                    .child(div().text_size(px(12.)).child(breadcrumb))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(4.))
                            .child(
                                Button::new("files-up")
                                    .compact()
                                    .label("..")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.files_dir.pop();
                                        this.file_preview = None;
                                        this.load_files(cx);
                                    })),
                            )
                            .child(
                                Button::new("files-refresh")
                                    .compact()
                                    .label("刷新")
                                    .on_click(cx.listener(|this, _, _, cx| this.load_files(cx))),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scrollbar()
                    .p(px(6.))
                    .child(entries),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .border_t(px(1.))
                    .border_color(border)
                    .p(px(6.))
                    .child(preview),
            )
            .children(
                self.files_error
                    .as_ref()
                    .map(|error| {
                        div()
                            .px(px(8.))
                            .py(px(4.))
                            .text_size(px(10.))
                            .text_color(danger)
                            .child(error.clone())
                    })
                    .into_iter(),
            )
            .into_any_element()
    }

    fn render_settings(
        &self,
        cx: &mut Context<Self>,
        muted: Hsla,
        border: Hsla,
        accent: Hsla,
        danger: Hsla,
    ) -> AnyElement {
        let writable = self
            .settings
            .as_ref()
            .map(|settings| settings.writable)
            .unwrap_or(false);
        let mut list = div().flex().flex_col().gap(px(4.));
        if let Some(settings) = &self.settings {
            for (index, namespace) in settings.namespaces.iter().enumerate() {
                let selected = index == self.settings_selected;
                list = list.child(
                    div()
                        .id(ElementId::Name(format!("settings-ns-{index}").into()))
                        .px(px(8.))
                        .py(px(6.))
                        .rounded_md()
                        .border(px(1.))
                        .border_color(if selected { accent } else { border })
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.settings_selected = index;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .text_size(px(12.))
                                .child(namespace.ns.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(muted)
                                .child(format!(
                                    "{} · rev {}",
                                    namespace.applies, namespace.revision
                                )),
                        ),
                );
            }
        }

        let mut detail = div().flex().flex_col().gap(px(8.));
        if let Some(settings) = &self.settings {
            if let Some(namespace) = settings.namespaces.get(self.settings_selected) {
                detail = detail
                    .child(div().text_size(px(16.)).child(namespace.ns.clone()))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(muted)
                            .child(format!(
                                "applies: {} · revision: {}",
                                namespace.applies, namespace.revision
                            )),
                    );
                if !namespace.secrets.is_empty() {
                    let mut secrets = div().flex().flex_col().gap(px(2.));
                    for secret in &namespace.secrets {
                        secrets = secrets.child(
                            div()
                                .text_size(px(11.))
                                .text_color(muted)
                                .child(format!(
                                    "secret {}: {}",
                                    secret.path.join("."),
                                    if secret.set { "已配置" } else { "未配置" }
                                )),
                        );
                    }
                    detail = detail.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .p(px(8.))
                            .rounded_md()
                            .border(px(1.))
                            .border_color(border)
                            .child(div().text_size(px(11.)).child("凭据槽位"))
                            .child(secrets),
                    );
                }
                let pretty = serde_json::to_string_pretty(&namespace.value)
                    .unwrap_or_else(|_| "{}".to_string());
                detail = detail.child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scrollbar()
                        .p(px(8.))
                        .rounded_md()
                        .border(px(1.))
                        .border_color(border)
                        .child(div().text_size(px(11.)).child(pretty)),
                );
            }
        } else if self.settings_loading {
            detail = detail.child(
                div()
                    .text_size(px(12.))
                    .text_color(muted)
                    .child("读取设置…"),
            );
        }

        let writable = self
            .settings
            .as_ref()
            .map(|settings| settings.writable)
            .unwrap_or(false);
        detail = detail.child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(muted)
                        .child("编辑（合并保存，需与 revision 匹配）"),
                )
                .child(
                    div()
                        .h(px(180.))
                        .w_full()
                        .child(Textarea::new(&self.settings_editor).h_full()),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(6.))
                        .child(
                            Button::new("settings-load-editor")
                                .compact()
                                .label("载入当前值")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.load_editor(window, cx);
                                })),
                        )
                        .child(
                            Button::new("settings-save")
                                .primary()
                                .compact()
                                .label(if self.settings_saving { "保存中…" } else { "保存" })
                                .disabled(!writable || self.settings_saving)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.save_settings(cx);
                                })),
                        ),
                ),
        );

        if let Some(notice) = &self.settings_notice {
            detail = detail.child(
                div()
                    .text_size(px(11.))
                    .text_color(accent)
                    .child(notice.clone()),
            );
        }
        if let Some(error) = &self.settings_error {
            detail = detail.child(
                div()
                    .text_size(px(11.))
                    .text_color(danger)
                    .child(error.clone()),
            );
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
                            .child(div().text_size(px(15.)).child("设置"))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(muted)
                                    .child(if writable { "可写" } else { "只读" }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(6.))
                            .child(
                                Button::new("settings-refresh")
                                    .compact()
                                    .label("刷新")
                                    .disabled(self.settings_loading)
                                    .on_click(cx.listener(|this, _, _, cx| this.load_settings(cx))),
                            )
                            .child(
                                Button::new("settings-doc")
                                    .compact()
                                    .label("打开设置文档")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.open_settings_document(cx)),
                                    ),
                            )
                            .child(
                                Button::new("settings-close")
                                    .compact()
                                    .label("返回")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_settings = false;
                                        cx.notify();
                                    })),
                            ),
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
                            .w(px(260.))
                            .flex()
                            .flex_col()
                            .p(px(10.))
                            .gap(px(4.))
                            .border_r(px(1.))
                            .border_color(border)
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(muted)
                                    .child("命名空间"),
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
                            .min_h(px(0.))
                            .p(px(12.))
                            .child(detail),
                    ),
            )
            .into_any_element()
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
        self.following_cwd = self
            .sessions
            .iter()
            .find(|session| session.session_id == session_id)
            .and_then(|session| session.cwd.clone());
        self.files_dir.clear();
        self.files_listing = None;
        self.file_preview = None;
        self.preview_absolute = None;
        self.files_error = None;
        self.history_cursor = None;
        self.oldest_seq = None;
        self.history_has_more = false;
        self.history_loading = false;
        if let Some(task) = self.watch_task.take() {
            task.abort();
        }
        cx.notify();
        if std::env::var("DSHG_OPEN_FILES").is_ok() {
            self.files_open = true;
        }
        if self.files_open {
            self.load_files(cx);
        }

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
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(muted)
                                    .child(format!("{} 个会话", self.sessions.len())),
                            )
                            .child(
                                Button::new("open-credentials")
                                    .compact()
                                    .label("凭据")
                                    .on_click(cx.listener(|this, _, _, cx| this.open_credentials(cx))),
                            )
                            .child(
                                Button::new("open-plugins")
                                    .compact()
                                    .label("插件")
                                    .on_click(cx.listener(|this, _, _, cx| this.open_plugins(cx))),
                            )
                            .child(
                                Button::new("toggle-files")
                                    .compact()
                                    .label(if self.files_open { "隐藏文件" } else { "文件" })
                                    .on_click(cx.listener(|this, _, _, cx| this.toggle_files(cx))),
                            )
                            .child(
                                Button::new("open-settings")
                                    .compact()
                                    .label("设置")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.open_settings(cx)),
                                    ),
                            ),
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
                    )
                    .child(if self.files_open {
                        self.render_files(cx, muted, border, accent, danger)
                    } else {
                        div().into_any_element()
                    }),
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

        if self.transcript.is_empty() && self.live_assistant.is_empty() {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(muted)
                        .child("载入会话…"),
                )
                .into_any_element();
        }

        let total = self.transcript.len() + usize::from(!self.live_assistant.is_empty());
        let sizes: Rc<Vec<Size<Pixels>>> = Rc::new(
            (0..total)
                .map(|index| {
                    let height = if index < self.transcript.len() {
                        item_height(&self.transcript[index])
                    } else {
                        item_height(&TranscriptItem::Assistant(self.live_assistant.clone()))
                    };
                    size(px(0.), px(height))
                })
                .collect(),
        );
        let view = cx.entity();
        let list = v_virtual_list(
            view,
            "transcript",
            sizes,
            move |this, range, _window, cx| {
                range
                    .map(|index| {
                        let item = if index < this.transcript.len() {
                            this.transcript[index].clone()
                        } else {
                            TranscriptItem::Assistant(this.live_assistant.clone())
                        };
                        this.render_item(index, &item, cx, muted, accent, success, danger)
                    })
                    .collect()
            },
        )
        .track_scroll(&self.transcript_scroll);

        let mut container = div().flex().flex_col().flex_1().min_h(px(0.));
        if self.history_has_more {
            container = container.child(
                div().p(px(8.)).child(
                    Button::new("load-older")
                        .compact()
                        .label(if self.history_loading { "加载中…" } else { "加载更早" })
                        .disabled(self.history_loading)
                        .on_click(cx.listener(|this, _, _, cx| this.load_older(cx))),
                ),
            );
        }
        container
            .child(div().flex_1().min_h(px(0.)).child(list))
            .into_any_element()
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn bulk_transcript(count: usize) -> Vec<TranscriptItem> {
    (0..count)
        .map(|index| {
            if index % 2 == 0 {
                TranscriptItem::User(format!("问题 #{index}：这是一条用于虚拟化冒烟的消息。"))
            } else {
                TranscriptItem::Assistant(format!(
                    "回答 #{index}：\n\n- 第一行\n- 第二行\n- 第三行\n\n以及一段更长的文字用于撑高行高。"
                ))
            }
        })
        .collect()
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

fn item_height(item: &TranscriptItem) -> f32 {
    let chars = match item {
        TranscriptItem::User(text) | TranscriptItem::Assistant(text) => text.chars().count(),
        TranscriptItem::Tool(call) => {
            call.arguments.chars().count()
                + call
                    .result
                    .as_ref()
                    .map(|result| result.text.chars().count().min(2000))
                    .unwrap_or(0)
        }
    };
    let lines = (chars / 80) + 1;
    (28.0 + lines as f32 * 16.0).clamp(28.0, 600.0)
}

fn render_diff(diff: &FileDiff, muted: Hsla, success: Hsla, danger: Hsla) -> AnyElement {
    let old_lines = diff
        .old_text
        .as_deref()
        .map(|text| text.lines().count())
        .unwrap_or(0);
    let new_lines = diff.new_text.lines().count();
    let mut block = div()
        .flex()
        .flex_col()
        .gap(px(1.))
        .p(px(6.))
        .rounded_md()
        .border(px(1.))
        .border_color(muted.opacity(0.2))
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .pb(px(2.))
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(muted)
                        .child(diff.path.clone()),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(muted)
                        .child(format!("+{new_lines} -{old_lines}")),
                ),
        );
    if let Some(old) = &diff.old_text {
        for line in old.lines().take(40) {
            block = block.child(
                div()
                    .px(px(4.))
                    .rounded(px(2.))
                    .bg(danger.opacity(0.08))
                    .text_size(px(10.))
                    .text_color(danger)
                    .child(format!("- {line}")),
            );
        }
    }
    for line in diff.new_text.lines().take(40) {
        block = block.child(
            div()
                .px(px(4.))
                .rounded(px(2.))
                .bg(success.opacity(0.08))
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

        let body = if self.credentials_open {
            self.render_credentials(cx, muted, border, accent, danger)
        } else if self.plugins_open {
            self.render_plugins(cx, muted, border, accent, danger)
        } else if self.show_settings {
            self.render_settings(cx, muted, border, accent, danger)
        } else if self.phase == Phase::Ready {
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

