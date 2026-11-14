
## Dollar estimate removed (v29)

- `turn_telemetry` no longer stores `estimated_dollars_saved`. Schema v29 drops the column. The telemetry summary reports cost and cache hit only.
- The dollar figure was cache-read tokens times list price, with no measured baseline. It was an estimate, not a saving.
