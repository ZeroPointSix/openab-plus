# ADR: Codeg 接入当前 OpenAB 后端的最小契约

- **状态：** Accepted
- **日期：** 2026-09-03
- **决策范围：** ZER-950 / 阶段二 P2-1
- **契约版本：** `openab.codeg.session.v1`

## 1. 决策

Codeg 在阶段二只通过一个前端 `OpenABTransport` 消费当前 OpenAB 的 Session Admin HTTP/SSE 接口。业务组件不直接拼接 URL，不读取 Rust 内部类型，也不同时消费 ACP WebSocket 来重建聊天状态。

本 ADR 只冻结 7 个接口，其中 5 个是已有接口，2 个是后续实现的最小写接口。SSE 是 Codeg 唯一的实时状态与结果来源。ACP 继续是 OpenAB 后端与 Agent CLI 之间的内部协议，不是 Codeg 的第二条数据通道。

```text
Codeg UI
  -> OpenABTransport（Codeg 内部适配层）
  -> 当前 OpenAB /api/v1
  -> LocalRuntime -> SessionPool -> ACP CLI
```

未来把 `LocalRuntime` 替换为远程 runtime 时，Codeg 业务组件不应重写。这只是兼容约束；本 ADR 不定义 Computer、daemon、机器注册、跨机器路由或分布式 Hub。

## 2. 通用约定

### 2.1 地址与会话标识

- `OpenABTransport` 接受可配置的 base URL；阶段二默认与 OpenAB 同源部署。
- `session_id` 沿用当前 `SessionSnapshot.session_id`。它是不透明字符串，客户端不得解析 `admin:*`、`acp:*` 或其他前缀。
- 路径参数必须使用标准 URL 编码。fixture 特意使用 `admin:contract-demo`，对应路径片段为 `admin%3Acontract-demo`。
- 不增加公开 ID 映射层、turn ID、消息幂等键或离线队列。

### 2.2 鉴权

- 7 个接口全部沿用 Session Admin 的现有鉴权：优先读取 `GATEWAY_ADMIN_TOKEN`，兼容回退到 `OPENAB_ADMIN_TOKEN`。
- Codeg 对 REST 和 fetch-based SSE 都发送 `Authorization: Bearer <token>`。
- token 不得进入 query string、URL、SSE cursor、响应正文或常规日志。
- 现有调用者使用的兼容鉴权路径不在本 ADR 中移除；Codeg 不使用 query token，也不把 `/acp` 的 `OPENAB_ACP_AUTH_KEY` 当作 Admin token。
- Admin token 未配置时返回 `503` 和 `{"error":"admin token is not configured"}`；凭据缺失或无效时返回 `401` 和 `{"error":"invalid or missing admin token"}`。

### 2.3 错误模型

阶段二继续使用当前最小 JSON 错误形状：

```json
{
  "error": "human-readable stable message"
}
```

不在本 ADR 中引入 Codeg 专用错误 envelope。`OpenABTransport` 可以在前端把 HTTP 状态与 `error` 字段转换成 UI 状态，但不得要求后端返回 Codeg 类型。

## 3. 冻结的 7 个接口

| # | 能力 | 方法与路径 | 成功响应 | 实现状态 | 后续任务 |
|---|---|---|---|---|---|
| 1 | 会话列表 | `GET /api/v1/sessions` | `200`，现有 session list 形状 | 已有，保持兼容 | ZER-951 |
| 2 | 创建会话 | `POST /api/v1/sessions` | `201`，现有 `SessionSnapshot` | 已有，保持兼容 | ZER-951 |
| 3 | 会话详情 | `GET /api/v1/sessions/{session_id}` | `200`，现有 `SessionSnapshot` | 已有，保持兼容 | ZER-951 |
| 4 | 历史与工具活动 | `GET /api/v1/sessions/{session_id}/transcript` | `200`，现有 `TranscriptSnapshot` | 已有，保持兼容 | ZER-951 |
| 5 | 实时更新与重放 | `GET /api/v1/sessions/events` | `200 text/event-stream` | 已有，保持兼容 | ZER-951 |
| 6 | 发送文本 | `POST /api/v1/sessions/{session_id}/messages` | `202`，空正文 | 待实现 | ZER-952 |
| 7 | 取消当前任务 | `POST /api/v1/sessions/{session_id}/cancel` | `204`，空正文 | 待实现 | ZER-952 |

鉴权一致性由 ZER-953 补充集成测试；Codeg 的 `OpenABTransport`、只读工作台与写入闭环分别由 ZER-954、ZER-955、ZER-956 消费本契约。

已有 5 个接口的字段、可选字段、状态枚举和错误形状以当前 `/api/v1` 为准。本 PR 不重命名字段、不包裹额外 envelope，也不改变现有调用者行为。

## 4. 发送文本

### 4.1 请求

```http
POST /api/v1/sessions/admin%3Acontract-demo/messages HTTP/1.1
Authorization: Bearer <token>
Content-Type: application/json

{"text":"Run the contract check"}
```

- 阶段二只接受 `text` 消息；附件、图片、permission、terminal、fs 和 Git 不属于本接口。
- `text.trim()` 为空时拒绝请求；原始非空文本在提交给现有 turn 驱动和写入 transcript 时不做业务改写。
- handler 必须先原子地占用该 session 的运行中回合，再返回 `202`。同一 session 的并发请求只能有一个被接受。

### 4.2 成功与失败

| 条件 | 状态码 | 正文 |
|---|---:|---|
| 回合已占用并成功排入现有 ACP turn 驱动 | `202 Accepted` | 空正文 |
| JSON 无效或 `text` 为空 | `400 Bad Request` | `{"error":"message text is required"}` |
| session 不存在 | `404 Not Found` | `{"error":"session not found"}` |
| session 已有运行中回合 | `409 Conflict` | `{"error":"session is busy"}` |
| 在返回 `202` 前无法安排回合 | `500 Internal Server Error` | `{"error":"failed to start session turn"}` |

`202` 只表示 OpenAB 已接受该回合，不表示 Agent 已完成。HTTP 响应不得包含 user/assistant 最终文本。返回 `202` 后的 ACP/Agent 错误通过现有 lifecycle `error` SSE 事件和 `SessionSnapshot.last_error` 呈现，不能回写为另一个 HTTP 响应。

实现必须提取并复用现有 headless ACP turn 驱动、事件分类和 transcript 记录，不能复制 JSON-RPC 或 stream parser。

## 5. 取消当前任务

### 5.1 请求与响应

```http
POST /api/v1/sessions/admin%3Acontract-demo/cancel HTTP/1.1
Authorization: Bearer <token>
Content-Length: 0
```

| 条件 | 状态码 | 正文 |
|---|---:|---|
| session 正在运行，cancel 请求已交给现有 `SessionPool::cancel_session` | `204 No Content` | 空正文 |
| session 空闲、已取消或当前没有活动 cancel handle | `204 No Content` | 空正文 |
| session 不存在 | `404 Not Found` | `{"error":"session not found"}` |
| 活动 cancel handle 存在，但发送 ACP cancel 失败 | `500 Internal Server Error` | `{"error":"failed to cancel session"}` |

`204` 的含义是“取消请求已处理，或没有需要取消的当前回合”，不是“下游进程已经终止”。接口可重复调用，且不得因为 session 空闲而返回错误。

取消继续使用 ACP best-effort 语义。它不新增进程级强杀，不跨机器，也不创建 Codeg 专用 cancel 事件。运行状态发生变化时，客户端只消费现有 `status_changed` 或 `error` SSE 事件。

## 6. Transcript 与 SSE

### 6.1 唯一实时来源

`POST .../messages` 返回后，所有可见结果均沿用已有 transcript 与共享 SSE：

- user：`role: "user"`，`status: "completed"`。
- assistant 渐进文本：同一 `entry_id` 被多次 upsert，先为 `status: "streaming"`，最终为 `status: "completed"`。
- thinking：`role: "assistant"`，`status: "thinking"`。
- tool call/update/result：`role: "tool"`，同一 `tool_call_id` 与 `entry_id` 被 upsert；终态保留 `tool_result`。
- session 生命周期：继续使用 `session.created`、`status_changed`、`error` 等现有事件名。

SSE frame 的 `id` 是 `<generation>:<global-sequence>`。`data.sequence` 是共享 stream 的全局 sequence；`data.entry.sequence` 是单个 session transcript 的 mutation sequence，两者不能混用。

### 6.2 建连与恢复

- 客户端使用 fetch 建立 SSE，以便在 `Authorization` header 中携带 token。
- 无 `Last-Event-ID` 时，服务器从当前进程保留的共享 ring buffer 起点重放，然后切换到 live。
- 有效 cursor 使用 `Last-Event-ID: <generation>:<sequence>`，服务器只重放更大的全局 sequence。
- generation 改变时，服务器发送现有 `cursor_reset` 事件；history overflow 或 receiver lag 使用现有 `error` 事件。客户端先重新获取 session list/detail/transcript，再从新 cursor 继续。
- 客户端按全局 sequence 去重 SSE，按 `entry_id`/`tool_call_id` upsert transcript 项，不按到达次数盲目追加。

消息成功、Agent 失败、tool upsert、取消后的可选状态变化以及 cursor reset 的完整例子在 [可执行 fixture](../fixtures/codeg-openab-minimum-contract.v1.json) 中。

## 7. 不支持能力

阶段二未接通的 Codeg 能力必须由 transport/UI 明确标记为 unsupported，不能调用空桩并伪装成功。至少包括：附件、图片、terminal、fs、permission、Git、持久化历史、跨机器迁移和 RemoteRuntime。

本 ADR 不增加 health/capability API，也不扩展完整 Codeg 扁平 RPC facade；这些能力不计入冻结的 7 个接口。

## 8. 可执行契约

fixture 文件是后续 ZER-951、ZER-952、ZER-953 和 Codeg transport 测试的共同输入。仓库内校验命令：

```bash
node scripts/validate-codeg-openab-contract.mjs
```

校验器会拒绝以下漂移：接口数量或路径变化、token 进入 URL、消息成功响应携带正文、busy 不再是 `409`、cancel 不再幂等、SSE sequence/entry upsert 规则被破坏，以及缺少 thinking/tool/Agent error/cursor reset 示例。

## 9. 已接受残余风险

- **Admin token 权限较大。** 当前仅接受单用户、默认同源部署；它不是长期 OAuth、多用户或 RBAC 方案。
- **完整跨域未覆盖。** base URL 保持可配置，但 CORS、跨站 cookie 与独立 UI origin 不属于本阶段验收。
- **历史只在内存中有界保存。** 当前默认容量与环境变量行为不变；仅承诺同一 OpenAB 进程内刷新与短暂断线恢复。进程重启可使 transcript 和 cursor 失效。
- **取消是 best-effort。** CLI 不响应 ACP cancel 时，任务可能自然运行到结束；`204` 不承诺强制终止。
- **流式粒度取决于 Agent。** 某些 Agent 可能只在回合末产生文本，本契约不承诺 token 级流式。

## 10. 后果

- Codeg 可以围绕一个稳定、很小的 transport 边界开发，现有 `/api/v1` 调用者不受影响。
- 后端只需在 ZER-952 增加两个写操作，其余会话数据继续复用当前 runtime、SessionPool、transcript 和 SSE。
- 未来 runtime/daemon 变化必须保持这 7 个产品接口或在 transport 层提供兼容迁移；该未来实现不属于本 ADR。
