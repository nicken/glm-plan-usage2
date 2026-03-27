#!/usr/bin/env node
"use strict";

const https = require("https");
const http = require("http");

// Config
const API_TIMEOUT = 5000;
const CACHE_TTL_MS = 120_000;

// Plan limits (prompts * 15)
const OLD_PLAN_5H = { lite: 1800, pro: 9000, max: 36000 };
const NEW_PLAN_5H = { lite: 1200, pro: 6000, max: 24000 };
const NEW_PLAN_WEEKLY = { lite: 6000, pro: 30000, max: 120000 };

let cache = null;

function getEnv(name) {
  return process.env[name] || "";
}

function request(url, token) {
  return new Promise((resolve, reject) => {
    const mod = url.startsWith("https") ? https : http;
    const req = mod.get(url, {
      timeout: API_TIMEOUT,
      headers: {
        Authorization: `Bearer ${token}`,
        "Content-Type": "application/json",
      },
    }, (res) => {
      if (res.statusCode !== 200) {
        res.resume();
        return reject(new Error(`HTTP ${res.statusCode}`));
      }
      let data = "";
      res.on("data", (c) => (data += c));
      res.on("end", () => {
        try { resolve(JSON.parse(data)); }
        catch { reject(new Error("JSON parse error")); }
      });
    });
    req.on("error", reject);
    req.on("timeout", () => { req.destroy(); reject(new Error("timeout")); });
  });
}

function buildClient() {
  const token = getEnv("ANTHROPIC_AUTH_TOKEN");
  const baseUrl = getEnv("ANTHROPIC_BASE_URL") || "https://open.bigmodel.cn/api/anthropic";
  const apiUrl = baseUrl.replace(/\/api\/anthropic/, "/api").replace(/\/anthropic$/, "");

  return {
    token,
    apiUrl,
    async fetchQuota() {
      return request(`${this.apiUrl}/monitor/usage/quota/limit`, this.token);
    },
    async fetchModelUsage(startTime, endTime) {
      const s = encodeURIComponent(startTime);
      const e = encodeURIComponent(endTime);
      return request(`${this.apiUrl}/monitor/usage/model-usage?startTime=${s}&endTime=${e}`, this.token);
    },
  };
}

function fmtReset(ms) {
  if (!ms) return "--:--";
  const d = new Date(ms);
  return `${d.getHours()}:${String(d.getMinutes()).padStart(2, "0")}`;
}

function fmtTokens(n) {
  if (n < 0) return "N/A";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 10_000) return `${(n / 1_000).toFixed(1)}K`;
  return `${n}`;
}

async function fetchStats(client) {
  if (cache && Date.now() - cache.ts < CACHE_TTL_MS) return cache.data;

  const [quota, modelUsage] = await Promise.allSettled([
    client.fetchQuota(),
    (async () => {
      try {
        // try fetching model usage
        const now = new Date();
        const start = new Date(now.getTime() - 5 * 3600_000);
        const fmt = (d) => `${d.getFullYear()}-${String(d.getMonth()+1).padStart(2,"0")}-${String(d.getDate()).padStart(2,"0")} ${String(d.getHours()).padStart(2,"0")}:${String(d.getMinutes()).padStart(2,"0")}:${String(d.getSeconds()).padStart(2,"0")}`;
        return await client.fetchModelUsage(fmt(start), fmt(now));
      } catch { return null; }
    })(),
  ]);

  if (quota.status === "rejected") return null;

  const q = quota.value;
  if (!q || !q.success) return null;

  const level = (q.data?.level || "pro").toLowerCase();
  const isNewPlan = q.data?.limits?.some(l => l.unit === 6);

  // Token usage (5h) - first TOKENS_LIMIT with unit=3
  const tokenLimit = q.data?.limits?.find(l => l.type === "TOKENS_LIMIT" && l.unit === 3);
  // Weekly usage - TOKENS_LIMIT with unit=6
  const weeklyLimit = q.data?.limits?.find(l => l.type === "TOKENS_LIMIT" && l.unit === 6);
  // MCP usage - TIME_LIMIT
  const mcpLimit = q.data?.limits?.find(l => l.type === "TIME_LIMIT");

  // Model usage
  let callCount = null, tokensUsed = null;
  if (modelUsage.status === "fulfilled" && modelUsage.value?.data?.totalUsage) {
    callCount = modelUsage.value.data.totalUsage.totalModelCallCount ?? null;
    tokensUsed = modelUsage.value.data.totalUsage.totalTokensUsage ?? null;
  }

  const result = { level, isNewPlan, tokenLimit, weeklyLimit, mcpLimit, callCount, tokensUsed };
  cache = { data: result, ts: Date.now() };
  return result;
}

function color256(code) {
  return `\x1b[38;5;${code}m`;
}

function reset() {
  return "\x1b[0m";
}

function format(stats) {
  if (!stats) return "";
  const parts = [];

  if (stats.tokenLimit) {
    parts.push(`🪙 ${stats.tokenLimit.percentage}% (⏰ ${fmtReset(stats.tokenLimit.nextResetTime)})`);
  }

  if (stats.callCount != null) {
    const limits = stats.isNewPlan ? NEW_PLAN_5H : OLD_PLAN_5H;
    const lim = limits[stats.level] || 9000;
    parts.push(`📊 ${stats.callCount}/${lim}`);
  }

  if (stats.weeklyLimit) {
    const lim = NEW_PLAN_WEEKLY[stats.level] || 30000;
    const used = Math.round(stats.weeklyLimit.percentage * lim / 100);
    parts.push(`📅 ${used}/${lim}`);
  }

  if (stats.mcpLimit) {
    parts.push(`🌐 ${stats.mcpLimit.currentValue}/${stats.mcpLimit.usage}`);
  }

  if (stats.tokensUsed != null) {
    parts.push(`⚡ ${fmtTokens(stats.tokensUsed)}`);
  }

  if (parts.length === 0) return "";

  // Color based on max percentage
  const maxPct = Math.max(
    stats.tokenLimit?.percentage ?? 0,
    stats.mcpLimit?.percentage ?? 0,
    stats.weeklyLimit?.percentage ?? 0,
  );
  const colorCode = maxPct <= 79 ? 109 : maxPct <= 94 ? 226 : 196;

  return `${color256(colorCode)}\x1b[1m${parts.join(" · ")}${reset()}`;
}

// Main
async function main() {
  const debug = process.env.GLM_DEBUG === "1";
  const logFile = require("fs").createWriteStream(require("path").join(require("os").homedir(), ".claude", "glm-plan-usage", "debug.log"), { flags: "a" });
  const log = (msg) => {
    const ts = new Date().toISOString();
    const line = `[${ts}] ${msg}\n`;
    if (debug) process.stderr.write(`[glm] ${msg}\n`);
    logFile.write(line);
  };

  // Read stdin
  let inputText = "";
  try {
    inputText = await new Promise((resolve, reject) => {
      const chunks = [];
      process.stdin.resume();
      process.stdin.on("data", (c) => chunks.push(c));
      process.stdin.on("end", () => resolve(Buffer.concat(chunks).toString()));
      process.stdin.on("error", reject);
      setTimeout(() => resolve(""), 1000);
    });
  } catch (e) { log(`stdin error: ${e.message}`); return; }

  log(`stdin: ${inputText.substring(0, 200)}`);

  let input;
  try {
    input = JSON.parse(inputText);
  } catch { input = {}; }

  log(`model: ${input.model?.id}`);

  // Only show for GLM models
  if (input.model?.id) {
    const id = input.model.id.toLowerCase();
    if (!id.includes("glm") && !id.includes("chatglm")) {
      log("not glm model, skipping");
      return;
    }
  }

  const client = buildClient();
  log(`token: ${client.token ? "present" : "MISSING"}`);
  if (!client.token) return;

  const stats = await fetchStats(client);
  log(`stats: ${stats ? "ok" : "null"}`);
  const output = format(stats);
  log(`output: ${output ? output.length + " chars" : "empty"}`);
  if (output) process.stdout.write(output);
}

main().catch(() => {});
