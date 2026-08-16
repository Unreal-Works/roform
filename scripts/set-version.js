import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

const version = Bun.argv[2];
const semverPattern =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;

if (!version || !semverPattern.test(version)) {
  console.error("Usage: bun run set-version <version>");
  console.error("Example: bun run set-version 0.5.0");
  process.exit(1);
}

const repositoryRoot = dirname(import.meta.dir);

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function writeJson(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

const cargoTomlPath = join(repositoryRoot, "Cargo.toml");
const cargoToml = readFileSync(cargoTomlPath, "utf8");
const cargoTomlPattern =
  /(\[package\]\r?\nname = "roform"\r?\nversion = ")[^"]+(")/;

if (!cargoTomlPattern.test(cargoToml)) {
  throw new Error("Could not find the package version in Cargo.toml");
}

const updatedCargoToml = cargoToml.replace(cargoTomlPattern, `$1${version}$2`);
writeFileSync(cargoTomlPath, updatedCargoToml);

const cargoLockPath = join(repositoryRoot, "Cargo.lock");
const cargoLock = readFileSync(cargoLockPath, "utf8");
const cargoLockPattern =
  /(\[\[package\]\]\r?\nname = "roform"\r?\nversion = ")[^"]+(")/;

if (!cargoLockPattern.test(cargoLock)) {
  throw new Error('Could not find the local "roform" package in Cargo.lock');
}

const updatedCargoLock = cargoLock.replace(cargoLockPattern, `$1${version}$2`);
writeFileSync(cargoLockPath, updatedCargoLock);

const rootPackagePath = join(repositoryRoot, "package.json");
const rootPackage = readJson(rootPackagePath);
rootPackage.version = version;

for (const [name, dependencyVersion] of Object.entries(
  rootPackage.optionalDependencies ?? {},
)) {
  if (
    name.startsWith("@unrealworks/roform-") &&
    dependencyVersion !== version
  ) {
    rootPackage.optionalDependencies[name] = version;
  }
}

writeJson(rootPackagePath, rootPackage);

const platformPackageDirectories = readdirSync(join(repositoryRoot, "npm"), {
  withFileTypes: true,
}).filter((entry) => entry.isDirectory() && entry.name.startsWith("roform-"));

for (const directory of platformPackageDirectories) {
  const packagePath = join(
    repositoryRoot,
    "npm",
    directory.name,
    "package.json",
  );
  const packageJson = readJson(packagePath);
  packageJson.version = version;
  writeJson(packagePath, packageJson);
}

console.log(`Updated project version to ${version}.`);
