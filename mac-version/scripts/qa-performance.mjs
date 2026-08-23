#!/usr/bin/env node

import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const root = join(import.meta.dirname, "..");
const metricsPath = process.env.FPV_METRICS_PATH
  ?? join(process.env.HOME, "Library/Application Support/com.lumizone.fpvdesktop/generation-metrics.jsonl");
const limit = Number(process.env.FPV_PERF_SAMPLES ?? 20);

function percentile(values, fraction) {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * fraction))];
}

function metric(values, fraction, suffix) {
  const value = percentile(values, fraction);
  return value === null ? "n/a" : `${value.toFixed(1)}${suffix}`;
}

const entries = existsSync(metricsPath)
  ? readFileSync(metricsPath, "utf8").split("\n").filter(Boolean).flatMap((line) => {
      try { return [JSON.parse(line)]; } catch { return []; }
    })
  : [];
const samples = entries.filter((entry) => entry.stage === "narration").slice(-limit);
const durations = samples.map((entry) => Number(entry.duration_ms)).filter(Number.isFinite);
const firstTokens = samples.map((entry) => Number(entry.first_token_ms)).filter(Number.isFinite);
const throughput = samples.map((entry) => Number(entry.tokens_per_second)).filter(Number.isFinite);
const report = [
  "# FPV Performance Report",
  "",
  `- Samples: ${samples.length}`,
  `- Metrics source: ${metricsPath}`,
  `- Total duration p50/p95: ${metric(durations, 0.5, " ms")} / ${metric(durations, 0.95, " ms")}`,
  `- First token p50/p95: ${metric(firstTokens, 0.5, " ms")} / ${metric(firstTokens, 0.95, " ms")}`,
  `- Throughput p50/p05: ${metric(throughput, 0.5, " tok/s")} / ${metric(throughput, 0.05, " tok/s")}`,
  "",
  samples.length === 0
    ? "No narration metrics yet. Generate several turns in FPV and run this command again."
    : "Use the same model, profile, and story when comparing reports.",
].join("\n");

const outputDir = join(root, "qa-reports");
mkdirSync(outputDir, { recursive: true });
const reportPath = join(outputDir, `performance-${new Date().toISOString().replace(/[:.]/g, "-")}.md`);
writeFileSync(reportPath, `${report}\n`, "utf8");
console.log(report);
console.log(`REPORT=${reportPath}`);
