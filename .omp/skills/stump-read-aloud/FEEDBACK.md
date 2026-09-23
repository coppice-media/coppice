# Skill Feedback Log

Record issues encountered while using this skill. Each entry should describe what went wrong, the context, and a suggested fix.

## Template

### <ISO timestamp> — <SEVERITY>

- **Issue:** <what went wrong>
- **Context:** <what the skill was being used for>
- **Suggested fix:** <if any>
- **Status:** open

Severity is one of `critical` | `high` | `medium` | `low` | `nit`.

Resolution: a verified resolved entry is DELETED from this file — git history is the record. Never leave an entry marked resolved.

---

## Open issues

(None yet — add entries below)
### 2026-09-22T14:24:45.712Z — HIGH
- **Issue:** The Status lookup table is stale and contradicts the current project source of truth: it labels media_sync_maps, the request ledger, and the worker-local Storyteller adapter as Planned, while .omp/PROJECT_STATE.md and .omp/NEXT_STEPS.md record persistent validated SyncMapV1 import/enqueue/cache delivery, the shipped metadata-backed request ledger, and the optional Storyteller delegate as shipped.
- **Context:** Consulted the skill before implementing the user-requested SMIL import, native alignment runner, and read-aloud playback acceptance on 2026-09-22.
- **Suggested fix:** Update the status table from the current project state: Tier 1 plus persistent SyncMapV1 import/deduplicated enqueue/authenticated cache-only delivery and Storyteller delegate are shipped; exact SMIL import, native CTC/Viterbi, complete playback/readiness and virtual composite delivery remain planned. Remove the obsolete claim that no request ledger/table/page exists.
- **Status:** open
### 2026-09-22T16:43:28.530Z — LOW
- **Issue:** RESOLVED: The skill did not specify the proven native-SMIL and external-native-aligner acceptance contract.
- **Context:** Implemented and passed the full 2026-09-22 Rust/Bun/build gate: EPUB preparation preserves source XHTML fragment ids; native SMIL import is accepted only when its embedded audio bytes match the sole paired external M4B by size and SHA-256; deterministic delivery streams that M4B into the EPUB; the external native runner accepts only the typed CPU/fp32/CTC profile and operator-local executable/model paths.
- **Suggested fix:** Add these invariants to the native SMIL import, derivative rendering, and worker backend sections, including the fail-closed byte identity check and no whole-M4B buffering rule.
- **Status:** open
