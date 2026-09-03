# ADR: Codeg UI OpenAB 会话最小契约

- **状态：** 提议
- **日期：** 2026-09-03
- **跟踪：** [ZER-950](https://linear.app/zerodotsix/issue/ZER-950/p2-1-openab-plus-冻结-codeg-ui-接入当前后端的最小契约)
- **父需求：** [ZER-949](https://linear.app/zerodotsix/issue/ZER-949)
- **对应仓库：** `ZeroPointSix/openab-plus`
- **契约 fixture：** [`docs/fixtures/codeg-session-contract-v1.json`](../fixtures/codeg-session-contract-v1.json)

---

## 1. 背景与问题

阶段二只需要让 Codeg 工作台接入当前 OpenAB 后端。Codeg 的组件不应依赖
`LocalRuntime`、`SessionPool` 或 ACP 内部类型；当前后端也不应为了迎合 Codeg
的组件模型而新增第二套会话存储、事件流或协议。

本 ADR 冻结前端 transport 可以依赖的最小 HTTP/SSE 面。它只描述当前后端的
接入边界，不提前设计未来的 Computer、远程 daemon、分布式 Hub 或完整的
Codeg RPC facade。

```text
Codeg UI
  ↓
OpenABTransport（Codeg 内部适配层）
  ↓
当前 OpenAB /api/v1
  ↓
LocalRuntime → SessionPool → ACP CLI
```

## 2. 决策

Codeg 只通过当前 `/api/v1` 会话控制面通信。当前已有的五个读接口保持原有
返回形状，阶段二只增加发送文本和取消当前任务两个接口。请求、响应与 SSE
样例固定在契约 fixture 中；后续实现以该 fixture 编写契约测试。

### 2.1 冻结的七个接口

| 能力 | 方法与路径 | 本阶段归属 |
| --- | --- | --- |
| 会话列表 | `GET /api/v1/sessions` | 已有，直接复用 |
| 创建会话 | `POST /api/v1/sessions` | 已有，直接复用 |
| 会话详情 | `GET /api/v1/sessions/{session_id}` | 已有，直接复用 |
| 历史与活动 | `GET /api/v1/sessions/{session_id}/transcript` | 已有，直接复用 |
| 实时更新与重放 | `GET /api/v1/sessions/events` | 已有，直接复用 |
| 发送文本 | `POST /api/v1/sessions/{session_id}/messages` | 阶段二新增 |
| 取消当前任务 | `POST /api/v1/sessions/{session_id}/cancel` | 阶段二新增 |

`session_id` 在 URL 中按单个路径段进行百分号编码，例如
`admin:fixture-session` 在 fixture 的请求路径中为
`admin%3Afixture-session`。UI 将其视为不透明字符串，不解析 `admin:`、
`acp:` 或其他前缀。

### 2.2 通用传输约定

- 所有七个接口都使用现有 Admin 鉴权：`Authorization: Bearer <token>`。
- Codeg 不把 token 放入 URL、query string、SSE cursor 或错误正文。
- `/api/v1/sessions/events` 由 fetch-based SSE 客户端建立，以便发送
  `Authorization` header；不依赖原生 `EventSource` 的 query token 方案。
- 默认部署为同源。独立 UI origin 是后续 transport 的配置能力，不改变本阶段
  的后端鉴权协议。
- 当前服务仍兼容历史 `x-openab-admin-token` header，但 Codeg fixture 和新
  transport 只使用 Bearer header。
- 成功和业务失败均使用 JSON；现有错误响应保持最小形状
  `{ "error": "..." }`。
- 部署必须配置 `GATEWAY_ADMIN_TOKEN` 或回退的 `OPENAB_ADMIN_TOKEN`；服务端未配置
  token 时返回 `503`，而不是把请求当作匿名请求处理。

### 2.3 现有接口形状

- `GET /api/v1/sessions` 返回 snapshot 数组。列表行可以额外带有后端根据首个
  user entry 推导的 `title` 字段；snapshot 本身的字段不变。
- `POST /api/v1/sessions` 接受现有 `profile_id` 与可选 `overrides`，成功返回
  `201 Created` 和完整 `SessionSnapshot`。
- `GET /api/v1/sessions/{session_id}` 成功返回完整 `SessionSnapshot`；不存在
  返回 `404` 与 `{ "error": "session not found" }`。
- transcript 全量请求返回 `TranscriptSnapshot`。带 `?after=<entry_sequence>`
  时返回当前 session 的有界 mutation history；`after` 使用每会话 entry 序号，
  不是 SSE 全局序号。
- transcript entry 的 `entry_id` 是稳定可 upsert 的身份；assistant 文本和 tool
  生命周期更新会复用同一个 `entry_id`。`role` 使用 `user`、`assistant`、
  `system`、`tool`，thinking 通过 `role: assistant` 与 `status: thinking`
  表示。

### 2.4 新增写接口

#### 发送文本

请求 body 必须只有一个文本字段：

```json
{
  "text": "请检查当前变更并运行测试"
}
```

`text` 去除首尾空白后不得为空；本阶段不接受图片、附件、ACP content block、
工具参数或其他消息类型。

成功只表示当前回合已接受，不等待 Agent 完成：

```http
HTTP/1.1 202 Accepted
Content-Type: application/json
```

```json
{
  "accepted": true,
  "session_id": "admin:fixture-session"
}
```

响应不得包含最终 assistant 文本。user entry、assistant 文本、thinking、tool
call/update/result 与最终状态都通过 transcript 和统一 SSE 流发布。

#### 取消当前任务

取消请求不携带业务字段；fixture 使用无 body 请求：

```http
POST /api/v1/sessions/admin%3Afixture-session/cancel
Authorization: Bearer <token>
```

只要 session 存在，运行中、空闲或已经取消的请求都返回相同的幂等确认：

```http
HTTP/1.1 200 OK
Content-Type: application/json
```

```json
{
  "accepted": true,
  "session_id": "admin:fixture-session"
}
```

运行中的请求复用现有 `SessionPool::cancel_session`，向当前 ACP session 发送
best-effort `session/cancel`。`accepted` 只代表取消请求被控制面接受，不代表
Agent 已经同步完成；真实 cancelled/idle 状态仍由 transcript/SSE 观察。

### 2.5 新接口错误语义

| 场景 | 状态码 | body |
| --- | --- | --- |
| 服务端未配置 Admin token | `503 Service Unavailable` | `{ "error": "admin token is not configured" }` |
| 缺失或错误 Admin Bearer | `401 Unauthorized` | `{ "error": "invalid or missing admin token" }` |
| `text` 缺失、不是文本或去空白后为空 | `400 Bad Request` | `{ "error": "text is required" }` |
| `session_id` 不存在 | `404 Not Found` | `{ "error": "session not found" }` |
| 同一 session 已有运行中回合，再次发送文本 | `409 Conflict` | `{ "error": "session is busy" }` |

一个 session 同时最多只有一个运行中回合。busy 检查必须在启动 ACP prompt
之前完成，失败的重复请求不得向 transcript 写入第二个 user entry。Agent 在
`202 Accepted` 之后发生的错误属于异步运行结果，进入现有状态和 SSE，不改写
已经返回的 HTTP 确认。

### 2.6 SSE 唯一实时来源

`GET /api/v1/sessions/events` 返回 `Content-Type: text/event-stream`。正常的会话
生命周期和 transcript 事件包含：

- `event`：现有事件名，例如 `session.created`、`status_changed`、`transcript`
  或 `error`。
- `id`：`<stream_generation>:<global_sequence>`，例如
  `fixture-generation:4`。
- `data`：现有 `SessionEvent` 或 `TranscriptEvent` JSON，不增加 Codeg 专用
  envelope。

`cursor_reset` 诊断也带有 generation-qualified `id`；当前实现产生的
`event history unavailable` 与 `event stream lagged` 诊断不带 `id`，客户端不得
因此把诊断误当作可继续确认的业务事件。

生命周期事件的 `data` 形状是：

```json
{
  "sequence": 1,
  "event": "session.created",
  "snapshot": { "session_id": "admin:fixture-session", "status": "idle" }
}
```

transcript 事件的 `sequence` 是统一 SSE 全局序号，entry 内的 `sequence` 仍是
该 session 的 mutation 序号：

```json
{
  "sequence": 3,
  "session_id": "admin:fixture-session",
  "entry": {
    "entry_id": "entry-1",
    "sequence": 1,
    "role": "user",
    "content": "请检查当前变更并运行测试",
    "status": "completed"
  }
}
```

客户端使用 `Last-Event-ID` 发送上次收到的完整 SSE id。新连接先重放保留历史，
再切换到 live receiver；不得另开 ACP WebSocket 来拼同一份聊天状态。

服务端已有三类恢复诊断，客户端必须重新拉取 snapshot/transcript：

- `cursor_reset`：generation 变化，通常表示 OpenAB 进程重启。
- `error` 且 `error` 为 `event history unavailable`：有界历史发生缺口。
- `error` 且 `error` 为 `event stream lagged`：broadcast receiver 落后。

### 2.7 有界状态与恢复承诺

- transcript 和 SSE replay 继续使用当前有界内存实现，不新增数据库或磁盘持久化。
- 只承诺同一 OpenAB 进程内的页面刷新、短暂断线与保留窗口 replay。
- OpenAB 重启后 generation 可以变化，旧 cursor 失效；客户端按
  `cursor_reset` 行为重新拉取，不假设进程重启恢复历史。

## 3. 范围与非目标

本 ADR 与 fixture 只冻结上述七个接口及其接入语义。明确不做：

- 不实现 Codeg UI、组件改造或 Codeg flat RPC `/api/{command}` 兼容层。
- 不实现 `RemoteRuntime`、Computer、daemon、机器注册、调度或跨机器迁移。
- 不增加 session ID 映射、turn/correlation ID、幂等消息 key 或离线队列。
- 不实现附件、图片、permission、terminal、fs、Git、OAuth、RBAC 或完整 CORS。
- 不复制 ACP JSON-RPC parser、事件分类或 transcript 存储；后续实现必须复用
  当前 core 能力。

## 4. 已接受残余风险

- 某些 ACP Agent 可能只在回合结束时产出文本；本契约不保证 token 级流式。
- ACP cancel 是 best-effort；下游 CLI 不响应时任务可能自然结束后才进入终态。
- Admin Bearer 对控制面权限较大，仅适合本阶段单用户、同源部署；多用户开放
  前必须另行完成身份与权限设计。
- 内存 transcript、有限 replay 和进程重启会导致历史不可恢复；fixture 中的
  `cursor_reset`、history gap 与 lag 事件是前端必须处理的恢复信号。

## 5. 验证方式

fixture [`codeg-session-contract-v1.json`](../fixtures/codeg-session-contract-v1.json)
固定：

- 七个接口的 method、path、请求 body、成功响应与错误响应；
- list/create/detail/transcript 的代表性现有 JSON；
- user、assistant、thinking、tool call/update/result 的 transcript/SSE 样例；
- generation-qualified SSE id、Last-Event-ID、cursor reset、history gap 与 lag。

可执行校验位于 `crates/openab-gateway/tests/codeg_session_contract_fixture.rs`，
运行：

```bash
cargo test -p openab-gateway --test codeg_session_contract_fixture
```

该测试只验证 fixture 与当前公开类型/形状一致，不宣称两个新增 endpoint 已在
当前分支实现。后续 ZER-952 的实现 PR 必须用同一 fixture 增加真实 HTTP 集成测试。
