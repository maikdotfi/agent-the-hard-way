// Tools the agent can call. Two are enough to bootstrap any project:
// write_file creates source files, run_shell does everything else.

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::json::Value;

pub fn specs() -> Vec<Value> {
    vec![
        function_spec(
            "write_file",
            "Write content to a file, creating any missing parent directories. \
             Overwrites the file if it already exists.",
            &[
                ("path", "string", "Path of the file to write."),
                ("content", "string", "Full content to write into the file."),
            ],
            &["path", "content"],
        ),
        function_spec(
            "run_shell",
            "Run a shell command via /bin/sh -c and return its stdout, stderr \
             and exit code. Use this for things like creating directories, \
             listing files or compiling code.",
            &[("command", "string", "Shell command to execute.")],
            &["command"],
        ),
    ]
}

pub fn run(name: &str, arguments: &Value) -> String {
    match name {
        "write_file" => write_file(arguments),
        "run_shell" => run_shell(arguments),
        other => format!("error: unknown tool '{}'", other),
    }
}

fn write_file(args: &Value) -> String {
    let path = match args.get("path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return "error: 'path' is required".into(),
    };
    let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");

    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = fs::create_dir_all(parent) {
                return format!("error creating parent directory '{}': {}", parent.display(), e);
            }
        }
    }

    match fs::write(path, content) {
        Ok(()) => format!("wrote {} bytes to {}", content.len(), path),
        Err(e) => format!("error writing '{}': {}", path, e),
    }
}

fn run_shell(args: &Value) -> String {
    let command = match args.get("command").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => return "error: 'command' is required".into(),
    };

    let output = match Command::new("sh").arg("-c").arg(command).output() {
        Ok(o) => o,
        Err(e) => return format!("error running command: {}", e),
    };

    let exit_code = output
        .status
        .code()
        .map(|c| c.to_string())
        .unwrap_or_else(|| "signal".into());

    format!(
        "exit_code: {}\nstdout:\n{}\nstderr:\n{}",
        exit_code,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}

fn function_spec(
    name: &str,
    description: &str,
    params: &[(&str, &str, &str)],
    required: &[&str],
) -> Value {
    let properties: Vec<(String, Value)> = params
        .iter()
        .map(|(pname, ptype, pdesc)| {
            (
                (*pname).to_string(),
                Value::Object(vec![
                    ("type".into(), Value::String((*ptype).into())),
                    ("description".into(), Value::String((*pdesc).into())),
                ]),
            )
        })
        .collect();

    let required_values: Vec<Value> = required
        .iter()
        .map(|r| Value::String((*r).to_string()))
        .collect();

    Value::Object(vec![
        ("type".into(), Value::String("function".into())),
        (
            "function".into(),
            Value::Object(vec![
                ("name".into(), Value::String(name.into())),
                ("description".into(), Value::String(description.into())),
                (
                    "parameters".into(),
                    Value::Object(vec![
                        ("type".into(), Value::String("object".into())),
                        ("properties".into(), Value::Object(properties)),
                        ("required".into(), Value::Array(required_values)),
                    ]),
                ),
            ]),
        ),
    ])
}
