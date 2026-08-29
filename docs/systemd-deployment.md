# systemd 部署（本机 daemon）

> 依据 ZER-707 / ZER-869。部署口径见 `docs/deployment-model.md`。

## 制品

默认制品只含 `openab` 服务本身，**不含任何 Agent CLI**。Agent CLI 由这台机器自己安装。

## 构建

用裁剪后的 `daemon` feature 组合构建，而不是默认 features：

```bash
cargo build --release --no-default-features --features daemon
```

`daemon` = `["discord", "slack"]`。相对默认 features，它去掉了 `agentcore`、`pre-seed`、`config-s3`、`secrets-aws`、`filestore` —— 这些是云侧能力，装在干活机器上的 daemon 不需要，只会增加体积和攻击面。

可以自己核对依赖差异：

```bash
# 默认 features：拉入 20+ 个 aws-* crate
cargo tree -e normal --prefix none | grep -oE '^aws-[a-z0-9-]+' | sort -u

# daemon feature：零个
cargo tree -e normal --prefix none --no-default-features --features daemon | grep -oE '^aws-[a-z0-9-]+' | sort -u
```

## 安装

```bash
sudo install -m 0755 target/release/openab /usr/local/bin/openab
sudo mkdir -p /etc/openab
sudo install -m 0644 deploy/systemd/openab.service /etc/systemd/system/openab.service
sudo install -m 0600 deploy/systemd/openab.env.example /etc/openab/openab.env
# 放好 /etc/openab/config.toml，然后
sudo systemctl daemon-reload
sudo systemctl enable --now openab
```

装完先自检：

```bash
openab doctor --config /etc/openab/config.toml
```

## unit 模板里三个不能省的设置

### `Environment=HOME` 必须是服务用户的真实主目录

Agent CLI 在 `\$HOME` 下找自己的登录态（`~/.claude`、`~/.codex`、`~/.config/gh`）。openab spawn 子进程时**特意保留宿主机真实 HOME**，正是为了这个。

所以 **不要用改 HOME 的方式做隔离**。会话之间的隔离走 `OPENAB_*` 与各家 CLI 自己的配置目录变量。这也是 Factory Droid 的做法：它的 unit 里 `HOME=/root` 保持真实，状态目录另用 `FACTORY_HOME` 外置。

### `Environment=PATH` 必须显式写

systemd 服务**不继承登录 shell 的 PATH**。nvm / bun / cargo / `~/.local/bin` 里装的 Agent CLI 在服务里根本看不见，除非在这里列出来。

这是「我 shell 里跑得通、systemd 里起不来」最常见的原因。

### `KillMode=control-group` 必须设

openab 给每个 spawn 出来的 agent 建独立进程组（`setpgid`），并在 drop 时杀掉整个组。但如果 systemd 只给主进程发信号，硬停时 agent 子树会变成孤儿进程留在机器上。

`KillMode=control-group` 让 systemd 清掉整个 cgroup，配合 `TimeoutStopSec=30` 给 agent 留出退出窗口。

> 参考实测：Factory 的 `droid-daemon.service` **没有**设 `KillMode`，也没有显式 `PATH`。这两条不是照抄来的，是我们要补的。

## 状态目录

unit 把 openab 自己的状态与工作目录都挪出 HOME，并且彼此分开：

| 变量 | 默认值 | 用途 |
| --- | --- | --- |
| `OPENAB_HOME` | `/var/lib/openab` | daemon 自己的状态 |
| `OPENAB_WORK_DIR` | `/var/lib/openab/worktrees` | 每会话工作目录的根 |
| `OPENAB_LOG_DIR` | `/var/log/openab` | 日志 |
| `TMPDIR` / `TEMP` / `TMP` | `/var/tmp/openab` | 临时文件，避免占根文件系统 |

## 凭证放哪

放 `EnvironmentFile`（`/etc/openab/openab.env`，权限 0600），**不要写进 unit 文件**——unit 文件是 world-readable 的。

## 平台

**仅 Linux + systemd。** 多 OS 铺开不在当前阶段。

## 排错

```bash
systemctl status openab
journalctl -u openab -n 200 --no-pager
sudo systemctl show openab -p Environment      # 确认 HOME/PATH 实际值
openab doctor --config /etc/openab/config.toml # 确认每个 agent 命中哪一级解析
```