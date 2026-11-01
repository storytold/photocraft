# Module 6 — Persistence Ignorance, IoC & Low-Ceremony Design (Jeremy D. Miller)

**Jeremy D. Miller** — creator of StructureMap (the first mature .NET IoC
container), Lamar, Marten, and Wolverine; long-running "Patterns in Practice"
(MSDN) and "The Shade Tree Developer" writings. His throughline: **push
infrastructure to the edges, keep the core persistence-ignorant, compose the
system explicitly, and relentlessly cut ceremony.**

## 6.1 Inversion of Control / Dependency Injection done right

- Dependencies are **injected** (constructor) and wired at a single
  **composition root** — not resolved via a static Service Locator scattered
  through the code (Service Locator is a widely-recognized anti-pattern when used
  as a global reach-through — see Mark Seemann, *Dependency Injection in .NET*).
- **Smells:** `container.GetInstance<T>()` / `ServiceLocator.Current` sprinkled
  in business code; `new`-ing dependencies inside methods; static singletons;
  no single place where wiring happens; over-registration of things that are
  never varied.

**Inspection prompt**
> Find every place a dependency is obtained other than via constructor
> injection: static service locators, container `Resolve/GetInstance` calls in
> business code, `new` of a collaborator with behavior, static/global
> singletons. Confirm whether wiring is centralized in one composition root.
> Report each service-locator reach-through and propose constructor injection.

## 6.2 Persistence Ignorance (POCO domain)

- The domain model should be plain objects unaware of how they are stored — no
  base class from the ORM, no persistence attributes driving the model's shape,
  no `Save()` on entities. (Miller's Marten deliberately stores POCOs as
  documents to preserve this.)
- **Smells:** entities inheriting an ORM base type; `[Table]`/`[Key]`/`[Column]`
  as the *primary* definition of the model; active-record `entity.Save()`;
  lazy-loading proxies leaking into domain logic.

**Inspection prompt**
> Determine whether the domain model is persistence-ignorant: are entities plain
> objects, or do they inherit ORM base classes / carry persistence attributes /
> expose `Save()`/`Load()` themselves? Quote offending types and propose a POCO
> model with mapping pushed to the infrastructure edge.

## 6.3 Low Ceremony / minimal abstraction

- Prefer the simplest thing that works; avoid layers of indirection that add no
  behavior. Miller is a critic of "enterprisey" ceremony — needless interfaces
  with one implementation, deep inheritance, and abstraction that exists only to
  satisfy dogma.
- **Smells:** an interface per class with exactly one implementation and no test
  or substitution need; anemic pass-through layers; heavy generic base classes;
  configuration ceremony that dwarfs the logic.
- **Balance note:** this is in tension with "always program to an interface."
  The rule is *abstract at real seams* (I/O, external systems, things you swap
  or fake in tests) — not everywhere reflexively.

**Inspection prompt**
> Find ceremony that adds indirection without behavior: interfaces with a single
> implementation that is never faked in tests or swapped, pass-through wrapper
> classes, and deep base-class hierarchies. Distinguish genuine seams (I/O,
> externals — keep them) from dogmatic abstraction (collapse it). Recommend
> inlining/collapsing where the abstraction earns nothing.

## 6.4 Command/Query Separation (CQS)

CQS is **Bertrand Meyer's** principle (*Object-Oriented Software Construction*),
championed in .NET practice by Miller and by Fowler.

- A method either **does** something (command, returns void, has side effects)
  or **answers** something (query, returns data, no side effects) — not both.
- **Smells:** `GetOrCreateX()`, queries that mutate state, properties with side
  effects, methods returning a value *and* writing to the DB.

**Inspection prompt**
> Find methods that both return data and cause side effects (mutation, I/O),
> and query-named methods/properties that mutate. Report each CQS violation and
> propose splitting into a command and a query.

**Remediation stance (all of Module 6):** one composition root; POCO domain with
mapping at the edge; interfaces only at real seams; commands and queries kept
separate; delete ceremony that buys nothing.
