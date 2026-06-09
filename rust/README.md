# Agent the Hard Way in Rust

Generated with: Claude Code + Opus 4.7

LOC:

```
     303 json.rs
     109 main.rs
     189 ollama.rs
     129 tools.rs
     730 total
```

## Observations

Fun observation from the [src/ollama.rs](src/ollama.rs):

```text
// Ollama Cloud chat API client. HTTPS is delegated to the system `curl`
// because the Rust standard library does not include TLS.
```

Because instructions said only stdlib can be used, I guess this is a reasonable way to implement the HTTPS calls then.
