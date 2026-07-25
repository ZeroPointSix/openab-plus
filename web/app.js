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

const ACTIVE_STATUSES = new Set(["starting", "idle", "running", "suspended"]);
const RUNNING_STATUSES = new Set(["starting", "running"]);
const FAILED_STATUSES = new Set(["error", "failed"]);

const state = {
  token: sessionStorage.getItem(TOKEN_KEY) || "",
  route: "overview",
  sessionId: null,
};

const sessionStore = {
  sessions: new Map(),
  timelines: new Map(),
  listeners: new Set(),
  filters: { status: "", agent: "", source: "" },
  sseAbort: null,
  sseConnected: false,
  unavailable: false,
  streamNotice: "",

  subscribe(listener) {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  },

  notify() {
    for (const listener of this.listeners) listener();
  },

  reset() {
    this.stopSSE();
    this.sessions.clear();
    this.timelines.clear();
    this.unavailable = false;
    this.sseConnected = false;
    this.streamNotice = "";
    this.notify();
  },

  list() {
    return [...this.sessions.values()].sort(compareSessions);
  },

  get(sessionId) {
    return this.sessions.get(sessionId) || null;
  },

  filteredList() {
    const { status, agent, source } = this.filters;
    return this.list().filter((session) => {
      if (status && sessionStatus(session) !== status) return false;
      if (agent && sessionAgent(session) !== agent) return false;
      if (source && sessionPlatform(session) !== source) return false;
      return true;
    });
  },

  setFilters(next) {
    this.filters = { ...this.filters, ...next };
    this.notify();
  },

  upsert(snapshot, eventName = null) {
    const id = sessionIdOf(snapshot);
    if (!id) return;

    const normalized = normalizeSession(snapshot);
    this.sessions.set(id, normalized);

    if (eventName) {
      this.recordTimeline(id, normalized, eventName);
    }
    this.notify();
  },

  recordTimeline(sessionId, session, eventName) {
    const timeline = this.timelines.get(sessionId) || [];

    if (!timeline.length && session.created_at) {
      timeline.push({
        at: session.created_at,
        event: "session.created",
        status: sessionStatus(session),
        error: null,
      });
    }

    const isDuplicateCreated =
      eventName === "session.created" &&
      timeline.length === 1 &&
      timeline[0].event === "session.created";

    if (!isDuplicateCreated) {
      timeline.push({
        at: session.updated_at || session.created_at || new Date().toISOString(),
        event: eventName,
        status: sessionStatus(session),
        error: session.last_error || null,
      });
    }

    this.timelines.set(sessionId, timeline.slice(-20));
  },

  timeline(sessionId) {
    const session = this.get(sessionId);
    const events = this.timelines.get(sessionId) || [];
    if (!session) return events;
    if (!events.length) {
      return [
        {
          at: session.created_at,
          event: "session.created",
          status: sessionStatus(session),
          error: null,
        },
      ];
    }
    return events;
  },

  async sync() {
    const sessions = await fetchSessions({ optional: true });
    if (sessions === null) {
      this.unavailable = true;
      this.sessions.clear();
      this.notify();
      return false;
    }

    this.unavailable = false;
    this.sessions.clear();
    for (const session of sessions) this.upsert(session);
    this.notify();
    return true;
  },

  startSSE(getToken) {
    this.stopSSE();
    const controller = new AbortController();
    this.sseAbort = controller;
    this.streamNotice = "";

    void runSessionEventStream({
      getToken,
      signal: controller.signal,
      onOpen: () => {
        this.sseConnected = true;
        this.streamNotice = "";
        updateTopbarMeta();
        this.notify();
      },
      onClose: () => {
        this.sseConnected = false;
        updateTopbarMeta();
        this.notify();
      },
      onEvent: (eventName, payload) => this.handleSSEEvent(eventName, payload),
      onReconnect: async () => {
        await this.sync();
      },
    });
  },

  stopSSE() {
    if (this.sseAbort) {
      this.sseAbort.abort();
      this.sseAbort = null;
    }
    this.sseConnected = false;
  },

  handleSSEEvent(eventName, payload) {
    if (!payload || typeof payload !== "object") return;

    if (payload.error && !payload.snapshot) {
      this.streamNotice = payload.error;
      if (payload.skipped !== undefined || eventName === "error") {
        void this.sync();
      } else {
        this.notify();
      }
      return;
    }

    if (payload.snapshot) {
      const event = eventName || payload.event || "update";
      this.upsert(payload.snapshot, event);
      return;
    }

    const id = payload.session_id || payload.sessionId;
    if (id) {
      const existing = this.get(id);
      if (existing) {
        this.upsert({ ...existing, ...payload }, eventName || "update");
      }
    }
  },
};

const app = document.querySelector("#app");

window.addEventListener("hashchange", () => {
  applyRouteFromHash();
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

function applyRouteFromHash() {
  const parsed = parseRoute(window.location.hash);
  state.route = parsed.route;
  state.sessionId = parsed.sessionId;
}

function parseRoute(hash) {
  const path = hash.replace(/^#\/?/, "");
  if (!path) return { route: "overview", sessionId: null };
  const segments = path.split("/").filter(Boolean);
  const route = segments[0] || "overview";
  if (route === "sessions" && segments.length > 1) {
    return {
      route: "session-detail",
      sessionId: decodeURIComponent(segments.slice(1).join("/")),
    };
  }
  if (Object.prototype.hasOwnProperty.call(ROUTES, route)) {
    return { route, sessionId: null };
  }
  return { route: "overview", sessionId: null };
}

function setRoute(route, sessionId = null) {
  if (route === "session-detail" && sessionId) {
    window.location.hash = `#/sessions/${encodeURIComponent(sessionId)}`;
    return;
  }
  window.location.hash = `#/${route}`;
}

function renderLogin(error = "") {
  sessionStore.reset();
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
      await enterDashboard();
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
    await enterDashboard();
  } catch (error) {
    state.token = "";
    sessionStorage.removeItem(TOKEN_KEY);
    renderLogin(readableError(error));
  }
}

async function enterDashboard() {
  applyRouteFromHash();
  renderShell();
  const hasSessionApi = await sessionStore.sync();
  if (hasSessionApi) {
    sessionStore.startSSE(() => state.token);
  }
  loadRoute();
}

function renderShell() {
  applyRouteFromHash();
  const pageTitle = state.route === "session-detail"
    ? "会话详情"
    : ROUTES[state.route] || ROUTES.overview;

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
            <a href="#/${key}" data-route="${key}" class="${navActiveClass(key)}">${label}</a>
          `).join("")}
        </nav>
      </aside>
      <section class="main">
        <header class="topbar">
          <div>
            <h1>${escapeHtml(pageTitle)}</h1>
            <div class="topbar-meta">${topbarMetaHtml()}</div>
          </div>
          <button id="logout-button" class="ghost-button" type="button">退出</button>
        </header>
        <main id="content" class="content" aria-live="polite"></main>
      </section>
    </div>`;

  document.querySelector("#logout-button").addEventListener("click", () => logout());
}

function updateTopbarMeta() {
  const meta = document.querySelector(".topbar-meta");
  if (meta) meta.innerHTML = topbarMetaHtml();
}

function topbarMetaHtml() {
  return `
    <span class="status-pill"><span class="status-dot"></span>Admin token 已启用</span>
    ${sessionStore.sseConnected ? '<span class="status-pill live"><span class="status-dot"></span>SSE 已连接</span>' : ""}
    ${sessionStore.streamNotice ? `<span class="status-pill warning">${escapeHtml(sessionStore.streamNotice)}</span>` : ""}`;
}

function navActiveClass(key) {
  if (state.route === "session-detail" && key === "sessions") return "active";
  return state.route === key ? "active" : "";
}

let routeUnsubscribe = null;

async function loadRoute() {
  routeUnsubscribe?.();
  routeUnsubscribe = null;

  const content = document.querySelector("#content");
  content.innerHTML = skeletonPanel(pageTitleForRoute());

  try {
    if (state.route === "sessions") {
      routeUnsubscribe = sessionStore.subscribe(() => renderSessions(content));
      await renderSessions(content);
    } else if (state.route === "session-detail") {
      routeUnsubscribe = sessionStore.subscribe(() => renderSessionDetail(content));
      await renderSessionDetail(content);
    } else if (state.route === "profiles") {
      await renderProfiles(content);
    } else if (state.route === "config") {
      await renderConfig(content);
    } else {
      routeUnsubscribe = sessionStore.subscribe(() => renderOverview(content));
      await renderOverview(content);
    }
  } catch (error) {
    if (error.name === "AuthError") return;
    content.innerHTML = errorPanel(readableError(error));
  }
}

function pageTitleForRoute() {
  if (state.route === "session-detail") return "会话详情";
  return ROUTES[state.route] || ROUTES.overview;
}

async function renderOverview(content) {
  if (sessionStore.unavailable) {
    content.innerHTML = sessionsUnavailablePanel("总览暂不可用：当前运行形态未挂载会话 API。");
    return;
  }

  const sessions = sessionStore.list();
  const profileDoc = await apiMaybe("/api/v1/agent-profiles");
  const status = await apiMaybe("/api/v1/config/status");
  const profiles = Array.isArray(profileDoc?.profiles) ? profileDoc.profiles : [];
  const activeCount = sessions.filter((s) => ACTIVE_STATUSES.has(sessionStatus(s))).length;
  const runningCount = sessions.filter((s) => RUNNING_STATUSES.has(sessionStatus(s))).length;
  const failedCount = sessions.filter((s) => FAILED_STATUSES.has(sessionStatus(s))).length;
  const recentErrors = sessions
    .filter((s) => s.last_error)
    .sort((a, b) => String(b.updated_at).localeCompare(String(a.updated_at)))
    .slice(0, 5);

  content.innerHTML = `
    <div class="readonly-banner">只读看板：可观察会话状态，不支持发送 Prompt 或接管会话。</div>
    <section class="metrics-grid">
      ${metricCard("总会话", sessions.length)}
      ${metricCard("活动会话", activeCount)}
      ${metricCard("运行中", runningCount)}
      ${metricCard("失败/错误", failedCount)}
      ${metricCard("Profiles", profiles.length)}
    </section>
    <section class="panel">
      <div class="panel-header"><h2>最近会话</h2><button class="ghost-button" data-jump="sessions">查看全部</button></div>
      <div class="panel-body">${sessionListPreview(sessions.slice(0, 10), { clickable: true })}</div>
    </section>
    <section class="panel">
      <div class="panel-header"><h2>最近错误</h2></div>
      <div class="panel-body">${recentErrorsPanel(recentErrors)}</div>
    </section>
    <section class="panel">
      <div class="panel-header"><h2>Gateway 状态</h2></div>
      <div class="panel-body">${statusSummary(status)}</div>
    </section>`;

  bindSessionPreviewLinks(content);
  content.querySelector('[data-jump="sessions"]')?.addEventListener("click", () => setRoute("sessions"));
}

async function renderSessions(content) {
  if (sessionStore.unavailable) {
    content.innerHTML = sessionsUnavailablePanel();
    return;
  }

  const sessions = sessionStore.filteredList();
  const options = filterOptions(sessionStore.list());

  content.innerHTML = `
    <div class="readonly-banner">只读看板：会话列表通过 SSE 自动刷新，无需手动 reload。</div>
    <section class="panel">
      <div class="panel-header">
        <h2>会话列表</h2>
        <div class="panel-actions">
          <span class="badge">${escapeHtml(String(sessions.length))} 条</span>
          <button class="ghost-button" id="refresh-sessions" type="button">全量校准</button>
        </div>
      </div>
      <div class="filter-bar">
        ${filterSelect("status", "状态", options.statuses, sessionStore.filters.status)}
        ${filterSelect("agent", "Agent", options.agents, sessionStore.filters.agent)}
        ${filterSelect("source", "来源", options.sources, sessionStore.filters.source)}
      </div>
      <div class="table-wrap">${sessionsTable(sessions, { clickable: true })}</div>
    </section>`;

  content.querySelectorAll(".session-filter").forEach((select) => {
    select.addEventListener("change", (event) => {
      sessionStore.setFilters({ [event.currentTarget.name]: event.currentTarget.value });
      renderSessions(content);
    });
  });
  content.querySelector("#refresh-sessions")?.addEventListener("click", async () => {
    await sessionStore.sync();
    renderSessions(content);
  });
  bindSessionRowLinks(content);
}

async function renderSessionDetail(content) {
  if (sessionStore.unavailable) {
    content.innerHTML = sessionsUnavailablePanel();
    return;
  }

  let session = sessionStore.get(state.sessionId);
  if (!session) {
    try {
      session = normalizeSession(await api(`/api/v1/sessions/${encodeURIComponent(state.sessionId)}`));
      sessionStore.upsert(session);
    } catch (error) {
      if (error.status === 404) {
        content.innerHTML = errorPanel("未找到该会话。");
        return;
      }
      throw error;
    }
  }

  const timeline = sessionStore.timeline(state.sessionId);

  content.innerHTML = `
    <div class="readonly-banner">只读会话详情：不提供 Prompt 输入、中断或恢复操作。</div>
    <section class="panel">
      <div class="panel-header">
        <h2>${escapeHtml(sessionIdOf(session))}</h2>
        <div class="panel-actions">
          ${statusBadge(sessionStatus(session))}
          <button class="ghost-button" type="button" data-back-sessions>返回列表</button>
        </div>
      </div>
      <div class="panel-body detail-grid">${sessionDetailGrid(session)}</div>
    </section>
    <section class="panel">
      <div class="panel-header"><h2>配置快照</h2></div>
      <div class="panel-body"><pre class="json-view">${escapeHtml(JSON.stringify(configSnapshot(session), null, 2))}</pre></div>
    </section>
    <section class="panel">
      <div class="panel-header"><h2>状态时间线</h2></div>
      <div class="panel-body">${timelinePanel(timeline)}</div>
    </section>
    <section class="panel">
      <div class="panel-header"><h2>最近错误</h2></div>
      <div class="panel-body">${session.last_error
    ? `<div class="error-box">${escapeHtml(session.last_error)}</div>`
    : `<div class="empty-state">暂无错误。</div>`}</div>
    </section>`;

  content.querySelector("[data-back-sessions]")?.addEventListener("click", () => setRoute("sessions"));
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

async function runSessionEventStream({ getToken, signal, onOpen, onClose, onEvent, onReconnect }) {
  let attempt = 0;

  while (!signal.aborted) {
    try {
      const response = await fetch("/api/v1/sessions/events", {
        headers: { Authorization: `Bearer ${getToken()}` },
        signal,
      });

      if (response.status === 401 || response.status === 403) {
        logout("Admin token 已失效或没有权限。");
        return;
      }
      if (response.status === 404) return;
      if (!response.ok) {
        throw new Error(`SSE unavailable (${response.status})`);
      }
      if (!response.body) {
        throw new Error("SSE response has no body");
      }

      attempt = 0;
      onOpen?.();

      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";

      while (!signal.aborted) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        buffer = consumeSSEBuffer(buffer, onEvent);
      }
    } catch (error) {
      if (signal.aborted || error.name === "AbortError") break;
    } finally {
      onClose?.();
    }

    if (signal.aborted) break;
    attempt += 1;
    await onReconnect?.();
    await sleep(Math.min(30_000, 1_000 * 2 ** Math.min(attempt, 5)));
  }
}

function consumeSSEBuffer(buffer, onEvent) {
  const normalized = buffer.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  const chunks = normalized.split("\n\n");
  const rest = chunks.pop() || "";

  for (const chunk of chunks) {
    const parsed = parseSSEChunk(chunk);
    if (parsed.data) {
      let payload = parsed.data;
      try {
        payload = JSON.parse(parsed.data);
      } catch {
        // keep raw string
      }
      onEvent(parsed.event || "message", payload);
    }
  }

  return rest;
}

function parseSSEChunk(chunk) {
  const lines = chunk.split(/\r?\n/);
  const dataLines = [];
  let event = "";

  for (const line of lines) {
    if (line.startsWith("event:")) event = line.slice(6).trim();
    if (line.startsWith("data:")) dataLines.push(line.slice(5).trimStart());
  }

  return { event, data: dataLines.join("\n") };
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
  routeUnsubscribe?.();
  routeUnsubscribe = null;
  sessionStore.reset();
  state.token = "";
  sessionStorage.removeItem(TOKEN_KEY);
  renderLogin(message);
}

function normalizeSession(session) {
  return {
    ...session,
    session_id: sessionIdOf(session),
    status: sessionStatus(session),
    source: session.source || {
      platform: sessionPlatform(session),
      thread_id: sessionThread(session),
    },
  };
}

function sessionIdOf(session) {
  return valueOf(session, ["session_id", "id", "sessionId"]) || "";
}

function sessionAcpId(session) {
  return valueOf(session, ["acp_session_id", "acpSessionId", "acp.id"]) || "-";
}

function sessionStatus(session) {
  return String(valueOf(session, ["status", "state", "phase"]) || "unknown").toLowerCase();
}

function sessionAgent(session) {
  return valueOf(session, ["agent", "agent_type"]) || "-";
}

function sessionPlatform(session) {
  return valueOf(session, ["source.platform", "platform", "adapter", "source_platform"]) || "-";
}

function sessionThread(session) {
  return valueOf(session, ["source.thread_id", "thread_id", "threadId"]) || "-";
}

function sessionChannel(session) {
  return valueOf(session, ["source.channel_id", "source.channelId", "channel_id", "channelId"]) || "-";
}

function sessionUser(session) {
  return valueOf(session, ["source.user_id", "source.userId", "user_id", "userId"]) || "-";
}

function compareSessions(a, b) {
  return String(b.updated_at || b.created_at || "").localeCompare(String(a.updated_at || a.created_at || ""));
}

function filterOptions(sessions) {
  const statuses = new Set();
  const agents = new Set();
  const sources = new Set();
  for (const session of sessions) {
    statuses.add(sessionStatus(session));
    agents.add(sessionAgent(session));
    sources.add(sessionPlatform(session));
  }
  return {
    statuses: [...statuses].sort(),
    agents: [...agents].sort(),
    sources: [...sources].sort(),
  };
}

function filterSelect(name, label, values, current) {
  const options = [`<option value="">全部${escapeHtml(label)}</option>`]
    .concat(values.map((value) => `<option value="${escapeHtml(value)}"${current === value ? " selected" : ""}>${escapeHtml(value)}</option>`));
  return `
    <label class="filter-field">
      <span>${escapeHtml(label)}</span>
      <select class="session-filter" name="${escapeHtml(name)}">${options.join("")}</select>
    </label>`;
}

function sessionDetailGrid(session) {
  const items = [
    ["Session ID", sessionIdOf(session), true],
    ["ACP Session", sessionAcpId(session), true],
    ["Agent", sessionAgent(session), false],
    ["Profile", sessionProfileLabel(session), false],
    ["工作目录", session.workdir || "-", true],
    ["来源", sessionPlatform(session), false],
    ["Channel", sessionChannel(session), true],
    ["Thread", sessionThread(session), true],
    ["User", sessionUser(session), true],
    ["模型", session.model || "-", false],
    ["创建时间", formatTime(session.created_at), false],
    ["最近活动", formatTime(session.updated_at), false],
  ];

  return items.map(([label, value, code]) => `
    <div>
      <strong>${escapeHtml(label)}</strong>
      <p class="${code ? "code-ish" : ""}">${escapeHtml(value)}</p>
    </div>`).join("");
}

function sessionProfileLabel(session) {
  const label = session.profile_name || session.profile_id || "-";
  if (label !== "-" && String(session.profile_status || "").toLowerCase() === "deleted") {
    return `${label} (deleted)`;
  }
  return label;
}

function configSnapshot(session) {
  const snapshot = {
    model: session.model || null,
    reasoning_effort: valueOf(session, ["reasoning_effort", "reasoningEffort", "config.reasoning_effort"]) || null,
    profile_id: session.profile_id || null,
    profile_name: session.profile_name || null,
    profile_status: session.profile_status || null,
    status: sessionStatus(session),
    workdir: session.workdir || null,
    source: session.source || null,
  };

  const acpSessionId = sessionAcpId(session);
  if (acpSessionId !== "-") snapshot.acp_session_id = acpSessionId;
  if (session.external_url) snapshot.external_url = session.external_url;

  for (const key of ["approval_policy", "sandbox_mode", "network_access"]) {
    const value = valueOf(session, [key, `config.${key}`]);
    if (value !== undefined) snapshot[key] = value;
  }

  return snapshot;
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

function sessionsUnavailablePanel(message = "当前运行形态未挂载会话 API。") {
  return `
    <section class="panel">
      <div class="panel-header"><h2>会话看板</h2></div>
      <div class="panel-body"><div class="empty-state">${escapeHtml(message)}</div></div>
    </section>`;
}

function sessionListPreview(sessions, { clickable = false } = {}) {
  if (!sessions.length) return `<div class="empty-state">暂无会话。</div>`;
  return `<div class="session-preview-list">${sessions.map((session) => {
    const id = sessionIdOf(session);
    const status = sessionStatus(session);
    const attrs = clickable ? ` data-session-id="${escapeHtml(id)}" class="session-link"` : "";
    return `<p${attrs}>${statusBadge(status)} <span class="code-ish">${escapeHtml(id)}</span> <span class="muted-inline">${escapeHtml(sessionAgent(session))}</span></p>`;
  }).join("")}</div>`;
}

function recentErrorsPanel(errors) {
  if (!errors.length) return `<div class="empty-state">暂无错误会话。</div>`;
  return errors.map((session) => `
    <article class="error-item">
      <div class="error-item-head">
        <button class="link-button session-link" data-session-id="${escapeHtml(sessionIdOf(session))}" type="button">${escapeHtml(sessionIdOf(session))}</button>
        ${statusBadge(sessionStatus(session))}
      </div>
      <p>${escapeHtml(session.last_error)}</p>
    </article>`).join("");
}

function sessionsTable(sessions, { clickable = false } = {}) {
  if (!sessions.length) return `<div class="empty-state">暂无会话。</div>`;
  const rowClass = clickable ? ' class="session-row"' : "";
  const rows = sessions.map((session) => {
    const id = sessionIdOf(session);
    const attrs = clickable ? ` data-session-id="${escapeHtml(id)}"${rowClass}` : "";
    return `<tr${attrs}>
      <td>${statusBadge(sessionStatus(session))}</td>
      <td class="code-ish">${escapeHtml(id)}</td>
      <td>${escapeHtml(sessionAgent(session))}</td>
      <td>${escapeHtml(sessionPlatform(session))}</td>
      <td class="code-ish">${escapeHtml(session.workdir || "-")}</td>
      <td>${escapeHtml(sessionProfileLabel(session))}</td>
      <td>${escapeHtml(session.model || "-")}</td>
      <td>${escapeHtml(formatTime(session.updated_at || session.created_at))}</td>
    </tr>`;
  }).join("");
  return `<table><thead><tr><th>Status</th><th>Session</th><th>Agent</th><th>Source</th><th>Workdir</th><th>Profile</th><th>Model</th><th>Updated</th></tr></thead><tbody>${rows}</tbody></table>`;
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

function timelinePanel(events) {
  if (!events.length) return `<div class="empty-state">暂无时间线事件。</div>`;
  return `<ol class="timeline">${events.map((entry) => `
    <li>
      <div class="timeline-head">
        <strong>${escapeHtml(entry.event)}</strong>
        ${statusBadge(entry.status)}
        <span class="muted-inline">${escapeHtml(formatTime(entry.at))}</span>
      </div>
      ${entry.error ? `<p class="timeline-error">${escapeHtml(entry.error)}</p>` : ""}
    </li>`).join("")}</ol>`;
}

function statusBadge(status) {
  const normalized = String(status || "unknown").toLowerCase();
  const tone = FAILED_STATUSES.has(normalized)
    ? "danger"
    : RUNNING_STATUSES.has(normalized)
      ? "success"
      : normalized === "exited"
        ? "muted"
        : "default";
  return `<span class="status-badge ${tone}">${escapeHtml(normalized)}</span>`;
}

function bindSessionPreviewLinks(root) {
  root.querySelectorAll("[data-session-id]").forEach((element) => {
    element.addEventListener("click", () => {
      setRoute("session-detail", element.dataset.sessionId);
    });
  });
}

function bindSessionRowLinks(root) {
  root.querySelectorAll("[data-session-id]").forEach((element) => {
    element.addEventListener("click", () => {
      setRoute("session-detail", element.dataset.sessionId);
    });
  });
}

function formatTime(value) {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return String(value);
  return date.toLocaleString();
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

function readableError(error) {
  if (error?.status === 404) return "当前网关没有可用的 admin API。";
  if (error?.status === 503) return "服务端未配置 admin token。";
  if (error?.message) return error.message;
  return "请求失败。";
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}
