import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';
import {transcriptChunks} from './dev/message-chunks.mjs';
import { mockWeb } from './dev/mock.mjs';
export default defineConfig({ plugins: [solid(), ...(process.env.AB_WEB_MOCK === '1' ? [mockWeb()] : [])], build: { target: 'es2022', sourcemap: false, cssCodeSplit: false, manifest: true, rollupOptions: { output: { chunkFileNames: chunk => `assets/${chunk.name === 'CardVideo' ? 'ViewerSession' : chunk.name}-[hash].js`, onlyExplicitManualChunks: true, manualChunks: transcriptChunks() } } }, server: { host: '127.0.0.1' }, preview: { host: '127.0.0.1', port: 0 } });
