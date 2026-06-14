# Versioned Model Record ID Implementation Decisions

## D1. `neg_relate` / `ngmr_relate` ID Ownership

Decision: use target-first array record ids.

```text
neg_relate:[target_ref0, target_ref1, target_sesno, carrier_ref0, carrier_ref1, carrier_sesno, geo_index]
ngmr_relate:[target_ref0, target_ref1, target_sesno, carrier_ref0, carrier_ref1, carrier_sesno, ngmr_ref0, ngmr_ref1, ngmr_sesno, geo_index]
```

Reason:

- Explicit model regeneration is driven by the target refno/session.
- Cleanup must be able to delete all model artifact rows for the target by SurrealDB record id range.
- A carrier-first `neg_relate` prefix would preserve carrier identity but would not let target-driven regen clean stale negative relations by prefix alone.

Implementation consequence:

- `model_record_id::neg_relate_id(target, carrier, geo_index)` and `model_record_id::ngmr_relate_id(target, carrier, ngmr, geo_index)` take `target` first.
- Generic `model_refno_range("neg_relate", target)` and `model_refno_range("ngmr_relate", target)` remain valid cleanup primitives for target-scoped deletion.
- Carrier and NGMR identities stay encoded after the target prefix, so read paths can still inspect or query those dimensions when needed.
