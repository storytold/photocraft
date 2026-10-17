# Module 4 — Code Smells (Fowler / Beck)

From **Martin Fowler**, *Refactoring: Improving the Design of Existing Code*
(smells catalogue co-created with **Kent Beck**). A "smell" is a surface
indication that usually corresponds to a deeper problem. Each pairs with named
refactorings. AI-generated code is dense with the duplication and bloat smells.

The smells are Fowler/Beck's; the five-group taxonomy below follows **Mäntylä's**
widely used classification (popularized by refactoring.guru), not the book's flat
list. The *Refactoring* 2nd ed. (2018) also adds **Global Data** and **Mutable
Data** (both high-signal in AI-generated code) and renames Switch Statements →
**Repeated Switches** and Inappropriate Intimacy → **Insider Trading**.

## The catalogue (grouped)

### Bloaters
- **Long Method** — do one thing; if you must comment sections, extract them.
  *Fix:* Extract Method, Replace Temp with Query, Decompose Conditional.
- **Large Class / God Class** — too many fields/methods/responsibilities.
  *Fix:* Extract Class, Extract Subclass, Extract Interface.
- **Primitive Obsession** — strings/ints for money, dates, ids, ranges.
  *Fix:* Replace Primitive with Object, introduce Value Objects.
- **Long Parameter List** — *Fix:* Introduce Parameter Object, Preserve Whole
  Object.
- **Data Clumps** — the same 3-4 fields travel together everywhere.
  *Fix:* Extract Class / Parameter Object.

### Object-orientation abusers
- **Switch Statements** on type codes — see OCP (Module 1.2). *Fix:* Replace
  Conditional with Polymorphism.
- **Refused Bequest** — subclass ignores inherited members — see LSP (1.3).
- **Temporary Field** — field only set in certain cases.
- **Alternative Classes with Different Interfaces** — same job, different names.

### Change preventers
- **Divergent Change** — one class changed for many different reasons (SRP).
- **Shotgun Surgery** — one change forces edits across many classes (CCP).
- **Parallel Inheritance Hierarchies.**

### Dispensables
- **Duplicated Code** — the #1 target. *Fix:* Extract Method/Function, Pull Up
  Method, Form Template Method, extract shared module.
- **Dead Code** — unreachable/unused. *Fix:* delete it.
- **Speculative Generality** — abstraction "for the future" nobody uses.
  *Fix:* Collapse Hierarchy, Inline Class, Remove Parameter.
- **Comments compensating for bad code** — *Fix:* make the code self-explaining.
- **Lazy Class** — not pulling its weight. *Fix:* Inline Class.

### Couplers
- **Feature Envy** — a method more interested in another class's data than its
  own. *Fix:* Move Method/Field.
- **Inappropriate Intimacy** — classes reaching into each other's internals.
- **Message Chains** — `a.getB().getC().getD()`. *Fix:* Hide Delegate.
- **Middle Man** — a class that only delegates. *Fix:* Remove Middle Man.

**Detection heuristics**
- Method/class length and cyclomatic complexity thresholds.
- Token-level duplication scan (copy-paste blocks).
- Repeated field groups (data clumps).
- Message chains: 3+ chained member accesses across distinct objects, e.g. regex
  `\.\w+\(\)\.\w+\(\)\.\w+` or `a.getB().getC().getD()`.
- Global/mutable shared state: `public static` mutable fields, singletons holding
  data, `static` collections written at runtime.

**Inspection prompt**
> Scan {scope} for Fowler code smells. Report the worst instances in each
> category: Bloaters (long methods >~30 lines, classes with many
> responsibilities, primitive obsession for money/date/id, long parameter
> lists, data clumps); Dispensables (duplicated blocks — quote both sites, dead
> code, speculative abstractions, lazy classes); Couplers (feature envy, message
> chains, inappropriate intimacy); Change preventers (divergent change / shotgun
> surgery). For each, cite file:line, name the smell, and name the specific
> refactoring that resolves it. Rank by how widely the smell is duplicated.

**Remediation stance:** refactor in small behavior-preserving steps *behind
tests*. Duplication and dead code are the highest-value, lowest-risk wins —
attack them first.
