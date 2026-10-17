# Implementation decisions

## 2026-10-13: Editable starter actions (#4779)

PhotoCraft Essentials uses ordinary editable actions. Add the set once after saved actions load, preserving names and order; persist a starter-set version so deleted or renamed actions stay that way. Explicit Load Default Actions adds missing names without replacing edits. Keep headless sessions empty until that command is called. Blur Background uses a single atomic edit on the target pixel layer: detect its subject, exclude subject coverage from Gaussian sampling, and blend the background inside the existing selection while preserving alpha. Detection failure and cancellation do not commit. A recorded Select Subject → Inverse → Blur sequence can act on an old selection when detection finds nothing, so the preset calls this dedicated command instead.

The browser queues generated defaults and their marker for persistence, but only user edits count as unsaved work. Startup writes can be reproduced from the loaded records and built-ins; denied storage alone should not display a loss-of-work banner or block closing a fresh session. Imports and edits to defaults remain unsaved through failed or in-flight transactions until their current contents are committed.

## 2026-10-10: Smart Object contents identity (#2163)

Smart Object instances carry a document-scoped contents ID separately from their layer ID. Normal duplicates preserve it; New Smart Object via Copy creates a new one. Contents edits update the matching instances together, retaining their individual placement and filters. Copies into another document remap contents IDs as a batch, preserving sharing within the copied layers without joining existing destination objects.

Native files persist this ID. Legacy embedded objects without an ID remain independent: identical file bytes do not prove that two objects were intended to share edits. Legacy nonempty linked paths and PSD source identifiers retain their explicit sharing relationship. PSD export keys embedded sources by contents identity as well as bytes, so independent equal-content objects stay independent.
