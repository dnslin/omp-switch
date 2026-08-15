import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";

const [catalogPath, destinationPath] = process.argv.slice(2);

if (!catalogPath || !destinationPath) {
  console.error("Usage: node scripts/generate-bundled-manifest.mjs <pi-catalog-package-dir> <manifest-dir>");
  process.exit(2);
}

const packageJson = JSON.parse(await readFile(join(catalogPath, "package.json"), "utf8"));
if (packageJson.name !== "@oh-my-pi/pi-catalog") {
  throw new TypeError("The manifest source must be the official @oh-my-pi/pi-catalog package.");
}
if (typeof packageJson.version !== "string" || !/^[0-9A-Za-z][0-9A-Za-z._-]*$/.test(packageJson.version)) {
  throw new TypeError("The official pi-catalog package must declare a safe exact version.");
}

const source = JSON.parse(await readFile(join(catalogPath, "src", "models.json"), "utf8"));
if (!source || typeof source !== "object" || Array.isArray(source)) {
  throw new TypeError("pi-catalog models.json must contain a provider object.");
}

function compareIdentifiers(left, right) {
  return left < right ? -1 : left > right ? 1 : 0;
}

const providers = Object.fromEntries(
  Object.entries(source)
    .sort(([left], [right]) => compareIdentifiers(left, right))
    .map(([provider, models]) => {
      if (!models || typeof models !== "object" || Array.isArray(models)) {
        throw new TypeError(`Provider ${provider} must contain a model object.`);
      }
      return [provider, Object.keys(models).sort(compareIdentifiers)];
    }),
);

await mkdir(destinationPath, { recursive: true });
const destination = join(destinationPath, `${packageJson.version}.json`);
await writeFile(destination, `${JSON.stringify({ version: packageJson.version, providers }, null, 2)}\n`);

