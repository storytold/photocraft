# Module 2 — Onion Architecture

Coined by **Jeffrey Palermo** (2008, "The Onion Architecture" series). A layered
model where **all dependencies point inward toward a domain core** that knows
nothing of infrastructure. Concentric rings:

```
        ┌─────────────────────────────────────────┐
        │  Infrastructure / UI / Tests (outer)      │
        │   ┌───────────────────────────────────┐   │
        │   │  Application Services              │   │
        │   │   ┌───────────────────────────┐    │   │
        │   │   │  Domain Services           │    │   │
        │   │   │   ┌───────────────────┐    │    │   │
        │   │   │   │  Domain Model     │    │    │   │
        │   │   │   │  (entities, core) │    │    │   │
        │   │   │   └───────────────────┘    │    │   │
        │   │   └───────────────────────────┘    │   │
        │   └───────────────────────────────────┘   │
        └───────────────────────────────────────────┘
     Dependencies point INWARD only. Interfaces live in the core;
     implementations live on the outer edge.
```

**Core tenets (Palermo)**
1. The application is built around an **independent object model** (the domain).
2. **Inner layers define interfaces; outer layers implement them.** The domain
   declares `IRepository`, `IEmailSender`, `IClock`; Infrastructure implements.
3. **All coupling is toward the center.** Nothing in the core references
   Infrastructure, the database, the UI, or a framework.
4. The **database is an infrastructure detail** at the outermost ring, plugged
   in — not the center of the application (a rejection of data-centric,
   database-first design).
5. Externals (UI, persistence, messaging, third parties) are all peers on the
   outer edge and are swappable.

**Symptoms of violation**
- The domain project references EF/ORM, `System.Data`, a web framework, or a
  specific database package.
- Entities carry ORM attributes, `[Table]`/`[Column]`, or serialization
  attributes as their primary shape.
- Business rules living in controllers, page code-behind, or stored procedures.
- No separate domain project at all — everything in one assembly, or a
  "layering" that is really just folders with no enforced dependency direction.
- Repository/service *interfaces* defined in the Infrastructure project rather
  than in the core.
- Data model (rows/DTOs) used as the domain model (see Anemic Domain, Module 5).

**Detection heuristics**
- Inspect project/assembly references (`.csproj`, module import graph). Draw the
  dependency arrows. Any arrow from an inner layer to an outer one is a
  violation.
- Grep the domain project for infrastructure/framework namespaces.
- Locate where interfaces are declared vs implemented — interfaces belong inside.

**Inspection prompt**
> Reconstruct the layer/dependency graph for {scope} from project references and
> import statements. Identify the domain core (entities + domain interfaces),
> application services, and infrastructure. Verify that **every dependency
> points inward**: no inner layer may reference an outer one, and no domain code
> may reference a database, ORM, web framework, or external SDK. Report each
> inward-pointing interface that is instead declared in an outer layer, each
> framework/persistence reference that has leaked into the core, and each
> business rule that lives in a controller/UI/stored procedure. Produce an ASCII
> dependency diagram and a numbered list of arrow violations.

**Remediation**
- Extract a dependency-free domain project; move entities and domain interfaces
  into it.
- Move persistence/HTTP/file implementations to an Infrastructure project that
  references the core (not vice versa).
- Introduce a composition root (Startup/Program) that wires implementations to
  the core's interfaces (ties to DIP, Module 1.5, and IoC, Module 6).
- Push business logic out of controllers into application/domain services.

**Relationship to Clean Architecture (Module 3):** same dependency rule, same
"database/UI are details" stance, expressed with different ring names. Uncle
Bob's Clean Architecture (2012+) generalizes Onion, Hexagonal (Ports &
Adapters, Cockburn), and DCI.
