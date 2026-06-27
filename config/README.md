# Configuration

This directory holds environment configuration for tredo.

## Files

- **`tredo.env`** — Your local environment variables (API keys, preferences, mode).  
  ⚠️ This file is untracked by git and contains your secrets. Never commit it.

- **`tredo.env.example`** — Template with all available environment variables,  
  documented with comments and placeholder values. Start here:

  ```bash
  cp config/tredo.env.example config/tredo.env
  ```

  Then edit `config/tredo.env` with your own keys and preferences.

## Usage

Source the env file before running tredo:

```bash
source config/tredo.env
```

Or use the launcher script which handles this automatically:

```bash
./tredo tui
```

## Key Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `LLM_MODEL` | `nemotron-3-nano:4b` | Ollama model to use for inference |
| `LLM_PROVIDER` | `ollama` | LLM provider (ollama, openai, anthropic, gemini) |
| `LLM_ENDPOINT` | `http://localhost:11434` | LLM API endpoint |
| `PORT` | `8080` | HTTP server port (TUI expects 8082) |
| `PAPER_MODE` | `true` | Paper trading mode (always keep true until validated) |

See [`tredo.env.example`](tredo.env.example) for the full list of variables with
explanations, including LLM provider, exchange API keys, notification services,
trading parameters, and service endpoints.
