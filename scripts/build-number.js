#!/usr/bin/env node
// Generates/increments build number in yyyymmdd-revision format.
// Stores state in build-number.json at project root. Cross-platform.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, "..");
const buildFile = path.join(projectRoot, "build-number.json");

const now = new Date();
const today =
  now.getFullYear().toString() +
  String(now.getMonth() + 1).padStart(2, "0") +
  String(now.getDate()).padStart(2, "0");

let revision = 1;
if (fs.existsSync(buildFile)) {
  try {
    const prev = JSON.parse(fs.readFileSync(buildFile, "utf8"));
    if (prev.date === today && Number.isInteger(prev.revision)) {
      revision = prev.revision + 1;
    }
  } catch {
    revision = 1;
  }
}

const data = {
  date: today,
  revision,
  buildNumber: `${today}-${revision}`,
};

fs.writeFileSync(buildFile, JSON.stringify(data, null, 2) + "\n");
process.stdout.write(`${today}-${revision}\n`);
