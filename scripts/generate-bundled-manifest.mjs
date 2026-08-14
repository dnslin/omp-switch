import { readFile, writeFile } from "node:fs/promises";

const [sourcePath, destinationPath, version] = process.argv.slice(2);

if (!sourcePath || !destinationPath || !version) {
  console.error("Usage: bun scripts/generate-bundled-manifest.mjs <models.json> <manifest.json> <omp-version>");
  process.exit(2);
}

const source = JSON.parse(await readFile(sourcePath, "utf8"));
if (!source || typeof source !== "object" || Array.isArray(source)) {
  throw new TypeError("models.json must contain a provider object");
}
const providers = Object.fromEntries(
  Object.entries(source)
    .sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0))
    .map(([provider, models]) => {
      if (!models || typeof models !== "object" || Array.isArray(models)) {
        throw new TypeError(`Provider ${provider} must contain a model object`);
      }
      return [provider, Object.keys(models).sort((left, right) => (left < right ? -1 : left > right ? 1 : 0))];
    }),
);

await writeFile(destinationPath, `${JSON.stringify({ version, providers }, null, 2)}\n`);
