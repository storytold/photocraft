# Module 7 — AI-Code Anti-Patterns

Composite module targeting the defects that Large-Language-Model code
generators reliably emit. These are high-signal and cheap to detect — run this
module early. They map back to the named principles but deserve their own sweep
because they cluster in generated code.

## 7.1 Magic strings & magic numbers
- Literal strings/numbers embedded in logic: status values, config keys, routes,
  SQL, error codes, thresholds, dictionary keys.
- **Why it hurts:** no single source of truth, typo bugs the compiler can't
  catch, duplication across files, impossible to grep confidently.
- **Fix:** constants, enums, typed options, resource files, named config.
- **Prompt:** *Find string and numeric literals used as identifiers, keys,
  statuses, thresholds, routes, or config in logic (exclude genuine one-off UI
  text and test data). Group duplicated literals across files. Propose
  constants/enums/typed config for each cluster.*

## 7.2 Duplication (DRY violations)
- LLMs re-emit near-identical blocks rather than reusing. Copy-pasted methods,
  parallel validation, repeated mapping/DTO code, duplicated error handling.
- **Fix:** Extract Method/Class/module; shared helpers; template method.
- **Prompt:** *Detect duplicated and near-duplicated code blocks across {scope}.
  Quote each cluster with all sites, estimate the divergence, and propose the
  single extraction that collapses it. Rank by number of copies.*

## 7.3 God classes / god functions
- One class or function that orchestrates everything (SRP, Module 1.1; Large
  Class, Module 4). Common in AI "just make it work" output.
- **Prompt:** *Identify the largest classes/functions by responsibility count,
  length, and fan-in. For each, enumerate the distinct responsibilities and
  propose the split.*

## 7.4 Hardcoded configuration & secrets
- Connection strings, API keys, URLs, credentials, feature toggles baked into
  source. **Secrets in source = Critical severity.**
- **Fix:** configuration providers, environment, secret stores; never in repo.
- **Prompt:** *Scan for hardcoded connection strings, API keys, passwords,
  tokens, base URLs, file paths, and environment-specific values. Flag any
  credential/secret as Critical. Propose externalized configuration.*

## 7.5 Inconsistent naming & structure
- Mixed conventions, same concept named 3 ways, folder layout that doesn't match
  the domain (Screaming Architecture, Module 3.2).
- **Prompt:** *Catalogue naming inconsistencies for the same concept and
  convention drift (casing, prefixes, layer suffixes). Propose a consistent
  vocabulary.*

## 7.6 Poor / swallowed error handling
- `catch (Exception) { }` empty catches; catching and rethrowing without
  context; returning null on error; logging then swallowing; no fail-fast.
- **Fix:** guard clauses, fail fast, meaningful exceptions, don't swallow.
- **Prompt:** *Find empty catch blocks, catch-all swallowing, null-on-error
  returns, and missing guard clauses/input validation at boundaries. Flag data-
  loss/silent-failure risks and propose fail-fast handling.*

## 7.7 Deep nesting & missing guard clauses
- Arrow code (nested ifs), no early returns, complex boolean conditions.
- **Fix:** guard clauses / early return, Decompose Conditional, invert.
- **Prompt:** *Find methods with nesting depth >3 or complex compound
  conditionals. Propose guard clauses and conditional decomposition.*

## 7.8 Async / resource misuse
- `async void`, missing `await`, sync-over-async (`.Result`/`.Wait()`),
  undisposed `IDisposable`, DB connections/HTTP clients not pooled.
- **Prompt:** *Find async/resource hazards: async void, unawaited tasks, sync-
  over-async blocking, undisposed disposables, per-call HttpClient/connection
  creation. Flag deadlock and leak risks.*

## 7.9 Security surface (quick pass)
- SQL built by string concatenation (injection), unvalidated input, missing
  authz checks, secrets in logs, overly broad CORS. Escalate anything real to
  the dedicated `/security-review` skill.
- **Prompt:** *Scan for injection (string-concatenated SQL/commands),
  unvalidated external input reaching sensitive sinks, missing authorization on
  endpoints, and secrets written to logs. List with severity; recommend running
  a full security review on hits.*

## 7.10 Dead code & speculative generality
- Unused methods/classes/params, commented-out blocks, unreached branches,
  abstractions with a single caller "for the future."
- **Prompt:** *Find unused/unreachable code, commented-out blocks, and
  unused-parameters, plus abstractions with exactly one use. Recommend deletion.*

## 7.11 Hallucinated / phantom dependencies (LLM-specific)
- Imports of non-existent APIs, wrong overloads, or packages that don't exist /
  aren't referenced (a supply-chain "slopsquatting" risk if later `add`ed blindly).
- **Prompt:** *Verify every imported package/module resolves to a real, referenced
  dependency and every called API member exists. Flag phantom imports, unused
  package references, and calls to members not present in the referenced version.*

## 7.12 Reinvented framework/stdlib functionality (LLM-specific)
- Hand-rolled JSON parsing, date math, retry/backoff, string casing, DI, or
  collection helpers that duplicate the framework/BCL.
- **Prompt:** *Find hand-written implementations of things the framework/stdlib
  already provides (serialization, date/time, retry, hashing, LINQ-able helpers).
  Recommend the built-in.*

## 7.13 Placeholder stubs & narrating comments (LLM-specific)
- Shipped `// TODO: implement`, `throw new NotImplementedException()` on live
  paths, lorem-ipsum/sample defaults, and line-by-line comments that restate the
  code instead of explaining *why*.
- **Prompt:** *Find NotImplemented/TODO stubs on non-test code paths, placeholder/
  sample values shipped as defaults, and comment blocks that merely narrate the
  next line. Flag stubs as correctness risks; recommend deleting narration.*

**Stance:** deleting duplication, dead code, and magic literals typically
removes more bug surface per hour than any other activity — and it is low-risk.
Do it behind whatever tests exist; if none exist, see Module 8 first.
