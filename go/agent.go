// A minimal coding agent that uses the Ollama Cloud API.
//
// Usage: go run agent.go "<task in natural language>"
//
// Authentication: set the OLLAMA_API_KEY environment variable.
// Model: override with OLLAMA_MODEL (default: gpt-oss:120b).
//
// The agent runs in a loop: it asks the model what to do, executes any
// tool calls the model requests, feeds the results back, and stops when
// the model emits a final text reply (no tool calls).
package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

const (
	apiURL = "https://ollama.com/api/chat"
	model  = "gpt-oss:120b"
)

// maxIterations caps the agent loop so a runaway model cannot loop forever.
const maxIterations = 50

// toolDef is the JSON description of a tool, as sent to the Ollama API.
// We declare it as a literal so the schema travels with the code.
var tools = []map[string]any{
	{
		"type": "function",
		"function": map[string]any{
			"name": "read_file",
			"description": "Read the contents of a file at the given path. " +
				"Use this to inspect existing files before editing them.",
			"parameters": map[string]any{
				"type": "object",
				"properties": map[string]any{
					"path": map[string]any{
						"type":        "string",
						"description": "Path to the file to read, relative to the current working directory.",
					},
				},
				"required": []string{"path"},
			},
		},
	},
	{
		"type": "function",
		"function": map[string]any{
			"name": "write_file",
			"description": "Create or overwrite a file with the given content. " +
				"Any missing parent directories will be created automatically.",
			"parameters": map[string]any{
				"type": "object",
				"properties": map[string]any{
					"path": map[string]any{
						"type":        "string",
						"description": "Path to the file to write, relative to the current working directory.",
					},
					"content": map[string]any{
						"type":        "string",
						"description": "The full content to write to the file.",
					},
				},
				"required": []string{"path", "content"},
			},
		},
	},
	{
		"type": "function",
		"function": map[string]any{
			"name": "run_command",
			"description": "Run a shell command in the current working directory and return its " +
				"combined stdout and stderr. Use this to create directories, run the code you wrote, " +
				"and verify your work.",
			"parameters": map[string]any{
				"type": "object",
				"properties": map[string]any{
					"command": map[string]any{
						"type":        "string",
						"description": "The shell command to run.",
					},
				},
				"required": []string{"command"},
			},
		},
	},
}

const systemPrompt = "You are a coding agent running on the user's machine. " +
	"You can read files, write files, and run shell commands via the tools provided.\n\n" +
	"To call a tool, emit a tool call in the model's native format. " +
	"The system will run it and report the result back to you.\n\n" +
	"Work step by step: inspect what's there if needed, make changes, and verify them " +
	"by running the code. When the task is fully done, reply with a short plain-text " +
	"summary and do not call any more tools."

func main() {
	if os.Getenv("OLLAMA_API_KEY") == "" {
		fmt.Fprintln(os.Stderr, "Error: OLLAMA_API_KEY is not set.")
		os.Exit(1)
	}
	if len(os.Args) < 2 {
		fmt.Fprintf(os.Stderr, "Usage: %s <task>\n", os.Args[0])
		os.Exit(1)
	}
	task := os.Args[1]

	// Conversation state. We start with the system prompt and the user's task.
	messages := []map[string]any{
		{"role": "system", "content": systemPrompt},
		{"role": "user", "content": task},
	}

	for i := 1; i <= maxIterations; i++ {
		assistant, err := chat(messages)
		if err != nil {
			fmt.Fprintln(os.Stderr, "Error:", err)
			os.Exit(1)
		}

		// Append the assistant turn to the conversation.
		messages = append(messages, assistant)

		toolCalls, _ := assistant["tool_calls"].([]any)
		if len(toolCalls) == 0 {
			// No tool calls -> the model is done.
			fmt.Println()
			fmt.Println("=== Agent finished ===")
			if content, ok := assistant["content"].(string); ok {
				fmt.Println(content)
			}
			return
		}

		fmt.Println()
		fmt.Printf("=== Iteration %d ===\n", i)
		for _, raw := range toolCalls {
			call := raw.(map[string]any)
			fn := call["function"].(map[string]any)
			argsJSON, _ := json.Marshal(fn["arguments"])
			fmt.Printf("  -> %s(%s)\n", fn["name"], string(argsJSON))
		}

		// Execute each requested tool call and feed the result back.
		for _, raw := range toolCalls {
			call := raw.(map[string]any)
			fn := call["function"].(map[string]any)
			name, _ := fn["name"].(string)
			// Some models return arguments as a JSON object, others as a JSON
			// string. Normalise to a map[string]any either way.
			args, err := normaliseArgs(fn["arguments"])
			if err != nil {
				messages = append(messages, toolMessage(name, fmt.Sprintf("Error: bad arguments: %v", err)))
				continue
			}

			result := runTool(name, args)
			messages = append(messages, toolMessage(name, result))
		}
	}

	fmt.Fprintf(os.Stderr, "Error: agent did not finish within %d iterations.\n", maxIterations)
	os.Exit(1)
}

// chat sends the conversation to the Ollama Cloud API and returns the
// assistant message from the response.
func chat(messages []map[string]any) (map[string]any, error) {
	body := map[string]any{
		"model":    envOr("OLLAMA_MODEL", model),
		"messages": messages,
		"tools":    tools,
		"stream":   false,
	}
	buf, err := json.Marshal(body)
	if err != nil {
		return nil, err
	}

	req, err := http.NewRequest("POST", apiURL, bytes.NewReader(buf))
	if err != nil {
		return nil, err
	}
	req.Header.Set("Authorization", "Bearer "+os.Getenv("OLLAMA_API_KEY"))
	req.Header.Set("Content-Type", "application/json")

	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	raw, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}

	// The API returns {"error": ...} on failure; surface it.
	var errResp struct {
		Error any `json:"error"`
	}
	if json.Unmarshal(raw, &errResp) == nil && errResp.Error != nil {
		return nil, fmt.Errorf("API error: %v", errResp.Error)
	}

	var ok struct {
		Message map[string]any `json:"message"`
	}
	if err := json.Unmarshal(raw, &ok); err != nil {
		return nil, fmt.Errorf("decoding response: %w (body: %s)", err, string(raw))
	}
	return ok.Message, nil
}

// normaliseArgs turns the model's arguments into a map regardless of
// whether the API delivered them as a JSON object or a JSON string.
func normaliseArgs(raw any) (map[string]any, error) {
	switch v := raw.(type) {
	case map[string]any:
		return v, nil
	case string:
		var m map[string]any
		if err := json.Unmarshal([]byte(v), &m); err != nil {
			return nil, err
		}
		return m, nil
	default:
		return nil, fmt.Errorf("unsupported arguments type %T", raw)
	}
}

// runTool dispatches to the right implementation based on the tool name.
func runTool(name string, args map[string]any) string {
	switch name {
	case "read_file":
		path, _ := args["path"].(string)
		data, err := os.ReadFile(path)
		if err != nil {
			return fmt.Sprintf("Error: %v", err)
		}
		return string(data)

	case "write_file":
		path, _ := args["path"].(string)
		content, _ := args["content"].(string)
		// Ensure the parent directory exists.
		if dir := filepath.Dir(path); dir != "." && dir != "" {
			if err := os.MkdirAll(dir, 0o755); err != nil {
				return fmt.Sprintf("Error: %v", err)
			}
		}
		if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
			return fmt.Sprintf("Error: %v", err)
		}
		return fmt.Sprintf("Wrote %d bytes to %s", len(content), path)

	case "run_command":
		command, _ := args["command"].(string)
		// Use "bash -c" so we get the same shell semantics as the bash agent.
		out, err := exec.Command("bash", "-c", command).CombinedOutput()
		status := 0
		if err != nil {
			// exec.ExitError carries the exit code; anything else is a
			// failure to even start the process.
			if ee, ok := err.(*exec.ExitError); ok {
				status = ee.ExitCode()
			} else {
				return fmt.Sprintf("Error: %v", err)
			}
		}
		header := fmt.Sprintf("exit_status: %d", status)
		if len(out) == 0 {
			return header
		}
		return header + "\n" + strings.TrimRight(string(out), "\n")

	default:
		return fmt.Sprintf("Error: unknown tool %q", name)
	}
}

// toolMessage builds the message we send back to the model after running a tool.
func toolMessage(name, content string) map[string]any {
	return map[string]any{
		"role":       "tool",
		"tool_name":  name,
		"content":    content,
	}
}

func envOr(key, fallback string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return fallback
}
