import { readFileSync } from "node:fs";

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function readCargoVersion(path) {
  const raw = readFileSync(path, "utf8");
  const match = raw.match(/^version\s*=\s*"([^"]+)"/m);
  if (!match) {
    throw new Error(`Could not find a version entry in ${path}.`);
  }

  return match[1];
}

const packageVersion = readJson("package.json").version;
const tauriVersion = readJson("src-tauri/tauri.conf.json").version;
const cargoVersion = readCargoVersion("src-tauri/Cargo.toml");
const versions = [
  ["package.json", packageVersion],
  ["src-tauri/tauri.conf.json", tauriVersion],
  ["src-tauri/Cargo.toml", cargoVersion],
];

const distinctVersions = new Set(versions.map(([, version]) => version));
if (distinctVersions.size !== 1) {
  const detail = versions.map(([name, version]) => `${name}=${version}`).join(", ");
  throw new Error(`Version metadata is out of sync: ${detail}`);
}

const tag = process.env.GITHUB_REF_NAME?.trim() || process.env.RELEASE_TAG?.trim() || "";
if (tag) {
  const normalizedTag = tag.startsWith("v") ? tag.slice(1) : tag;
  if (normalizedTag !== packageVersion) {
    throw new Error(`Git tag ${tag} does not match project version ${packageVersion}.`);
  }
}

console.log(`Version metadata is aligned at ${packageVersion}.`);