const TOKEN_KEY = "openab.adminToken";
const ROUTES = {
  overview: "总览",
  sessions: "会话",
  profiles: "Profile",
  config: "Gateway 配置",
};

const AUTH_PROBE_PATHS = [
  "/api/v1/sessions",
  "/api/v1/agent-profiles",
  "/api/v1/config/status",
];

const state = {
  token: sessionStorage.getItem(TOKEN_KEY) || "",
  route: currentRoute(),
};

const app = document.querySelector("#app");

window.addEventListener("hashchange", () => {
  state.route = currentRoute();
  if (state.token) {
    renderShell();
    loadRoute();
  }
});

document.addEventListener("DOMContentLoaded", () => {
  if (state.token) {
    bootstrapStoredToken();
  } else {
    renderLogin();
  }
});

function currentRoute() {
  const route = window.location.hash.replace(/^#\/?/, "") || "overview";
  return Object.prototype.hasOwnProperty.call(ROUTES, route) ? route : "overview";
}

function setRoute(route) {
  window.location.hash = `#/${route}`;
}

function renderLogin(error = "") {
  app.dataset.state = "login";
  app.innerHTML = `
    <main class="login-screen">
      <section class="login-panel" aria-labelledby="login-title">
        <div class="brand-mark">OA</div>
        <h1 id="login-title">OpenAB Admin</h1>
        <p>输入 admin token 后进入管理控制台。</p>
        <form id="login-form" class="login-row">
          <div>
            <label for="admin-token">Admin token</label>
            <input id="admin-token" name="token" type="password" autocomplete="current-password" required autofocus />
          </div>
          <button class="primary-button" type="submit">进入控制台</button>
        </form>
        ${error ? `<div class="error-box" role="alert">${escapeHtml(error)}</div>` : ""}
      </section>
    </main>`;

  document.querySelector("#login-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const token = String(form.get("token") || "").trim();
    if (!token) return;

    state.token = token;
    try {
      await validateAdminToken();
      sessionStorage.setItem(TOKEN_KEY, token);
      if (!window.location.hash) setRoute("overview");
      renderShell();
      loadRoute();
    } catch (error) {
      state.token = "";
      sessionStorage.removeItem(TOKEN_KEY);
      renderLogin(readableError(error));
    }
  });
}

async function bootstrapStoredToken() {
  renderShell();
  const content = document.querySelector("#content");
  content.innerHTML = skeletonPanel("验证登录");

  try {
    await validateAdminToken();
    loadRoute();
  } catch (error) {
    state.token = "";
    sessionStorage.removeItem(TOKEN_KEY);
    renderLogin(readableError(error));
  }
}

function renderShell() {
  app.dataset.state = "dashboard";
  app.innerHTML = `
    <div class="dashboard">
      <aside class="sidebar">
        <div class="brand">
          <div class="brand-mark">OA</div>
          <div>
            <div class="brand-title">OpenAB Admin</div>
            <div class="brand-subtitle">Gateway Console</div>
          </div>
        </div>
        <nav class="nav" aria-label="Admin navigation">
          ${Object.entries(ROUTES).map(([key, label]) => `
            <a href="#/${key}" data-route="${key}" class="${state.route === key ? "active" : ""}">${label}</a>
          `).join("")}
        </nav>
      </aside>
      <section class="main">
        <header class="topbar">
          <div>
            <h1>${ROUTES[state.route]}</h1>
            <span class="status-pill"><span class="status-dot"></span>Admin token 已启用</span>
          </div>
          <button id="logout-button" class="ghost-button" type="button">退出</button>
        </header>
        <main id="content" class="content" aria-live="polite"></main>
      </section>
    </div>`;

  document.querySelector("#logout-button").addEventListener("click", () => logout());
}

async function loadRoute() {
  const content = document.querySelector("#content");
  content.innerHTML = skeletonPanel(ROUTES[state.route]);

  try {
    if (state.route === "sessions") await renderSessions(content);
    else if (state.route === "profiles") await renderProfiles(content);
    else if (state.route === "config") await renderConfig(content);
    else await renderOverview(content);
  } catch (error) {
    if (error.name === "AuthError") return;
    content.innerHTML = errorPanel(readableError(error));
  }
}

async function renderOverview(content) {
  const sessions = await fetchSessions({ optional: true }) || [];
  const profileDoc = await apiMaybe("/api/v1/agent-profiles");
  const status = await apiMaybe("/api/v1/config/status");
  const profiles = Array.isArray(profileDoc?.profiles) ? profileDoc.profiles : [];
  const activeCount = sessions.filter(isActiveSession).length;

  content.innerHTML = `
    <section class="metrics-grid">
      ${metricCard("会话", sessions.length)}
      ${metricCard("活动会话", activeCount)}
      ${metricCard("Profiles", profiles.length)}
    </section>
    <section class="panel">
      <div class="panel-header"><h2>最近会话</h2><button class="ghost-button" data-jump="sessions">查看全部</button></div>
      <div class="panel-body">${sessionListPreview(sessions)}</div>
    </section>
    <section class="panel">
      <div class="panel-header"><h2>Gateway 状态</h2></div>
      <div class="panel-body">${statusSummary(status)}</div>
    </section>`;

  document.querySelector('[data-jump="sessions"]')?.addEventListener("click", () => setRoute("sessions"));
}

async function renderSessions(content) {
  const sessions = await fetchSessions({ optional: true });
  if (!sessions) {
    content.innerHTML = sessionsUnavailablePanel();
    return;
  }

  content.innerHTML = `
    <section class="panel">
      <div class="panel-header"><h2>会话列表</h2><button class="ghost-button" id="refresh-sessions">刷新</button></div>
      <div class="table-wrap">${sessionsTable(sessions)}</div>
    </section>`;
  document.querySelector("#refresh-sessions").addEventListener("click", loadRoute);
}

async function renderProfiles(content) {
  const [profileDoc, agents] = await Promise.all([
    apiMaybe("/api/v1/agent-profiles"),
    apiMaybe("/api/v1/agents"),
  ]);
  const profiles = Array.isArray(profileDoc?.profiles) ? profileDoc.profiles : [];
  content.innerHTML = `
    <section class="panel">
      <div class="panel-header"><h2>Agent Profiles</h2><span class="badge">Default: ${escapeHtml(profileDoc?.default_profile || "未设置")}</span></div>
      <div class="table-wrap">${profilesTable(profiles)}</div>
    </section>
    <section class="panel">
      <div class="panel-header"><h2>Agent 类型</h2></div>
      <div class="panel-body">${agentsSummary(agents)}</div>
    </section>`;
}

async function renderConfig(content) {
  const [doc, status] = await Promise.all([
    apiMaybe("/api/v1/config"),
    apiMaybe("/api/v1/config/status"),
  ]);
  content.innerHTML = `
    <section class="panel">
      <div class="panel-header"><h2>配置状态</h2></div>
      <div class="panel-body">${statusSummary(status)}</div>
    </section>
    <section class="panel">
      <div class="panel-header"><h2>当前配置</h2><span class="badge">敏感字段已遮罩</span></div>
      <div class="panel-body"><pre class="json-view">${escapeHtml(JSON.stringify(doc?.values || {}, null, 2))}</pre></div>
    </section>`;
}

async function fetchSessions({ optional = false } = {}) {
  try {
    const data = await api("/api/v1/sessions");
    if (Array.isArray(data)) return data;
    if (Array.isArray(data.sessions)) return data.sessions;
    return [];
  } catch (error) {
    if (error.name === "AuthError") throw error;
    if (optional && error.status === 404) return null;
    throw error;
  }
}

async function validateAdminToken() {
  let sawMissingProbe = false;

  for (const path of AUTH_PROBE_PATHS) {
    const result = await apiProbe(path);
    if (result.ok) return;
    if (result.status === 404) {
      sawMissingProbe = true;
      continue;
    }

    const error = new Error(result.message || "Admin token 校验失败。");
    error.status = result.status;
    throw error;
  }

  const error = new Error(sawMissingProbe ? "当前网关没有可用的 admin API。" : "Admin token 校验失败。");
  error.status = 404;
  throw error;
}

async function apiProbe(path) {
  const headers = new Headers();
  headers.set("Authorization", `Bearer ${state.token}`);

  const response = await fetch(path, { headers });
  const payload = await parsePayload(response);
  if (response.ok) return { ok: true, status: response.status };

  return {
    ok: false,
    status: response.status,
    message: payload?.error || payload?.message || response.statusText,
  };
}

async function api(path, options = {}) {
  const headers = new Headers(options.headers || {});
  headers.set("Authorization", `Bearer ${state.token}`);
  if (options.body && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }

  const response = await fetch(path, { ...options, headers });
  const payload = await parsePayload(response);

  if (response.status === 401 || response.status === 403) {
    logout("Admin token 已失效或没有权限。");
    const error = new Error("unauthorized");
    error.name = "AuthError";
    throw error;
  }

  if (!response.ok) {
    const message = payload?.error || payload?.message || response.statusText;
    const error = new Error(message);
    error.status = response.status;
    throw error;
  }

  return payload;
}

async function apiMaybe(path) {
  try {
    return await api(path);
  } catch (error) {
    if (error.name === "AuthError") throw error;
    return null;
  }
}

async function parsePayload(response) {
  const text = await response.text();
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

function logout(message = "") {
  state.token = "";
  sessionStorage.removeItem(TOKEN_KEY);
  renderLogin(message);
}

function metricCard(label, value) {
  return `<article class="metric-card"><p class="metric-label">${escapeHtml(label)}</p><p class="metric-value">${escapeHtml(String(value))}</p></article>`;
}

function skeletonPanel(title) {
  return `<section class="panel"><div class="panel-header"><h2>${escapeHtml(title)}</h2></div><div class="panel-body"><div class="skeleton-list"><span class="skeleton-line"></span><span class="skeleton-line"></span><span class="skeleton-line short"></span></div></div></section>`;
}

function errorPanel(message) {
  return `<section class="panel"><div class="panel-header"><h2>加载失败</h2></div><div class="panel-body"><div class="error-box">${escapeHtml(message)}</div></div></section>`;
}

function sessionsUnavailablePanel() {
  return `
    <section class="panel">
      <div class="panel-header"><h2>会话列表</h2></div>
      <div class="panel-body"><div class="empty-state">当前运行形态未挂载会话 API。</div></div>
    </section>`;
}

function sessionListPreview(sessions) {
  if (!sessions.length) return `<div class="empty-state">暂无会话。</div>`;
  return sessions.slice(0, 5).map((session) => {
    const id = valueOf(session, ["id", "session_id", "sessionId"]) || "unknown";
    const status = valueOf(session, ["status", "state", "phase"]) || "active";
    return `<p><span class="badge">${escapeHtml(String(status))}</span> <span class="code-ish">${escapeHtml(String(id))}</span></p>`;
  }).join("");
}

function sessionsTable(sessions) {
  if (!sessions.length) return `<div class="empty-state">暂无会话。</div>`;
  const rows = sessions.map((session) => {
    const id = valueOf(session, ["id", "session_id", "sessionId"]) || "unknown";
    const platform = valueOf(session, ["platform", "adapter", "source_platform", "source.platform"]) || "-";
    const channel = valueOf(session, ["channel_id", "channelId", "thread_id", "threadId", "source.channel_id", "source.channelId", "source.thread_id", "source.threadId"]) || "-";
    const status = valueOf(session, ["status", "state", "phase"]) || "active";
    const updated = valueOf(session, ["updated_at", "last_active_at", "lastActivityAt", "created_at"]) || "-";
    return `<tr><td class="code-ish">${escapeHtml(String(id))}</td><td>${escapeHtml(String(platform))}</td><td class="code-ish">${escapeHtml(String(channel))}</td><td><span class="badge">${escapeHtml(String(status))}</span></td><td>${escapeHtml(String(updated))}</td></tr>`;
  }).join("");
  return `<table><thead><tr><th>Session</th><th>Platform</th><th>Channel / Thread</th><th>Status</th><th>Updated</th></tr></thead><tbody>${rows}</tbody></table>`;
}

function profilesTable(profiles) {
  if (!profiles.length) return `<div class="empty-state">暂无 Profile。</div>`;
  const rows = profiles.map((profile) => `<tr><td class="code-ish">${escapeHtml(profile.id || "-")}</td><td>${escapeHtml(profile.name || "-")}</td><td>${escapeHtml(profile.agent_type || "-")}</td><td><span class="badge">${profile.enabled === false ? "disabled" : "enabled"}</span></td></tr>`).join("");
  return `<table><thead><tr><th>ID</th><th>Name</th><th>Agent</th><th>Status</th></tr></thead><tbody>${rows}</tbody></table>`;
}

function agentsSummary(agents) {
  if (!Array.isArray(agents) || !agents.length) return `<div class="empty-state">暂无 Agent 类型数据。</div>`;
  return agents.map((agent) => `<p><span class="badge">${escapeHtml(agent.agent_type || "agent")}</span> ${escapeHtml(String(agent.enabled_profile_count ?? 0))}/${escapeHtml(String(agent.profile_count ?? 0))} enabled</p>`).join("");
}

function statusSummary(status) {
  if (!status) return `<div class="empty-state">暂无配置状态。</div>`;
  const pending = Array.isArray(status.pending_restart) && status.pending_restart.length
    ? status.pending_restart.join(", ")
    : "无";
  const validation = status.last_validation?.ok === false ? "存在错误" : "通过或未运行";
  return `
    <p><strong>配置路径：</strong><span class="code-ish">${escapeHtml(status.config_path || "-")}</span></p>
    <p><strong>待重启字段：</strong>${escapeHtml(pending)}</p>
    <p><strong>回滚文件：</strong>${status.rollback_available ? "可用" : "无"}</p>
    <p><strong>最近校验：</strong>${escapeHtml(validation)}</p>`;
}

function valueOf(object, keys) {
  for (const key of keys) {
    const value = key.split(".").reduce((current, segment) => {
      if (current === undefined || current === null) return undefined;
      return current[segment];
    }, object);
    if (value !== undefined && value !== null) return value;
  }
  return undefined;
}

function isActiveSession(session) {
  const status = String(valueOf(session, ["status", "state", "phase"]) || "active");
  return !status.match(/closed|ended|stopped|exited|error|failed|cancelled|canceled/i);
}

function readableError(error) {
  if (error?.status === 404) return "当前网关没有可用的 admin API。";
  if (error?.status === 503) return "服务端未配置 admin token。";
  if (error?.message) return error.message;
  return "请求失败。";
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}
