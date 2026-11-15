import { defineConfig, devices } from '@playwright/test';

const port = process.env.REGENTE_E2E_PORT ?? '17717';
const baseURL = process.env.REGENTE_E2E_URL ?? `http://127.0.0.1:${port}`;
export default defineConfig({
  testDir: './e2e',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  use: { baseURL, ...devices['Desktop Chrome'], launchOptions: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE } : undefined },
  webServer: process.env.REGENTE_E2E_URL ? undefined : {
    command: 'node ../scripts/e2e-daemon.mjs',
    url: `${baseURL}/api/v1/health`,
    reuseExistingServer: false,
    timeout: 30_000,
  },
});
