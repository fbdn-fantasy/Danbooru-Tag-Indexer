import { createReadStream, createWriteStream } from "node:fs";
import { access, rename, rm } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { pipeline } from "node:stream/promises";
import { createGunzip } from "node:zlib";

const seed = fileURLToPath(new URL("../src-tauri/resources/codex-v1.sqlite", import.meta.url));
try {
  await access(seed);
} catch {
  try {
    await access(`${seed}.gz`);
  } catch {
    throw new Error("The offline seed is not in this checkout. Copy codex-v1.sqlite.gz from a release source ZIP into src-tauri/resources, then retry.");
  }
  const partial = `${seed}.partial`;
  try {
    await pipeline(createReadStream(`${seed}.gz`), createGunzip(), createWriteStream(partial));
    await rename(partial, seed);
    console.log("Offline database prepared.");
  } finally {
    await rm(partial, { force: true });
  }
}
