---
category: AI Agents & Workflow
tagline: TypeSafe Jev System One — a judgment asked as a typed question and answered as a typed value, with the probabilities and the confidence behind it
---

## assay.jev

A script that needs a small semantic answer — which bucket, does this hold, how severe — has two
ways to get one. It can write a prompt and pull the answer back out of the prose, which means the
branch it takes depends on how the model phrased itself that morning. Or it can ask a typed question
and get a typed value.

[TypeSafe](https://typesafe.ai)'s System One is the second kind. You hand it some state and a set of
questions; it hands back one answer per question, each a value your code can branch on directly,
together with the probability of every option it considered and — for Choice and Score — a
calibrated confidence. The threshold is then written in your script, in Lua, where you can read it.

```lua
local jev = require("assay.jev")
local c = jev.client()   -- api_key from TYPESAFE_API_KEY

local answers, info = c:system_one(
  { from = "dana@example.test", subject = "Re: pricing", body = "Can you send the quote by Friday?" },
  {
    bucket = jev.choice("Which does the reader owe this email?", {
      needs_reply  = "They are waiting on an answer",
      needs_action = "Something must be done, by a date or on request",
      fyi          = "Nothing is owed",
    }),
    urgency  = jev.score("How soon does it need attention?", { "whenever", "this week", "today" }),
    deadline = jev.noul("Does it name a deadline?"),
  })

if answers.bucket.confidence >= 0.8 then
  route(answers.bucket.choice)             -- "needs_reply" | "needs_action" | "fyi"
else
  hold_for_a_human(answers.bucket.probabilities)
end

print(answers.urgency.score, answers.deadline.noul, info.usage.input_tokens)   -- noul is 0.0 to 1.0
```

### The client

`jev.client(opts?)` — `opts.api_key` or the `TYPESAFE_API_KEY` environment variable; neither set is
an error, not an unauthenticated call. `opts.base_url` defaults to `https://api.typesafe.ai` and
`opts.model` to `jev-latest`. `opts.attempts` and `opts.retry_delay_s` set the retry budget.

`c:system_one(state, questions, opts?)` posts `/v1/systemone` and returns two values: the `answers`
map, keyed exactly as you keyed `questions`, and `{usage, model}` — what the call cost in tokens and
which model actually answered. `state` goes over the wire as written: a string stays a string, a
record stays a record. `opts.model` overrides the client's model for one call.

### The three primitives

| Builder                              | The question                                                         | The answer                                                                       |
| ------------------------------------ | -------------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| `jev.choice(instructions, criteria)` | `criteria` maps each option name to what it means, up to 255 of them | `{type, choice, probabilities, confidence}` — `choice` is one of your own names  |
| `jev.score(instructions, levels)`    | `levels` is an ordered list low to high, 2 to 10 of them             | `{type, score, probabilities, legend, confidence}` — `score` indexes your ladder |
| `jev.noul(instructions, criteria?)`  | optionally `{ [true] = "...", [false] = "..." }`                     | `{type, noul}` — a probability 0 to 1                                            |

The option names in a Choice are yours and come back verbatim, so `answers.bucket.choice` is
something a script can compare against a string literal. Describing each option is not decoration:
the description is the whole of what the model is told the option stands for.

In a Score the order _is_ the scale — the index of a level is the number that comes back — and the
probabilities are keyed `"0"`, `"1"`, … to match.

A Noul answers with the probability that the statement holds — 0 is a no, 1 is a yes — and carries
no confidence beside it. That one number already describes a two-sided reading completely; a second
would be the same information twice. It is a number, not a side, so compare it against a threshold
you have written down: every number but zero is truthy in Lua, and `if answers.deadline.noul then`
reads a 0.04 as a confident yes. Lua spells booleans as booleans, so `{ [true] = ... }` is accepted
in the _criteria_ and normalised to the `"true"` / `"false"` keys the API wants.

Each builder refuses what the API would refuse anyway — an empty option set, a ladder of one rung, a
Noul keyed on anything but the two sides — so the error names the mistake in your own words rather
than arriving later as a 422 about a field.

### Confidence is the point

`probabilities` says what the model thought of each option. `confidence` says how much that
distribution is worth. A threshold written against it is the difference between a pipeline that
routes what it knows and holds what it does not, and one that files a coin-toss as a decision. Where
to put the threshold is a policy question, which is why it lives in your script rather than in this
module.

### When it does not answer

There is no silent fallback anywhere. A judgment a caller is about to branch on is not something to
guess at when the service did not answer.

| What happened                                | What you get                                                                    |
| -------------------------------------------- | ------------------------------------------------------------------------------- |
| 401                                          | `jev: unauthorized`                                                             |
| 422                                          | `jev: <the field message the API sent>` — or the body, clipped, if it sent none |
| 429                                          | Tried again with backoff; then `jev: rate limited (HTTP 429) after 3 attempts`  |
| 529                                          | Tried again with backoff; then `jev: overloaded (HTTP 529) after 3 attempts`    |
| Any other non-200                            | `jev: POST /v1/systemone HTTP <status>: <the body, clipped>`                    |
| A 200 that is not JSON                       | `jev: POST /v1/systemone returned unparseable JSON`                             |
| A 200 with no `answers`                      | `jev: POST /v1/systemone returned no answers`                                   |
| A Noul answer that is not a number on 0 to 1 | `jev: noul answer out of range: <what came back>`                               |

Three attempts in all, backing off from half a second and doubling to a five-second ceiling. A
`Retry-After` on the response is the server's own number and beats the guess, up to a minute. Only
429 and 529 are repeated: nothing else is a different answer a second later, and retrying it only
triples the wait before you are told.

None of those messages carries your key. Every occurrence of it is replaced with `[redacted]` before
the error leaves the module, and vendor text quoted into one stops at 300 characters — what is cut
is counted, not dropped silently, so you can see the server said more. An error text is usually the
thing a caller logs, or writes into a record of why a judgment was not read; a service, or a proxy
in front of one, that echoes back the `Authorization` header it rejected would otherwise put your
key in both.

### A third reading for email triage

[`assay.email_triage`](ai-agents.html) gains `categorize_jev(emails, jev_client, opts?)` beside
`categorize` and `categorize_llm` — the same three buckets, asked as one Choice per email:

```lua
local triage = require("assay.email_triage")
local out = triage.categorize_jev(emails, jev.client(), { min_confidence = 0.7 })

route(out.needs_reply, out.needs_action, out.fyi)   -- the shape the other two return
for _, u in ipairs(out.uncertain) do                -- and what it was not sure about
  log.info(u.email.subject .. ": " .. u.choice .. " at " .. u.confidence)
end
print(out.usage.input_tokens, out.usage.output_tokens)
```

`needs_reply`, `needs_action` and `fyi` come back in exactly the shape the other two readings
return, so nothing calling them has to change. An email the model was less sure of than
`opts.min_confidence` (0.6 by default) lands in `fyi`, where nothing is owed, and is listed in
`uncertain` with what the model picked and how sure it was — the bare email alone would throw away
the reason it is there. A call that fails raises: an email quietly filed as `fyi` because the
service was down is a message nobody will ever look at again.

One question per email rather than one call over the batch. The state is genuinely a single message,
a failure names the message it happened on, and the confidence gate is per-email — batched, one
unreadable message would take the reading of every other one down with it.

`opts.instructions` and `opts.criteria` reword the question. The three bucket _names_ are the
contract — they are the lists the answer is filed into and the keys a caller already reads — so
`opts.criteria` may rewrite what each one means but not rename them, and a table naming anything
else is refused before the first email goes out rather than failing once per message. An inbox where
"needs_action" means something particular is exactly what it is for.
