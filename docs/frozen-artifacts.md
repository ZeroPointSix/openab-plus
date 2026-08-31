# 冻结制品清单（Frozen Artifacts）

> **状态：冻结（frozen）。以下路径不再维护，也不再作为设计或验收依据。**
> 依据：Linear [ZER-707](https://linear.app/zerodotsix/issue/ZER-707) 2026-08-14 收敛稿；拆卡见 ZER-875。
> **本轮不删除任何文件**。整批清理是 P0 验收通过后的独立 PR。

## 为什么冻结

项目方向已从「按 Agent 打镜像 + K8s / ECS 编排」改为「单二进制本机 daemon + 用机器已装的 CLI」。
我们只提供中控（一个 \`openab\` 服务、一份配置、\`doctor\`、按配置 spawn），不提供环境。
因此下列围绕「为每个 Agent 构建镜像并编排容器」的制品失去了目标形态支撑。

新方向的部署口径见 [\`docs/deployment-model.md\`](deployment-model.md)。

## 冻结清单

### 镜像构建

- 所有 \`Dockerfile.*\` 中**为单个 Agent CLI 构建镜像**的变体：
  \`Dockerfile.antigravity\`、\`Dockerfile.claude\`、\`Dockerfile.codex\`、\`Dockerfile.copilot\`、
  \`Dockerfile.cursor\`、\`Dockerfile.devin\`、\`Dockerfile.gemini\`、\`Dockerfile.grok\`、
  \`Dockerfile.hermes\`、\`Dockerfile.kimi\`、\`Dockerfile.mimocode\`、\`Dockerfile.opencode\`、
  \`Dockerfile.pi\`、\`Dockerfile.package\`、\`Dockerfile.unified\`、\`Dockerfile.native\`、
  \`Dockerfile.final\`、\`Dockerfile.agentcore\`
- \`docker-bake.hcl\`

> **例外**：\`Dockerfile\`、\`Dockerfile.ci\`、\`Dockerfile.builder\`、\`Dockerfile.gateway\`
> 构建的是**我们自己的服务**，不是 Agent CLI 运行时，**不在冻结范围内**。
> 「我们自己的服务走 CI 镜像」与「agent CLI 用本机已装 CLI」是两件事。

### 编排

- \`charts/\`（Helm chart，含「每 agent 一套 Deployment / ConfigMap / Secret / PVC」的模型）
- \`k8s/\`
- \`operator/\`（\`oabctl\` 的 ECS / Fargate provisioner，以及 \`oab.dev/v2\` 的 OABService / OABFleet manifests）

### 配置段

- \`config.toml\` 中的 \`[agentcore]\`（AWS Bedrock AgentCore Runtime，不是本机 CLI）
- Cargo feature \`agentcore\`

### 文档

- \`docs/oabctl.md\`
- \`docs/image-tags.md\`
- \`docs/helm-publishing.md\`
- \`docs/ai-install-upgrade.md\`
- \`docs/agentcore.md\`
- \`docs/adr/agentcore-runtime-backend.md\`
- \`docs/multi-agent.md\` 中基于 Helm \`agents.<name>\` 的多 agent 模型（新模型是一份配置里的 agents 数组，见 ZER-866）

### CI

- 与上述镜像相关的 build / publish job

### 僵尸支持

下列 CLI 有镜像但 \`config.toml.example\` 里没有对应的 \`[agent]\` 契约，属于「有镜像、无契约」，一并冻结：
\`kimi\`、\`pi\`、\`antigravity\`。

## 规则

1. **不要在冻结路径上新增功能或修 bug**，除非是安全修复且现网仍在使用。
2. **不要把冻结路径当作新功能的设计参考。**
3. **不要在 P0 验收通过前删除任何冻结文件。** 生产 Compose 镜像同样先不动。
4. 清理时合成**一个** PR，不要零散删。

## 现状备注

- \`kuoya-hk-001\` 未装 openab；\`kuoya-hk-002\` 与 \`kuoya-sgp-001\` 上有 Factory Droid。
- 无存量 openab 部署需要迁移，因此冻结不影响现网。
