#!/usr/bin/env node

const assert = require("node:assert/strict")
const fs = require("node:fs")
const path = require("node:path")
const { chromium } = require("playwright")

const baseUrl = (process.env.OPENAB_BASE_URL || "http://127.0.0.1:18080").replace(
  /\/$/,
  ""
)
const token = process.env.OPENAB_E2E_TOKEN
const profileId = process.env.OPENAB_E2E_PROFILE_ID || "codex-default"
const screenshotPath =
  process.env.OPENAB_E2E_SCREENSHOT ||
  path.resolve("artifacts/codeg-e2e/codeg-workbench.png")
const evidencePath =
  process.env.OPENAB_E2E_EVIDENCE ||
  path.resolve("artifacts/codeg-e2e/evidence.json")

assert(token, "OPENAB_E2E_TOKEN is required")

function ensureParent(filePath) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true })
}

async function waitUntil(predicate, message, timeout = 20_000) {
  const deadline = Date.now() + timeout
  while (Date.now() < deadline) {
    if (await predicate()) return
    await new Promise((resolve) => setTimeout(resolve, 200))
  }
  throw new Error(message)
}

async function main() {
  ensureParent(screenshotPath)
  ensureParent(evidencePath)

  const expectedOrigin = new URL(baseUrl).origin
  const requestOrigins = new Set()
  const sessionResponses = []
  const consoleErrors = []
  const pageErrors = []
  const browser = await chromium.launch({ headless: true })
  const context = await browser.newContext({
    viewport: { width: 1440, height: 1000 },
    locale: "en-US",
  })
  const page = await context.newPage()

  page.on("request", (request) => {
    const url = request.url()
    if (url.startsWith("http://") || url.startsWith("https://")) {
      requestOrigins.add(new URL(url).origin)
      assert(!url.includes(token), "authentication token appeared in a request URL")
    }
  })
  page.on("response", (response) => {
    const url = response.url()
    if (url.includes("/api/v1/sessions")) {
      sessionResponses.push({
        method: response.request().method(),
        path: new URL(url).pathname,
        status: response.status(),
        contentType: response.headers()["content-type"] || "",
      })
    }
  })
  page.on("console", (message) => {
    if (message.type() === "error") consoleErrors.push(message.text())
  })
  page.on("pageerror", (error) => pageErrors.push(error.message))

  try {
    const fixtureProfile = await context.request.post(
      `${baseUrl}/api/v1/agent-profiles`,
      {
        headers: {
          Accept: "application/json",
          Authorization: `Bearer ${token}`,
          "Content-Type": "application/json",
        },
        data: {
          id: profileId,
          name: "Codeg browser E2E",
          agent_type: "codex",
          enabled: true,
          command: "node",
          args: [
            "/opt/openab/codeg-e2e-agent.mjs",
            "/artifacts/agent.log",
          ],
          working_dir: "/tmp/openab-e2e-home",
        },
      }
    )
    assert.equal(fixtureProfile.status(), 201)

    const rootResponse = await context.request.get(`${baseUrl}/`)
    assert.equal(rootResponse.status(), 200)
    assert.match(rootResponse.headers()["content-type"] || "", /^text\/html/)
    assert.equal(rootResponse.headers()["cache-control"], "no-cache")
    const rootHtml = await rootResponse.text()
    const hashedAsset = rootHtml.match(
      /(?:src|href)=["']([^"']*\/_next\/static\/[^"']+)["']/
    )
    assert(hashedAsset, "Codeg root did not reference a Next.js static asset")
    const assetUrl = new URL(hashedAsset[1], baseUrl).toString()
    const assetResponse = await context.request.get(assetUrl)
    assert.equal(assetResponse.status(), 200)
    assert.equal(
      assetResponse.headers()["cache-control"],
      "public, max-age=31536000, immutable"
    )

    const retiredAdmin = await context.request.get(`${baseUrl}/admin`, {
      maxRedirects: 0,
    })
    assert.equal(retiredAdmin.status(), 308)
    assert.equal(retiredAdmin.headers().location, "/")

    const reservedApiMiss = await context.request.get(
      `${baseUrl}/api/v1/definitely-missing`,
      {
        headers: {
          Accept: "text/html",
          Authorization: `Bearer ${token}`,
        },
      }
    )
    assert.equal(reservedApiMiss.status(), 404)
    assert.doesNotMatch(
      reservedApiMiss.headers()["content-type"] || "",
      /^text\/html/
    )

    await page.goto(`${baseUrl}/workspace`, { waitUntil: "domcontentloaded" })
    await page.waitForURL(/\/login(?:\?|$)/)
    await page.getByLabel("OpenAB URL").fill(baseUrl)
    await page.getByLabel("Profile ID").fill(profileId)
    await page.locator('input[type="password"]').fill(token)
    await Promise.all([
      page.waitForURL(/\/workspace(?:\?|$)/),
      page.getByRole("button", { name: "Connect" }).click(),
    ])

    const composer = page.locator('[role="textbox"]:visible').last()
    await composer.waitFor({ state: "visible" })

    const thinkingRenderedPromise = page
      .getByText("checking context", { exact: true })
      .last()
      .waitFor({ state: "visible", timeout: 10_000 })
    const toolRenderedPromise = page
      .getByText("Inspect workspace", { exact: true })
      .last()
      .waitFor({ state: "visible", timeout: 10_000 })

    const promptResponsePromise = page.waitForResponse(
      (response) =>
        response.request().method() === "POST" &&
        /^\/api\/v1\/sessions\/[^/]+\/messages$/.test(
          new URL(response.url()).pathname
        ),
      { timeout: 20_000 }
    )
    await composer.fill("exercise unified Codeg")
    await page.locator('button[title="Send"]:visible').last().click()
    const promptResponse = await promptResponsePromise
    assert.equal(promptResponse.status(), 202)
    const promptPath = new URL(promptResponse.url()).pathname
    const promptMatch =
      /^\/api\/v1\/sessions\/([^/]+)\/messages$/.exec(promptPath)
    assert(promptMatch, `unexpected prompt response path: ${promptPath}`)
    const session = {
      session_id: decodeURIComponent(promptMatch[1]),
    }
    assert.match(session.session_id, /^[A-Za-z0-9_-]+/)
    assert(
      sessionResponses.some(
        (response) =>
          response.method === "POST" &&
          response.path === "/api/v1/sessions" &&
          response.status === 201
      ),
      "session creation response was not observed"
    )

    await thinkingRenderedPromise
    await toolRenderedPromise
    await page
      .getByText("control-plane reply", { exact: true })
      .last()
      .waitFor({ state: "visible", timeout: 10_000 })

    const transcript = await page.evaluate(async (sessionId) => {
      const response = await fetch(
        `/api/v1/sessions/${encodeURIComponent(sessionId)}/transcript`,
        {
          headers: {
            Accept: "application/json",
            Authorization: `Bearer ${localStorage.getItem("codeg_token") || ""}`,
          },
        }
      )
      if (!response.ok) throw new Error(`transcript HTTP ${response.status}`)
      return response.json()
    }, session.session_id)
    const transcriptJson = JSON.stringify(transcript)
    for (const marker of [
      "checking context",
      "Inspect workspace",
      "workspace inspected",
      "control-plane reply",
    ]) {
      assert(transcriptJson.includes(marker), `transcript missing ${marker}`)
    }

    await page.locator('[role="textbox"]:visible').last().fill("block")
    await page.locator('button[title="Send"]:visible').last().click()
    const cancelButton = page.locator('button[title="Cancel"]:visible').last()
    await cancelButton.waitFor({ state: "visible" })
    const cancelResponse = page.waitForResponse(
      (response) =>
        response.request().method() === "POST" &&
        new URL(response.url()).pathname ===
          `/api/v1/sessions/${session.session_id}/cancel`,
      { timeout: 20_000 }
    )
    await cancelButton.click()
    assert.equal((await cancelResponse).status(), 204)
    await page.locator('button[title="Send"]:visible').last().waitFor()

    const sseCountBeforeOffline = sessionResponses.filter((response) =>
      response.contentType.startsWith("text/event-stream")
    ).length
    assert(sseCountBeforeOffline >= 1, "the browser never established session SSE")
    await context.setOffline(true)
    await page.waitForTimeout(700)
    await context.setOffline(false)
    await waitUntil(
      () =>
        sessionResponses.filter((response) =>
          response.contentType.startsWith("text/event-stream")
        ).length > sseCountBeforeOffline,
      "session SSE did not reconnect after an offline interval"
    )

    const refreshResponse = await page.reload({ waitUntil: "domcontentloaded" })
    assert(refreshResponse)
    assert.equal(refreshResponse.status(), 200)
    assert.equal(refreshResponse.headers()["cache-control"], "no-cache")
    await page.getByText("control-plane reply", { exact: false }).waitFor()

    // Hydrated progress is folded behind the completed-turn disclosure, then
    // completed tools are folded into Codeg's tool-group chip.
    const completedTurn = page
      .getByRole("button", { name: /^(?:Finished working|Worked for)/ })
      .last()
    await completedTurn.waitFor({ state: "visible", timeout: 10_000 })
    await completedTurn.click()
    const toolGroup = page
      .locator(".reply-fold-body button.ws-msg-chip:visible")
      .last()
    await toolGroup.waitFor({ state: "visible", timeout: 10_000 })
    await toolGroup.click()
    await page
      .getByText("workspace inspected", { exact: true })
      .waitFor({ state: "visible", timeout: 10_000 })
    await page.locator('[role="textbox"]:visible').last().waitFor()

    for (const origin of requestOrigins) {
      assert.equal(origin, expectedOrigin, `cross-origin browser request: ${origin}`)
    }
    assert.equal(pageErrors.length, 0, `page errors: ${pageErrors.join(" | ")}`)

    await page.screenshot({ path: screenshotPath, fullPage: true })
    const evidence = {
      ok: true,
      origin: expectedOrigin,
      codegRevision:
        process.env.CODEG_REVISION ||
        "29018340851d8e569a52b4bd139dcfed09efc7bd",
      sessionId: session.session_id,
      checks: [
        "Codeg static root and immutable Next.js assets",
        "legacy /admin permanent redirect",
        "reserved API miss does not fall back to HTML",
        "login with bearer token in an Authorization header",
        "session creation and prompt through same-origin unified listener",
        "live assistant, thinking, and tool rendering from ACP/SSE",
        "cancel through the workbench",
        "SSE reconnect after an offline interval",
        "direct /workspace refresh and transcript recovery",
      ],
      sessionHttp: sessionResponses,
      requestOrigins: [...requestOrigins],
      consoleErrorCount: consoleErrors.length,
      pageErrorCount: pageErrors.length,
    }
    fs.writeFileSync(evidencePath, `${JSON.stringify(evidence, null, 2)}\n`)
    process.stdout.write(`${JSON.stringify(evidence)}\n`)
  } catch (error) {
    await page.screenshot({ path: screenshotPath, fullPage: true }).catch(() => {})
    throw error
  } finally {
    await browser.close()
  }
}

main().catch((error) => {
  const message = String(error?.stack || error?.message || error).replaceAll(
    token,
    "[REDACTED]"
  )
  console.error(message)
  process.exit(1)
})
