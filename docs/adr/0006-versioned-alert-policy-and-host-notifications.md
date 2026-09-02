# ADR 0006: Versioned alert policy and host notification boundary

- Status: Accepted when merged
- Date: 2026-09-02
- Decision issue: [#19](https://github.com/giedrius-at-home/personal-finance-assistant/issues/19)

## Context

Proactive alerts must warn early enough to be useful without producing a storm
after repeated ingestion, webhook replay, process restart, or desktop sleep.
Financial severity must remain deterministic and explainable. The gateway runs
inside Linux containers, while native macOS notifications belong to the logged-in
host session and require user permission.

## Decision

Adopt policy version `2026-09-v1` with these initial rules:

- budget critical at exhaustion or when protected cash is not covered;
- budget warning when exact linear forecast exceeds the limit;
- budget heads-up at 65% spent or 1.25 times elapsed-period pace;
- goal critical when incomplete and overdue;
- goal warning when behind by at least one current required weekly contribution;
- goal heads-up for a smaller negative schedule variance;
- cooldowns of seven days, three days, and 24 hours for heads-up, warning, and
  critical respectively;
- severity escalation bypasses cooldown; recovery resolves the lifecycle;
- exact consecutive candidate replay never dispatches twice;
- failed, disabled, or unavailable delivery becomes retryable after one hour;
- evaluate after a successful source event and once daily at 18:00 in the
  configured timezone;
- after sleep or downtime, claim only the latest missed slot using an expiring
  lease rather than replaying every interval.

Persist exact canonical aggregate evidence, its hash, policy version, reasons,
state transitions, and content-free delivery outcomes. Do not persist raw
transactions, webhook bodies, notification messages, or credentials.

Keep the macOS adapter host-side, disabled by default, and invoked through an
argument-safe `osascript` program: message content is passed as `argv`, never
interpolated into AppleScript. Non-macOS and CI environments use the disabled or
unavailable outcome without attempting delivery.

## Consequences

- Every alert can be reproduced from exact aggregate evidence and policy version.
- Threshold and cooldown changes affect future evaluations without rewriting
  historical decisions.
- Duplicate events, concurrent ticks, restart, and sleep cannot multiply delivery.
- A notification attempt is persisted only as outcome metadata; its content
  remains transient.
- Native delivery requires an explicit host-side opt-in and macOS permission.
- The initial completion forecast extrapolates average saving since the goal
  start date; it is deterministic but not a promise or financial advice.

## Alternatives rejected

- **Let Qwen choose severity:** non-deterministic and unsuitable for financial
  protection decisions.
- **Notify on every update:** creates storms under legitimate webhook retries and
  frequent ingestion.
- **Run `osascript` inside the container:** it cannot target the logged-in macOS
  notification session and would mix trust boundaries.
- **Interpolate message text into AppleScript:** permits script injection from
  untrusted names or descriptions.
- **Store only hashes:** proves duplicate identity but cannot explain the exact
  aggregate inputs that produced an alert.
