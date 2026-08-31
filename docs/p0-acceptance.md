# P0 验收记录与试验机前置（ZER-870）

> 父项：[ZER-707](https://linear.app/zerodotsix/issue/ZER-707)。拆卡：[ZER-870](https://linear.app/zerodotsix/issue/ZER-870)。
> 部署口径见 [ZER-873](https://linear.app/zerodotsix/issue/ZER-873) / PR [#60](https://github.com/ZeroPointSix/openab-plus/pull/60)（`docs/deployment-model.md`，合并前以该 PR 为准）。
> 旧镜像 / 编排制品冻结见 [ZER-875](https://linear.app/zerodotsix/issue/ZER-875) / PR [#59](https://github.com/ZeroPointSix/openab-plus/pull/59)；**本验收不删除任何 Dockerfile**。

## 验收口径变更（2026-08-29 / 08-30）

原卡面把 P0 合并验收写成「在干净机 `kuoya-sgp-001` 装上 openab 并跑通 2 个 CLI」。实机核查后口径改为：

1. **库侧 / 配置侧证据**以 CLI Gateway **Colab CPU** 沙箱为准（不预装 Agent CLI）。
2. **真实 claude / codex ACP spawn** 与 **systemd / daemon 安装**仍属未验证，不得在文档或评论里假装已通。
3. `kuoya-sgp-001` 若仍要用作试验机，必须先满足下文「试验机前置」；**先向人确认再装**，不要在远程机上擅自装 systemd。

ZER-869（systemd unit + daemon feature）已按指示 **Canceled**，不作为本验收门禁。

## 验收环境：Colab CPU

| 项 | 值 |
| --- | --- |
| 入口 | CLI Gateway `colab new` |
| 会话名 | `openab-p0-accept` |
| assignment | `m-s-kkb-ase1a1-3sh28xfwjo5si` |
| 规格 | 2 vCPU / overlay ~226G |
| Agent CLI | **不预装**（与「我们只交中控」一致） |
| 会话状态 | 已 `colab stop`（用完即停，避免空转烧 compute） |

更早还有一轮 Daytona CPU 沙箱（`--no-default-features --features slack`）作对照，**以 Colab 默认 features 矩阵为权威数字**。

## 已验证：openab-core lib 测试矩阵

命令形态：`cargo test -p openab-core --lib`（各 P0 分支在 `git checkout FETCH_HEAD` + `cargo clean -p openab-core` 后重编）。

### main（默认 features，含 AWS SDK）

| 基线 | 结果 |
| --- | --- |
| `origin/main` 默认 features | **808 passed / 0 failed** |

### 各 P0 分支模块测试（Colab）

| 卡 | 提交 | 结果 | 备注 |
| --- | --- | --- | --- |
| ZER-865 worktree | `e6e6001` | **11 passed** | |
| ZER-889 path_bounds | `2713a7b` | **6 passed / 1 failed** | `root_must_be_writable`：Colab 以 **root** 跑，`chmod 000` 目录对 root 仍可写，环境假设失效，**不是产品回归** |
| ZER-866 catalog | `bfcb991` | **2 passed** | |
| ZER-867 doctor | `28f5f73` | **1 passed** | |
| ZER-888 cli_config | `a7da7ec` | **13 passed** | |
| ZER-887 runtime | `df466f9` | **12 passed** | |
| ZER-868 cli_config | `c293041` | **11 passed** | |

权威评论留在 ZER-707：[Colab CPU 真实验收](https://linear.app/zerodotsix/issue/ZER-707)。

## 未验证（明确排除）

以下**不得**记为 P0 已通：

- 真实 **claude / codex**（或其它 Agent）经 ACP **stdio spawn** 的端到端会话
- 两个真实 CLI 会话的工作目录互不冲突（需在有 CLI 的机器上再验）
- `openab doctor` 在「缺真实 CLI 二进制」场景下的宿主机端到端报错（Colab 只跑了 crate 单测）
- **systemd unit 安装 / 启停 / 重启恢复**（ZER-869 已取消；本验收禁止在远程机装 systemd）
- 生产 Compose / 镜像路径（冻结，见 ZER-875）

## 试验机 `kuoya-sgp-001` 前置（若仍要用）

`kuoya-sgp-001` **不是干净机器**。只读核查结论（未读凭证内容）：

- 已在跑：`droid-daemon.service`、`wg-quick@factorywg.service`、`openab-pr46-preview.service`、`factory-reasoning-sync.service`、`factory-slack-unstick.service`
- `/usr/local/bin/droid` 已装；`/data/factory/{home,work}` 被 Factory 占用
- `/root/.openab` 已存在（含 `media/`、`thread_map.json`），但**没有** openab / oabctl 二进制
- `/root/.claude` 存在

因此若仍坚持在该机做「装上并跑通 2 个 CLI」：

1. **先处理** `/root/.openab` 残留（尤其 `thread_map.json` 是否与新配置语义冲突）。
2. **确认不与 droid / preview 抢端口与 CPU/内存**（机型约 4c8g；droid daemon + openab + 2 个 agent CLI 要留余量）。
3. **先向人确认再装** openab；不要擅自写 systemd、不要静默覆盖 Factory / droid 环境。
4. 若需要真正干净环境，另指机器或先完成清理方案，再开验收。

细节与决策回执见 ZER-707 / ZER-870 评论及行动文档。

## 和部署模型的关系

- 我们交：`openab` 服务、配置、doctor、按配置 spawn。
- 我们不交：Node / Python / 各家 Agent CLI；不按 Agent 打镜像。
- 缺 CLI 时显式报错，**不静默拉镜像**。
- 原生配置 / Profile 变更**只保证新会话生效**。

完整口径以 ZER-873 的 `docs/deployment-model.md` 为准（合并后可把本节缩成交叉链接）。
