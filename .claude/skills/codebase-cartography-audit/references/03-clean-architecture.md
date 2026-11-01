# Module 3 — Clean Architecture & Component Rules

**Robert C. Martin**, *Clean Architecture* (2017). Adds two things beyond Onion:
the **Dependency Rule** stated crisply, and a set of **component-level**
principles for organizing code into deployable/releasable units.

## 3.1 The Dependency Rule

> Source-code dependencies must point only **inward**, toward higher-level
> policy. Nothing in an inner circle can name anything in an outer circle.

Rings (outer→inner): Frameworks & Drivers → Interface Adapters
(controllers, presenters, gateways) → Use Cases (application-specific business
rules) → Entities (enterprise-wide business rules). Data crossing a boundary
inward is a simple structure the inner layer owns — never an entity/row/framework
object passed outward-in.

## 3.2 Screaming Architecture

> The top-level structure should "scream" the **domain** (Billing, Ordering,
> Underwriting), not the framework (Controllers, Models, Views).

**Symptom:** the folder tree tells you it's an MVC/React app but not what the
business does. AI scaffolds are almost always framework-screaming.

## 3.3 Component Cohesion (which classes belong together)

- **REP — Reuse/Release Equivalence:** the unit of reuse is the unit of release;
  a component should be independently versionable.
- **CCP — Common Closure:** classes that change together (for the same reason)
  belong in the same component. (Component-level SRP.)
- **CRP — Common Reuse:** classes used together belong together; don't force
  consumers to depend on things they don't use. (Component-level ISP.)

## 3.4 Component Coupling (relationships between components)

- **ADP — Acyclic Dependencies Principle:** no cycles in the component
  dependency graph. Cycles make independent build/test/release impossible.
- **SDP — Stable Dependencies Principle:** depend in the direction of stability;
  volatile components should depend on stable ones, not the reverse.
- **SAP — Stable Abstractions Principle:** a stable component should be abstract
  (interfaces) so it can be extended without modification. Stable + concrete =
  the "zone of pain."

**Detection heuristics**
- Build the component/assembly dependency graph; run a cycle check (ADP).
- Look for a stable core that is concrete rather than abstract (SAP violation).
- Check whether things that change together are scattered across components
  (CCP violation) — a single feature change touching many projects is the tell.

**Inspection prompt**
> For {scope}: (1) State whether the top-level structure screams the domain or
> the framework. (2) Build the component dependency graph and detect any cycles
> (ADP). (3) Identify the most stable (most-depended-upon) components and check
> whether they are abstract (SAP) or concrete "zone of pain". (4) Find features
> whose changes force edits across many components (CCP violation) and DTOs/
> entities crossing boundaries in the wrong direction (Dependency Rule). Report
> violations with the dependency arrows and a proposed re-grouping.

**Remediation:** re-group by feature/domain; break cycles with Dependency
Inversion (introduce an interface owned by the stable side); extract abstract
interface packages for stable cores; align folders to business capabilities.
