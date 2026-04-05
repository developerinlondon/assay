declare module "node:fs" {
  export const constants: {
    X_OK: number;
  };
}

declare module "node:fs/promises" {
  export function access(path: string, mode?: number): Promise<void>;
}

declare module "node:path" {
  const path: {
    delimiter: string;
    normalize(input: string): string;
    isAbsolute(input: string): boolean;
    resolve(...parts: string[]): string;
    relative(from: string, to: string): string;
    join(...parts: string[]): string;
  };
  export default path;
}

declare module "node:child_process" {
  type Listener<T = unknown> = (value: T) => void;

  export function spawn(
    command: string,
    args?: string[],
    options?: {
      cwd?: string;
      stdio?: [string, string, string];
      env?: Record<string, string | undefined>;
      windowsHide?: boolean;
    },
  ): {
    stdout?: {
      setEncoding(encoding: string): void;
      on(event: "data", listener: Listener<string>): void;
    };
    stderr?: {
      setEncoding(encoding: string): void;
      on(event: "data", listener: Listener<string>): void;
    };
    kill(signal?: string): void;
    once(event: "error", listener: Listener<Error>): void;
    once(event: "exit", listener: Listener<number | null>): void;
  };
}

declare module "@sinclair/typebox" {
  export const Type: {
    Object(schema: Record<string, unknown>): unknown;
    Optional(schema: unknown): unknown;
    String(options?: Record<string, unknown>): unknown;
    Number(options?: Record<string, unknown>): unknown;
    Record(key: unknown, value: unknown): unknown;
    Unsafe<T>(schema: Record<string, unknown>): T;
  };
}

declare module "openclaw/plugin-sdk/lobster" {
  export type AnyAgentTool = unknown;

  export type OpenClawPluginToolContext = {
    sandboxed?: boolean;
  };

  export type OpenClawPluginToolFactory = (
    ctx: OpenClawPluginToolContext,
  ) => AnyAgentTool | AnyAgentTool[] | null | undefined;

  export type OpenClawPluginApi = {
    pluginConfig?: Record<string, unknown>;
    runtime?: { version?: string };
    logger?: { debug?: (message: string) => void };
    registerTool: (tool: AnyAgentTool | OpenClawPluginToolFactory, opts?: { optional?: boolean }) => void;
  };
}

declare const process: {
  cwd(): string;
  env: Record<string, string | undefined>;
  platform: string;
};

declare const Buffer: {
  byteLength(value: string, encoding?: string): number;
};
