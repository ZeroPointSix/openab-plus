const { chromium } = require("playwright");

const baseUrl = process.env.CODEG_BASE_URL ?? "http://127.0.0.1:18080";
const token = process.env.CODEG_TEST_TOKEN ?? "ci-codeg-token";
const screenshotPath =
  process.env.CODEG_SCREENSHOT_PATH ?? "/tmp/codeg-browser-smoke.png";

function isIgnorableUrl(url) {
  try {
    const { pathname } = new URL(url);
    return (
      pathname === "/favicon.ico" ||
      pathname === "/apple-touch-icon.png" ||
      pathname === "/apple-touch-icon-precomposed.png" ||
      pathname.startsWith("/.well-known/")
    );
  } catch {
    return false;
  }
}

async function main() {
  const browser = await chromium.launch({ headless: true });
  try {
    const context = await browser.newContext({
      viewport: { width: 1440, height: 900 },
    });
    await context.addInitScript((adminToken) => {
      localStorage.setItem("codeg_token", adminToken);
    }, token);

    const page = await context.newPage();
    const fatalErrors = [];
    page.on("pageerror", (error) =>
      fatalErrors.push(`page error: ${error.message}`),
    );
    page.on("response", (httpResponse) => {
      if (httpResponse.status() < 400 || isIgnorableUrl(httpResponse.url())) {
        return;
      }
      const request = httpResponse.request();
      fatalErrors.push(
        `HTTP ${httpResponse.status()} ${request.method()} ${request.resourceType()} ${httpResponse.url()}`,
      );
    });
    page.on("requestfailed", (request) => {
      if (isIgnorableUrl(request.url())) {
        return;
      }
      fatalErrors.push(
        `request failed ${request.method()} ${request.resourceType()} ${request.url()}: ${request.failure()?.errorText ?? "unknown error"}`,
      );
    });
    page.on("console", (message) => {
      if (message.type() !== "error") {
        return;
      }
      const text = message.text();
      // Chromium logs 4xx fetches without a URL; the response listener already
      // records method + URL for the real request.
      if (
        /Failed to load resource: the server responded with a status of \d{3}/.test(
          text,
        )
      ) {
        return;
      }
      const location = message.location();
      const source = location.url
        ? ` at ${location.url}:${location.lineNumber}:${location.columnNumber}`
        : "";
      fatalErrors.push(`console: ${text}${source}`);
    });

    const response = await page.goto(baseUrl, {
      waitUntil: "networkidle",
      timeout: 30000,
    });
    if (!response || !response.ok()) {
      throw new Error(`workbench navigation returned HTTP ${response?.status()}`);
    }

    await page.waitForURL((url) => url.pathname.startsWith("/workspace"), {
      timeout: 30000,
    });
    await page.waitForFunction(
      () => document.body.innerText.trim().length >= 20,
      { timeout: 30000 },
    );
    // Tab persist is debounced 500ms after hydrate; wait long enough to catch
    // a missing `save_opened_tabs` (or similar deferred RPC) as an HTTP 404.
    await page.waitForTimeout(1500);
    const pathname = new URL(page.url()).pathname;
    if (pathname.startsWith("/login")) {
      throw new Error("existing admin token did not pass the Codeg login gate");
    }

    const bodyText = (await page.locator("body").innerText()).trim();
    if (bodyText.length < 20) {
      throw new Error("Codeg workbench rendered a blank or near-blank page");
    }
    if (fatalErrors.length > 0) {
      throw new Error(`fatal browser errors: ${fatalErrors.join(" | ")}`);
    }

    await page.screenshot({ path: screenshotPath, fullPage: true });
    console.log(`Codeg browser smoke passed at ${page.url()}`);
  } finally {
    await browser.close();
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
