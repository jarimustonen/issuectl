# AI-First CLI Design Principles

These principles apply to all CLI tools in this repo unless otherwise
mentioned. The primary caller is often an AI agent (Claude Code), not
a human typing in a terminal. Some conventions differ from
human-oriented software — follow these deliberately.

## 1. Strict input validation — no silent fixups

Validate strictly. Reject malformed, empty, whitespace-only, or otherwise
suspicious inputs with clear errors. Do not coerce, trim silently, or fall
back to defaults for obviously-wrong inputs. The AI caller is responsible
for sending well-formed input — surface problems as errors so it can fix
its output and retry.

Concretely:

- Empty or whitespace-only required arguments → error, not default
- Unknown options/flags → error, not ignored
- Out-of-range values → error, not coerced
- Report the actual invalid value in the error message — the AI can parse
  it and fix its input

Rationale: a lenient parser hides the caller's mistakes. An AI caller can
read the error, correct its output, and retry. Surfacing defects is
cheaper than papering over them.

## 2. Structured, parseable output

CLI tools should support machine-readable output alongside human-readable
output:

- Provide `--json` flag for structured JSON output where applicable
- Errors go to stderr, data to stdout — keep them separate
- Include metadata in output (status codes, URLs, counts) so the caller
  doesn't need to infer them
- Exit codes must be meaningful: 0 = success, 1 = user error, 2 = system error

Rationale: AI agents parse stdout programmatically. Mixed human/machine
output forces format sniffing.

### Logs: JSONL, one event per line

Logs (whether emitted to stderr, a file, or a journal) must be
**JSONL** — one self-contained JSON object per line, one event per
line. No multi-line records, no plain-text fallback in production
mode, no human-formatted prefixes wrapping JSON payloads. A grep, a
`jq`, or a `tail -F | jq 'select(...)'` is the canonical reading
tool.

Each log line carries **trace-shaped context** so logs are filterable
by the actors and resources involved:

- `user_id` / `tenant_id` whenever a request, job, or message is
  attributable to a user or tenant
- `trace_id` / `run_id` / `request_id` so multiple log lines from one
  logical operation can be correlated
- `message_id`, `receipt_id`, `attachment_id`, etc. — domain entity
  ids relevant to the event
- The originating subsystem/module (`target`, `component`) so cross-
  cutting filters work

Avoid embedding user-identifying context into free-form `message`
strings only — put it in dedicated fields. `grep '"user_id":42'` and
`jq 'select(.tenant_id == 7 and .level == "ERROR")'` should both work
without parsing prose.

Rationale: production debugging looks like "what happened to user
X's message Y" — that question is answered by structured filters,
not by reading prose. Per-line JSON also keeps logs streamable
(every line is a complete record) and resilient to truncation.

## 3. No interactive prompts

No `press y to continue`, no confirmation dialogs, no interactive Y/N
prompts, no TTY-dependent behavior. All commands must be non-interactive:
valid input succeeds, invalid input fails with a clear diagnostic and
non-zero exit.

- Destructive actions opted in via explicit flags (e.g. `--force`, `--yes`)
- One-shot execution: all inputs via arguments, output to stdout/stderr
- No pagers, no `less`, no `$EDITOR` invocations

Rationale: AI agents cannot respond meaningfully to interactive prompts.

## 4. Informative error messages

Error messages should contain enough context for the AI caller to
understand and fix the problem without additional investigation:

- Include the actual invalid value: `"Invalid target 'foobar'. Available: local, staging, demo, prod"`
- Include the expected format: `"URL must start with / or http"`
- For multi-step failures, indicate which step failed and why
- Stack traces and internal details go to stderr with `--verbose`, not by default

## 5. Composable commands

Design commands to work well in pipelines and with other tools:

- Fetch commands output to stdout by default (pipe-friendly)
- `--output FILE` as an alternative to stdout redirection
- Support stdin where it makes sense (e.g. reading URLs from a list)
- Consistent flag naming across commands (`--target`, `--output`, `--json`)
