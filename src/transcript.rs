//! Pure transcript decoding, kept free of GPUI so it can be unit-tested.

use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    pub text: String,
    pub error: bool,
    pub meta: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
    pub result: Option<ToolResult>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TranscriptItem {
    User(String),
    Assistant(String),
    Tool(ToolCall),
}

#[derive(Clone, Debug, PartialEq)]
pub enum TranscriptEvent {
    User(String),
    Assistant(String),
    ToolCall {
        call_id: String,
        name: String,
        arguments: String,
    },
    ToolResult {
        call_id: String,
        text: String,
        error: bool,
        meta: Option<String>,
    },
}

pub fn message_text(message: &Value) -> String {
    message
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

pub fn parse_event(event: &Value) -> Option<TranscriptEvent> {
    let kind = event.get("type")?.as_str()?;
    let data = event.get("data")?;
    match kind {
        "user/message" => {
            let text = message_text(data.get("message")?);
            (!text.is_empty()).then_some(TranscriptEvent::User(text))
        }
        "assistant/message" => {
            let text = message_text(data.get("message")?);
            (!text.is_empty()).then_some(TranscriptEvent::Assistant(text))
        }
        "tool/call" => Some(TranscriptEvent::ToolCall {
            call_id: data.get("callId")?.as_str()?.to_string(),
            name: data.get("name")?.as_str()?.to_string(),
            arguments: data
                .get("arguments")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        "tool/result" => {
            let message = data.get("message")?;
            let call_id = message
                .get("source")
                .and_then(|source| source.get("callId"))
                .or_else(|| message.get("toolCallId"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Some(TranscriptEvent::ToolResult {
                call_id,
                text: message_text(message),
                error: message
                    .get("isError")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                meta: data
                    .get("meta")
                    .map(|meta| serde_json::to_string(meta).unwrap_or_default()),
            })
        }
        _ => None,
    }
}

/// Returns `(is_snapshot, events)`.
pub fn parse_frame(value: &Value) -> (bool, Vec<TranscriptEvent>) {
    match value.get("type").and_then(Value::as_str) {
        Some("snapshot") => {
            let mut events = Vec::new();
            if let Some(records) = value.get("records").and_then(Value::as_array) {
                for record in records {
                    if let Some(event) = record.get("event")
                        && let Some(event) = parse_event(event)
                    {
                        events.push(event);
                    }
                }
            }
            (true, events)
        }
        Some("event") => {
            let events = value
                .get("event")
                .and_then(parse_event)
                .into_iter()
                .collect();
            (false, events)
        }
        _ => (false, Vec::new()),
    }
}

/// Apply decoded events to the visible transcript, pairing tool results by call id.
pub fn apply_events(items: &mut Vec<TranscriptItem>, events: Vec<TranscriptEvent>) {
    for event in events {
        match event {
            TranscriptEvent::User(text) => items.push(TranscriptItem::User(text)),
            TranscriptEvent::Assistant(text) => items.push(TranscriptItem::Assistant(text)),
            TranscriptEvent::ToolCall {
                call_id,
                name,
                arguments,
            } => items.push(TranscriptItem::Tool(ToolCall {
                call_id,
                name,
                arguments,
                result: None,
            })),
            TranscriptEvent::ToolResult {
                call_id,
                text,
                error,
                meta,
            } => {
                let result = ToolResult { text, error, meta };
                if let Some(TranscriptItem::Tool(call)) = items
                    .iter_mut()
                    .rev()
                    .find(|item| matches!(item, TranscriptItem::Tool(call) if call.call_id == call_id))
                {
                    call.result = Some(result);
                } else {
                    items.push(TranscriptItem::Tool(ToolCall {
                        call_id,
                        name: "tool".to_string(),
                        arguments: String::new(),
                        result: Some(result),
                    }));
                }
            }
        }
    }
}

/// One-line argument summary for a tool card, keyed by the tool name.
pub fn tool_summary(name: &str, arguments: &str) -> String {
    let value: Value = serde_json::from_str(arguments).unwrap_or(Value::Null);
    let field = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    match name {
        "bash" => format!("$ {}", field("command")),
        "read" | "write" | "edit" | "str_replace_editor" => field("path"),
        "grep" => format!("{}  {}", field("pattern"), field("path")),
        "glob" => field("pattern"),
        "web_search" => field("query"),
        "web_fetch" => field("url"),
        _ => {
            let pretty = serde_json::to_string_pretty(&value).unwrap_or_default();
            if pretty.chars().count() > 200 {
                let mut out: String = pretty.chars().take(200).collect();
                out.push('…');
                out
            } else {
                pretty
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(text: &str) -> Value {
        serde_json::from_str(text).expect("json")
    }

    #[test]
    fn parses_user_and_assistant_messages() {
        let user = value(
            r#"{"type":"user/message","seq":1,"time":0,"data":{"message":{"role":"user","content":[{"type":"text","text":"hi"}]}}}"#,
        );
        assert_eq!(parse_event(&user), Some(TranscriptEvent::User("hi".into())));

        let assistant = value(
            r#"{"type":"assistant/message","seq":2,"time":0,"data":{"message":{"role":"assistant","content":[{"type":"text","text":"hello"}]}}}"#,
        );
        assert_eq!(
            parse_event(&assistant),
            Some(TranscriptEvent::Assistant("hello".into()))
        );
    }

    #[test]
    fn parses_tool_call_and_result() {
        let call = value(
            r#"{"type":"tool/call","seq":3,"time":0,"data":{"callId":"c1","name":"bash","arguments":"{\"command\":\"ls\"}"}}"#,
        );
        assert_eq!(
            parse_event(&call),
            Some(TranscriptEvent::ToolCall {
                call_id: "c1".into(),
                name: "bash".into(),
                arguments: "{\"command\":\"ls\"}".into(),
            })
        );

        let result = value(
            r#"{"type":"tool/result","seq":4,"time":0,"data":{"message":{"role":"tool","source":{"kind":"tool","callId":"c1"},"content":[{"type":"text","text":"ok"}],"isError":false}}}"#,
        );
        assert_eq!(
            parse_event(&result),
            Some(TranscriptEvent::ToolResult {
                call_id: "c1".into(),
                text: "ok".into(),
                error: false,
                meta: None,
            })
        );
    }

    #[test]
    fn pairs_results_with_calls() {
        let mut items = Vec::new();
        apply_events(
            &mut items,
            vec![
                TranscriptEvent::ToolCall {
                    call_id: "c1".into(),
                    name: "bash".into(),
                    arguments: "{}".into(),
                },
                TranscriptEvent::ToolResult {
                    call_id: "c1".into(),
                    text: "ok".into(),
                    error: false,
                    meta: None,
                },
            ],
        );
        match &items[0] {
            TranscriptItem::Tool(call) => {
                assert_eq!(call.name, "bash");
                assert!(call.result.as_ref().unwrap().text == "ok");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn summarizes_tool_arguments() {
        assert_eq!(tool_summary("bash", r#"{"command":"ls -la"}"#), "$ ls -la");
        assert_eq!(tool_summary("read", r#"{"path":"/tmp/a.txt"}"#), "/tmp/a.txt");
        assert_eq!(
            tool_summary("grep", r#"{"pattern":"todo","path":"src"}"#),
            "todo  src"
        );
    }

    #[test]
    fn snapshot_collects_records() {
        let frame = value(
            r#"{"type":"snapshot","records":[{"type":"event","event":{"type":"user/message","seq":1,"time":0,"data":{"message":{"role":"user","content":[{"type":"text","text":"q"}]}}}}]}"#,
        );
        let (is_snapshot, events) = parse_frame(&frame);
        assert!(is_snapshot);
        let mut items = Vec::new();
        apply_events(&mut items, events);
        assert_eq!(items, vec![TranscriptItem::User("q".into())]);
    }
}
