# 多 agent 配置（agents 数组）

> 依据 ZER-707 / ZER-866。一份配置文件是唯一真相源。

## 为什么会有这一节

在此之前 openab 是**一个进程一个 agent**：配置里只有一个 `[agent]` 段，下游全部按这个假设写。要跑多个 agent 就得跑多个进程（Helm 下是每 agent 一个 Deployment）。

本机 daemon 方向反过来：**一台机器一个 daemon，驱动这台机器上已装的所有 agent CLI**。所以 agent 集合必须变成数据。

## 最小示例

```toml
[[agents]]
id = "codex"
command = "codex-acp"
default = true

[[agents]]
id = "claude"
command = "claude-agent-acp"
channels = ["C_CLAUDE_ONLY"]
```

## 字段

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `id` | string，必填 | 稳定标识。用于 pool key、日志、doctor 输出、渠道绑定。必须唯一非空，且不能含 `:` 或 `+`（pool key 保留） |
| `protocol` | `"acp"` / `"exec"` | 默认 `acp`。`exec` 是**保留但未实现**的占位（给没有 ACP 子命令的 agent，例如 droid）。启用 `exec` 的 agent 会被拒绝启动 |
| `command` | string | 显式路径或裸名。**一旦设置就完全跳过自动发现** |
| `args` | string 数组 | 传给可执行文件的参数 |
| `enabled` | bool | 默认 `true`。`false` 的条目完全不进注册表 |
| `default` | bool | 标记为默认 agent。**最多一个**。都不标时取第一个 enabled 的条目 |
| `workdir` | string | 会话工作目录。不填则回落到旧的 `working_dir` 解析 |
| `native_config` | string | 该 agent 自己的原生配置文件路径，daemon 会改写它 |
| `native_config_reload` | `"on_start"` | 只有这一个值，见下 |
| `env` | table | 传给子进程的环境变量 |
| `inherit_env` | string 数组 | 从 daemon 进程继承的变量名 |
| `images` | `"send"` / `"skip"` | 入站图片处理，不填用全局默认 |
| `channels` | string 数组 | 绑定到该 agent 的渠道。一个渠道**只能**绑一个 agent |

## 可执行文件解析：四级优先级

按顺序取第一个命中的，**启动时只解析一次并固化**：

1. **命令行 flag** → `cli-override`
2. **配置里显式 `command`** → `explicit-config`
3. **发现路径命中** → `discover-path`
4. **裸名交给 PATH** → `path-lookup`

固化的意思是：进程启动后再改 PATH 也不会让某个 agent 悄悄指向另一个二进制。

`openab doctor` 会打印**每个 agent 命中了哪一级**。agent 行为异常时第一个要问的就是「到底选中了哪个二进制，为什么是它」。

### 找不到怎么办

**不会导致启动失败。** 解析退回到 `path-lookup`，由 doctor 显式报出。一个 CLI 没装不该让 daemon 拒绝为其他 agent 服务。

## 发现路径

只在没有显式 `command` 时才用。支持 `~` 展开，以及**一个** `*` 组件（展开一层目录），够覆盖版本管理器的目录结构。

```toml
[[agents]]
id = "codex"
# 没有 command，走发现

[agents.discover]
paths = ["/opt/codex/bin"]

[defaults]
discover_paths = [
  "~/.local/bin",
  "/usr/local/bin",
  "~/.nvm/versions/node/*/bin",
  "~/.bun/bin",
]
```

搜索顺序：先该 agent 自己的 `[agents.discover] paths`，再 `[defaults] discover_paths`。在目录下按 `id` 找同名可执行文件（必须是文件且有执行位）。

### 为什么共享列表在 `[defaults]` 而不是顶层 `[discover]`

`[[agents]]` 是 TOML 的**表数组**。紧跟其后的 `[agents.discover]` 头会挂到**那一个**条目上，表达不了共享列表。所以共享的那份放在 `[defaults] discover_paths`。两种形式都支持，按上面的顺序搜索。

（08-14 草案里写的 `[agents.discover].paths` 作为全局列表在 TOML 语义上是不成立的，这里做了修正。）

## 选路

**默认 agent + 可选渠道绑定。刻意不做逐条消息的 agent 指令。**

- 消息渠道在某个 agent 的 `channels` 里 → 路由到它
- 否则 → 默认 agent

## 生效语义：`native_config_reload`

只有 `"on_start"`。

08-14 草案里还画过一个 `"per_turn"`（每轮重读原生配置），但 ZER-707 已拍板：原生配置变更**只保证新会话生效**，永远不推送到活会话。

接受 `per_turn` 等于对外宣称一个 daemon 并不实现的语义，所以配置里写 `per_turn` 会**直接报错并解释原因**，而不是静默降级成 `on_start`。

## 兼容旧配置

`[[agents]]` 为空时，旧的单个 `[agent]` 段会被当成唯一 agent（id 为 `default`），现有配置**不需要改动**。

## pool key

agent 复用现有的 pool key 维度，不新增维度：

- 无 profile：`agent:<id>`
- 有 profile：`agent:<id>+profile:<profile-id>`

这样会话池、thread 映射、快照、事件总线都不用动。这也是 `id` 不允许含 `:` 和 `+` 的原因。