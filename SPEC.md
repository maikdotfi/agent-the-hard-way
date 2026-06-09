# Coding Agent the Hard Way

Write a coding agent in bash with following restrictions:

- only use common shell utilities like grep, sed, awk etc. nothing exotic
- additionally you can use jq for JSON and curl for HTTP(S)
- only support Ollama Cloud API, no other

Guidelines:

- favor readability over speed
- don't create abstractions that are not used

You know your agent works when it can generate a hello world application in the given language in sub directory and that code prints "hello world" when executed outside the agent's loop.

You are already authenticated to Ollama Cloud API using `OLLAMA_API_KEY` env variable, trust it is there.
