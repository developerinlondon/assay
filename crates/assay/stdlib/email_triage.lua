--- @module assay.email_triage
--- @description Email triage: what a message says about itself, read from its own headers and words with no model and no network — whether it declares itself a machine, whether it is a bounce, whether the sender is away and until when, whether they asked to be left alone, and whether they pointed at someone else. Plus the older subject-keyword bucketing and the OpenClaw-assisted one.
--- @category comms
--- @icon mail-search
--- @keywords email, triage, signals, auto-reply, bounce, dsn, out of office, unsubscribe, referral, deterministic, gmail, inbox, classify, openclaw, llm
--- @quickref M.signals(msg, opts?) -> signals | msg is {headers, subject, text, html?}; opts.now is epoch seconds so a test can fix the date
--- @quickref signals -> {auto_reply, bounce, out_of_office, unsubscribe, referral} | Each is {present, evidence, ...}; present is always a boolean and evidence appears only when present
--- @quickref M.own_words(text) -> string | The reply above the quoted thread; phrases matched below it are the letter being answered, not the answer
--- @quickref M.fold(text) -> string | Lowercase and strip the accents Lua's own lower() cannot reach
--- @quickref M.return_date(text, now?) -> "YYYY-MM-DD" | nil | The day an away message names, as the next occurrence of it
--- @quickref M.header(headers, name) -> string | nil | One header, whatever case its name was written in
--- @quickref M.text_of(msg) -> string | The body as text, falling back to the HTML with its tags stripped
--- @quickref M.categorize(emails, opts?) -> buckets | The older pass: buckets by subject keyword and sender only
--- @quickref M.categorize_llm(emails, openclaw_client, opts?) -> buckets | Bucketing through an OpenClaw LLM task

local M = {}

-- Evidence is quoted so a reading can be argued with. A whole quoted thread is
-- not evidence, so it is cut to the sentence that decided the signal.
local EVIDENCE_CAP = 400

-- Only the top of a reply. Below it sits the letter being answered, and cold
-- outreach carries an unsubscribe line on every send — matched there, every
-- reply anyone ever sends reads as somebody asking to be left alone.
local HEAD_LINES = 40

-- `string.lower` is byte-wise ASCII: "BÜRO" lowercases to "bÜRO", which never
-- matches "buro", so both cases are folded here rather than lowercased. The
-- curly apostrophe is on the list because mail clients substitute it silently,
-- and "don’t contact me" would otherwise miss a list written with the straight
-- one.
local FOLD = {
  ["á"] = "a", ["Á"] = "a", ["à"] = "a", ["À"] = "a", ["â"] = "a", ["Â"] = "a",
  ["ä"] = "a", ["Ä"] = "a", ["ã"] = "a", ["Ã"] = "a", ["å"] = "a", ["Å"] = "a",
  ["é"] = "e", ["É"] = "e", ["è"] = "e", ["È"] = "e", ["ê"] = "e", ["Ê"] = "e",
  ["ë"] = "e", ["Ë"] = "e",
  ["í"] = "i", ["Í"] = "i", ["ì"] = "i", ["Ì"] = "i", ["î"] = "i", ["Î"] = "i",
  ["ï"] = "i", ["Ï"] = "i",
  ["ó"] = "o", ["Ó"] = "o", ["ò"] = "o", ["Ò"] = "o", ["ô"] = "o", ["Ô"] = "o",
  ["ö"] = "o", ["Ö"] = "o", ["õ"] = "o", ["Õ"] = "o",
  ["ú"] = "u", ["Ú"] = "u", ["ù"] = "u", ["Ù"] = "u", ["û"] = "u", ["Û"] = "u",
  ["ü"] = "u", ["Ü"] = "u",
  ["ç"] = "c", ["Ç"] = "c",
  ["ñ"] = "n", ["Ñ"] = "n",
  ["ß"] = "ss", ["ẞ"] = "ss", ["’"] = "'",
}

-- The wording is what the sender chose; the lists are written in folded ASCII
-- so case and accents cannot hide a phrase. Exported because a caller adding a
-- language should extend the list rather than fork the reader.

M.UNSUBSCRIBE_PHRASES = {
  "unsubscribe", "opt out", "opt-out", "remove me", "take me off",
  "stop emailing", "stop contacting", "do not contact", "don't contact",
  "no longer wish to receive", "take my name off",
  "abmelden", "keine weiteren e-mails", "nicht mehr kontaktieren",
  "aus dem verteiler",
  "desabonner", "desinscrire", "ne plus me contacter", "retirez-moi",
  "darme de baja", "cancelar suscripcion", "no me contacte",
}

M.AWAY_PHRASES = {
  "out of office", "out of the office", "on annual leave", "on holiday",
  "on vacation", "away from my desk", "away until", "currently away",
  "parental leave", "maternity leave", "paternity leave",
  "abwesend", "nicht im buro", "ausser haus", "im urlaub", "urlaub bis",
  "abwesenheitsnotiz",
  "absent du bureau", "absente du bureau", "en conge", "en vacances",
  "de retour le", "absence du bureau",
  "fuera de la oficina", "de vacaciones", "ausente de la oficina",
  "estare ausente",
}

-- A subject a mail system writes for itself. The body of one of these is the
-- sender's own away wording, which is why this list is separate from the away
-- phrases: the subject says a machine sent it, the body says why.
M.AUTO_REPLY_SUBJECTS = {
  "automatic reply", "auto-reply", "autoreply", "auto reply",
  "automatische antwort", "reponse automatique", "respuesta automatica",
}

-- The wording non-delivery reports use. Each one is a phrase a person writing
-- prose would not produce by accident; "does not exist" and its like are
-- deliberately absent, because a person writes that about a department.
M.BOUNCE_PHRASES = {
  "delivery status notification", "undeliverable", "delivery has failed",
  "could not be delivered", "delivery failure", "returned to sender",
  "recipient address rejected", "user unknown", "mailbox unavailable",
  "mailbox is full", "address not found", "permanent failure",
  "unzustellbar", "nicht zustellbar", "echec de la remise",
  "no se pudo entregar",
}

M.REFERRAL_PHRASES = {
  "please contact", "please speak to", "you should contact",
  "you should speak to", "reach out to", "the right person",
  "better person to", "forwarding this to", "forwarded your",
  "i have cc'd", "i've cc'd", "copying in", "no longer with us",
  "has left the company", "is the person who",
  "wenden sie sich an", "nicht mehr bei uns",
  "veuillez contacter", "adressez-vous a", "n'est plus chez nous",
  "por favor contacte", "pongase en contacto con", "ya no trabaja",
}

local MONTHS = {
  january = 1, february = 2, march = 3, april = 4, may = 5, june = 6,
  july = 7, august = 8, september = 9, october = 10, november = 11,
  december = 12,
  januar = 1, februar = 2, marz = 3, mai = 5, juni = 6, juli = 7,
  oktober = 10, dezember = 12,
  janvier = 1, fevrier = 2, mars = 3, avril = 4, juin = 6, juillet = 7,
  aout = 8, septembre = 9, octobre = 10, novembre = 11, decembre = 12,
  enero = 1, febrero = 2, marzo = 3, abril = 4, mayo = 5, junio = 6,
  julio = 7, agosto = 8, septiembre = 9, octubre = 10, noviembre = 11,
  diciembre = 12,
}

local MONTH_LENGTHS = { 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31 }

local EMAIL_PATTERN = "[%w%.%_%%%+%-]+@[%w%.%-]+%.%a%a+"

--- Lowercase, and fold the accents `string.lower` cannot reach.
---
--- Every phrase list above is written folded, so a German or French message
--- matches whatever case and accents the sender's client produced.
function M.fold(text)
  local lowered = tostring(text or ""):lower()
  return (lowered:gsub("[\194-\244][\128-\191]*", function(ch) return FOLD[ch] or ch end))
end

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end

--- One header, whatever case the sender wrote its name in.
---
--- A header present but empty is a header that says nothing, and reads the same
--- as one that is absent.
function M.header(headers, name)
  if type(headers) ~= "table" then return nil end
  local wanted = tostring(name or ""):lower()
  for key, value in pairs(headers) do
    if tostring(key):lower() == wanted and type(value) == "string" and trim(value) ~= "" then
      return value
    end
  end
  return nil
end

-- A line that begins the quoted letter underneath a reply, in the four
-- languages the phrase lists cover. Written folded, because they are matched
-- against folded text: folding a pattern instead would mangle any character
-- class an accent ever landed in.
local QUOTE_MARKERS = {
  "^%s*>",
  "^%s*on .-wrote:",
  "^%s*%-%-%-%-%-* ?original message",
  "^%s*_____",
  "^%s*am .-schrieb",
  "^%s*le .-a ecrit",
  "^%s*el .-escribio",
}

-- Outlook and its like quote with no marker at all: a bare header block, in
-- whatever language the client runs in. Missing a language does not mean
-- reading a little less of the message — it means reading the quoted letter as
-- the sender's own, and the letter carries an unsubscribe line.
--
-- These two lists are the ones Neutron's own reader uses
-- (apps/api/src/crm/triage/signals.ts) and they have to stay identical in
-- content: a language one of them knows and the other does not is one product
-- reading the same message two different ways.
M.HEADER_FROM_WORDS = { "from", "de", "von", "da", "van", "fran", "fra" }
M.HEADER_NEXT_WORDS = {
  "sent", "date", "to", "cc", "subject",
  "enviado", "gesendet", "inviato", "verzonden", "skickat", "envoye",
  "para", "an", "oggetto", "betreff", "asunto", "objet", "aan", "onderwerp",
}

-- One line naming a sender is not a quote block. A reply may perfectly well
-- open "From: our end, this looks fine", and cut there it loses everything the
-- sender went on to say — including an opt-out. The block counts only when a
-- second header follows the sender line within a few lines of it.
local HEADER_WINDOW = 4

local function header_next(folded)
  for _, word in ipairs(M.HEADER_NEXT_WORDS) do
    if folded:find("^%s*" .. word .. "%s*:%s") then return true end
  end
  return false
end

-- An address on the line as well, because the sender line of a quoted header
-- block always carries one and a sentence beginning with the same word does
-- not.
local function header_from(folded)
  for _, word in ipairs(M.HEADER_FROM_WORDS) do
    if folded:find("^%s*" .. word .. "%s*:%s.*[%w%._%+%-]+@") then return true end
  end
  return false
end

local function marked(folded)
  for _, marker in ipairs(QUOTE_MARKERS) do
    if folded:find(marker) then return true end
  end
  return false
end

--- The sender's own words: the reply above the quoted thread.
---
--- Read whole, a reply matches every phrase in the letter it answers, and the
--- letter carries an unsubscribe line. The cut is at the first quote marker,
--- and at forty lines regardless, because a client that quotes without a marker
--- still puts the sender's own words at the top.
function M.own_words(text)
  local lines, folded = {}, {}
  for line in (tostring(text or "") .. "\n"):gmatch("([^\n]*)\n") do
    if #lines >= HEAD_LINES then break end
    lines[#lines + 1] = line
    folded[#folded + 1] = M.fold(line)
  end

  local at
  for i = 1, #lines do
    if marked(folded[i]) then
      at = i
      break
    end
    if header_from(folded[i]) then
      for j = i + 1, math.min(i + HEADER_WINDOW, #lines) do
        if header_next(folded[j]) then
          at = i
          break
        end
      end
      if at then break end
    end
  end
  if not at then return table.concat(lines, "\n") end

  local kept = {}
  for i = 1, at - 1 do kept[i] = lines[i] end
  -- A cut that leaves nothing did not find a quote, it found the whole
  -- message: a reply that opens with a header block is a forward typed out,
  -- and reading it as an empty message loses the one thing it says.
  for _, line in ipairs(kept) do
    if trim(line) ~= "" then return table.concat(kept, "\n") end
  end
  return table.concat(lines, "\n")
end

--- The body as text: the plain part, or the HTML with its tags taken out.
---
--- A message with only an HTML part still says what it says, and reading
--- nothing from one would make every HTML-only away message invisible.
function M.text_of(msg)
  if type(msg) ~= "table" then return "" end
  local text = trim(msg.text)
  if text ~= "" then return msg.text end
  local html = tostring(msg.html or "")
  if html == "" then return "" end
  local stripped = html
    :gsub("<%s*[bB][rR]%s*/?>", "\n")
    :gsub("</%s*[pPdD][iIvV]?[vV]?%s*>", "\n")
    :gsub("<[^>]*>", " ")
    :gsub("&nbsp;", " ")
    :gsub("&amp;", "&")
    :gsub("&lt;", "<")
    :gsub("&gt;", ">")
  -- Tag removal leaves the gaps the tags stood in. Evidence is quoted text, and
  -- a quote full of double spaces is a quote nobody recognises.
  return (stripped:gsub("[ 	]+", " "))
end

--- The message split where a reader would stop reading one thought.
---
--- The original is split rather than the folded text, so evidence can be quoted
--- as written: folding changes byte lengths, and a position found in folded
--- text does not point at the same place in the original.
local function sentences(text)
  local marked = (tostring(text or ""):gsub("([%.!%?])%s", "%1\1"))
  local out = {}
  for piece in ((marked:gsub("\n", "\1")) .. "\1"):gmatch("([^\1]*)\1") do
    local one = trim(piece)
    if one ~= "" then out[#out + 1] = one:sub(1, EVIDENCE_CAP) end
  end
  return out
end

--- The first of `phrases` that appears, and the sentence carrying it.
---
--- Sentences are walked in the order they were written, so evidence is the
--- first place the sender said it rather than the last.
local function phrase_in(text, phrases)
  for _, one in ipairs(sentences(text)) do
    local folded = M.fold(one)
    for _, phrase in ipairs(phrases) do
      if folded:find(phrase, 1, true) then return phrase, one end
    end
  end
  return nil
end

local function absent() return { present = false } end

local function found(evidence, extra)
  local out = extra or {}
  out.present = true
  out.evidence = tostring(evidence):sub(1, EVIDENCE_CAP)
  return out
end

local function days_in_month(year, month)
  if month == 2 and (year % 4 == 0 and (year % 100 ~= 0 or year % 400 == 0)) then return 29 end
  return MONTH_LENGTHS[month]
end

--- The day an away message names, as a date.
---
--- The year is almost never written, so it is the next occurrence of that day.
--- Read as this year, a December message naming January lands in the past and
--- the follow-up goes out the same day, into an inbox nobody is reading. A day
--- that is today is today: they said they are back, and they are.
---
--- A date the calendar does not have is not a date. "31 February" reads as
--- nothing rather than as the first of March.
--- A token that is a day of the month, ordinal suffix and all.
---
--- Any run of digits is a candidate; what a day of the month can actually be is
--- decided once, by the calendar, in `return_date`. A second rule here saying
--- the same thing in a different way is a rule that can disagree with it.
local function day_of(token)
  local d = token:match("^(%d+)%a?%a?$")
  return d and tonumber(d) or nil
end

function M.return_date(text, now)
  local tokens = {}
  for token in M.fold(text):gmatch("[%a%d]+") do tokens[#tokens + 1] = token end
  local day, month
  for i = 1, #tokens do
    local d = day_of(tokens[i])
    if d then
      -- "3rd of april" puts a word between the two, which is why the month is
      -- looked for a little past the day rather than immediately after it.
      for j = i + 1, math.min(i + 2, #tokens) do
        if MONTHS[tokens[j]] then
          day, month = d, MONTHS[tokens[j]]
          break
        end
      end
    elseif MONTHS[tokens[i]] then
      local after = tokens[i + 1] and day_of(tokens[i + 1])
      if after then day, month = after, MONTHS[tokens[i]] end
    end
    if day then break end
  end
  if not day or not month or day < 1 then return nil end
  local today = os.date("!*t", now or os.time())
  if day > days_in_month(today.year, month) then return nil end
  local year = today.year
  if month < today.month or (month == today.month and day < today.day) then
    year = year + 1
    if day > days_in_month(year, month) then return nil end
  end
  return string.format("%04d-%02d-%02d", year, month, day)
end

--- A message that says it is a machine, in the words the RFCs give it.
---
--- `Auto-Submitted` is the standard one and the others predate it. The header
--- exists on ordinary mail too, where it says the opposite, so `no` is a person
--- writing. `Precedence: bulk` is reported beside the verdict rather than as
--- one: bulk is what a mailing list sets, and a person can write from a list.
local function auto_reply_signal(headers, subject)
  local submitted = M.fold(M.header(headers, "auto-submitted") or "")
  local precedence = M.fold(M.header(headers, "precedence") or "")
  local bulk = (precedence == "bulk" or precedence == "list") or nil
  if submitted ~= "" and submitted ~= "no" then
    return found("Auto-Submitted: " .. submitted, { bulk = bulk })
  end
  for _, name in ipairs({ "x-autoreply", "x-autorespond", "x-auto-response-suppress" }) do
    if M.header(headers, name) then
      return found(name .. ": " .. M.header(headers, name), { bulk = bulk })
    end
  end
  if precedence == "auto_reply" then
    return found("Precedence: auto_reply", { bulk = bulk })
  end
  local _, line = phrase_in(subject, M.AUTO_REPLY_SUBJECTS)
  if line then return found(line, { bulk = bulk }) end
  local out = absent()
  out.bulk = bulk
  return out
end

--- A delivery report, and the address it says could not be reached.
---
--- The whole body is read rather than the sender's own words, because a report
--- has no sender's words: the machine-readable part sits below the human one
--- and cutting at the quote marker would throw the evidence away.
local function bounce_signal(headers, subject, text)
  local address = M.header(headers, "x-failed-recipients")
    or text:match("[Ff]inal%-[Rr]ecipient:%s*[%w]*;?%s*(" .. EMAIL_PATTERN .. ")")
    or text:match("[Oo]riginal%-[Rr]ecipient:%s*[%w]*;?%s*(" .. EMAIL_PATTERN .. ")")
  local extra = { address = address and trim(address) or nil }

  local content_type = M.fold(M.header(headers, "content-type") or "")
  if content_type:find("report-type=delivery-status", 1, true)
    or content_type:find("message/delivery-status", 1, true)
  then
    return found("Content-Type: " .. content_type, extra)
  end
  local from = M.fold((M.header(headers, "from") or "") .. " " .. (M.header(headers, "return-path") or ""))
  if from:find("mailer-daemon", 1, true) or from:find("postmaster@", 1, true) then
    return found("From: " .. trim(M.header(headers, "from") or M.header(headers, "return-path")), extra)
  end
  local _, line = phrase_in(subject .. "\n" .. text, M.BOUNCE_PHRASES)
  if line then return found(line, extra) end
  return absent()
end

--- An away message, and the day it says they are back.
---
--- The date is read from the same words the phrase was found in, so a date sat
--- in the quoted thread below cannot become the day this sender returns.
local function out_of_office_signal(said, now)
  local _, line = phrase_in(said, M.AWAY_PHRASES)
  if not line then return absent() end
  return found(line, { ["until"] = M.return_date(said, now) })
end

--- Somebody asking to be left alone.
---
--- `List-Unsubscribe` is noted and never counted: it is a header put on
--- outbound mail, and a reply that quotes the letter carries it back. Counted,
--- every reply to a compliant campaign would read as an opt-out.
local function unsubscribe_signal(headers, said)
  local list_header = M.header(headers, "list-unsubscribe") ~= nil or nil
  local _, line = phrase_in(said, M.UNSUBSCRIBE_PHRASES)
  if line then return found(line, { list_unsubscribe = list_header }) end
  local out = absent()
  out.list_unsubscribe = list_header
  return out
end

--- Being pointed at somebody else, and their address when one is given.
---
--- The address is taken from the sentence that made the referral rather than
--- from anywhere in the message: the first address in a reply is usually a
--- signature block, and pointing a campaign at a signature is worse than
--- pointing it nowhere.
local function referral_signal(said)
  local _, line = phrase_in(said, M.REFERRAL_PHRASES)
  if not line then return absent() end
  return found(line, { address = line:match(EMAIL_PATTERN) })
end

--- What the message says about itself, with no model and no network.
---
--- The five readings are independent and none of them ranks the others: a
--- message can be an away notice that also asks to be left alone, and which of
--- those wins is the caller's policy, not this module's. Every signal is
--- present in the answer whether or not it fired, so a caller reads
--- `s.bounce.present` without checking for the key first, and no signal is ever
--- reported without the header or the sentence that decided it.
---
--- Only the sender's own words are searched for the phrase signals. A bounce is
--- the exception and reads the whole body, because a delivery report has no
--- quoted reply to cut at.
function M.signals(msg, opts)
  msg = type(msg) == "table" and msg or {}
  opts = opts or {}
  local headers = type(msg.headers) == "table" and msg.headers or {}
  local subject = tostring(msg.subject or "")
  local text = M.text_of(msg)
  local said = subject .. "\n" .. M.own_words(text)
  return {
    auto_reply = auto_reply_signal(headers, subject),
    bounce = bounce_signal(headers, subject, text),
    out_of_office = out_of_office_signal(said, opts.now),
    unsubscribe = unsubscribe_signal(headers, said),
    referral = referral_signal(said),
  }
end

local function empty_buckets()
  return {
    needs_reply = {},
    needs_action = {},
    fyi = {},
  }
end

local function is_automated(email)
  local from = (email.from or ""):lower()
  local subject = (email.subject or ""):lower()
  return email.automated == true
    or from:match("noreply") ~= nil
    or from:match("no%-reply") ~= nil
    or from:match("newsletter") ~= nil
    or subject:match("newsletter") ~= nil
    or subject:match("automated") ~= nil
  end

local function needs_action(email)
  local subject = (email.subject or ""):lower()
  return subject:match("action required") ~= nil
    or subject:match("urgent") ~= nil
    or subject:match("deadline") ~= nil
end

local function normalize(result)
  local buckets = result.categories or result.result or result
  if type(buckets) ~= "table" then
    error("email_triage: invalid LLM response")
  end
  buckets.needs_reply = buckets.needs_reply or {}
  buckets.needs_action = buckets.needs_action or {}
  buckets.fyi = buckets.fyi or {}
  return buckets
end

function M.categorize(emails, opts)
  opts = opts or {}
  local buckets = empty_buckets()
  for _, email in ipairs(emails or {}) do
    if needs_action(email) then
      buckets.needs_action[#buckets.needs_action + 1] = email
    elseif not is_automated(email) then
      buckets.needs_reply[#buckets.needs_reply + 1] = email
    else
      buckets.fyi[#buckets.fyi + 1] = email
    end
  end
  return buckets
end

function M.categorize_llm(emails, openclaw_client, opts)
  opts = opts or {}
  if not openclaw_client or not openclaw_client.llm_task then
    error("email_triage: openclaw_client with llm_task is required")
  end

  local prompt = opts.prompt or [[
Classify the provided email artifacts into exactly three buckets: needs_reply, needs_action, and fyi.

- needs_reply: human emails that likely need a response
- needs_action: emails that require urgent or explicit action
- fyi: newsletters, noreply mail, automated mail, or informational updates

Return only the bucketed emails.
]]

  local result = openclaw_client:llm_task(prompt, {
    artifacts = emails or {},
    output_schema = opts.output_schema or {
      type = "object",
      properties = {
        needs_reply = { type = "array" },
        needs_action = { type = "array" },
        fyi = { type = "array" },
      },
      required = { "needs_reply", "needs_action", "fyi" },
    },
  })

  return normalize(result)
end

return M
