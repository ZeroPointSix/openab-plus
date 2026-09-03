const { chromium } = require("playwright");

const baseUrl = process.env.CODEG_BASE_URL ?? "http://127.0.0.1:18080";
const token = process.env.CODEG_TEST_TOKEN ?? "ci-codeg-token";
const screenshotPath =
  process.env.CODEG_SCREENSHOT_PATH ?? "/tmp/codeg-browser-smoke.png";

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
    page.on("pageerror", (error) => fatalErrors.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error") {
        fatalErrors.push(message.text());
      }
    });

    const response = await page.goto(baseUrl, {
      waitUntil: "networkidle",
      timeout: 30000,
    });
    if (!response || !response.ok()) {
      throw new Error(`workbench navigation returned HTTP ${response?.status()}`);
    }

    await page.waitForTimeout(2000);
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
