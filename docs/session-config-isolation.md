# 会话级 CLI 配置与凭证隔离

> 依据 ZER-707 决策 **D2**，拆卡 ZER-888。

## 要解决的问题

openab 会改写各家 agent 的原生配置文件，让 model / provider 变更不必重建任何东西就生效。但这些写入的目标是**每个 CLI 一个进程级全局路径**：

- codex → `~/.codex/config.toml`
- claude → `~/.claude/settings.json`

正确性目前靠「持每 agent 类型一把锁跨越写入与 spawn」维持：写完立刻起进程，CLI 启动时读到的就是对的值。

**这个前提只在会话基本串行时成立。** 一旦一个 daemon 驱动多个 agent、多个 profile，「同一个 CLI、不同 model 或 provider、并发」就从边缘情况变成常态：

1. 会话 A 写全局文件（model X），spawn
2. 会话 B 写同一个文件（model Y），spawn
3. A 在被重建 / 恢复 / 池内重启之前没事——一旦重建，它读到的是 **B 的值**

所以「改配置后新会话生效」字面成立、实际错位：新会话拿到的是**最后一个写入者**留下的值，不是它自己 profile 的值。

## 为什么不能改 HOME

D2 明确：参照 cc-switch 与 Factory 的做法，**不动 `HOME`**。

这不是风格问题。`AcpConnection::spawn` 是**特意**把宿主机真实 `HOME` 传给子进程的，为的是让 agent CLI 找到自己的 OAuth / 登录文件。为了隔离配置去改写 `HOME`，会直接打断 agent 鉴权。

实机也印证了这条：`kuoya-sgp-001` 上 Factory 的 unit 里是 `Environment=HOME=/root` 保持真实，只用 `FACTORY_HOME` 把**自己的**状态目录挪走。

## 做法

两个第一批 CLI 都提供了重定向整个配置目录的环境变量，而且**连凭证一起重定向**：

| CLI | 环境变量 | 重定向内容 |
| --- | --- | --- |
| codex | `CODEX_HOME` | `config.toml`、`auth.json` 等 |
| claude | `CLAUDE_CONFIG_DIR` | `settings.json`、凭证等 |

直接把它们指向一个空的每会话目录，会连登录态一起丢掉——每个会话都变成未登录。所以本模块做的切分是：

- **openab 自己渲染的文件** → 每会话一份
- **全局配置目录里其余每个条目** → 软链进来

结果：**设置按会话隔离，身份保持机器级共享。**

## 目录布局

```
$OPENAB_HOME/sessions/<sanitized-session-key>/.codex/
    config.toml      <- 每会话渲染（openab 拥有）
    auth.json        -> 软链到 ~/.codex/auth.json（共享）
    ...              -> 其余条目同样软链
```

根目录优先取 `OPENAB_HOME`（systemd unit 模板会设，把 daemon 状态放在 HOME 之外），否则回落到 daemon home 下的 `.openab/sessions`。

## 渲染怎么做到不重复实现

`materialize()` 复用现有渲染器，不另写一套：

1. 取现有的每 agent 类型锁
2. 调 `cli_config::apply_unlocked()`，照旧渲染到全局路径
3. **在同一个临界区内**把产出的受管文件快照进会话目录

于是全局文件降级成一个「渲染暂存区」，每个会话读的是自己那份不可变副本。锁本来就串行化了渲染，快照放在同一临界区内所以无竞争。

## 安全性

- 会话目录 `0700`，会话配置文件 `0600`
- 会话 key 会被规范化成单个路径组件：不允许的字符折成 `-`，`..` 序列折叠（thread id 形如 `slack:C0123:1699.5`，点号必须保留但 `..` 不能留），过长的 key 截断后附加短摘要以防前缀相同的两个 key 撞在一起
- `cleanup()` 只删 daemon 拥有的会话子树。共享状态是通过软链访问的，**删软链永远不会动到它指向的凭证**，有测试断言这一条

## 不支持的 agent 怎么办

没有已知配置目录变量的 agent（`opencode`、`droid` 等）会得到 `IsolationStatus::Degraded`，并带一条说明「为什么降级、哪些是支持的」。

这类 agent 仍然走进程级全局路径加锁的老路。**如实上报而不是静默降级**——静默降级恰恰是 `openab doctor` 存在要防的事。

## 明确不在范围内

**多用户 / 多租户的凭证隔离与托管。** 本模块解决的是「同一台机器上不同会话之间」的配置目录隔离，属 P0 可用性问题。ZER-707 「不做」清单里的凭证隔离指的是多租户托管，两者不是同一件事。