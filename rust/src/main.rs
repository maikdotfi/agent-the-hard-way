use std::env;
use std::io::{self, Read};

mod json;
mod ollama;
mod tools;

const DEFAULT_MODEL: &str = "qwen3-coder:480b-cloud";
const MAX_ITERATIONS: usize = 25;

const SYSTEM_PROMPT: &str = "\
You are a small coding agent running on the user's machine. The user gives \
you a task; you complete it by calling the tools provided.

Available tools:
  - write_file(path, content): create or overwrite a file. Parent directories \
    are created automatically.
  - run_shell(command): run a shell command via /bin/sh -c and read back \
    stdout, stderr and exit code.

When asked to create a program:
  1. Pick a sensible subdirectory of the current working directory.
  2. Write the source files needed to run it.
  3. If the language needs a build manifest (Cargo.toml, package.json, etc.) \
     create it too.
  4. You do not need to execute the finished program -- the user will do that \
     themselves.

Keep tool calls focused. Once the task is done, reply with a short summary \
and stop calling tools.";

fn main() {
    let api_key = match env::var("OLLAMA_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            eprintln!("OLLAMA_API_KEY is not set");
            std::process::exit(1);
        }
    };

    let model = env::var("OLLAMA_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into());
    let prompt = read_prompt();
    if prompt.is_empty() {
        eprintln!("no prompt provided");
        std::process::exit(1);
    }

    let mut messages = vec![
        ollama::Message::system(SYSTEM_PROMPT),
        ollama::Message::user(&prompt),
    ];
    let tool_specs = tools::specs();

    for iteration in 1..=MAX_ITERATIONS {
        eprintln!("\n[iteration {}] calling {}...", iteration, model);

        let response = match ollama::chat(&api_key, &model, &messages, &tool_specs) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("error: {}", e);
                std::process::exit(1);
            }
        };

        if !response.content.trim().is_empty() {
            println!("\n{}", response.content.trim());
        }

        messages.push(response.assistant_message.clone());

        if response.tool_calls.is_empty() {
            eprintln!("\n[done]");
            return;
        }

        for call in &response.tool_calls {
            let args_preview = preview(&json::stringify(&call.arguments), 160);
            eprintln!("[tool] {}({})", call.name, args_preview);

            let result = tools::run(&call.name, &call.arguments);
            eprintln!("[tool] -> {}", preview(&result, 200));

            messages.push(ollama::Message::tool(&call.name, &result));
        }
    }

    eprintln!("\n[stopped: reached maximum of {} iterations]", MAX_ITERATIONS);
    std::process::exit(2);
}

fn read_prompt() -> String {
    let args: Vec<String> = env::args().skip(1).collect();
    if !args.is_empty() {
        return args.join(" ");
    }
    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf).ok();
    buf.trim().to_string()
}

fn preview(s: &str, max: usize) -> String {
    let collapsed: String = s.chars().map(|c| if c == '\n' { ' ' } else { c }).collect();
    if collapsed.chars().count() <= max {
        collapsed
    } else {
        let truncated: String = collapsed.chars().take(max).collect();
        format!("{}... ({} chars total)", truncated, collapsed.chars().count())
    }
}
