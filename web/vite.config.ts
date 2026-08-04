import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import packageJson from './package.json';

const projectDir = fileURLToPath(new URL('.', import.meta.url));

export default defineConfig({
  root: resolve(projectDir, 'src'),
  cacheDir: resolve(projectDir, 'node_modules/.vite'),
  base: '/admin/',
  plugins: [react()],
  define: {
    __APP_VERSION__: JSON.stringify(packageJson.version),
  },
  build: {
    outDir: resolve(projectDir, 'dist'),
    emptyOutDir: true,
    cssCodeSplit: false,
    sourcemap: false,
    rollupOptions: {
      output: {
        entryFileNames: 'app.js',
        chunkFileNames: '[name].js',
        assetFileNames: (assetInfo) =>
          assetInfo.name?.endsWith('.css') ? 'styles.css' : 'assets/[name][extname]',
      },
    },
  },
  server: {
    port: 4173,
    proxy: {
      '/api': process.env.ADMIN_DEV_PROXY_TARGET || 'http://127.0.0.1:8080',
    },
  },
});
