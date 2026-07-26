const profileEditorState = {
  selectedId: null,
  draft: null,
  notice: null,
  schemaCache: new Map(),
  schemaRefreshSeq: 0,
};

const WORKDIR_STRATEGIES = [
  ["system_default", "System default"],
  ["profile_default", "Profile default"],
  ["ephemeral_per_session", "Ephemeral per session"],
];

const RECOVERY_STRATEGIES = [
  ["none", "None"],
  ["restart_process", "Restart process"],
  ["resume_session", "Resume session"],
];

const COMMON_AGENT_TYPES = [
  "opencode",
  "codex",
  "claude",
  "kiro",
  "cursor",
  "gemini",
  "system-default",
];

const RESERVED_CONFIG_FIELDS = new Set(["command", "working_dir"]);

async function renderProfiles(content) {
  const [profileDoc, agents] = await Promise.all([
    apiMaybe("/api/v1/agent-profiles"),
    apiMaybe("/api/v1/agents"),
  ]);

  if (!profileDoc) {
    content.innerHTML = `
      <section class="panel">
        <div class="panel-header"><h2>Agent Profiles</h2></div>
        <div class="panel-body">
          <div class="empty-state">当前运行形态未挂载 Agent Profile API。</div>
        </div>
      </section>`;
    return;
  }

  const profiles = Array.isArray(profileDoc.profiles) ? profileDoc.profiles.map(normalizeProfile) : [];
  const defaultProfileId = profileDoc.default_profile || "";
  const agentTypes = collectAgentTypes(profiles, agents);
  const selectedProfile = pickSelectedProfile(profiles, agentTypes);
  const schema = selectedProfile.agent_type
    ? await fetchProfileSchema(selectedProfile.agent_type)
    : null;

  content.innerHTML = `
    <section class="profile-workspace">
      <div class="profiles-list-panel">
        <div class="panel-header">
          <h2>Agent Profiles</h2>
          <div class="panel-actions">
            <span class="badge">${escapeHtml(String(profiles.length))} 个</span>
            <button class="primary-button compact" type="button" data-profile-create>新建</button>
          </div>
        </div>
        <div class="profile-list-scroll">
          ${profileListHtml(profiles, defaultProfileId)}
        </div>
      </div>

      <div class="profile-editor-panel">
        ${profileNoticeHtml(profileEditorState.notice)}
        ${profileEditorHtml(selectedProfile, {
    defaultProfileId,
    agentTypes,
    schema,
    isNew: profileEditorState.selectedId === "__new__",
  })}
      </div>
    </section>

    <section class="panel">
      <div class="panel-header"><h2>Agent 类型</h2></div>
      <div class="panel-body">${agentsSummary(agents)}</div>
    </section>`;

  bindProfilesPage(content, { profiles, defaultProfileId });
}

function pickSelectedProfile(profiles, agentTypes) {
  if (profileEditorState.draft) return normalizeProfile(profileEditorState.draft);

  const current = profiles.find((profile) => profile.id === profileEditorState.selectedId);
  if (current) return current;

  if (profiles.length) {
    profileEditorState.selectedId = profiles[0].id;
    return profiles[0];
  }

  profileEditorState.selectedId = "__new__";
  return newProfileDraft(agentTypes[0] || "opencode");
}

function newProfileDraft(agentType) {
  return normalizeProfile({
    id: "",
    name: "",
    agent_type: agentType,
    enabled: true,
    workdir_strategy: "system_default",
    recovery_strategy: "resume_session",
  });
}

function normalizeProfile(profile) {
  return {
    id: profile.id || "",
    name: profile.name || "",
    agent_type: profile.agent_type || "",
    enabled: profile.enabled !== false,
    command: profile.command || "",
    args: Array.isArray(profile.args) ? profile.args : [],
    default_model: profile.default_model || "",
    reasoning_effort: profile.reasoning_effort || "",
    workdir_strategy: profile.workdir_strategy || "system_default",
    working_dir: profile.working_dir || "",
    env_refs: profile.env_refs && typeof profile.env_refs === "object" ? profile.env_refs : {},
    inherit_env: Array.isArray(profile.inherit_env) ? profile.inherit_env : [],
    timeout_secs: profile.timeout_secs ?? "",
    recovery_strategy: profile.recovery_strategy || "resume_session",
    config_options: profile.config_options && typeof profile.config_options === "object" ? profile.config_options : {},
    created_at: profile.created_at || "",
    updated_at: profile.updated_at || "",
  };
}

function collectAgentTypes(profiles, agents) {
  const types = new Set(COMMON_AGENT_TYPES);
  for (const profile of profiles) {
    if (profile.agent_type) types.add(profile.agent_type);
  }
  if (Array.isArray(agents)) {
    for (const agent of agents) {
      if (agent.agent_type) types.add(agent.agent_type);
    }
  }
  return [...types].sort();
}

async function fetchProfileSchema(agentType, { force = false } = {}) {
  const key = String(agentType || "").trim();
  if (!key) return null;
  if (!force && profileEditorState.schemaCache.has(key)) {
    return profileEditorState.schemaCache.get(key);
  }

  const schema = await apiMaybe(`/api/v1/agents/${encodeURIComponent(key)}/config-schema`);
  profileEditorState.schemaCache.set(key, schema);
  return schema;
}

function profileListHtml(profiles, defaultProfileId) {
  if (!profiles.length) {
    return `<div class="empty-state">暂无 Profile。点击“新建”开始配置。</div>`;
  }

  const rows = profiles.map((profile) => {
    const isSelected = profile.id === profileEditorState.selectedId;
    const isDefault = profile.id === defaultProfileId;
    return `
      <article class="profile-row ${isSelected ? "selected" : ""}" data-profile-id="${escapeHtml(profile.id)}">
        <button class="profile-row-main" type="button" data-profile-select="${escapeHtml(profile.id)}">
          <span>
            <strong>${escapeHtml(profile.name || profile.id)}</strong>
            <span class="code-ish">${escapeHtml(profile.id || "-")}</span>
          </span>
          <span class="profile-row-badges">
            <span class="badge">${escapeHtml(profile.agent_type || "-")}</span>
            ${profile.enabled ? `<span class="status-badge success">enabled</span>` : `<span class="status-badge muted">disabled</span>`}
            ${isDefault ? `<span class="status-badge default">default</span>` : ""}
          </span>
        </button>
        <div class="profile-row-meta">
          <span>Model: ${escapeHtml(profile.default_model || "-")}</span>
          <span>Reasoning: ${escapeHtml(profile.reasoning_effort || "-")}</span>
          <span>Updated: ${escapeHtml(formatTime(profile.updated_at))}</span>
        </div>
        <div class="profile-row-actions">
          <button class="ghost-button compact" type="button" data-profile-validate="${escapeHtml(profile.id)}">校验</button>
          <button class="ghost-button compact" type="button" data-profile-default="${escapeHtml(profile.id)}" ${isDefault ? "disabled" : ""}>设为默认</button>
          <button class="ghost-button compact danger-action" type="button" data-profile-delete="${escapeHtml(profile.id)}">删除</button>
        </div>
      </article>`;
  }).join("");

  return `<div class="profile-list">${rows}</div>`;
}

function profileEditorHtml(profile, { defaultProfileId, agentTypes, schema, isNew }) {
  const isDefault = profile.id && profile.id === defaultProfileId;
  const title = isNew ? "新建 Profile" : "编辑 Profile";

  return `
    <form id="profile-form" class="profile-form">
      <section class="panel">
        <div class="panel-header">
          <div>
            <h2>${escapeHtml(title)}</h2>
            <p class="panel-subtitle">Profile 只影响之后创建的新会话，不修改运行中的 Agent。</p>
          </div>
          <div class="panel-actions">
            <button class="ghost-button" type="button" data-profile-check>校验表单</button>
            <button class="primary-button" type="submit">保存</button>
          </div>
        </div>
        <div class="panel-body form-grid">
          ${profileTextField("id", "Profile ID", profile.id, { required: true, disabled: !isNew, code: true })}
          ${profileTextField("name", "显示名称", profile.name, { required: true })}
          ${agentTypeField(profile.agent_type, agentTypes)}
          <label class="toggle-row">
            <input type="checkbox" name="enabled" ${profile.enabled ? "checked" : ""} />
            <span>启用 Profile</span>
          </label>
          <label class="toggle-row">
            <input type="checkbox" name="default_profile" ${isDefault ? "checked" : ""} />
            <span>保存后设为默认 Profile</span>
          </label>
        </div>
      </section>

      <section class="panel">
        <div class="panel-header"><h2>启动与运行策略</h2></div>
        <div class="panel-body form-grid">
          ${profileTextField("command", "Agent 启动命令", profile.command, { placeholder: "opencode acp" })}
          ${profileTextArea("args", "启动参数", profile.args.join("\n"), "每行一个参数；command 已包含参数时可留空。")}
          ${profileSelectField("workdir_strategy", "工作目录策略", WORKDIR_STRATEGIES, profile.workdir_strategy)}
          ${profileTextField("working_dir", "默认工作目录", profile.working_dir, { code: true, placeholder: "/workspace/project" })}
          ${profileNumberField("timeout_secs", "超时时间（秒）", profile.timeout_secs, { min: 5, max: 86400 })}
          ${profileSelectField("recovery_strategy", "恢复策略", RECOVERY_STRATEGIES, profile.recovery_strategy)}
        </div>
      </section>

      <section class="panel">
        <div class="panel-header">
          <div>
            <h2>动态配置</h2>
            <p id="profile-schema-source" class="panel-subtitle">${schemaSubtitle(schema, profile.agent_type)}</p>
          </div>
        </div>
        <div class="panel-body">
          <div id="profile-config-fields" class="form-grid" data-schema-agent="${escapeHtml(profile.agent_type || "")}">
            ${configFieldsHtml(schema, profile)}
          </div>
        </div>
      </section>

      <section class="panel">
        <div class="panel-header"><h2>环境变量引用</h2></div>
        <div class="panel-body form-grid">
          ${profileTextArea("env_refs", "Secret 引用", mapToLines(profile.env_refs), "每行 KEY=${ENV_NAME}、KEY=env://NAME、KEY=vault://path 等，不保存明文密钥。")}
          ${profileTextArea("inherit_env", "继承环境变量", profile.inherit_env.join("\n"), "每行一个变量名。")}
        </div>
      </section>

      <section class="panel">
        <div class="panel-header"><h2>额外配置</h2></div>
        <div class="panel-body">
          ${profileTextArea("config_options_extra", "Config options", extraConfigLines(schema, profile), "每行 key=value；已在动态字段中展示的 key 不需要重复填写。")}
        </div>
      </section>

      <div id="profile-validation" class="profile-validation" aria-live="polite"></div>
    </form>`;
}

function profileTextField(name, label, value, options = {}) {
  return `
    <label class="form-field">
      <span>${escapeHtml(label)}</span>
      <input name="${escapeHtml(name)}" value="${escapeHtml(value || "")}" ${options.required ? "required" : ""} ${options.disabled ? "disabled" : ""} ${options.placeholder ? `placeholder="${escapeHtml(options.placeholder)}"` : ""} class="${options.code ? "code-input" : ""}" />
    </label>`;
}

function profileNumberField(name, label, value, options = {}) {
  return `
    <label class="form-field">
      <span>${escapeHtml(label)}</span>
      <input type="number" name="${escapeHtml(name)}" value="${escapeHtml(value || "")}" ${options.min ? `min="${escapeHtml(options.min)}"` : ""} ${options.max ? `max="${escapeHtml(options.max)}"` : ""} />
    </label>`;
}

function profileTextArea(name, label, value, help = "") {
  return `
    <label class="form-field wide">
      <span>${escapeHtml(label)}</span>
      <textarea name="${escapeHtml(name)}" rows="4">${escapeHtml(value || "")}</textarea>
      ${help ? `<small>${escapeHtml(help)}</small>` : ""}
    </label>`;
}

function profileSelectField(name, label, options, current) {
  return `
    <label class="form-field">
      <span>${escapeHtml(label)}</span>
      <select name="${escapeHtml(name)}">
        ${options.map(([value, text]) => `<option value="${escapeHtml(value)}" ${current === value ? "selected" : ""}>${escapeHtml(text)}</option>`).join("")}
      </select>
    </label>`;
}

function agentTypeField(current, agentTypes) {
  const listId = "agent-type-options";
  return `
    <label class="form-field">
      <span>Agent 类型</span>
      <input name="agent_type" list="${listId}" value="${escapeHtml(current || "")}" required />
      <datalist id="${listId}">
        ${agentTypes.map((type) => `<option value="${escapeHtml(type)}"></option>`).join("")}
      </datalist>
    </label>`;
}

function schemaSubtitle(schema, agentType) {
  if (!agentType) return "选择 Agent 类型后加载可编辑字段。";
  if (!schema || !Array.isArray(schema.fields)) return "未获取到配置 schema；仍可保存基础字段。";
  return `来自 ${schema.source || "config-schema"}，保存时写入当前 Profile。`;
}

function configFieldsHtml(schema, profile) {
  const fields = profileConfigFields(schema, profile);
  if (!fields.length) {
    return `<div class="empty-state wide">当前 Agent 暂无可动态渲染的配置字段。</div>`;
  }

  return fields.map((field) => configFieldHtml(field, configFieldValue(profile, field.id))).join("");
}

function profileConfigFields(schema, profile) {
  const fields = Array.isArray(schema?.fields) ? schema.fields : [];
  const normalized = fields
    .filter((field) => field?.id && !RESERVED_CONFIG_FIELDS.has(field.id))
    .map((field) => ({
      id: field.id,
      label: field.label || field.id,
      kind: String(field.kind || field.type || "string").toLowerCase(),
      options: Array.isArray(field.options) ? field.options : [],
      dynamic: field.dynamic !== false,
    }));

  const existingKeys = new Set(normalized.map((field) => field.id));
  if (!existingKeys.has("model")) {
    normalized.unshift({ id: "model", label: "Model", kind: "string", options: [], dynamic: true });
    existingKeys.add("model");
  }
  if (!existingKeys.has("reasoning_effort")) {
    const modelIndex = normalized.findIndex((field) => field.id === "model");
    normalized.splice(modelIndex + 1, 0, { id: "reasoning_effort", label: "Reasoning effort", kind: "string", options: [], dynamic: true });
    existingKeys.add("reasoning_effort");
  }
  return normalized;
}

function configFieldHtml(field, value) {
  const attrs = `data-config-field data-config-id="${escapeHtml(field.id)}" data-config-kind="${escapeHtml(field.kind)}"`;
  const label = `${field.label}${field.dynamic ? "" : "（静态）"}`;
  if (field.options.length) {
    const optionSet = new Set(field.options);
    if (value && !optionSet.has(value)) optionSet.add(value);
    return `
      <label class="form-field">
        <span>${escapeHtml(label)}</span>
        <select ${attrs}>
          <option value="">未设置</option>
          ${[...optionSet].map((option) => `<option value="${escapeHtml(option)}" ${value === option ? "selected" : ""}>${escapeHtml(option)}</option>`).join("")}
        </select>
      </label>`;
  }

  if (field.kind === "boolean" || field.kind === "bool") {
    return `
      <label class="toggle-row">
        <input type="checkbox" ${attrs} ${String(value).toLowerCase() === "true" ? "checked" : ""} />
        <span>${escapeHtml(label)}</span>
      </label>`;
  }

  const type = field.kind === "number" || field.kind === "integer" ? "number" : "text";
  return `
    <label class="form-field">
      <span>${escapeHtml(label)}</span>
      <input type="${type}" value="${escapeHtml(value || "")}" ${attrs} />
    </label>`;
}

function configFieldValue(profile, fieldId) {
  if (fieldId === "model") return profile.default_model || "";
  if (fieldId === "reasoning_effort") return profile.reasoning_effort || "";
  return profile.config_options?.[fieldId] || "";
}

function extraConfigLines(schema, profile) {
  const visible = new Set(profileConfigFields(schema, profile).map((field) => field.id));
  return Object.entries(profile.config_options || {})
    .filter(([key]) => !visible.has(key))
    .map(([key, value]) => `${key}=${value}`)
    .join("\n");
}

function mapToLines(map) {
  return Object.entries(map || {}).map(([key, value]) => `${key}=${value}`).join("\n");
}

function bindProfilesPage(content, context) {
  const form = content.querySelector("#profile-form");
  if (form) {
    form.dataset.initialSnapshot = profileFormSnapshot(form);
  }

  content.querySelector("[data-profile-create]")?.addEventListener("click", () => {
    if (!confirmProfileNavigation(form)) return;
    profileEditorState.selectedId = "__new__";
    profileEditorState.draft = newProfileDraft(collectAgentTypes(context.profiles, null)[0] || "opencode");
    profileEditorState.notice = null;
    renderProfiles(content);
  });

  content.querySelectorAll("[data-profile-select]").forEach((button) => {
    button.addEventListener("click", () => {
      const nextProfileId = button.dataset.profileSelect;
      if (nextProfileId === profileEditorState.selectedId) return;
      if (!confirmProfileNavigation(form)) return;
      profileEditorState.selectedId = nextProfileId;
      profileEditorState.draft = null;
      profileEditorState.notice = null;
      renderProfiles(content);
    });
  });

  content.querySelectorAll("[data-profile-default]").forEach((button) => {
    button.addEventListener("click", async () => {
      await setDefaultProfile(content, button.dataset.profileDefault);
    });
  });

  content.querySelectorAll("[data-profile-validate]").forEach((button) => {
    button.addEventListener("click", async () => {
      await validateSavedProfile(content, button.dataset.profileValidate);
    });
  });

  content.querySelectorAll("[data-profile-delete]").forEach((button) => {
    button.addEventListener("click", async () => {
      await deleteProfile(content, button.dataset.profileDelete);
    });
  });

  form?.addEventListener("submit", async (event) => {
    event.preventDefault();
    await saveProfile(content, context.defaultProfileId);
  });

  form?.querySelector("[data-profile-check]")?.addEventListener("click", () => {
    const result = validateProfileDraft(collectProfileForm(form));
    renderValidation(form, result);
  });

  const agentTypeInput = form?.querySelector('[name="agent_type"]');
  if (agentTypeInput) {
    const refreshAgentTypeFields = debounce(() => {
      refreshDynamicFields(form);
    }, 250);

    agentTypeInput.addEventListener("input", refreshAgentTypeFields);
    const refreshAgentTypeNow = async () => {
      refreshAgentTypeFields.cancel();
      await refreshDynamicFields(form);
    };

    agentTypeInput.addEventListener("change", refreshAgentTypeNow);
    agentTypeInput.addEventListener("blur", refreshAgentTypeNow);
  }
}

function confirmProfileNavigation(form) {
  if (!hasUnsavedProfileChanges(form)) return true;
  return window.confirm("当前 Profile 有未保存修改，继续切换会丢失这些修改。");
}

function hasUnsavedProfileChanges(form) {
  return Boolean(form?.dataset.initialSnapshot && form.dataset.initialSnapshot !== profileFormSnapshot(form));
}

function profileFormSnapshot(form) {
  const fields = [];
  for (const [key, value] of new FormData(form).entries()) {
    fields.push([key, String(value)]);
  }
  form.querySelectorAll("[data-config-field]").forEach((input) => {
    fields.push([
      `config:${input.dataset.configId || ""}`,
      input.type === "checkbox" ? String(input.checked) : String(input.value || ""),
    ]);
  });
  fields.sort((left, right) => left[0].localeCompare(right[0]) || left[1].localeCompare(right[1]));
  return JSON.stringify(fields);
}

function debounce(callback, wait) {
  let timeoutId = null;
  const debounced = (...args) => {
    if (timeoutId) window.clearTimeout(timeoutId);
    timeoutId = window.setTimeout(() => {
      timeoutId = null;
      callback(...args);
    }, wait);
  };
  debounced.cancel = () => {
    if (timeoutId) window.clearTimeout(timeoutId);
    timeoutId = null;
  };
  return debounced;
}

async function refreshDynamicFields(form, retryDepth = 0) {
  const refreshId = ++profileEditorState.schemaRefreshSeq;
  const target = form.querySelector("#profile-config-fields");
  const sourceTarget = form.querySelector("#profile-schema-source");
  if (!target) return;

  try {
    const draft = collectProfileForm(form, { prune: false });
    const requestedAgentType = String(draft.agent_type || "").trim();
    profileEditorState.draft = draft;
    target.innerHTML = `<div class="empty-state wide">正在加载配置字段。</div>`;
    const schema = await fetchProfileSchema(requestedAgentType, {
      force: target.dataset.schemaAgent !== requestedAgentType,
    });
    if (refreshId !== profileEditorState.schemaRefreshSeq) return;

    const currentAgentType = String(form.querySelector('[name="agent_type"]')?.value || "").trim();
    if (currentAgentType !== requestedAgentType) {
      if (retryDepth < 4) {
        return refreshDynamicFields(form, retryDepth + 1);
      }
      const currentDraft = collectProfileForm(form, { prune: false });
      const currentSchema = await fetchProfileSchema(currentAgentType, { force: true });
      if (refreshId !== profileEditorState.schemaRefreshSeq) return;
      if (currentAgentType === String(form.querySelector('[name="agent_type"]')?.value || "").trim()) {
        target.dataset.schemaAgent = currentAgentType || "";
        if (sourceTarget) sourceTarget.textContent = schemaSubtitle(currentSchema, currentAgentType);
        target.innerHTML = configFieldsHtml(currentSchema, currentDraft);
        return;
      }
      return;
    }

    target.dataset.schemaAgent = currentAgentType || "";
    if (sourceTarget) sourceTarget.textContent = schemaSubtitle(schema, currentAgentType);
    target.innerHTML = configFieldsHtml(schema, collectProfileForm(form, { prune: false }));
  } catch (error) {
    if (refreshId !== profileEditorState.schemaRefreshSeq) return;
    const currentAgentType = String(form.querySelector('[name="agent_type"]')?.value || "").trim();
    if (sourceTarget) sourceTarget.textContent = schemaSubtitle(null, currentAgentType);
    target.innerHTML = `<div class="empty-state wide">配置字段加载失败：${escapeHtml(error.message || "未知错误")}</div>`;
  }
}

async function ensureDynamicFieldsFresh(form) {
  const target = form.querySelector("#profile-config-fields");
  const currentAgentType = String(form.querySelector('[name="agent_type"]')?.value || "").trim();
  if (!target || target.dataset.schemaAgent === currentAgentType) return false;
  await refreshDynamicFields(form);
  return true;
}

async function saveProfile(content, defaultProfileId) {
  const form = content.querySelector("#profile-form");
  if (!form) return;
  if (await ensureDynamicFieldsFresh(form)) {
    renderValidation(form, {
      ok: false,
      errors: [{ path: "agent_type", message: "配置字段已根据 Agent 类型刷新，请确认动态配置后再次保存。" }],
    });
    return;
  }
  const profile = collectProfileForm(form);
  const wantsDefault = form.querySelector('[name="default_profile"]')?.checked || false;
  const validation = validateProfileDraft(profile, { wantsDefault });
  renderValidation(form, validation);
  if (!validation.ok) return;

  const isNew = profileEditorState.selectedId === "__new__";
  const path = isNew
    ? "/api/v1/agent-profiles"
    : `/api/v1/agent-profiles/${encodeURIComponent(profileEditorState.selectedId || profile.id)}`;
  const method = isNew ? "POST" : "PUT";

  try {
    await profileRequest(path, { method, body: JSON.stringify(profile) });
    if (wantsDefault) {
      await profileRequest(`/api/v1/agent-profiles/default/${encodeURIComponent(profile.id)}`, { method: "PUT" });
    } else if (defaultProfileId === profile.id) {
      await profileRequest("/api/v1/agent-profiles/default", { method: "DELETE" });
    }

    profileEditorState.selectedId = profile.id;
    profileEditorState.draft = null;
    profileEditorState.notice = { tone: "success", message: "Profile 已保存。" };
    await renderProfiles(content);
  } catch (error) {
    const result = error.responsePayload?.validation || {
      ok: false,
      errors: [{ path: "server", message: error.message || "保存失败。" }],
    };
    renderValidation(form, normalizeValidation(result));
  }
}

async function setDefaultProfile(content, profileId) {
  if (!profileId) return;
  try {
    await profileRequest(`/api/v1/agent-profiles/default/${encodeURIComponent(profileId)}`, { method: "PUT" });
    profileEditorState.notice = { tone: "success", message: "默认 Profile 已更新。" };
    await renderProfiles(content);
  } catch (error) {
    profileEditorState.notice = { tone: "danger", message: error.message || "默认 Profile 更新失败。" };
    await renderProfiles(content);
  }
}

async function validateSavedProfile(content, profileId) {
  if (!profileId) return;
  try {
    const validation = normalizeValidation(await profileRequest(`/api/v1/agent-profiles/${encodeURIComponent(profileId)}/validate`, { method: "POST" }));
    profileEditorState.notice = validation.ok
      ? { tone: "success", message: `${profileId} 服务端校验通过。` }
      : { tone: "danger", message: `${profileId} 服务端校验未通过。`, errors: validation.errors };
    await renderProfiles(content);
  } catch (error) {
    const validation = normalizeValidation(error.responsePayload?.validation);
    profileEditorState.notice = validation.errors.length
      ? { tone: "danger", message: error.message || "校验失败。", errors: validation.errors }
      : { tone: "danger", message: error.message || "校验失败。" };
    await renderProfiles(content);
  }
}

async function deleteProfile(content, profileId) {
  if (!profileId) return;
  if (!window.confirm(`删除 Profile ${profileId}？已有会话不会中断，快照会标记为 deleted。`)) return;
  try {
    await profileRequest(`/api/v1/agent-profiles/${encodeURIComponent(profileId)}`, { method: "DELETE" });
    if (profileEditorState.selectedId === profileId) {
      profileEditorState.selectedId = null;
      profileEditorState.draft = null;
    }
    profileEditorState.notice = { tone: "success", message: "Profile 已删除。" };
    await renderProfiles(content);
  } catch (error) {
    profileEditorState.notice = { tone: "danger", message: error.message || "删除失败。" };
    await renderProfiles(content);
  }
}

function collectProfileForm(form, { prune = true } = {}) {
  const fields = new FormData(form);
  const rawId = String(fields.get("id") || "").trim();
  const profile = normalizeProfile({
    id: rawId || (profileEditorState.selectedId === "__new__" ? "" : profileEditorState.selectedId || ""),
    name: String(fields.get("name") || "").trim(),
    agent_type: String(fields.get("agent_type") || "").trim(),
    enabled: fields.get("enabled") === "on",
    command: optionalString(fields.get("command")),
    args: parseList(fields.get("args")),
    workdir_strategy: String(fields.get("workdir_strategy") || "system_default"),
    working_dir: optionalString(fields.get("working_dir")),
    timeout_secs: parseOptionalNumber(fields.get("timeout_secs")),
    recovery_strategy: String(fields.get("recovery_strategy") || "resume_session"),
    env_refs: parseKeyValueLines(fields.get("env_refs")),
    inherit_env: parseList(fields.get("inherit_env")),
    config_options: {},
  });

  form.querySelectorAll("[data-config-field]").forEach((input) => {
    const key = input.dataset.configId;
    if (!key) return;
    if (input.type === "checkbox") {
      if (!input.checked) return;
      assignConfigField(profile, key, "true");
      return;
    }
    const value = String(input.value || "").trim();
    if (!value) return;
    assignConfigField(profile, key, value);
  });

  const extraOptions = parseKeyValueLines(fields.get("config_options_extra"));
  for (const [key, value] of Object.entries(extraOptions)) {
    assignConfigField(profile, key, value);
  }

  return prune ? pruneProfile(profile) : profile;
}

function assignConfigField(profile, key, value) {
  if (key === "model") {
    profile.default_model = value;
  } else if (key === "reasoning_effort") {
    profile.reasoning_effort = value;
  } else {
    if (!profile.config_options) profile.config_options = {};
    profile.config_options[key] = value;
  }
}

function pruneProfile(profile) {
  const pruned = { ...profile };
  for (const key of ["command", "default_model", "reasoning_effort", "working_dir"]) {
    if (!pruned[key]) delete pruned[key];
  }
  if (pruned.timeout_secs === "" || pruned.timeout_secs === null || Number.isNaN(pruned.timeout_secs)) {
    delete pruned.timeout_secs;
  }
  if (!pruned.args.length) delete pruned.args;
  if (!Object.keys(pruned.env_refs).length) delete pruned.env_refs;
  if (!pruned.inherit_env.length) delete pruned.inherit_env;
  if (!Object.keys(pruned.config_options).length) delete pruned.config_options;
  delete pruned.created_at;
  delete pruned.updated_at;
  return pruned;
}

function validateProfileDraft(profile, { wantsDefault = false } = {}) {
  const errors = [];
  if (!profile.id) pushProfileError(errors, "id", "Profile ID 必填。");
  if (profile.id && !/^[A-Za-z0-9_.-]+$/.test(profile.id)) {
    pushProfileError(errors, "id", "Profile ID 只能包含 ASCII 字母、数字、点、横线或下划线。");
  }
  if (!profile.name) pushProfileError(errors, "name", "显示名称必填。");
  if (!profile.agent_type) pushProfileError(errors, "agent_type", "Agent 类型必填。");
  if (wantsDefault && profile.enabled === false) {
    pushProfileError(errors, "default_profile", "默认 Profile 必须保持启用。");
  }
  if (profile.timeout_secs !== undefined && profile.timeout_secs !== "") {
    if (!Number.isInteger(profile.timeout_secs)) {
      pushProfileError(errors, "timeout_secs", "超时时间必须是整数秒。");
    } else if (!(profile.timeout_secs >= 5 && profile.timeout_secs <= 86400)) {
      pushProfileError(errors, "timeout_secs", "超时时间必须在 5 到 86400 秒之间。");
    }
  }
  for (const [key, value] of Object.entries(profile.env_refs || {})) {
    if (!key.trim()) pushProfileError(errors, "env_refs", "环境变量名不能为空。");
    if (!isExternalSecretRef(value)) {
      pushProfileError(errors, `env_refs.${key}`, "环境变量值必须引用外部 Secret 或环境变量，不能写明文。");
    }
  }
  return { ok: errors.length === 0, errors };
}

function pushProfileError(errors, path, message) {
  errors.push({ path, message });
}

function normalizeValidation(validation) {
  if (!validation) return { ok: false, errors: [{ path: "validation", message: "校验失败。" }] };
  return {
    ok: validation.ok !== false,
    errors: Array.isArray(validation.errors) ? validation.errors : [],
  };
}

function renderValidation(form, validation) {
  const target = form.querySelector("#profile-validation");
  if (!target) return;
  const result = normalizeValidation(validation);
  if (result.ok) {
    target.innerHTML = `<div class="profile-message success">表单校验通过。保存时服务端会再次校验。</div>`;
    return;
  }
  target.innerHTML = `
    <div class="profile-message danger">
      <strong>校验未通过</strong>
      <ul>${result.errors.map((error) => `<li><span class="code-ish">${escapeHtml(error.path || "-")}</span> ${escapeHtml(error.message || error.code || "无效字段")}</li>`).join("")}</ul>
    </div>`;
}

function profileNoticeHtml(notice) {
  if (!notice) return "";
  const errors = Array.isArray(notice.errors) && notice.errors.length
    ? `<ul>${notice.errors.map((error) => `<li><span class="code-ish">${escapeHtml(error.path || "-")}</span> ${escapeHtml(error.message || error.code || "无效字段")}</li>`).join("")}</ul>`
    : "";
  return `<div class="profile-message ${notice.tone || "success"}">${escapeHtml(notice.message)}${errors}</div>`;
}

async function profileRequest(path, options = {}) {
  const headers = new Headers(options.headers || {});
  headers.set("Authorization", `Bearer ${state.token}`);
  if (options.body && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }

  const response = await fetch(path, { ...options, headers });
  const payload = await profileParsePayload(response);
  if (response.status === 401 || response.status === 403) {
    logout("Admin token 已失效或没有权限。");
    const error = new Error("unauthorized");
    error.name = "AuthError";
    throw error;
  }
  if (!response.ok) {
    const message = payload?.error || payload?.message || validationMessage(payload?.validation) || response.statusText;
    const error = new Error(message);
    error.status = response.status;
    error.responsePayload = payload;
    throw error;
  }
  return payload;
}

async function profileParsePayload(response) {
  const text = await response.text();
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

function validationMessage(validation) {
  if (!validation || !Array.isArray(validation.errors)) return "";
  return validation.errors.map((error) => error.message || error.code).filter(Boolean).join("; ");
}

function optionalString(value) {
  const text = String(value || "").trim();
  return text || undefined;
}

function parseOptionalNumber(value) {
  const text = String(value || "").trim();
  if (!text) return undefined;
  return Number(text);
}

function parseList(value) {
  return String(value || "")
    .split(/\r?\n|,/)
    .map((item) => item.trim())
    .filter(Boolean);
}

function parseKeyValueLines(value) {
  const result = {};
  for (const rawLine of String(value || "").split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line) continue;
    const index = line.indexOf("=");
    if (index <= 0) continue;
    const key = line.slice(0, index).trim();
    const val = line.slice(index + 1).trim();
    if (key && val) result[key] = val;
  }
  return result;
}

function isExternalSecretRef(value) {
  const text = String(value || "").trim();
  return (
    (text.startsWith("${") && text.endsWith("}")) ||
    text.startsWith("env://") ||
    text.startsWith("aws-sm://") ||
    text.startsWith("vault://") ||
    text.startsWith("gcp-sm://") ||
    text.startsWith("azure-kv://") ||
    text.startsWith("exec://")
  );
}
