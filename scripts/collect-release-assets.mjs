import { copyFile, mkdir, readdir, rm } from "node:fs/promises";
import { basename, join, resolve } from "node:path";

const [sourceArgument, destinationArgument] = process.argv.slice(2);
if (!sourceArgument || !destinationArgument) {
  console.error("Usage: node scripts/collect-release-assets.mjs <bundle-directory> <release-assets-directory>");
  process.exit(2);
}

const sourceRoot = resolve(sourceArgument);
const destinationRoot = resolve(destinationArgument);
const allowedExtensions = new Set([".appimage", ".deb", ".dmg", ".exe", ".msi", ".pkg", ".rpm"]);
const assets = [];

async function collect(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      await collect(path);
      continue;
    }
    if (!entry.isFile()) continue;
    const extension = entry.name.slice(entry.name.lastIndexOf(".")).toLowerCase();
    if (allowedExtensions.has(extension)) assets.push(path);
  }
}

await rm(destinationRoot, { recursive: true, force: true });
await mkdir(destinationRoot, { recursive: true });
await collect(sourceRoot);
if (assets.length === 0) {
  throw new Error(`No allowed installer assets found under ${sourceRoot}`);
}

const names = new Set();
for (const source of assets) {
  const name = basename(source);
  if (!names.add(name)) throw new Error(`Duplicate installer asset name: ${name}`);
  await copyFile(source, join(destinationRoot, name));
}

console.log(JSON.stringify({ sourceRoot, destinationRoot, assets: [...names].sort() }, null, 2));
