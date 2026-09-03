const baseUrl = process.env.CODEG_BASE_URL ?? "http://127.0.0.1:18080";
const token = process.env.CODEG_TEST_TOKEN ?? "ci-codeg-token";

const commands = [
  "automation_list",
  "work_task_list",
  "science_list",
  "science_list_all_install_statuses",
  "experts_list",
  "experts_list_all_install_statuses",
  "officecli_skill_list_all_install_statuses",
  "app_update_status",
  "app_update_state",
  "check_app_update",
  "get_feedback_settings",
  "health",
  "get_system_language_settings",
  "get_system_terminal_settings",
  "list_folder_groups",
  "list_all_folder_details",
  "list_open_folder_details",
  "list_opened_tabs",
  "list_all_conversations",
  "create_chat_dir",
  "acp_list_agents",
  "acp_list_agent_skills",
];

const root = await fetch(baseUrl);
if (!root.ok || (await root.text()).length < 1000) {
  throw new Error("Codeg static root was not served");
}

const admin = await fetch(`${baseUrl}/admin`);
if (!admin.ok || !(await admin.text()).includes("OpenAB Admin")) {
  throw new Error("Existing /admin UI was not preserved");
}

for (const command of commands) {
  const response = await fetch(`${baseUrl}/api/${command}`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${token}`,
      "content-type": "application/json",
    },
    body: "{}",
  });
  if (response.status !== 200) {
    throw new Error(`${command} returned HTTP ${response.status}`);
  }
  await response.json();
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

console.log(`Codeg transport smoke passed for ${commands.length} commands`);
