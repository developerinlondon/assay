# Assay OpenClaw Extension

Adds the `assay` agent tool to OpenClaw so agents can run checked-in Assay Lua workflows with resumable approvals.

## Installation

Install from npm:

```bash
openclaw plugins install @assay/openclaw-extension
```

Install from a local checkout:

```bash
openclaw plugins install -l ./openclaw-extension
```

## What this plugin does

- Runs `assay run --mode tool <script>` for deterministic workflow execution
- Supports approval pauses via `assay resume --token <token> --approve yes|no`
- Exposes Assay as an optional OpenClaw tool so you can allowlist it per agent
- Keeps execution local to the configured script directory

## Configuration

Set plugin config under `plugins.entries.assay.config`.

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `binaryPath` | string | PATH lookup | Explicit path to the `assay` binary |
| `timeout` | number | `20` | Execution timeout in seconds |
| `maxOutputSize` | number | `524288` | Maximum stdout collected from Assay |
| `scriptsDir` | string | workspace root | Root directory for Lua scripts |

Example:

```json
{
  "plugins": {
    "entries": {
      "assay": {
        "enabled": true,
        "config": {
          "binaryPath": "/usr/local/bin/assay",
          "timeout": 30,
          "maxOutputSize": 524288,
          "scriptsDir": "./assay-workflows"
        }
      }
    }
  }
}
```

## Usage

Run a workflow:

```json
{
  "action": "run",
  "script": "deploy/check-rollout.lua",
  "args": {
    "APP": "payments",
    "NAMESPACE": "prod"
  }
}
```

Resume after approval:

```json
{
  "action": "resume",
  "token": "<resumeToken>",
  "approve": "yes"
}
```

## Security

- Script paths must be relative and stay inside `scriptsDir`
- Absolute paths and `..` traversal are rejected
- The subprocess is killed if stdout exceeds `maxOutputSize`
- The subprocess is killed if it exceeds the configured timeout
- The tool is not registered inside OpenClaw sandboxed contexts

## Requirements

- The `assay` binary must already be installed
- Binary resolution order is plugin `binaryPath`, then `ASSAY_BINARY`, then `PATH`
- OpenClaw loads the TypeScript entrypoint directly with `jiti`; no separate build step is required

## Recommended agent policy

Because Assay scripts can perform side effects, enable this tool only for agents that should run infrastructure or workflow automation.
