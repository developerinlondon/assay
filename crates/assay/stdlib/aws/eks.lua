--- @module assay.aws.eks
--- @description EKS authentication token minting (the aws eks get-token equivalent), in-process — no aws CLI subprocess, works in readonly mode. Used by assay.k8s kubeconfig contexts.
--- @category cloud
--- @keywords aws, eks, kubernetes, token, get-token, authentication, k8s-aws-v1, presigned
--- @quickref M.get_token(cluster_name, opts?) -> token | Mint a k8s-aws-v1 bearer token for an EKS cluster

local sigv4 = require("assay.aws.sigv4")
local sts = require("assay.aws.sts")

local M = {}

-- EKS validates the presigned URL's X-Amz-Date within a 15-minute window; the
-- aws CLI uses a 60-second X-Amz-Expires. Callers should treat minted tokens
-- as valid for ~10 minutes (assay.k8s caches them accordingly).
local TOKEN_EXPIRES_SECS = 60

local function base64url(s)
  return (base64.encode(s):gsub("+", "-"):gsub("/", "_"):gsub("=", ""))
end

--- Mint an EKS bearer token: a presigned sts:GetCallerIdentity URL carrying the
--- cluster name in a signed x-k8s-aws-id header, base64url-encoded with the
--- k8s-aws-v1. prefix — exactly what `aws eks get-token` produces, minted
--- in-process so it works where subprocesses are blocked (readonly mode).
---
--- @param cluster_name string The EKS cluster name
--- @param opts table|nil { region, profile, role_arn, credentials, access_key, secret_key, session_token, time }
--- @return string The bearer token ("k8s-aws-v1.…")
function M.get_token(cluster_name, opts)
  opts = opts or {}
  local creds = opts.credentials or sts.credentials(opts)
  local region = opts.region or env.get("AWS_REGION") or env.get("AWS_DEFAULT_REGION")
    or "us-east-1"

  local url = sigv4.presign({
    access_key = creds.access_key,
    secret_key = creds.secret_key,
    session_token = creds.session_token,
    service = "sts",
    region = region,
    method = "GET",
    host = "sts." .. region .. ".amazonaws.com",
    path = "/",
    query = { Action = "GetCallerIdentity", Version = "2011-06-15" },
    headers = { ["x-k8s-aws-id"] = cluster_name },
    expires = TOKEN_EXPIRES_SECS,
    time = opts.time,
  })
  return "k8s-aws-v1." .. base64url(url)
end

return M
