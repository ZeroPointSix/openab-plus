# ADR: Multi-Model Aggregation Endpoint (Mixture of Agents)

- **Status:** Proposed
- **Date:** 2026-06-29
- **Author:** @chaodu-agent
- **References:** [Hermes Agent — Mixture of Agents](https://hermes-agent.nousresearch.com/docs/user-guide/features/mixture-of-agents), [Ambient Mode](../ambient.md), [Multi-Agent Setup](../multi-agent.md)

---

## 1. User Story & Requirements

As an OpenAB operator running multiple agents (Kiro, Claude, Codex, OpenCode, Copilot, Grok) in the same Discord channel, I want to expose a single OpenAI-compatible API endpoint that fans out a prompt to multiple agents, collects their responses, and returns an aggregated result — so that external callers get multi-model consensus through one standard LLM API call.

As an API consumer, I want to call a single `POST /v1/chat/completions` endpoint and receive a response synthesized from multiple LLM backends, without needing to know which models are behind it or how they communicate.

### Requirements

- Expose an OpenAI-compatible HTTP endpoint (`/v1/chat/completions`) on `localhost`
- Fan out the incoming prompt to N configured agents in a Discord channel
- Collect responses within a configurable timeout window (30–60 seconds)
- Aggregate collected responses into a single final response
- Support multiple aggregation strategies (synthesis, best-of-N, majority vote)
- Return standard OpenAI response format to the caller
- Work with existing multi-agent Discord setup — no changes to agent containers
- Gracefully handle partial results (some agents timeout or fail)
- Optional: support streaming (`stream: true`) after aggregation completes

---

## 2. High-Level Design

### Prior Art: Hermes Agent MoA

Hermes Agent implements Mixture of Agents (MoA) as a **virtual model provider** integrated into its agent loop:

1. User selects an MoA preset via `/model <preset> --provider moa`
2. For each model call, Hermes runs configured **reference models** (without tool schemas) to get diverse perspectives
3. Reference outputs are appended as private context to the **aggregator** model
4. The aggregator produces the final response and can emit tool calls
5. MoA is NOT a separate API endpoint — it's a model-selection concept within the agent

**Key difference for OpenAB:** Hermes directly calls each model's API. OpenAB's approach leverages Discord as the message bus — agents are already running as bots, each with their own backend. We route through Discord rather than making direct API calls.

### OpenAB Architecture

```
                        External Caller
                              │
                    POST /v1/chat/completions
                              │
                              ▼
               ┌──────────────────────────────┐
               │     MoA Gateway Service      │
               │       (localhost:8787)        │
               │                              │
               │  ┌────────────────────────┐  │
               │  │   Request Handler      │  │
               │  │  • Auth (API key)      │  │
               │  │  • Parse OAI format    │  │
               │  └──────────┬─────────────┘  │
               │             │                │
               │             ▼                │
               │  ┌────────────────────────┐  │
               │  │    Fan-Out Engine      │  │
               │  │  • Post prompt to      │  │
               │  │    Discord channel     │  │
               │  │  • Use coordinator bot │  │
               │  │    identity            │  │
               │  └──────────┬─────────────┘  │
               │             │                │
               │             ▼                │
               │  ┌────────────────────────┐  │
               │  │   Response Collector   │  │
               │  │  • Listen for replies  │  │
               │  │  • Timeout window      │  │
               │  │  • Partial results OK  │  │
               │  └──────────┬─────────────┘  │
               │             │                │
               │             ▼                │
               │  ┌────────────────────────┐  │
               │  │     Aggregator         │  │
               │  │  • Synthesis / Vote    │  │
               │  │  • Format as OAI resp  │  │
               │  └────────────────────────┘  │
               └──────────────────────────────┘
                              │
              Discord Channel (message bus)
                              │
          ┌───────────┬───────┼───────┬───────────┐
          ▼           ▼       ▼       ▼           ▼
       ┌─────┐   ┌───────┐ ┌─────┐ ┌──────┐  ┌──────┐
       │Kiro │   │Claude │ │Codex│ │Grok  │  │ ...  │
       │Agent│   │Agent  │ │Agent│ │Agent │  │      │
       └─────┘   └───────┘ └─────┘ └──────┘  └──────┘
```

### Message Flow

```
1. Caller → MoA Gateway:  POST /v1/chat/completions { messages: [...] }
2. Gateway → Discord:     Posts prompt in designated MoA channel using coordinator bot
3. Discord → Agents:      Each agent sees the message (ambient mode or @mention)
4. Agents → Discord:      Each agent replies in the thread
5. Gateway ← Discord:     Collector gathers replies within timeout window
6. Gateway (Aggregator):  Synthesizes collected responses into one
7. Gateway → Caller:      Returns OpenAI-format response
```

---

## 3. Fan-Out Strategies

### Option A: Ambient Mode (Recommended)

Leverage existing ambient mode. The MoA channel has all agents configured with `allow_bot_messages = true`. The gateway posts a prompt; agents naturally respond within their `flush_interval_seconds`.

**Pros:** No per-agent @mention logic, scales by simply adding bots to the channel
**Cons:** Relies on ambient flush timing, agents may not all respond

### Option B: Explicit @mention

Gateway posts a message @mentioning each configured agent. Each agent responds immediately to the mention.

**Pros:** Guaranteed immediate response from each agent, predictable timing
**Cons:** Requires knowing each agent's Discord ID, more intrusive

### Option C: Hybrid

Post the prompt normally (triggers ambient), but also @mention agents that haven't responded after half the timeout.

---

## 4. Response Collection

The collector uses a mechanism similar to ambient mode's buffered collection:

```toml
[moa]
enabled = true
channel_id = "1234567890"           # Dedicated MoA channel
timeout_seconds = 45                # Max wait for responses
min_responses = 2                   # Minimum responses before aggregating
max_responses = 6                   # Stop collecting after N responses
early_complete_seconds = 10         # If min met, wait this long for stragglers
```

### Collection Logic

```
start_time = now()
responses = []

loop:
  if len(responses) >= max_responses → break
  if elapsed > timeout_seconds → break
  if len(responses) >= min_responses AND elapsed > early_complete_seconds → break
  wait for next reply in thread
  responses.push(reply)

return responses  # may be partial (>= 0)
```

---

## 5. Aggregation Strategies

### Strategy 1: Synthesis (Default)

Call a designated aggregator model (e.g., the coordinator's own LLM backend) with all collected responses as context:

```
System: You are an aggregator. Multiple AI models have answered the same question.
        Synthesize their responses into one high-quality answer.
        Preserve the best insights from each. Resolve contradictions.

User: [original prompt]

Context:
- Model A (Kiro/Claude): [response A]
- Model B (Codex): [response B]
- Model C (Grok): [response C]

Produce a single, coherent response.
```

### Strategy 2: Best-of-N

Use a judge model to rank responses and return the highest-quality one unchanged.

### Strategy 3: Majority Vote

For tasks with discrete answers (code review verdicts, yes/no decisions), count the majority answer.

---

## 6. API Interface

### Request (OpenAI-compatible)

```bash
curl http://localhost:8787/v1/chat/completions \
  -H "Authorization: Bearer $MOA_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "moa-default",
    "messages": [
      {"role": "user", "content": "Review this architecture and suggest improvements..."}
    ],
    "temperature": 0.7
  }'
```

### Response (OpenAI-compatible)

```json
{
  "id": "moa-abc123",
  "object": "chat.completion",
  "created": 1719619200,
  "model": "moa-default",
  "choices": [
    {
      "index": 0,
      "message": {
        "role": "assistant",
        "content": "Based on analysis from multiple models..."
      },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 0,
    "completion_tokens": 0,
    "total_tokens": 0
  },
  "metadata": {
    "responses_collected": 4,
    "agents_responded": ["kiro", "claude", "codex", "grok"],
    "aggregation_strategy": "synthesis",
    "collection_time_ms": 32450
  }
}
```

### Model Names

Multiple presets can be configured, each mapping to a different channel or agent subset:

| Model Name | Channel | Agents | Strategy |
|------------|---------|--------|----------|
| `moa-default` | #moa-general | All agents | Synthesis |
| `moa-review` | #moa-review | Claude, Kiro, Codex | Synthesis |
| `moa-vote` | #moa-vote | All agents | Majority vote |

---

## 7. Configuration

```toml
[moa]
enabled = true
listen_address = "127.0.0.1:8787"
api_key = "sk-moa-..."                      # Simple bearer token auth

[moa.presets.default]
channel_id = "1234567890"
timeout_seconds = 45
min_responses = 2
max_responses = 6
early_complete_seconds = 10
aggregation_strategy = "synthesis"           # synthesis | best_of_n | majority_vote
aggregator_model = "coordinator"             # which agent's LLM does the synthesis

[moa.presets.review]
channel_id = "9876543210"
timeout_seconds = 60
min_responses = 3
aggregation_strategy = "synthesis"
```

---

## 8. Who Calls This Endpoint?

### Use Cases

1. **Other services in the cluster** — A CI pipeline or internal tool calls the MoA endpoint for multi-model code review or analysis, treating it like any other LLM API.

2. **Local development tools** — IDE extensions, CLI tools, or scripts configured to use `http://localhost:8787/v1/chat/completions` as their LLM endpoint get automatic multi-model consensus.

3. **LLM routers / orchestrators** — Tools like LiteLLM, OpenRouter proxies, or custom orchestrators can register the MoA endpoint as a "model" and route specific tasks to it.

4. **The coordinator agent itself** — The coordinator (超渡法師) could use this endpoint for tasks that benefit from multi-model consensus before producing a final answer.

5. **Hermes Agent integration** — Configure Hermes to use the MoA endpoint as a custom provider, giving Hermes access to OpenAB's multi-agent consensus as a single model.

### Exposure Options

| Scope | How | When to use |
|-------|-----|-------------|
| Pod-local only | `127.0.0.1:8787` | Single-pod testing |
| Cluster-internal | K8s Service (ClusterIP) | Other services in same cluster |
| External | Ingress + auth | Remote callers (with proper auth) |

The default is **localhost-only** — safe by default, opt-in to broader exposure.

---

## 9. Differences from Hermes MoA

| Aspect | Hermes MoA | OpenAB MoA |
|--------|-----------|------------|
| Message bus | Direct API calls to each provider | Discord channel as message bus |
| Agent management | Config file with provider/model pairs | Existing bot deployments |
| Latency | ~5–15s (parallel API calls) | ~30–60s (Discord message flow) |
| Tool calls | Aggregator can emit tool calls | Aggregator returns text only (v1) |
| Exposure | Internal to agent loop, not an API | Standalone OpenAI-compatible endpoint |
| Adding models | Edit config.yaml | Add a bot to the channel |
| Cost model | Pay per API call to each provider | Each bot uses its own credentials |

---

## 10. Future Considerations

- **Streaming support:** Buffer aggregated response, then stream it back to the caller
- **Tool-call passthrough:** Let the aggregator emit tool calls (requires tool schema in the MoA endpoint)
- **Caching:** Cache identical prompts to avoid re-querying agents
- **Metrics:** Track per-agent response times, quality scores, participation rates
- **Weighted aggregation:** Weight agent responses by historical quality on similar tasks
- **Recursive MoA:** Allow a preset's aggregator to be another MoA preset (Hermes explicitly blocks this; we should evaluate)

---

## 11. Open Questions

1. **Should MoA be a separate binary or built into the main OAB gateway?**
   - Separate: simpler, independently deployable, clear boundary
   - Built-in: shares Discord connection, less operational overhead

2. **How to handle conversation context (multi-turn)?**
   - Option A: Stateless — each call is independent, caller manages history
   - Option B: Session-based — gateway maintains a thread per conversation

3. **Should agents know they're in MoA mode?**
   - If yes: they can tailor responses (shorter, more analytical)
   - If no: responses are natural but may be verbose for aggregation
