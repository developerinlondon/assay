/// <reference path="../types.d.ts" />

import { constants } from "node:fs";
import { access } from "node:fs/promises";
import path from "node:path";
import { spawn } from "node:child_process";
import { Type } from "@sinclair/typebox";
import type { OpenClawPluginApi } from "openclaw/plugin-sdk/lobster";

type AssayPluginConfig = {
  binaryPath?: unknown;
  timeout?: unknown;
  maxOutputSize?: unknown;
  scriptsDir?: unknown;
};

type ToolEnvelope =
  | {
      ok: true;
      status: "ok" | "needs_approval";
      output: unknown;
      requiresApproval: null | {
        prompt: string;
        context?: unknown;
        resumeToken: string;
      };
      truncated?: boolean;
    }
  | {
      ok: false;
      status: "error" | "timeout";
      error: string;
    };

function parseToolOutput(stdout: string): ToolEnvelope {
  const trimmed = stdout.trim();
  try {
    return JSON.parse(trimmed) as ToolEnvelope;
  } catch {
    const match = trimmed.match(/(\{[\s\S]*\}|\[[\s\S]*\])\s*$/);
    if (match) {
      return JSON.parse(match[1]) as ToolEnvelope;
    }
    throw new Error("No valid JSON found in assay output");
  }
}

function normalizeSandboxPath(input: string): string {
  const normalized = path.normalize(input);
  return process.platform === "win32" ? normalized.toLowerCase() : normalized;
}

function ensureRelativeScriptPath(scriptRaw: unknown): string {
  if (typeof scriptRaw !== "string" || !scriptRaw.trim()) {
    throw new Error("script required");
  }
  const script = scriptRaw.trim();
  if (path.isAbsolute(script)) {
    throw new Error("script must be a relative path within scriptsDir");
  }
  return script;
}

function resolveScriptsDir(config: AssayPluginConfig): string {
  const raw = typeof config.scriptsDir === "string" && config.scriptsDir.trim()
    ? config.scriptsDir.trim()
    : process.cwd();
  return path.isAbsolute(raw) ? raw : path.resolve(process.cwd(), raw);
}

function resolveScriptPath(scriptsDir: string, scriptRaw: unknown): string {
  const script = ensureRelativeScriptPath(scriptRaw);
  const resolved = path.resolve(scriptsDir, script);
  const rel = path.relative(
    normalizeSandboxPath(scriptsDir),
    normalizeSandboxPath(resolved),
  );
  if (rel === "" || rel === ".") {
    return resolved;
  }
  if (rel.startsWith("..") || path.isAbsolute(rel)) {
    throw new Error("script must stay within scriptsDir");
  }
  return resolved;
}

async function isExecutable(filePath: string): Promise<boolean> {
  try {
    await access(filePath, constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

async function resolveBinaryPath(config: AssayPluginConfig): Promise<string> {
  const configured =
    typeof config.binaryPath === "string" && config.binaryPath.trim() ? config.binaryPath.trim() : "";
  if (configured) {
    return configured;
  }

  const fromEnv = typeof process.env.ASSAY_BINARY === "string" ? process.env.ASSAY_BINARY.trim() : "";
  if (fromEnv) {
    return fromEnv;
  }

  const pathValue = process.env.PATH ?? "";
  const pathEntries = pathValue.split(path.delimiter).filter(Boolean);
  const candidates = process.platform === "win32"
    ? ["assay.exe", "assay.cmd", "assay.bat", "assay"]
    : ["assay"];

  for (const entry of pathEntries) {
    for (const candidate of candidates) {
      const fullPath = path.join(entry, candidate);
      if (await isExecutable(fullPath)) {
        return fullPath;
      }
    }
  }

  throw new Error("assay binary not found; set plugins.entries.assay.config.binaryPath or ASSAY_BINARY");
}

function resolveTimeoutSeconds(config: AssayPluginConfig): number {
  const timeout = typeof config.timeout === "number" && Number.isFinite(config.timeout) ? config.timeout : 20;
  return Math.max(1, timeout);
}

function resolveMaxOutputSize(config: AssayPluginConfig): number {
  const size =
    typeof config.maxOutputSize === "number" && Number.isFinite(config.maxOutputSize)
      ? config.maxOutputSize
      : 524_288;
  return Math.max(1_024, Math.floor(size));
}

function resolveArgsEnv(argsRaw: unknown): Record<string, string> {
  if (argsRaw === undefined) {
    return {};
  }
  if (!argsRaw || typeof argsRaw !== "object" || Array.isArray(argsRaw)) {
    throw new Error("args must be an object of string values");
  }

  const env: Record<string, string> = {};
  for (const [key, value] of Object.entries(argsRaw)) {
    if (typeof value !== "string") {
      throw new Error(`args.${key} must be a string`);
    }
    env[key] = value;
  }
  return env;
}

async function runAssaySubprocess(params: {
  execPath: string;
  argv: string[];
  cwd: string;
  timeoutSeconds: number;
  maxOutputSize: number;
  env?: Record<string, string>;
}): Promise<{ stdout: string }> {
  const timeoutMs = Math.max(200, Math.floor(params.timeoutSeconds * 1000) + 1000);
  const maxOutputSize = Math.max(1_024, params.maxOutputSize);
  const env = {
    ...process.env,
    ...(params.env ?? {}),
  } as Record<string, string | undefined>;

  return await new Promise<{ stdout: string }>((resolve, reject) => {
    const child = spawn(params.execPath, params.argv, {
      cwd: params.cwd,
      stdio: ["ignore", "pipe", "pipe"],
      env,
      windowsHide: true,
    });

    let stdout = "";
    let stdoutBytes = 0;
    let stderr = "";
    let settled = false;

    const settle = (result: { ok: true; value: { stdout: string } } | { ok: false; error: Error }) => {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timer);
      if (result.ok) {
        resolve(result.value);
      } else {
        reject(result.error);
      }
    };

    const failAndTerminate = (message: string) => {
      try {
        child.kill("SIGKILL");
      } finally {
        settle({ ok: false, error: new Error(message) });
      }
    };

    child.stdout?.setEncoding("utf8");
    child.stderr?.setEncoding("utf8");

    child.stdout?.on("data", (chunk: string) => {
      const text = String(chunk);
      stdoutBytes += Buffer.byteLength(text, "utf8");
      if (stdoutBytes > maxOutputSize) {
        failAndTerminate("assay output exceeded maxOutputSize");
        return;
      }
      stdout += text;
    });

    child.stderr?.on("data", (chunk: string) => {
      stderr += String(chunk);
    });

    const timer = setTimeout(() => {
      failAndTerminate(`assay subprocess timed out after ${params.timeoutSeconds}s`);
    }, timeoutMs);

    child.once("error", (error: Error) => {
      settle({ ok: false, error });
    });

    child.once("exit", (code: number | null) => {
      if (code !== 0) {
        settle({
          ok: false,
          error: new Error(`assay failed (${code ?? "?"}): ${stderr.trim() || stdout.trim()}`),
        });
        return;
      }
      settle({ ok: true, value: { stdout } });
    });
  });
}

function toToolResult(envelope: Extract<ToolEnvelope, { ok: true }>) {
  return {
    content: [{ type: "text", text: JSON.stringify(envelope, null, 2) }],
    details: envelope,
  };
}

export function createAssayTool(api: OpenClawPluginApi) {
  return {
    name: "assay",
    label: "Assay Workflow Runtime",
    description:
      "Run Assay Lua workflow scripts for infrastructure automation, DevOps checks, data processing, and resumable approval flows.",
    parameters: Type.Object({
      action: Type.Unsafe<"run" | "resume">({ type: "string", enum: ["run", "resume"] }),
      script: Type.Optional(Type.String({ description: "Relative Lua script path inside scriptsDir." })),
      args: Type.Optional(Type.Record(Type.String(), Type.String())),
      token: Type.Optional(Type.String({ description: "Resume token from a prior approval request." })),
      approve: Type.Optional(Type.Unsafe<"yes" | "no">({ type: "string", enum: ["yes", "no"] })),
    }),
    async execute(_id: string, params: Record<string, unknown>) {
      const config = (api.pluginConfig ?? {}) as AssayPluginConfig;
      const action = typeof params.action === "string" ? params.action.trim() : "";
      if (!action) {
        throw new Error("action required");
      }

      const execPath = await resolveBinaryPath(config);
      const timeoutSeconds = resolveTimeoutSeconds(config);
      const maxOutputSize = resolveMaxOutputSize(config);
      const scriptsDir = resolveScriptsDir(config);

      let argv: string[];
      let cwd: string;
      let env: Record<string, string> | undefined;

      if (action === "run") {
        const scriptPath = resolveScriptPath(scriptsDir, params.script);
        argv = ["run", "--mode", "tool", "--timeout", String(timeoutSeconds), scriptPath];
        cwd = scriptsDir;
        env = resolveArgsEnv(params.args);
      } else if (action === "resume") {
        const token = typeof params.token === "string" ? params.token.trim() : "";
        const approve = typeof params.approve === "string" ? params.approve.trim() : "";
        if (!token) {
          throw new Error("token required");
        }
        if (approve !== "yes" && approve !== "no") {
          throw new Error("approve must be 'yes' or 'no'");
        }
        argv = ["resume", "--token", token, "--approve", approve];
        cwd = scriptsDir;
      } else {
        throw new Error(`Unknown action: ${action}`);
      }

      if (api.runtime?.version && api.logger?.debug) {
        api.logger.debug(`assay plugin runtime=${api.runtime.version}`);
      }

      const { stdout } = await runAssaySubprocess({
        execPath,
        argv,
        cwd,
        timeoutSeconds,
        maxOutputSize,
        env,
      });

      const envelope = parseToolOutput(stdout);
      if (!envelope.ok) {
        throw new Error(envelope.error);
      }
      if (envelope.status === "needs_approval") {
        return toToolResult(envelope);
      }
      if (envelope.status === "ok") {
        return toToolResult(envelope);
      }
      throw new Error("Unsupported assay tool status");
    },
  };
}
