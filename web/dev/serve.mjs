import { preview } from 'vite';
import { writeFile } from 'node:fs/promises';
const server = await preview({ preview: { host: '127.0.0.1', port: 0, strictPort: true } });
const address = server.httpServer.address();
await writeFile(process.env.AB_WEB_READY, `http://localhost:${address.port}`);
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => server.httpServer.close(() => process.exit(0)));
