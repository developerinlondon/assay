--- @module assay.aws.ec2
--- @description AWS EC2 read-only queries via Signature V4. Describe instances, volumes, security groups.
--- @category cloud
--- @keywords aws, ec2, instances, volumes, security-groups, describe, sigv4, compute, reservations
--- @quickref client(opts) -> client | Create an EC2 client (opts = {access_key?, secret_key?, profile?, role_arn?, region?, session_token?, endpoint?}; no keys -> standard chain via assay.aws.sts)
--- @quickref c:describe_instances(opts?) -> [instance] | List instances (Action=DescribeInstances)
--- @quickref c:describe_volumes(opts?) -> [volume] | List EBS volumes (Action=DescribeVolumes)
--- @quickref c:describe_security_groups(opts?) -> [group] | List security groups (Action=DescribeSecurityGroups)

local M = {}

local sigv4 = require("assay.aws.sigv4")

local API_VERSION = "2016-11-15"

local function encode(value)
  local out = tostring(value):gsub("([^A-Za-z0-9%-_.~])", function(ch)
    return string.format("%%%02X", string.byte(ch))
  end)
  return out
end

local function build_query(params)
  local parts = {}
  for k, v in pairs(params) do
    parts[#parts + 1] = encode(k) .. "=" .. encode(v)
  end
  table.sort(parts)
  return table.concat(parts, "&")
end

local function xml_first(body, tag)
  return body:match("<" .. tag .. ">(.-)</" .. tag .. ">")
end

-- Return the inner text of each top-level <item> in `inner`, tracking depth so
-- nested <item> elements (attachments, rule references) are not mistaken for
-- top-level ones.
local function top_level_items(inner)
  local items = {}
  local depth = 0
  local start = nil
  local i = 1
  while true do
    local o = inner:find("<item>", i, true)
    local ce = inner:find("</item>", i, true)
    if not o and not ce then break end
    if o and (not ce or o < ce) then
      depth = depth + 1
      if depth == 1 then start = o + #"<item>" end
      i = o + #"<item>"
    else
      if depth == 1 and start then
        items[#items + 1] = inner:sub(start, ce - 1)
        start = nil
      end
      depth = depth - 1
      i = ce + #"</item>"
    end
  end
  return items
end

-- Flatten every top-level <item> across all `set_tag` containers in the body.
local function set_items(body, set_tag)
  local items = {}
  for chunk in body:gmatch("<" .. set_tag .. ">(.-)</" .. set_tag .. ">") do
    for _, item in ipairs(top_level_items(chunk)) do
      items[#items + 1] = item
    end
  end
  return items
end

local function apply_filters(params, filters)
  if not filters then return end
  for i, f in ipairs(filters) do
    params["Filter." .. i .. ".Name"] = f.name
    for j, val in ipairs(f.values or {}) do
      params["Filter." .. i .. ".Value." .. j] = val
    end
  end
end

function M.client(opts)
  opts = opts or {}
  -- No explicit keys → resolve via the standard chain (profile / env / IRSA),
  -- honoring opts.profile and opts.role_arn. See assay.aws.sts.
  local creds = opts
  if not opts.access_key then
    creds = require("assay.aws.sts").credentials(opts)
  end
  local access_key = creds.access_key or error("aws.ec2.client: access_key is required")
  local secret_key = creds.secret_key or error("aws.ec2.client: secret_key is required")
  local region = opts.region or env.get("AWS_REGION") or env.get("AWS_DEFAULT_REGION")
    or error("aws.ec2.client: region is required")
  local session_token = creds.session_token
  local endpoint = opts.endpoint

  local url, host
  if endpoint and endpoint ~= "" then
    if endpoint:match("^https?://") then
      url = endpoint:gsub("/+$", "")
      host = url:gsub("^https?://", "")
    else
      host = endpoint
      url = "https://" .. host
    end
  else
    host = "ec2." .. region .. ".amazonaws.com"
    url = "https://" .. host
  end

  local function request(params)
    local query = build_query(params)
    local signed = sigv4.sign({
      access_key = access_key,
      secret_key = secret_key,
      session_token = session_token,
      service = "ec2",
      region = region,
      method = "GET",
      host = host,
      path = "/",
      query = query,
      payload = "",
    })
    local resp = http.get(url .. "/?" .. query, { headers = signed })
    if resp.status ~= 200 then
      error("aws.ec2: GET " .. (params.Action or "") .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return resp.body
  end

  local c = {}

  function c:describe_instances(desc_opts)
    desc_opts = desc_opts or {}
    local params = { Action = "DescribeInstances", Version = API_VERSION }
    if desc_opts.instance_ids then
      for i, id in ipairs(desc_opts.instance_ids) do
        params["InstanceId." .. i] = id
      end
    end
    apply_filters(params, desc_opts.filters)

    local body = request(params)
    local instances = {}
    for _, item in ipairs(set_items(body, "instancesSet")) do
      local state_block = xml_first(item, "instanceState")
      instances[#instances + 1] = {
        instance_id = xml_first(item, "instanceId"),
        instance_type = xml_first(item, "instanceType"),
        state = state_block and xml_first(state_block, "name"),
        private_ip = xml_first(item, "privateIpAddress"),
        public_ip = xml_first(item, "ipAddress"),
        availability_zone = xml_first(item, "availabilityZone"),
      }
    end
    return instances
  end

  function c:describe_volumes(desc_opts)
    desc_opts = desc_opts or {}
    local params = { Action = "DescribeVolumes", Version = API_VERSION }
    if desc_opts.volume_ids then
      for i, id in ipairs(desc_opts.volume_ids) do
        params["VolumeId." .. i] = id
      end
    end
    apply_filters(params, desc_opts.filters)

    local body = request(params)
    local volumes = {}
    for _, item in ipairs(set_items(body, "volumeSet")) do
      local size = xml_first(item, "size")
      volumes[#volumes + 1] = {
        volume_id = xml_first(item, "volumeId"),
        size = size and tonumber(size) or nil,
        state = xml_first(item, "status"),
        availability_zone = xml_first(item, "availabilityZone"),
        volume_type = xml_first(item, "volumeType"),
      }
    end
    return volumes
  end

  function c:describe_security_groups(desc_opts)
    desc_opts = desc_opts or {}
    local params = { Action = "DescribeSecurityGroups", Version = API_VERSION }
    if desc_opts.group_ids then
      for i, id in ipairs(desc_opts.group_ids) do
        params["GroupId." .. i] = id
      end
    end
    apply_filters(params, desc_opts.filters)

    local body = request(params)
    local groups = {}
    for _, item in ipairs(set_items(body, "securityGroupInfo")) do
      groups[#groups + 1] = {
        group_id = xml_first(item, "groupId"),
        group_name = xml_first(item, "groupName"),
        description = xml_first(item, "groupDescription"),
        vpc_id = xml_first(item, "vpcId"),
      }
    end
    return groups
  end

  return c
end

return M
