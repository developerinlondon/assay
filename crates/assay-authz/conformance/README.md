# Conformance fixtures

`cases/*.json` is a **vendored, unmodified copy** of the language-neutral golden-decision fixtures
from the reference implementation:

| | |
| --- | --- |
| Upstream | [`developerinlondon/agentauthz`](https://github.com/developerinlondon/agentauthz), path `src/conformance/cases` |
| Version | `0.6.0` (copied at `0.5.0`; the fixtures and the whole decision path are byte-identical between the two) |
| Cases | 149 across 9 suites |

The fixtures are the contract, not a convenience: an engine conforms when it decides every case
identically to the reference. `tests/conformance.rs` loads all nine suite files and runs every case
twice — once through the pure evaluator over the raw grant universe, once through the composed
`Engine` with the synthesizer grants kept separate — and both must agree with the fixture's
`expect`.

## Refreshing

Copy the upstream directory over `cases/` verbatim and update the version in the table above. Do
not hand-edit a fixture: a case that fails is a bug in this engine until proven otherwise, and a
locally patched fixture silently stops being the same contract the reference passes.

The files are excluded from `dprint` so a formatting pass can never make the vendored copy drift
from upstream byte-for-byte.

## `storable: false`

Four cases carry `"storable": false`. They assert a conditions *shape* that a storage layer's
at-rest constraint must refuse — the reference library's Postgres backend rejects the write, so the
row can never exist there. This engine has no storage layer, so it mirrors the reference's
**in-memory** treatment: the case is evaluated like any other and must fail closed. On top of that,
`tests/conformance.rs` asserts `Engine::validate` refuses the same shapes at validate time, which is
the closest in-process equivalent of the at-rest check.
