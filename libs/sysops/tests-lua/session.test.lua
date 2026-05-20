--! sysops.session tests — HMAC-signed cookies + in-memory store.
--!
--! Run via:
--!   LUA_PATH='libs/?.lua;libs/?/init.lua;libs/sysops/?.lua;libs/sysops/tests-lua/?.lua;;' \
--!     assay libs/sysops/tests-lua/session.test.lua

local session = require("sysops.session")

print("[sysops.session]")

local KEY = "0123456789abcdef0123456789abcdef" -- exactly 32 bytes

-- ---------------------------------------------------------------------
-- 1. issue() / verify() round-trip.
-- ---------------------------------------------------------------------

do
  local s = session.new({ signing_key = KEY, ttl_seconds = 3600 })
  local cookie = s:issue({ sub = "alice@example", email = "alice@example" })
  assert.not_nil(cookie:find("%."), "cookie is payload.sig")
  local claims, err = s:verify(cookie)
  assert.eq(err, nil, "verify returns no error")
  assert.eq(claims.sub, "alice@example", "sub roundtrips")
  assert.eq(claims.email, "alice@example", "email roundtrips")
  assert.not_nil(claims.exp, "exp auto-populated")
  assert.not_nil(claims.iat, "iat auto-populated")
  print("  ok issue/verify round-trip")
end

-- ---------------------------------------------------------------------
-- 2. Tampered payload is rejected.
-- ---------------------------------------------------------------------

do
  local s = session.new({ signing_key = KEY })
  local cookie = s:issue({ sub = "alice@example" })
  local payload, sig = cookie:match("^([^.]+)%.([^.]+)$")
  -- Flip a single byte of payload; b64url alphabet keeps it the same
  -- length so we just substitute 'a' for 'b' (or similar) in the payload.
  local tampered_payload = payload:sub(1, -2) .. (payload:sub(-1) == "a" and "b" or "a")
  local tampered = tampered_payload .. "." .. sig
  local claims, err = s:verify(tampered)
  assert.eq(claims, nil, "tampered cookie rejected")
  assert.eq(err, "bad signature", "error is bad signature")
  print("  ok tampered payload rejected")
end

-- ---------------------------------------------------------------------
-- 3. Tampered signature is rejected.
-- ---------------------------------------------------------------------

do
  local s = session.new({ signing_key = KEY })
  local cookie = s:issue({ sub = "alice@example" })
  local payload, sig = cookie:match("^([^.]+)%.([^.]+)$")
  local bad_sig_char = sig:sub(-1) == "a" and "b" or "a"
  local tampered = payload .. "." .. sig:sub(1, -2) .. bad_sig_char
  local claims, err = s:verify(tampered)
  assert.eq(claims, nil, "bad-sig cookie rejected")
  assert.eq(err, "bad signature", "error is bad signature")
  print("  ok tampered signature rejected")
end

-- ---------------------------------------------------------------------
-- 4. Expired cookie is rejected (caller supplies exp in the past).
-- ---------------------------------------------------------------------

do
  local s = session.new({ signing_key = KEY })
  local cookie = s:issue({ sub = "alice@example", exp = os.time() - 1 })
  local claims, err = s:verify(cookie)
  assert.eq(claims, nil, "expired cookie rejected")
  assert.eq(err, "expired", "error is expired")
  print("  ok expired cookie rejected")
end

-- ---------------------------------------------------------------------
-- 5. Malformed cookie is rejected.
-- ---------------------------------------------------------------------

do
  local s = session.new({ signing_key = KEY })
  local claims, err = s:verify("not-a-valid-cookie")
  assert.eq(claims, nil, "no dot → malformed")
  assert.eq(err, "malformed", "error is malformed")

  claims, err = s:verify("")
  assert.eq(claims, nil, "empty string rejected")

  claims, err = s:verify(nil)
  assert.eq(claims, nil, "nil rejected")
  assert.eq(err, "missing", "error is missing for nil input")
  print("  ok malformed/empty/nil cookies rejected")
end

-- ---------------------------------------------------------------------
-- 6. Different key rejects valid-looking cookie.
-- ---------------------------------------------------------------------

do
  local s1 = session.new({ signing_key = KEY })
  local s2 = session.new({ signing_key = string.rep("z", 32) })
  local cookie = s1:issue({ sub = "alice@example" })
  local claims, err = s2:verify(cookie)
  assert.eq(claims, nil, "cookie minted with different key rejected")
  assert.eq(err, "bad signature", "error is bad signature")
  print("  ok cross-key forgery rejected")
end

-- ---------------------------------------------------------------------
-- 7. Pending OIDC state: store, take (one-shot), GC.
-- ---------------------------------------------------------------------

do
  local store = session.store_new()
  store:put_pending("state-abcdef12", { verifier = "v1", return_to = "/" })

  local p = store:take_pending("state-abcdef12")
  assert.not_nil(p, "first take_pending returns the value")
  assert.eq(p.verifier, "v1", "verifier roundtrips")
  assert.eq(p.return_to, "/", "return_to roundtrips")

  local p2 = store:take_pending("state-abcdef12")
  assert.eq(p2, nil, "second take_pending returns nil (one-shot)")

  print("  ok pending state is one-shot")
end

-- ---------------------------------------------------------------------
-- 8. Refresh-token store: put / get / revoke.
-- ---------------------------------------------------------------------

do
  local store = session.store_new()
  assert.eq(store:get_refresh("alice"), nil, "empty store returns nil")
  store:put_refresh("alice", "rt-1", os.time() + 3600)
  local r = store:get_refresh("alice")
  assert.eq(r.refresh_token, "rt-1", "refresh token stored")
  store:revoke("alice")
  assert.eq(store:get_refresh("alice"), nil, "revoke clears the entry")
  print("  ok refresh-token store + revoke")
end

print("[sysops.session] ok")
