# Reference Architecture: OpenAB with Defined Kiro Agents

> **Agent-friendly prompt:**
>
> ```text
> Configure three global Kiro agents named opus48, sonnet5, and auto from
> https://github.com/openabdev/openab/blob/main/docs/refarch/kiro-with-defined-agents.md,
> then configure this OpenAB instance to launch the agent I select.
> ```

Use named Kiro agents to give each OpenAB deployment an explicit model policy. This example defines three reusable agents:

| Agent name | Agent `model` value | Intended selection |
|---|---|---|
| `opus48` | `claude-opus-4.8` | Claude Opus 4.8 |
| `sonnet5` | `claude-sonnet-5` | Claude Sonnet 5 |
| `auto` | `auto` | Kiro automatic model selection |

The agent names are aliases. OpenAB selects an alias when it launches Kiro ACP; Kiro then loads that agent's configuration and uses its configured model.

## Why This Architecture

- **Deterministic startup** — each OpenAB instance explicitly selects a named agent instead of relying on the ambient Kiro default.
- **Separation of concerns** — Kiro agent JSON owns the model and tool policy, while OpenAB `config.toml` only selects the agent.
- **Easy model changes** — switch the agent name in `config.toml`, restart the instance, and new ACP sessions use the selected policy.
- **Reusable configuration** — the same named agents can be used by Kiro CLI directly and by multiple OpenAB deployments.

## Architecture

```text
+--------------------------- OpenAB instance ---------------------------+
|                                                                       |
|  config.toml                                                          |
|  [agent]                                                              |
|  command = "kiro-cli"                                                 |
|  args = ["acp", "--agent", "opus48", "--trust-all-tools"]             |
|                         |                                             |
+-------------------------|---------------------------------------------+
                          | starts ACP over stdio
                          v
                 +--------------------+
                 | kiro-cli acp       |
                 | --agent opus48     |
                 +---------+----------+
                           | loads
                           v
              $HOME/.kiro/agents/opus48.json
                           |
                           | "model": "claude-opus-4.8"
                           v
                    Claude Opus 4.8

Change only the selected alias for another deployment:
  --agent sonnet5  ->  claude-sonnet-5
  --agent auto     ->  auto
```

No additional OpenAB service or infrastructure is required. The model used by each agent is billed according to the user's Kiro plan.

## Prerequisites

- OpenAB with the Kiro CLI backend installed and authenticated. See [Kiro CLI (Default Agent)](../kiro.md).
- A Kiro CLI release whose `kiro-cli acp --help` output includes `--agent <AGENT>`.
- Access to the requested models in your Kiro account and region. Run `/model` in Kiro CLI to inspect currently available models.
- Write access to the Kiro home directory used by the OpenAB process.

## Create the Three Kiro Agents

Global agents live in `$HOME/.kiro/agents/` and are available across workspaces. Create the directory first:

```bash
mkdir -p "$HOME/.kiro/agents"
```

### `opus48`

Create `$HOME/.kiro/agents/opus48.json`:

```json
{
  "name": "opus48",
  "description": "General-purpose agent using Claude Opus 4.8",
  "model": "claude-opus-4.8",
  "tools": ["*"],
  "allowedTools": ["@builtin"]
}
```

### `sonnet5`

Create `$HOME/.kiro/agents/sonnet5.json`:

```json
{
  "name": "sonnet5",
  "description": "General-purpose agent using Claude Sonnet 5",
  "model": "claude-sonnet-5",
  "tools": ["*"],
  "allowedTools": ["@builtin"]
}
```

### `auto`

Create `$HOME/.kiro/agents/auto.json`:

```json
{
  "name": "auto",
  "description": "General-purpose agent using automatic model selection",
  "model": "auto",
  "tools": ["*"],
  "allowedTools": ["@builtin"]
}
```

`"tools": ["*"]` makes every available tool visible to the agent. `"allowedTools": ["@builtin"]` auto-approves all built-in tools; `"*"` is not supported in `allowedTools`. The OpenAB launch examples below also use `--trust-all-tools`, which auto-approves tool permission requests for the non-interactive ACP process.

> **Security note:** `--trust-all-tools` grants broad tool execution authority. Restrict which Discord, Slack, or gateway users and channels can reach the OpenAB instance. For a least-privilege deployment, omit this flag or replace it with Kiro's selective trust controls.

## Validate the Agent Files

Validate all three files against the installed Kiro agent schema:

```bash
for agent in opus48 sonnet5 auto; do
  kiro-cli agent validate --path "$HOME/.kiro/agents/${agent}.json"
done

kiro-cli agent list
```

Kiro also supports workspace agents under `.kiro/agents/`. A workspace agent takes precedence over a global agent with the same name. For containers, make sure the files are created or mounted under the `$HOME` used by the OpenAB process; a different user's home directory will not be discovered.

## Select an Agent in OpenAB `config.toml`

An OpenAB config has one `[agent]` backend. Set `command` to `kiro-cli` and pass the selected agent name as separate entries in `args`.

### Select `opus48`

```toml
[agent]
command = "kiro-cli"
args = ["acp", "--agent", "opus48", "--trust-all-tools"]
```

### Select `sonnet5`

```toml
[agent]
command = "kiro-cli"
args = ["acp", "--agent", "sonnet5", "--trust-all-tools"]
```

### Select `auto`

```toml
[agent]
command = "kiro-cli"
args = ["acp", "--agent", "auto", "--trust-all-tools"]
```

The resulting command is, for example:

```bash
kiro-cli acp --agent opus48 --trust-all-tools
```

`--agent <AGENT>` tells Kiro which agent to use when starting the first ACP session. Because the selected agent JSON contains `model`, the session starts with that agent's configured model. This avoids depending on the global `chat.defaultAgent` or `chat.defaultModel` settings.

To run all three policies concurrently, deploy three OpenAB instances and give each instance its own `config.toml` with a different `--agent` value. Do not add three `[agent]` sections to one config.

## Deploy and Verify

1. Restart or redeploy OpenAB after changing `config.toml` so it starts a new Kiro ACP process.
2. Confirm that the expected argument is present:

   ```bash
   ps -ef | grep '[k]iro-cli acp'
   ```

3. Send a test message through the configured OpenAB adapter and confirm the ACP session responds.
4. If Kiro reports that a model is unavailable, run `/model` in an interactive Kiro session to check the model IDs available to the account and region, then update the corresponding agent JSON.

## Day-2 Operations

### Change the model behind an alias

Edit the alias's JSON file, validate it, and restart OpenAB. The OpenAB `config.toml` does not need to change:

```bash
kiro-cli agent validate --path "$HOME/.kiro/agents/opus48.json"
```

### Change the selected alias

Change only the value after `--agent` in `config.toml`, then restart OpenAB:

```toml
[agent]
command = "kiro-cli"
args = ["acp", "--agent", "sonnet5", "--trust-all-tools"]
```

### Set a Kiro-wide fallback

Agent-specific `model` values are preferred for this architecture. Kiro's global fallback can still be configured separately:

```bash
kiro-cli settings chat.defaultModel <model-id>
```

If an agent does not specify `model`, Kiro uses the default model. If its configured model is unavailable, Kiro falls back to the default model.

## Troubleshooting

| Symptom | Check |
|---|---|
| `Agent not found` | Confirm `$HOME/.kiro/agents/<name>.json` exists for the same user running OpenAB and that the filename exactly matches `--agent`. |
| Agent file is rejected | Run `kiro-cli agent validate --path <file>` and fix the reported JSON or schema error. |
| Wrong agent starts | Inspect the running process arguments and check for a workspace agent with the same name overriding the global file. |
| Wrong or unavailable model | Check the exact `model` value and use `/model` to see models available in the current account and region. |
| Tool approval blocks ACP | Include `--trust-all-tools`, or configure the precise trusted tools required by the deployment. |
| Config change has no effect | Restart OpenAB so it launches a new ACP process and starts a new session with the updated `--agent` value. |

## Important Notes

- The Kiro agent field is named `model`, not `default_model`.
- Agent names and filenames are case-sensitive.
- `--agent` selects the agent for the first ACP session; it is not an OpenAB model-setting flag.
- Keep agent files on persistent storage in containerized deployments so they survive restarts.
- Model availability is account- and region-dependent and may change over time.
