import { EventEmitter } from "node:events";
import path from "node:path";
import { PassThrough } from "node:stream";
import type { OpenClawPluginApi, OpenClawPluginToolContext } from "openclaw/plugin-sdk/lobster";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const spawnState = vi.hoisted(() => ({
  queue: [] as Array<{ stdout: string; stderr?: string; exitCode?: number }>,
  spawn: vi.fn(),
}));

vi.mock("node:child_process", async (importOriginal) => {
  const actual = await importOriginal<typeof import("node:child_process")>();
  return {
    ...actual,
    spawn: (...args: unknown[]) => spawnState.spawn(...args),
  };
});

let createAssayTool: typeof import("./assay-tool.js").createAssayTool;

function fakeApi(overrides: Partial<OpenClawPluginApi> = {}): OpenClawPluginApi {
  return {
    id: "assay",
    name: "assay",
    source: "test",
    registrationMode: "full",
    config: {},
    pluginConfig: {},
    runtime: { version: "test" },
    logger: { info() {}, warn() {}, error() {}, debug() {} },
    registerTool() {},
    registerChannel() {},
    registerGatewayMethod() {},
    registerCli() {},
    registerService() {},
    registerProvider() {},
    registerWebSearchProvider() {},
    registerInteractiveHandler() {},
    registerHook() {},
    registerHttpRoute() {},
    registerCommand() {},
    registerContextEngine() {},
    on() {},
    resolvePath: (value: string) => value,
    ...overrides,
  } as OpenClawPluginApi;
}

function fakeCtx(overrides: Partial<OpenClawPluginToolContext> = {}): OpenClawPluginToolContext {
  return {
    config: {},
    workspaceDir: "/tmp",
    agentDir: "/tmp",
    agentId: "main",
    sessionKey: "main",
    messageChannel: undefined,
    agentAccountId: undefined,
    sandboxed: false,
    ...overrides,
  } as OpenClawPluginToolContext;
}

function queueEnvelope(envelope: unknown) {
  spawnState.queue.push({ stdout: JSON.stringify(envelope) });
}

describe("assay plugin tool", () => {
  const originalAssayBinary = process.env.ASSAY_BINARY;

  afterEach(() => {
    if (originalAssayBinary === undefined) {
      delete process.env.ASSAY_BINARY;
      return;
    }
    process.env.ASSAY_BINARY = originalAssayBinary;
  });

  beforeEach(async () => {
    if (!createAssayTool) {
      ({ createAssayTool } = await import("./assay-tool.js"));
    }

    process.env.ASSAY_BINARY = "/fake/bin/assay";
    spawnState.queue.length = 0;
    spawnState.spawn.mockReset();
    spawnState.spawn.mockImplementation(() => {
      const next = spawnState.queue.shift() ?? { stdout: "" };
      const stdout = new PassThrough();
      const stderr = new PassThrough();
      const child = new EventEmitter() as EventEmitter & {
        stdout: PassThrough;
        stderr: PassThrough;
        kill: (signal?: string) => boolean;
      };
      child.stdout = stdout;
      child.stderr = stderr;
      child.kill = () => true;

      setImmediate(() => {
        if (next.stderr) {
          stderr.end(next.stderr);
        } else {
          stderr.end();
        }
        stdout.end(next.stdout);
        child.emit("exit", next.exitCode ?? 0);
      });

      return child;
    });
  });

  it("runs assay and returns parsed envelope", async () => {
    queueEnvelope({
      ok: true,
      status: "ok",
      output: [{ hello: "world" }],
      requiresApproval: null,
    });

    const tool = createAssayTool(fakeApi());
    const result = await tool.execute("call-run", {
      action: "run",
      script: "scripts/noop.lua",
      args: { NAME: "world" },
    });

    expect(spawnState.spawn).toHaveBeenCalledWith(
      "/fake/bin/assay",
      [
        "run",
        "--mode",
        "tool",
        "--timeout",
        "20",
        path.resolve(process.cwd(), "scripts/noop.lua"),
      ],
      expect.objectContaining({
        cwd: process.cwd(),
        windowsHide: true,
        env: expect.objectContaining({ NAME: "world" }),
      }),
    );
    expect(result.details).toMatchObject({ ok: true, status: "ok", output: [{ hello: "world" }] });
  });

  it("tolerates noisy stdout before JSON envelope", async () => {
    const payload = { ok: true, status: "ok", output: [], requiresApproval: null };
    spawnState.queue.push({ stdout: `noise before json\n${JSON.stringify(payload)}` });

    const tool = createAssayTool(fakeApi());
    const result = await tool.execute("call-noisy", {
      action: "run",
      script: "scripts/noop.lua",
    });

    expect(result.details).toMatchObject({ ok: true, status: "ok" });
  });

  it("requires action", async () => {
    const tool = createAssayTool(fakeApi());
    await expect(tool.execute("call-action-missing", {})).rejects.toThrow(/action required/);
  });

  it("requires script for run action", async () => {
    const tool = createAssayTool(fakeApi());
    await expect(
      tool.execute("call-script-missing", {
        action: "run",
      }),
    ).rejects.toThrow(/script required/);
  });

  it("requires token for resume action", async () => {
    const tool = createAssayTool(fakeApi());
    await expect(
      tool.execute("call-token-missing", {
        action: "resume",
        approve: "yes",
      }),
    ).rejects.toThrow(/token required/);
  });

  it("requires approve for resume action", async () => {
    const tool = createAssayTool(fakeApi());
    await expect(
      tool.execute("call-approve-missing", {
        action: "resume",
        token: "resume-token",
      }),
    ).rejects.toThrow(/approve must be 'yes' or 'no'/);
  });

  it("rejects unknown action", async () => {
    const tool = createAssayTool(fakeApi());
    await expect(
      tool.execute("call-action-unknown", {
        action: "explode",
      }),
    ).rejects.toThrow(/Unknown action/);
  });

  it("rejects absolute script path", async () => {
    const tool = createAssayTool(fakeApi());
    await expect(
      tool.execute("call-script-absolute", {
        action: "run",
        script: "/etc/passwd",
      }),
    ).rejects.toThrow(/relative path/);
  });

  it("rejects path traversal", async () => {
    const tool = createAssayTool(fakeApi());
    await expect(
      tool.execute("call-script-traversal", {
        action: "run",
        script: "../../etc/passwd",
      }),
    ).rejects.toThrow(/must stay within/);
  });

  it("rejects invalid JSON from assay", async () => {
    spawnState.queue.push({ stdout: "nope" });

    const tool = createAssayTool(fakeApi());
    await expect(
      tool.execute("call-invalid-json", {
        action: "run",
        script: "scripts/noop.lua",
      }),
    ).rejects.toThrow(/valid JSON/);
  });

  it("handles needs_approval envelope", async () => {
    queueEnvelope({
      ok: true,
      status: "needs_approval",
      output: { phase: "awaiting_approval" },
      requiresApproval: {
        prompt: "approve deployment?",
        context: { environment: "prod" },
        resumeToken: "resume-123",
      },
    });

    const tool = createAssayTool(fakeApi());
    const result = await tool.execute("call-needs-approval", {
      action: "run",
      script: "scripts/deploy.lua",
    });

    expect(result.details).toMatchObject({
      ok: true,
      status: "needs_approval",
      requiresApproval: { resumeToken: "resume-123" },
    });
  });

  it("handles error envelope by throwing", async () => {
    queueEnvelope({
      ok: false,
      status: "error",
      error: "deployment failed",
    });

    const tool = createAssayTool(fakeApi());
    await expect(
      tool.execute("call-error-envelope", {
        action: "run",
        script: "scripts/fail.lua",
      }),
    ).rejects.toThrow(/deployment failed/);
  });

  it("can be gated off in sandboxed contexts", async () => {
    const api = fakeApi();
    const factoryTool = (ctx: OpenClawPluginToolContext) => {
      if (ctx.sandboxed) {
        return null;
      }
      return createAssayTool(api);
    };

    expect(factoryTool(fakeCtx({ sandboxed: true }))).toBeNull();
    expect(factoryTool(fakeCtx({ sandboxed: false }))?.name).toBe("assay");
  });
});
