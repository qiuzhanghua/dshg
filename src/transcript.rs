//! Pure transcript decoding, kept free of GPUI so it can be unit-tested.

use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub enum TranscriptItem {
    User(String),
    Assistant(String),
    ToolCall { name: String, arguments: String },
    ToolResult { text: String, error: bool },
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

pub fn parse_event(event: &Value) -> Option<TranscriptItem> {
    let kind = event.get("type")?.as_str()?;
    let data = event.get("data")?;
    match kind {
        "user/message" => {
            let text = message_text(data.get("message")?);
            (!text.is_empty()).then_some(TranscriptItem::User(text))
        }
        "assistant/message" => {
            let text = message_text(data.get("message")?);
            (!text.is_empty()).then_some(TranscriptItem::Assistant(text))
        }
        "tool/call" => Some(TranscriptItem::ToolCall {
            name: data.get("name")?.as_str()?.to_string(),
            arguments: data
                .get("arguments")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }),
        "tool/result" => {
            let message = data.get("message")?;
            Some(TranscriptItem::ToolResult {
                text: message_text(message),
                error: message
                    .get("isError")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        }
        _ => None,
    }
}

/// Returns `(is_snapshot, items)`.
pub fn parse_frame(value: &Value) -> (bool, Vec<TranscriptItem>) {
    match value.get("type").and_then(Value::as_str) {
        Some("snapshot") => {
            let mut items = Vec::new();
            if let Some(records) = value.get("records").and_then(Value::as_array) {
                for record in records {
                    if let Some(event) = record.get("event")
                        && let Some(item) = parse_event(event)
                    {
                        items.push(item);
                    }
                }
            }
            (true, items)
        }
        Some("event") => {
            let items = value
                .get("event")
                .and_then(parse_event)
                .into_iter()
                .collect();
            (false, items)
        }
        _ => (false, Vec::new()),
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
        assert_eq!(parse_event(&user), Some(TranscriptItem::User("hi".into())));

        let assistant = value(
            r#"{"type":"assistant/message","seq":2,"time":0,"data":{"message":{"role":"assistant","content":[{"type":"text","text":"hello"}]}}}"#,
        );
        assert_eq!(
            parse_event(&assistant),
            Some(TranscriptItem::Assistant("hello".into()))
        );
    }

    #[test]
    fn parses_tool_call_and_result() {
        let call = value(
            r#"{"type":"tool/call","seq":3,"time":0,"data":{"name":"bash","arguments":"{\"command\":\"ls\"}"}}"#,
        );
        assert_eq!(
            parse_event(&call),
            Some(TranscriptItem::ToolCall {
                name: "bash".into(),
                arguments: "{\"command\":\"ls\"}".into(),
            })
        );

        let result = value(
            r#"{"type":"tool/result","seq":4,"time":0,"data":{"message":{"role":"tool","content":[{"type":"text","text":"ok"}],"isError":false}}}"#,
        );
        assert_eq!(
            parse_event(&result),
            Some(TranscriptItem::ToolResult {
                text: "ok".into(),
                error: false,
            })
        );
    }

    #[test]
    fn snapshot_collects_records() {
        let frame = value(
            r#"{"type":"snapshot","records":[{"type":"event","event":{"type":"user/message","seq":1,"time":0,"data":{"message":{"role":"user","content":[{"type":"text","text":"q"}]}}}}]}"#,
        );
        let (is_snapshot, items) = parse_frame(&frame);
        assert!(is_snapshot);
        assert_eq!(items, vec![TranscriptItem::User("q".into())]);
    }
}
