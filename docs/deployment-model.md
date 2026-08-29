# 部署模型：我们只提供中控，环境归机器自己

> 依据：Linear [ZER-707](https://linear.app/zerodotsix/issue/ZER-707) 2026-08-14 收敛稿（拆卡 ZER-873）。
> 旧的「按 Agent 打镜像 + K8s / ECS 编排」形态已冻结，见 [\`docs/frozen-artifacts.md\`](frozen-artifacts.md)。

## 一句话

云侧一个 hub，机器上一个 daemon。**机器已装什么 CLI 就用什么。**
我们只提供中控，不提供环境。不再按 Agent 打镜像。

## 形态

\`\`\`
Slack / Discord / Web / Admin
        ↓
hub（云侧）
  渠道、配置、Profile、会话注册表、transcript
  自己不 spawn 任何 Agent
        ↑  daemon 出站 WSS（机器无入站端口）
openab（每台干活机器，systemd 单二进制）
  显式 command + doctor 发现兜底
  每会话一个 git worktree
  ACP stdio spawn
        ↓
机器已装的 claude / codex / …
\`\`\`

## 我们交什么

- 一个 \`openab\` 后台服务（单二进制 + systemd unit 模板）
- 一份配置文件作为唯一真相源
- \`openab doctor\`：环境自检与显式报错
- 按配置 spawn 本机已装的 Agent CLI

## 我们不交什么

- **不预装** Node / Python / 任何 Agent CLI
- **不按 Agent 打镜像**
- 不为了跑一两个 Agent 起三四个容器

**环境是这台机器自己的。用户自己装 CLI。** 若用户想把整台机器打成镜像，那是用户的事。

## 缺依赖时会发生什么

\`openab doctor\` 显式报出缺失项并给出可操作建议，**不静默拉镜像、不静默降级**。
退出码可用于 CI 与安装脚本判定。

## 平台支持

**当前阶段仅支持 Linux + systemd。**

发行路径是「单二进制 + systemd unit」，因此：

- 受支持：Linux（systemd）
- 不在当前阶段：macOS、Windows、非 systemd 的 init 系统

代码里存在 Windows 条件编译分支（进程组处理、\`USERPROFILE\` 等），但**多 OS 铺开不在第一阶段范围内**，不要按 Windows 预期安装。

## 状态目录与凭证边界

daemon 会把**自己的**状态目录外置到可控位置（通过环境变量指定），并与工作目录分开存放。

边界要说清：

- 我们**不接管各家 Agent CLI 的凭证**。\`claude\` / \`codex\` 等各自的登录态仍由它们自己管理，放在它们各自的配置目录里。
- daemon spawn 子进程时**保留宿主机真实 \`HOME\`**，正是为了让 Agent CLI 能找到自己的 OAuth / 登录文件。
- 我们会为**不同会话之间**做 CLI 配置目录的隔离，避免并发会话互相覆盖模型 / provider 设置。但这**不是**多用户 / 多租户的凭证隔离与托管——后者不在当前阶段范围内。

## 配置生效语义

改 Agent 的原生配置或 Profile 后，**只保证新会话生效**。不承诺把变更推送到已经在跑的活会话。

## 不在当前阶段

危险命令审批、workdir 限制、用户与权限白名单、多租户凭证隔离与云凭证托管、触发权限、多 OS 铺开、接 droid。
