const baseUrl = process.env.CODEG_BASE_URL ?? "http://127.0.0.1:18080";
const token = process.env.CODEG_TEST_TOKEN ?? "ci-codeg-token";

const arrayCommands = new Set([
  "automation_list",
  "work_task_list",
  "science_list",
  "science_list_all_install_statuses",
  "experts_list",
  "experts_list_all_install_statuses",
  "officecli_skill_list_all_install_statuses",
  "list_folder_groups",
  "list_all_folder_details",
  "list_open_folder_details",
  "list_workspace_files",
  "list_all_conversations",
  "acp_list_agents",
]);

const objectCommands = {
  app_update_status: [
    "currentVersion",
    "selfUpdateSupported",
    "capability",
    "runtime",
    "restartDelayMs",
    "rollbackAvailable",
  ],
  app_update_state: ["seq", "status"],
  check_app_update: [
    "currentVersion",
    "update",
    "selfUpdateSupported",
    "capability",
    "runtime",
    "restartDelayMs",
    "rollbackAvailable",
  ],
  get_feedback_settings: ["enabled"],
  health: ["status"],
  get_system_language_settings: ["mode", "language"],
  get_system_terminal_settings: ["default_shell"],
  list_opened_tabs: ["items", "version"],
  create_chat_dir: ["path"],
  acp_list_agent_skills: ["supported", "message", "locations", "skills"],
};

const commands = [...arrayCommands, ...Object.keys(objectCommands)];

function requireObject(command, value, keys) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(
      `${command} expected an object, got ${JSON.stringify(value)}`,
    );
  }
  for (const key of keys) {
    if (!Object.prototype.hasOwnProperty.call(value, key)) {
      throw new Error(
        `${command} missing ${key}: ${JSON.stringify(value)}`,
      );
    }
  }
}

async function postCommand(command, body = {}) {
  const response = await fetch(`${baseUrl}/api/${command}`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${token}`,
      "content-type": "application/json",
    },
    body: JSON.stringify(body),
  });
  if (response.status !== 200) {
    throw new Error(`${command} returned HTTP ${response.status}`);
  }
  return response.json();
}

const root = await fetch(baseUrl);
if (!root.ok || (await root.text()).length < 1000) {
  throw new Error("Codeg static root was not served");
}

const admin = await fetch(`${baseUrl}/admin`);
if (!admin.ok || !(await admin.text()).includes("OpenAB Admin")) {
  throw new Error("Existing /admin UI was not preserved");
}

for (const command of commands) {
  const body = await postCommand(command);
  if (arrayCommands.has(command)) {
    if (!Array.isArray(body)) {
      throw new Error(
        `${command} expected an array, got ${JSON.stringify(body)}`,
      );
    }
    continue;
  }
  requireObject(command, body, objectCommands[command]);
}

const updateStatus = await postCommand("app_update_status");
if (updateStatus.selfUpdateSupported !== false) {
  throw new Error(
    `app_update_status.selfUpdateSupported must be false, got ${JSON.stringify(updateStatus)}`,
  );
}

const updateState = await postCommand("app_update_state");
if (updateState.seq !== 0 || updateState.status !== "idle") {
  throw new Error(
    `app_update_state must be idle seq 0, got ${JSON.stringify(updateState)}`,
  );
}

const feedback = await postCommand("get_feedback_settings");
if (feedback.enabled !== false) {
  throw new Error(
    `get_feedback_settings.enabled must be false, got ${JSON.stringify(feedback)}`,
  );
}

const saved = await postCommand("save_opened_tabs", {
  items: [],
  expectedVersion: 0,
  origin: "smoke",
});
requireObject("save_opened_tabs", saved, ["accepted", "version", "tabs"]);
if (
  saved.accepted !== true ||
  saved.version !== 1 ||
  !Array.isArray(saved.tabs)
) {
  throw new Error(
    `save_opened_tabs returned an invalid outcome: ${JSON.stringify(saved)}`,
  );
}

const unauthorized = await fetch(`${baseUrl}/api/health`, {
  method: "POST",
  headers: { "content-type": "application/json" },
  body: "{}",
});
if (unauthorized.status !== 401) {
  throw new Error(`missing token returned HTTP ${unauthorized.status}`);
}

const websocketUrl = baseUrl.replace(/^http/, "ws") + "/ws/events";
const encodedToken = Buffer.from(token).toString("base64url");
await new Promise((resolve, reject) => {
  const timeout = setTimeout(
    () => reject(new Error("timed out waiting for Codeg WebSocket readiness")),
    5000,
  );
  const socket = new WebSocket(websocketUrl, [
    "codeg-events",
    `codeg-token.${encodedToken}`,
  ]);

  socket.addEventListener("open", () => {
    if (socket.protocol !== "codeg-events") {
      reject(new Error(`unexpected WebSocket protocol: ${socket.protocol}`));
    }
  });
  socket.addEventListener("message", (event) => {
    const payload = JSON.parse(String(event.data));
    if (payload.channel !== "__ready__") {
      reject(new Error(`unexpected readiness payload: ${event.data}`));
      return;
    }
    clearTimeout(timeout);
    socket.close();
    resolve();
  });
  socket.addEventListener("error", () => {
    clearTimeout(timeout);
    reject(new Error("Codeg WebSocket handshake failed"));
  });
});

console.log(
  `Codeg transport smoke passed for ${commands.length} startup commands plus save_opened_tabs`,
);
