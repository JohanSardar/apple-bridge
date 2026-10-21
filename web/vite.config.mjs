import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';
import { mockWeb } from './dev/mock.mjs';
export default defineConfig({ plugins: [solid(), ...(process.env.AB_WEB_MOCK === '1' ? [mockWeb()] : [])], build: { target: 'es2022', sourcemap: false, cssCodeSplit: false }, server: { host: '127.0.0.1' }, preview: { host: '127.0.0.1', port: 0 } });
