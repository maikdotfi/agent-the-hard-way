#!/usr/bin/env python3
"""Minimal coding agent using only the Python stdlib and Ollama Cloud API."""

import json
import os
import subprocess
import sys
import urllib.error
import urllib.request


OLLAMA_BASE = os.environ.get("OLLAMA_BASE_URL", "https://api.ollama.com")
OLLAMA_MODEL = os.environ.get("OLLAMA_MODEL", "qwen2.5-coder")

SYSTEM_PROMPT = """You are a coding agent with access to these tools:

- write_file(path, content): Write content to a file (creates parent directories).
- read_file(path): Read the contents of a file.
- run_command(command): Run a shell command and return the combined stdout/stderr.
- done(): Call when the task is complete.

When you want to use a tool, respond with ONLY a JSON object:
{"tool": "write_file", "args": {"path": "example.py", "content": "print('hello')"}}

When done:
{"tool": "done", "args": {}}

If you need to ask the user something, respond with plain text.
Do not wrap JSON in markdown code blocks.
"""


def chat(messages):
    """Send messages to the Ollama Cloud chat API and return the assistant's content."""
    api_key = os.environ["OLLAMA_API_KEY"]
    url = f"{OLLAMA_BASE}/api/chat"

    payload = {
        "model": OLLAMA_MODEL,
        "messages": messages,
        "stream": False,
    }

    request = urllib.request.Request(
        url,
        data=json.dumps(payload).encode(),
        headers={
            "Authorization": f"Bearer {api_key}",
            "Content-Type": "application/json",
        },
    )

    try:
        with urllib.request.urlopen(request) as response:
            body = json.loads(response.read().decode())
            return body["message"]["content"]
    except urllib.error.HTTPError as exc:
        error_body = exc.read().decode()
        raise RuntimeError(f"Ollama API error {exc.code}: {error_body}") from exc


def parse_tool_call(text):
    """Try to parse a JSON tool call from the model's response."""
    text = text.strip()

    # Tolerate markdown code blocks if the model wraps JSON in them
    if text.startswith("```"):
        lines = text.splitlines()
        if lines[0].strip().startswith("``"):
            lines = lines[1:]
        if lines and lines[-1].strip() == "```":
            lines = lines[:-1]
        text = "\n".join(lines).strip()

    try:
        data = json.loads(text)
    except json.JSONDecodeError:
        return None

    if isinstance(data, dict) and "tool" in data:
        return data
    return None


def write_file(path, content):
    """Write content to a file, creating parent directories as needed."""
    directory = os.path.dirname(path)
    if directory:
        os.makedirs(directory, exist_ok=True)
    with open(path, "w") as f:
        f.write(content)
    return f"Wrote {path}"


def read_file(path):
    """Read and return the contents of a file."""
    with open(path, "r") as f:
        return f.read()


def run_command(command):
    """Run a shell command and return its combined stdout and stderr."""
    result = subprocess.run(command, shell=True, capture_output=True, text=True)
    output = result.stdout
    if result.stderr:
        output += "\n" + result.stderr
    if result.returncode != 0:
        output += f"\n[exit code {result.returncode}]"
    return output


TOOLS = {
    "write_file": write_file,
    "read_file": read_file,
    "run_command": run_command,
}


def main():
    if len(sys.argv) > 1:
        task = " ".join(sys.argv[1:])
    else:
        task = sys.stdin.read().strip()

    if not task:
        print("Usage: python agent.py <task>", file=sys.stderr)
        sys.exit(1)

    messages = [
        {"role": "system", "content": SYSTEM_PROMPT},
        {"role": "user", "content": task},
    ]

    while True:
        response = chat(messages)
        print(f"Agent: {response}", file=sys.stderr)
        messages.append({"role": "assistant", "content": response})

        call = parse_tool_call(response)
        if call is None:
            # Not a tool call; ask user for input
            try:
                user_input = input("You: ")
            except EOFError:
                break
            messages.append({"role": "user", "content": user_input})
            continue

        tool_name = call["tool"]
        if tool_name == "done":
            print("Done.", file=sys.stderr)
            break

        func = TOOLS.get(tool_name)
        args = call.get("args", {})

        if func is None:
            result = f"Error: unknown tool {tool_name}"
        else:
            try:
                result = func(**args)
            except Exception as exc:
                result = f"Error: {exc}"

        print(f"Result: {result}", file=sys.stderr)
        messages.append({"role": "user", "content": f"Result: {result}"})


if __name__ == "__main__":
    main()
