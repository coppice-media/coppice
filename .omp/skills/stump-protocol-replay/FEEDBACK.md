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

### 2026-09-13T11:39:23.297Z — LOW

- **Issue:** RESOLVED: the local Hurl 6.1.1 executable could not start after a system libxml2 SONAME change (`libxml2.so.2` missing), but the skill had no isolated runner fallback.
- **Context:** Running the post-build Komga fixture replay from `../komga-compat`; the fixture was healthy and the harness validation reached the Hurl executable.
- **Suggested fix:** Add an evidence-preserving fallback using the pinned official `ghcr.io/orange-opensource/hurl:6.1.1` container with host networking and the sibling harness mounted read-only/read-write as required. Keep all credentials and fixture IDs as runtime variables, and still label the result server-contract evidence.
- **Status:** open
