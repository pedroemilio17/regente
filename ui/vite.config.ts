import react from '@vitejs/plugin-react';
import { defineConfig } from 'vitest/config';

const DAEMON = 'http://127.0.0.1:7717';

export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      '/api': { target: DAEMON, ws: true, changeOrigin: true },
      '/mcp': { target: DAEMON, changeOrigin: true },
    },
  },
  build: {
    outDir: 'dist',
    target: 'es2022',
  },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.{ts,tsx}'],
    restoreMocks: true,
  },
});
