# Module 1 — SOLID Principles

Five class-design principles collected and popularized by **Robert C. Martin
("Uncle Bob")**. They are the load-bearing rules for object-oriented
maintainability. AI-generated code frequently satisfies none of them because it
optimizes for "works on the happy path," not for change.

---

## 1.1 SRP — Single Responsibility Principle

> "A module should have one, and only one, reason to change." — R.C. Martin.
> Reframed: a module should be responsible to one **actor** (one source of
> change requests).

**Symptoms**
- A class that mixes policy (business rules) with detail (I/O, formatting, SQL).
- Methods that would change for unrelated reasons (a `User` class that validates,
  persists, and renders HTML).
- "Manager", "Helper", "Utils", "Service" classes that accrete unrelated methods.
- A class that imports both `System.Data.SqlClient` and a UI/serialization
  namespace.

**Detection heuristics**
- Count the distinct reasons-to-change: persistence, validation, formatting,
  orchestration, external calls. >1 in one class = candidate.
- High method count + low cohesion (methods use disjoint field subsets — LCOM, Lack of Cohesion of Methods).
- Class name is a vague noun ("Handler", "Processor").

**Inspection prompt**
> Examine each class in {scope}. For each, list the distinct *reasons it would
> change* (business rule, persistence, presentation, external integration,
> orchestration). Flag any class with more than one. Quote the methods/fields
> that belong to each responsibility and propose how to split them (Extract
> Class / Extract Service).

**Remediation:** Extract Class, Extract Method, move I/O to the edges, separate
policy from mechanism.

---

## 1.2 OCP — Open/Closed Principle

> "Software entities should be open for extension, but closed for modification."
> — Bertrand Meyer, popularized by Martin.

Adding a behavior should mean adding code, not editing existing tested code.

**Symptoms**
- Growing `switch`/`if-else` chains on a type code or enum that must be edited
  for every new variant.
- `if (type == "pdf") … else if (type == "csv") …` sprinkled in multiple places.
- Feature flags bolted into core logic instead of injected strategies.

**Detection heuristics**
- Find enums/`type` strings switched on in more than one location — each is an
  OCP hotspot and a duplication risk.
- Search for repeated `switch`/`case` over the same discriminator.

**Inspection prompt**
> Find every `switch`/`if-else` chain that branches on a type discriminator
> (enum, string kind, class name). For each, identify whether adding a new case
> requires editing existing code in multiple places. Recommend a polymorphic
> replacement (Strategy, Replace Conditional with Polymorphism, or a registry/
> factory).

**Remediation:** Strategy pattern, polymorphism, Replace Conditional with
Polymorphism, plugin/registry.

---

## 1.3 LSP — Liskov Substitution Principle

> Subtypes must be substitutable for their base types without breaking callers.
> — Barbara Liskov.

**Symptoms**
- Overrides that throw `NotImplementedException`/`NotSupportedException`.
- Subclasses that tighten preconditions or weaken postconditions.
- Callers that type-check (`if (x is SpecialType)`) to work around a subtype.
- The classic `Square : Rectangle` / `ReadOnlyList : List` breakage.

**Detection heuristics**
- Grep for `NotImplementedException`, `NotSupportedException` in overrides.
- Find `is`/`as`/`GetType()` checks in code that consumes a base type.

**Inspection prompt**
> Inspect inheritance hierarchies in {scope}. Flag any override that throws
> "not supported", narrows accepted inputs, or changes expected behavior such
> that a caller written against the base type could break. Flag caller-side type
> checks that betray a broken substitution. Recommend composition or interface
> segregation instead.

**Remediation:** favor composition over inheritance; segregate interfaces so no
implementer is forced to fake a member.

---

## 1.4 ISP — Interface Segregation Principle

> No client should be forced to depend on methods it does not use. Prefer many
> small, role-specific interfaces over one fat interface.

**Symptoms**
- "God interfaces" with 10+ members; implementers stub half of them.
- Consumers that use only one method of a large injected interface.
- `IRepository` with 20 methods where each caller needs one or two.

**Detection heuristics**
- Interfaces with many members; check how many each implementer/consumer
  actually uses.

**Inspection prompt**
> List interfaces with more than ~4 members. For each, determine which members
> each implementer actually implements meaningfully and which each consumer
> actually calls. Flag fat interfaces and propose role-based splits.

**Remediation:** split into role interfaces; consumers depend only on what they
call.

---

## 1.5 DIP — Dependency Inversion Principle

> High-level modules should not depend on low-level modules; both depend on
> abstractions. Abstractions should not depend on details; details depend on
> abstractions.

This is the principle that *enables* Onion/Clean Architecture (Module 2/3).

**Symptoms**
- Business logic that `new`s up a `SqlConnection`, `HttpClient`, `File`, or
  `DateTime.Now` directly.
- Domain classes referencing infrastructure namespaces.
- No interfaces at module boundaries; concrete dependencies everywhere.
- Static/global singletons reached into from core logic.

**Detection heuristics**
- In core/domain code, grep for `new` of infrastructure types, `DateTime.Now`,
  `Console`, `File.`, direct DB/HTTP client construction, static service
  locators.
- Check whether the composition happens at a single root (Main/Startup) or is
  scattered.

**Inspection prompt**
> In the core/business layer of {scope}, find every direct dependency on a
> concrete low-level detail: database clients, HTTP clients, file system, system
> clock (`DateTime.Now`), environment/config access, static singletons. For each,
> confirm whether an abstraction (interface) sits between the policy and the
> detail, and whether the concrete is supplied via constructor injection from a
> composition root. Flag every inward dependency on a detail and propose the
> interface + injection.

**Remediation:** define interfaces owned by the core; inject concretes from a
composition root; wrap the clock, config, and I/O.
