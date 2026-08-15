import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";

const generator = resolve("scripts/generate-bundled-manifest.mjs");

async function fixtureCatalog(packageJson, models) {
  const root = await mkdtemp(join(tmpdir(), "omp-switch-catalog-"));
  const catalog = join(root, "pi-catalog");
  const destination = join(root, "bundled-manifests");
  await mkdir(join(catalog, "src"), { recursive: true });
  await writeFile(join(catalog, "package.json"), `${JSON.stringify(packageJson)}\n`);
  await writeFile(join(catalog, "src", "models.json"), `${JSON.stringify(models)}\n`);
  return { root, catalog, destination };
}

function generate(catalog, destination) {
  return spawnSync(process.execPath, [generator, catalog, destination], {
    encoding: "utf8",
  });
}

test("generates an exact-version bundled manifest from the official pi-catalog package", async (t) => {
  const fixture = await fixtureCatalog(
    { name: "@oh-my-pi/pi-catalog", version: "17.2.15" },
    {
      OpenAI: { "GPT-5.6-SOL": { name: "Ignored metadata" }, "gpt-5.6-mini": {} },
      custom: { zeta: {} },
    },
  );
  t.after(() => rm(fixture.root, { recursive: true, force: true }));

  const result = generate(fixture.catalog, fixture.destination);

  assert.equal(result.status, 0, result.stderr);
  const manifest = JSON.parse(await readFile(join(fixture.destination, "17.2.15.json"), "utf8"));
  assert.deepEqual(manifest, {
    version: "17.2.15",
    providers: {
      OpenAI: ["GPT-5.6-SOL", "gpt-5.6-mini"],
      custom: ["zeta"],
    },
  });
});

test("rejects a catalog package that cannot prove its official source and version", async (t) => {
  const fixture = await fixtureCatalog(
    { name: "not-pi-catalog", version: "17.2.15" },
    { custom: { model: {} } },
  );
  t.after(() => rm(fixture.root, { recursive: true, force: true }));

  const result = generate(fixture.catalog, fixture.destination);

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /@oh-my-pi\/pi-catalog/);
});

test("rejects a catalog package without the canonical src/models.json source", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "omp-switch-catalog-"));
  const catalog = join(root, "pi-catalog");
  const destination = join(root, "bundled-manifests");
  await mkdir(catalog);
  await writeFile(join(catalog, "package.json"), `${JSON.stringify({ name: "@oh-my-pi/pi-catalog", version: "17.2.15" })}\n`);
  await writeFile(join(catalog, "models.json"), `${JSON.stringify({ custom: { model: {} } })}\n`);
  t.after(() => rm(root, { recursive: true, force: true }));

  const result = generate(catalog, destination);

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /src\/models\.json/);
});
