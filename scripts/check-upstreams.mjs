#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

const manifest = JSON.parse(
  readFileSync(new URL("./upstreams.json", import.meta.url), "utf8"),
);
const emitJson = process.argv.includes("--json");
const results = [];
let failed = false;

function github(path, { optional = false, jq = null } = {}) {
  const args = ["api", path];
  if (jq) args.push("--jq", jq);

  try {
    return JSON.parse(
      execFileSync("gh", args, {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
        maxBuffer: 16 * 1024 * 1024,
      }),
    );
  } catch (error) {
    const stderr = error.stderr?.toString() ?? error.message;
    if (optional && /HTTP 404|Not Found/i.test(stderr)) {
      return null;
    }
    throw new Error(`${path}: ${stderr.trim()}`);
  }
}

for (const entry of manifest.repositories) {
  try {
    const repository = github(`repos/${entry.repo}`);
    const branch = entry.branch ?? repository.default_branch;
    const head = github(`repos/${entry.repo}/commits/${branch}`);
    const release = github(`repos/${entry.repo}/releases/latest`, {
      optional: true,
    });
    const row = {
      name: entry.name,
      repo: entry.repo,
      branch,
      head: head.sha,
      headDate: head.commit.committer?.date ?? head.commit.author?.date ?? null,
      latestRelease: release?.tag_name ?? null,
      releaseDate: release?.published_at ?? null,
      sourcePolicy: entry.sourcePolicy,
      baseline: entry.baseline ?? null,
      status: "unbaselined",
      delta: null,
      url: `https://github.com/${entry.repo}`,
    };

    if (entry.baseline?.kind === "commit") {
      const comparison = github(
        `repos/${entry.repo}/compare/${entry.baseline.value}...${head.sha}`,
        { jq: "{status,ahead_by,behind_by,total_commits,html_url}" },
      );
      row.status = comparison.status;
      row.delta = {
        ahead: comparison.ahead_by,
        behind: comparison.behind_by,
        commits: comparison.total_commits,
      };
      row.url = comparison.html_url;
    } else if (entry.baseline?.kind === "release") {
      row.status = release?.tag_name === entry.baseline.value ? "current" : "changed";
      row.delta = {
        reviewedRelease: entry.baseline.value,
        latestRelease: release?.tag_name ?? null,
      };
      row.url = release?.html_url ?? row.url;
    }

    results.push(row);
  } catch (error) {
    failed = true;
    results.push({
      name: entry.name,
      repo: entry.repo,
      sourcePolicy: entry.sourcePolicy,
      status: "error",
      error: error.message,
      url: `https://github.com/${entry.repo}`,
    });
  }
}

if (emitJson) {
  console.log(JSON.stringify({ checkedAt: new Date().toISOString(), results }, null, 2));
} else {
  const headings = ["Project", "Head", "Release", "Delta", "Policy"];
  const rows = results.map((row) => {
    let delta = row.status;
    if (row.delta?.commits != null) {
      delta = `${row.delta.commits} commits`;
    } else if (row.delta?.reviewedRelease) {
      delta = `${row.delta.reviewedRelease} -> ${row.delta.latestRelease ?? "none"}`;
    }
    return [
      row.name,
      row.head?.slice(0, 8) ?? "-",
      row.latestRelease ?? "-",
      delta,
      row.sourcePolicy,
    ];
  });
  const widths = headings.map((heading, index) =>
    Math.max(heading.length, ...rows.map((row) => row[index].length)),
  );
  const format = (row) =>
    row.map((value, index) => value.padEnd(widths[index])).join("  ");

  console.log(format(headings));
  console.log(format(widths.map((width) => "-".repeat(width))));
  for (const row of rows) console.log(format(row));
  console.log("\nChanged/error details:");
  for (const row of results.filter((item) =>
    ["ahead", "diverged", "changed", "error", "unbaselined"].includes(item.status),
  )) {
    console.log(`- ${row.name}: ${row.url}${row.error ? ` (${row.error})` : ""}`);
  }
}

if (failed) process.exitCode = 1;
