# systemd host install (P0)

Install OpenAB as a single Linux binary under systemd. Scope for this guide:

- **P0:** Linux + systemd only
- **Artifact:** `openab` service binary only — **Agent CLIs are not preinstalled**
- **D2:** `HOME` is the **real** service-account home directory (not a per-session HOME)
- State lives under `OPENAB_*` paths outside `HOME`

This does **not** cover doctor, `[[agents]]`, worktree helpers, Compose/K8s/operator changes, or remote `systemctl` execution.

## Build the daemon binary

Default Cargo features still include AWS-oriented extras (`agentcore`, `pre-seed`, `config-s3`, `secrets-aws`, `filestore`) for Docker/unified images. Host systemd builds opt into the trimmed **`daemon`** feature instead (Discord + Slack only):

```bash
cargo build --release --no-default-features --features daemon
```

The `daemon` feature is **opt-in** and is **not** part of `default`. It does **not** enable:

- `agentcore`, `pre-seed`, `config-s3`, `secrets-aws`, `filestore`
- `unified` / gateway adapters: `telegram`, `line`, `feishu`, `googlechat`, `wecom`, `teams`, `acp`

Install the binary:

```bash
install -m 0755 target/release/openab /usr/local/bin/openab
```

## Layout

| Path | Role |
|------|------|
| `/usr/local/bin/openab` | Binary |
| `/etc/openab/config.toml` | Runtime config (`ExecStart … -c …`) |
| `/etc/openab/openab.env` | Optional `EnvironmentFile` (tokens); missing file is OK |
| `$HOME` | Real login home for the service user (auth/CLI dotfiles that expect HOME) |
| `$OPENAB_HOME` | OpenAB-owned home/state (default `/var/lib/openab/home`) |
| `$OPENAB_WORK_DIR` | Working directory (default `/var/lib/openab/work`) |
| `$OPENAB_LOG_DIR` | Logs / watchdog stamp (default `/var/log/openab`) |
| `$TMPDIR` / `$TEMP` / `$TMP` | Temp (default `/var/lib/openab/tmp`) |

Unit template: [`packaging/systemd/openab.service`](../packaging/systemd/openab.service)  
Env example (names only): [`packaging/systemd/openab.env.example`](../packaging/systemd/openab.env.example)

## Install & enable

Run as root on the target host (this repository does not run these remotely):

```bash
# Directories
install -d -m 0755 /etc/openab \
  /var/lib/openab/home /var/lib/openab/work /var/lib/openab/tmp \
  /var/log/openab

# Config + optional env file
install -m 0640 /path/to/config.toml /etc/openab/config.toml
cp packaging/systemd/openab.env.example /etc/openab/openab.env
chmod 0640 /etc/openab/openab.env
# edit /etc/openab/openab.env — put real tokens only on the host

# Unit
install -m 0644 packaging/systemd/openab.service /etc/systemd/system/openab.service
# Edit User=/Group=/Environment=HOME= if the service account is not root.
# HOME must remain that account's real home (D2).

systemctl daemon-reload
systemctl enable --now openab.service
systemctl status openab.service
```

Restart after config or binary updates:

```bash
systemctl restart openab.service
journalctl -u openab.service -f
```

## Unit design notes

- `Type=simple`, `Restart=always`, `RestartSec=5`
- Explicit `Environment=PATH=…` (Factory sgp-001 shipped `EnvironmentFile` only)
- Explicit `Environment=HOME=…` with the real home path
- `EnvironmentFile=-/etc/openab/openab.env` (optional)
- `KillMode=control-group` + `TimeoutStopSec=90` so agent child process groups in the cgroup are cleaned on stop/kill
- `OPENAB_AUTO_UPDATE=false` mirrors Factory `FACTORY_DROID_AUTO_UPDATE_ENABLED=false` as a **placeholder**: this binary does not auto-upgrade Agent CLIs or pull container images
- **No** new inbound listen (`ListenStream=` / `Accept=` are intentionally absent); do not add `-p` to `ExecStart`
- Optional `ExecStartPost` writes a UTC stamp under `$OPENAB_LOG_DIR/watchdog.stamp` via inline shell (no required external script)

## Agent CLI

The daemon package does not ship or auto-install Agent CLIs. Install the ACP-compatible CLI you configure in `config.toml` separately and ensure it is on `PATH` (or set an absolute `[agent].command`).
