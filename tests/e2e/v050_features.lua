-- v0.5.0 feature E2E tests
-- Verifies stdlib module loading, builtins availability, and basic functionality

-- === Stdlib module loading ===
log.info("Testing stdlib module loading...")

local grafana = require("assay.grafana")
assert.not_nil(grafana, "assay.grafana should be loadable")
assert.not_nil(grafana.client, "assay.grafana should have client function")

local vault = require("assay.vault")
assert.not_nil(vault, "assay.vault should be loadable")
assert.not_nil(vault.client, "assay.vault should have client function")

local prom = require("assay.prometheus")
assert.not_nil(prom, "assay.prometheus should be loadable")

local k8s = require("assay.k8s")
assert.not_nil(k8s, "assay.k8s should be loadable")

local argocd = require("assay.argocd")
assert.not_nil(argocd, "assay.argocd should be loadable")

log.info("Stdlib modules loaded successfully")

-- === Builtins availability ===
log.info("Testing builtins availability...")

assert.not_nil(http, "http builtin should be available")
assert.not_nil(json, "json builtin should be available")
assert.not_nil(yaml, "yaml builtin should be available")
assert.not_nil(toml, "toml builtin should be available")
assert.not_nil(fs, "fs builtin should be available")
assert.not_nil(crypto, "crypto builtin should be available")
assert.not_nil(base64, "base64 builtin should be available")
assert.not_nil(regex, "regex builtin should be available")
assert.not_nil(log, "log builtin should be available")
assert.not_nil(env, "env builtin should be available")
assert.not_nil(assert, "assert builtin should be available")

log.info("All builtins available")

-- === JSON roundtrip ===
log.info("Testing JSON roundtrip...")

local data = { name = "assay", version = "0.5.0" }
local encoded = json.encode(data)
local decoded = json.parse(encoded)
assert.eq(decoded.name, "assay", "json roundtrip should preserve name")
assert.eq(decoded.version, "0.5.0", "json roundtrip should preserve version")

log.info("JSON roundtrip OK")

-- === YAML roundtrip ===
log.info("Testing YAML roundtrip...")

local yaml_str = yaml.encode({ key = "value", num = 42 })
assert.not_nil(yaml_str, "yaml.encode should return a string")
local yaml_decoded = yaml.parse(yaml_str)
assert.eq(yaml_decoded.key, "value", "yaml roundtrip should preserve key")
assert.eq(yaml_decoded.num, 42, "yaml roundtrip should preserve num")

log.info("YAML roundtrip OK")

-- === TOML roundtrip ===
log.info("Testing TOML roundtrip...")

local toml_str = toml.encode({ title = "test", count = 7 })
assert.not_nil(toml_str, "toml.encode should return a string")
local toml_decoded = toml.parse(toml_str)
assert.eq(toml_decoded.title, "test", "toml roundtrip should preserve title")
assert.eq(toml_decoded.count, 7, "toml roundtrip should preserve count")

log.info("TOML roundtrip OK")

-- === Base64 roundtrip ===
log.info("Testing base64 roundtrip...")

local original = "assay v0.5.0 universal engine"
local b64 = base64.encode(original)
assert.not_nil(b64, "base64.encode should return a string")
local decoded_b64 = base64.decode(b64)
assert.eq(decoded_b64, original, "base64 roundtrip should work")

log.info("Base64 roundtrip OK")

-- === Regex ===
log.info("Testing regex...")

assert.eq(regex.match("assay v0.5.0", "v\\d+\\.\\d+\\.\\d+"), true, "regex match should work")
local found = regex.find("version 0.5.0 release", "(\\d+\\.\\d+\\.\\d+)")
assert.not_nil(found, "regex find should return a result")
local replaced = regex.replace("hello world", "world", "assay")
assert.eq(replaced, "hello assay", "regex replace should work")

log.info("Regex OK")

-- === Time and env ===
log.info("Testing time and env...")

local t = time()
assert.gt(t, 0, "time() should return positive timestamp")

-- env.get returns nil for missing vars, not an error
local missing = env.get("ASSAY_TEST_NONEXISTENT_VAR_XYZ")
assert.eq(missing, nil, "env.get for missing var should return nil")

log.info("Time and env OK")

-- === Crypto ===
log.info("Testing crypto...")

local hash = crypto.hash("hello", "sha256")
assert.not_nil(hash, "crypto.hash should return a hash")
assert.gt(#hash, 0, "crypto hash should not be empty")

local rand = crypto.random(16)
assert.not_nil(rand, "crypto.random should return a string")
assert.gt(#rand, 0, "crypto random should not be empty")

log.info("Crypto OK")

log.info("All v0.5.0 E2E tests passed!")
