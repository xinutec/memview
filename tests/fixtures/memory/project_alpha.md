---
name: project_alpha
description: "~/Code/alpha — the alpha service, deployed on isis"
metadata:
  node_type: memory
  type: project
  originSessionId: 00000000-0000-0000-0000-000000000000
---

`~/Code/alpha` — the alpha service. Reaches the café's façade over WireGuard;
naïve retries are disabled. See [[reference_beta]] and [[feedback_gamma|the
habit]], plus [[project_not_written_yet]] which has no file.

Typed links, plain mentions, and a misspelt relation that must NOT quietly
become one: [[governs:feedback_gamma]] restates the habit above,
[[because:reference_beta]] is why, and [[superseeds:reference_beta]] is a typo
that has to surface rather than pass as untyped.

**Decisions:**
- Deploy on **isis** so it reads the in-namespace secret.
