# Room Verify-Json Mission Notes

## Intent
Add a post-compute CLI verifier for room computation results:

```bash
aios-database room compute ...
aios-database room verify-json --input verification/room/compute/room_compute_validation.json
```

The verifier is not a compute entrypoint. It inspects persisted post-compute state and compares it against the JSON fixture.

## Source of Truth

- Room-to-panel membership: persisted room/panel relationships written by `room compute`
- Expected component membership: persisted room/component relationships written by `room compute`
- Input contract: `verification/room/compute/room_compute_validation.json`

## Required Operator Semantics

- Default invocation is read-only
- Missing or malformed input fails fast
- Missing compute coverage is reported as a precondition/scope problem
- Real expectation mismatches are reported per case
- Success returns exit code `0`; any failed case returns non-zero

## Manual Acceptance

```bash
cargo run --bin aios-database -- room compute ...
cargo run --bin aios-database -- room verify-json --input verification/room/compute/room_compute_validation.json
```

Optional rebuild/index repair behavior, if implemented, must be explicit and separate from the default verify-only path.

## Common Failure Classes

1. Input failure
   - missing file
   - unreadable file
   - malformed JSON
2. Precompute coverage failure
   - `room compute` not run
   - compute scope did not cover fixture cases
   - persisted result surface incomplete
3. True data mismatch
   - room does not contain expected panel
   - expected component missing from persisted room result
