// Ollama Cloud chat API client. HTTPS is delegated to the system `curl`
// because the Rust standard library does not include TLS.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::json::{self, Value};

const ENDPOINT: &str = "https://ollama.com/api/chat";

#[derive(Debug, Clone)]
pub struct Message {
    pub role: String,
    pub content: String,
    pub tool_name: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

impl Message {
    pub fn system(content: &str) -> Self {
        Self::plain("system", content)
    }

    pub fn user(content: &str) -> Self {
        Self::plain("user", content)
    }

    pub fn tool(name: &str, content: &str) -> Self {
        Self {
            role: "tool".into(),
            content: content.into(),
            tool_name: Some(name.into()),
            tool_calls: Vec::new(),
        }
    }

    fn plain(role: &str, content: &str) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            tool_name: None,
            tool_calls: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: String,
    pub arguments: Value,
}

pub struct Response {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub assistant_message: Message,
}

pub fn chat(
    api_key: &str,
    model: &str,
    messages: &[Message],
    tools: &[Value],
) -> Result<Response, String> {
    let body = build_request(model, messages, tools);
    let raw = http_post_json(api_key, &body)?;

    let parsed = json::parse(&raw)
        .map_err(|e| format!("failed to parse Ollama response: {}\nbody: {}", e, raw))?;

    if let Some(err) = parsed.get("error") {
        let msg = err.as_str().unwrap_or("unknown");
        return Err(format!("Ollama returned an error: {}", msg));
    }

    let message = parsed
        .get("message")
        .ok_or_else(|| format!("missing 'message' in response: {}", raw))?;

    let content = message
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut tool_calls = Vec::new();
    if let Some(calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
        for call in calls {
            let function = call.get("function").ok_or("tool_call missing 'function'")?;
            let name = function
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("tool_call missing 'function.name'")?
                .to_string();
            let arguments = function
                .get("arguments")
                .cloned()
                .unwrap_or(Value::Object(Vec::new()));
            tool_calls.push(ToolCall { name, arguments });
        }
    }

    let assistant_message = Message {
        role: "assistant".into(),
        content: content.clone(),
        tool_name: None,
        tool_calls: tool_calls.clone(),
    };

    Ok(Response { content, tool_calls, assistant_message })
}

fn build_request(model: &str, messages: &[Message], tools: &[Value]) -> String {
    let messages_json: Vec<Value> = messages.iter().map(message_to_json).collect();

    let mut entries = vec![
        ("model".into(), Value::String(model.into())),
        ("messages".into(), Value::Array(messages_json)),
        ("stream".into(), Value::Bool(false)),
    ];
    if !tools.is_empty() {
        entries.push(("tools".into(), Value::Array(tools.to_vec())));
    }
    json::stringify(&Value::Object(entries))
}

fn message_to_json(m: &Message) -> Value {
    let mut entries = vec![
        ("role".into(), Value::String(m.role.clone())),
        ("content".into(), Value::String(m.content.clone())),
    ];
    if let Some(name) = &m.tool_name {
        entries.push(("tool_name".into(), Value::String(name.clone())));
    }
    if !m.tool_calls.is_empty() {
        let calls: Vec<Value> = m
            .tool_calls
            .iter()
            .map(|call| {
                Value::Object(vec![(
                    "function".into(),
                    Value::Object(vec![
                        ("name".into(), Value::String(call.name.clone())),
                        ("arguments".into(), call.arguments.clone()),
                    ]),
                )])
            })
            .collect();
        entries.push(("tool_calls".into(), Value::Array(calls)));
    }
    Value::Object(entries)
}

fn http_post_json(api_key: &str, body: &str) -> Result<String, String> {
    let mut child = Command::new("curl")
        .arg("--silent")
        .arg("--show-error")
        .arg("--request").arg("POST")
        .arg("--header").arg("Content-Type: application/json")
        .arg("--header").arg(format!("Authorization: Bearer {}", api_key))
        .arg("--data-binary").arg("@-")
        .arg(ENDPOINT)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not spawn curl: {}", e))?;

    {
        let stdin = child.stdin.as_mut().ok_or("could not open curl stdin")?;
        stdin
            .write_all(body.as_bytes())
            .map_err(|e| format!("failed writing request body: {}", e))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("failed waiting for curl: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "curl exited with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    String::from_utf8(output.stdout).map_err(|e| format!("non-UTF-8 response: {}", e))
}
