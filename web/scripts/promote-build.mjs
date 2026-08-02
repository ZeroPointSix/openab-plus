import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const projectDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const outputs = ['index.html', 'app.js', 'styles.css'];

await mkdir(projectDir, { recursive: true });
await Promise.all(
  outputs.map(async (file) => {
    const source = resolve(projectDir, 'dist', file);
    const destination = resolve(projectDir, file);
    const contents = await readFile(source, 'utf8');

    await writeFile(destination, contents.replace(/[ \t]+$/gm, ''), 'utf8');
  }),
);
