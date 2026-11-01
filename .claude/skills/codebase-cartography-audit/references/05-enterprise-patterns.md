# Module 5 — Enterprise & Domain Patterns

From **Martin Fowler**, *Patterns of Enterprise Application Architecture* (PoEAA)
and **Eric Evans**, *Domain-Driven Design*. These describe how domain logic and
persistence should be organized — the layer where inherited/AI code most often
degenerates into procedural spaghetti or an anemic shell around a database.

## 5.1 Domain Model vs Transaction Script

- **Transaction Script** — organizes logic as a procedure per request. Fine for
  simple apps; collapses under growing business complexity (duplication,
  no reuse of rules).
- **Domain Model** — an object model where behavior and data live together.
  Preferred as complexity grows.
- **Smell:** a "Service" layer holding all logic while entities are bags of
  getters/setters = Transaction Script masquerading as OO.

## 5.2 Anemic Domain Model (Fowler — an anti-pattern)

> Objects that carry data but no behavior, with all logic in separate service
> classes. It has the *cost* of a domain model (mapping, object graph) with none
> of the *benefit* (encapsulated behavior). Fowler explicitly calls it an
> anti-pattern.

**Symptoms:** entities are all public get/set with zero invariants; validation
and rules live in `XxxService`/`XxxManager`; no method on the entity enforces
its own consistency. **This is the single most common shape of AI-generated
"domain" code.**

**Fix:** move behavior onto the entities; make invalid states unrepresentable;
use value objects; keep services thin (orchestration only).

## 5.3 Repository & Unit of Work

- **Repository** — a collection-like abstraction over persistence; the domain
  talks to `IRepository`, not to SQL/ORM. Interface belongs in the core
  (ties to Onion, Module 2).
- **Unit of Work** — tracks changes and commits them atomically.
- **Smells:** repositories that leak `IQueryable`/ORM types to callers; a
  "repository" that is just a thin passthrough with a method per query (a
  generic `IRepository<T>` that adds nothing); business logic inside repository
  methods; direct DB access bypassing the repository in some places.

## 5.4 Service Layer / Application Services

- Thin boundary defining the app's operations; orchestrates domain objects and
  transactions. **Smell:** "fat services" that contain the business rules that
  belong on the domain (see 5.2).

## 5.5 Value Objects & Domain Primitives

- Wrap primitives that have rules (Money, EmailAddress, DateRange, Quantity).
  Counters Primitive Obsession (Module 4). Immutable, equality by value,
  self-validating.

**Detection heuristics**
- Ratio of behavior to data on "entities" (methods vs. plain properties).
- Location of validation/business rules (entity vs. service).
- Repository interfaces returning ORM/`IQueryable` types.
- Money/dates/ids passed as raw `decimal`/`string`/`DateTime`.

**Inspection prompt**
> Assess the domain layer of {scope}. (1) Classify it as Domain Model or
> Transaction Script, and specifically flag Anemic Domain Model: quote entities
> that are pure get/set and show where their business rules actually live.
> (2) Check the persistence abstraction: are Repository/Unit-of-Work present, do
> their interfaces live in the core, and do they leak ORM/`IQueryable` types?
> (3) Find Primitive Obsession — money, dates, ids, emails handled as raw
> primitives — and propose Value Objects. For each finding cite file:line and
> give the pattern-named remediation.

## Aggregates & consistency boundaries (Evans)

- **Aggregate / aggregate root (Evans, DDD):** a cluster of entities/value objects
  treated as one consistency unit, mutated only through its root, which enforces
  the invariants. **Symptom of absence:** entities mutated from anywhere (handlers,
  UI, mappers) with no root guarding invariants — the structural cause of the
  Anemic Domain smell above. Related: Ubiquitous Language (code names match the
  domain) and Bounded Contexts (a model is valid within one context, not global).
- **Detection:** child entities created/edited without going through a root;
  invariants (totals, state transitions, membership rules) enforced in services
  instead of the aggregate; one giant shared model spanning unrelated concerns.

**Remediation:** relocate rules onto entities and aggregate roots; introduce value
objects; define repository interfaces in the domain (one per aggregate root) and
keep them collection-like; keep services orchestration-only.
