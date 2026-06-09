#!/usr/bin/env bash
#
# A minimal coding agent that uses the Ollama Cloud API.
#
# Usage: ./agent.sh "<task in natural language>"
#
# Authentication: set the OLLAMA_API_KEY environment variable.
# Model: override with OLLAMA_MODEL (default: gpt-oss:120b).
#
# The agent runs in a loop: it asks the model what to do, executes any
# tool calls the model requests, feeds the results back, and stops when
# the model emits a final text reply (no tool calls).

set -euo pipefail

# ---------- Configuration ----------

API_URL="${OLLAMA_API_URL:-https://ollama.com/api/chat}"
MODEL="${OLLAMA_MODEL:-gpt-oss:120b}"

if [[ -z "${OLLAMA_API_KEY:-}" ]]; then
  echo "Error: OLLAMA_API_KEY is not set." >&2
  exit 1
fi

if [[ $# -lt 1 ]]; then
  echo "Usage: $0 <task>" >&2
  exit 1
fi

TASK="$1"

# ---------- Tool definitions ----------
#
# Three tools are exposed to the model: read_file, write_file, run_command.
# The model is told to keep working until the task is complete.

TOOLS=$(cat <<'JSON'
[
  {
    "type": "function",
    "function": {
      "name": "read_file",
      "description": "Read the contents of a file at the given path. Use this to inspect existing files before editing them.",
      "parameters": {
        "type": "object",
        "properties": {
          "path": {
            "type": "string",
            "description": "Path to the file to read, relative to the current working directory."
          }
        },
        "required": ["path"]
      }
    }
  },
  {
    "type": "function",
    "function": {
      "name": "write_file",
      "description": "Create or overwrite a file with the given content. Any missing parent directories will be created automatically.",
      "parameters": {
        "type": "object",
        "properties": {
          "path": {
            "type": "string",
            "description": "Path to the file to write, relative to the current working directory."
          },
          "content": {
            "type": "string",
            "description": "The full content to write to the file."
          }
        },
        "required": ["path", "content"]
      }
    }
  },
  {
    "type": "function",
    "function": {
      "name": "run_command",
      "description": "Run a shell command in the current working directory and return its combined stdout and stderr. Use this to create directories, run the code you wrote, and verify your work.",
      "parameters": {
        "type": "object",
        "properties": {
          "command": {
            "type": "string",
            "description": "The shell command to run."
          }
        },
        "required": ["command"]
      }
    }
  }
]
JSON
)

# ---------- System prompt ----------

SYSTEM_PROMPT="You are a coding agent running on the user's machine. You can read files, write files, and run shell commands via the tools provided.

To call a tool, emit a tool call in the model's native format. The system will run it and report the result back to you.

Work step by step: inspect what's there if needed, make changes, and verify them by running the code. When the task is fully done, reply with a short plain-text summary and do not call any more tools."

# ---------- Conversation state ----------

# Messages is a JSON array. We start with the system prompt and the user's task.
MESSAGES=$(jq -n \
  --arg system "$SYSTEM_PROMPT" \
  --arg task "$TASK" \
  '[{role: "system", content: $system}, {role: "user", content: $task}]')

# ---------- Tool implementations ----------

run_tool() {
  # $1 is the tool name, $2 is the arguments as a JSON object string.
  local name="$1"
  local args="$2"

  case "$name" in
    read_file)
      local path
      path=$(echo "$args" | jq -r '.path')
      if [[ ! -f "$path" ]]; then
        echo "Error: file not found: $path"
        return 0
      fi
      cat "$path"
      ;;

    write_file)
      local path content
      path=$(echo "$args" | jq -r '.path')
      content=$(echo "$args" | jq -r '.content')
      # Ensure the parent directory exists.
      local dir
      dir=$(dirname "$path")
      if [[ "$dir" != "." && ! -d "$dir" ]]; then
        mkdir -p "$dir"
      fi
      printf '%s' "$content" > "$path"
      echo "Wrote $(wc -c < "$path" | tr -d ' ') bytes to $path"
      ;;

    run_command)
      local command
      command=$(echo "$args" | jq -r '.command')
      # Run the command, capture stdout+stderr, and prepend a header
      # so the model can see exit status.
      local output
      local status
      output=$(bash -c "$command" 2>&1) && status=0 || status=$?
      echo "exit_status: $status"
      if [[ -n "$output" ]]; then
        echo "$output"
      fi
      ;;

    *)
      echo "Error: unknown tool '$name'"
      ;;
  esac
}

# ---------- Main loop ----------

ITERATION=0
MAX_ITERATIONS=50

while (( ITERATION < MAX_ITERATIONS )); do
  ITERATION=$(( ITERATION + 1 ))

  # Build the request body. `stream: false` keeps the response a single
  # JSON object, which is much easier to parse.
  REQUEST=$(jq -n \
    --arg model "$MODEL" \
    --argjson messages "$MESSAGES" \
    --argjson tools "$TOOLS" \
    '{
       model: $model,
       messages: $messages,
       tools: $tools,
       stream: false
     }')

  # Call the Ollama Cloud API.
  RESPONSE=$(curl -sS -X POST "$API_URL" \
    -H "Authorization: Bearer $OLLAMA_API_KEY" \
    -H "Content-Type: application/json" \
    -d "$REQUEST")

  # Surface API errors.
  if echo "$RESPONSE" | jq -e 'has("error")' >/dev/null 2>&1; then
    echo "API error: $(echo "$RESPONSE" | jq -c '.error')" >&2
    exit 1
  fi

  # Pull the assistant message out of the response.
  ASSISTANT=$(echo "$RESPONSE" | jq '.message')

  # Append the assistant turn to the conversation.
  MESSAGES=$(echo "$MESSAGES" | jq --argjson msg "$ASSISTANT" '. + [$msg]')

  # If there are no tool calls, the model is done; print its reply and exit.
  TOOL_CALL_COUNT=$(echo "$ASSISTANT" | jq '.tool_calls | length // 0')
  if (( TOOL_CALL_COUNT == 0 )); then
    echo
    echo "=== Agent finished ==="
    echo "$ASSISTANT" | jq -r '.content // ""'
    exit 0
  fi

  # Show what the agent is doing, for the human watching.
  echo
  echo "=== Iteration $ITERATION ==="
  echo "$ASSISTANT" | jq -r '.tool_calls[] | "  -> \(.function.name)(\(.function.arguments | tostring))"'

  # Execute each requested tool call and feed the result back as a tool message.
  for i in $(seq 0 $(( TOOL_CALL_COUNT - 1 ))); do
    CALL=$(echo "$ASSISTANT" | jq -c ".tool_calls[$i]")
    TOOL_NAME=$(echo "$CALL" | jq -r '.function.name')
    # Some models return arguments as a JSON object, others as a JSON
    # string. Normalise to a JSON object string either way.
    RAW_ARGS=$(echo "$CALL" | jq -c '.function.arguments')
    if [[ "$RAW_ARGS" == \"* ]]; then
      TOOL_ARGS=$(echo "$CALL" | jq -r '.function.arguments')
    else
      TOOL_ARGS="$RAW_ARGS"
    fi

    RESULT=$(run_tool "$TOOL_NAME" "$TOOL_ARGS")

    TOOL_MSG=$(jq -n \
      --arg name "$TOOL_NAME" \
      --arg content "$RESULT" \
      '{role: "tool", tool_name: $name, content: $content}')

    MESSAGES=$(echo "$MESSAGES" | jq --argjson msg "$TOOL_MSG" '. + [$msg]')
  done
done

echo "Error: agent did not finish within $MAX_ITERATIONS iterations." >&2
exit 1
