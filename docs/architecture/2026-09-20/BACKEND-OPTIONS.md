# Backend options for ME

Date: 2026-09-20. Status: Rust + Axum + Tokio selected by the user. The comparison
below records the reasoning; the backend is not yet implemented. SQLx, hosting,
identity, and other supporting components remain recommendations to finalize.
Read [the account and sync decision](ACCOUNTS-SYNC-AND-LOGINS.md) for requirements
and [the proposed repository/release model](REPOSITORY-AND-RELEASES.md) for organization.

## Selected direction and rationale

Rust + Axum + Tokio is selected for ME's core account/device/sync API. SQLx and
PostgreSQL are the proposed data-access stack under the priorities of resource efficiency,
performance, and alignment with the existing Rust application. Laravel remains a
credible choice when ready-made account, billing, notification, and administration
workflows are the dominant delivery priority. ASP.NET Core is a strong alternative
when a broad integrated backend framework is preferred alongside high performance.

This is an architectural judgment, not a measured speed ranking. No ME backend
benchmark has been run. AI-assisted coding changes typing cost; it does not remove
dependency integration, protocol design, security review, upgrades, or operations.

## Workload that matters

The cloud service authenticates accounts, authorizes enrolled devices, stores
opaque encrypted changes, delivers updates, coordinates file transfers, enforces
quotas, handles entitlements, and records content-free operational diagnostics.
It does not decrypt the personal vault, run semantic queries over plaintext
logins, or receive passwords for TypeSafe matching.

Expect substantial database/network waiting and reconnect bursts. CPU-heavy
vault encryption and local search stay on clients. The actual limiting resources
must be measured. Efficient queries, bounded batches, direct encrypted object
transfers, indexes, database pools, and durable retry behavior may matter more
to perceived speed than small HTTP framework differences.

Local cached autofill should not wait for an online API call on each use. Server
performance should be assessed through synchronization latency, tail latency,
recovery, and operating cost, not a hello-world requests-per-second chart.

## Candidates

| Stack | Strengths for ME | Costs and tradeoffs | Assessment |
| --- | --- | --- | --- |
| Rust + Axum/Tokio | Explicit typed request handling, concurrency control, no tracing garbage collector, existing Rust toolchain, potential shared protocol code | Axum is an HTTP foundation, not a complete Laravel-like product framework. Identity integration, admin, billing, and job operations require deliberate component selection. Build times and integration complexity remain costs. | Selected core API stack under the current priorities. |
| Laravel + Octane | Integrated conventions for accounts, validation, database work, jobs, notifications, and billing; existing operator familiarity | Additional PHP stack; Octane workers retain application state and need disciplined request isolation. Runtime and memory costs must be measured on the real workload. | Strong choice for a product centered on business/admin workflows; not excluded by performance assumptions. |
| Java + Spring Boot | Extensive security/data ecosystem, production diagnostics, established conventions, virtual-thread option for I/O concurrency | Adds JVM/build/deployment knowledge; framework surface and resource use need measurement. Native-image builds add their own constraints. | Appropriate if enterprise integrations and established Java operations become leading needs. |
| C# + ASP.NET Core | Integrated auth/authorization, dependency injection, diagnostics, and a modular high-performance HTTP server | Additional .NET stack and no direct reuse of Rust protocol implementation without an interop boundary or generated contracts | Strong balance of framework completeness and performance; serious alternative to Rust. |
| Go + standard HTTP stack | Straightforward concurrent services and lightweight goroutines; comparatively simple language/runtime model | Account/product workflows still require choosing components; another implementation of shared logic | Good operationally simple service option; weaker reuse argument than Rust in this repository. |

The tradeoffs above are judgments about fit. None of these frameworks supplies
ME's end-to-end encryption, device enrollment, authorization policy, conflict
semantics, or recovery protocol automatically. Memory safety does not establish
correct authorization or prevent credential disclosure.

## Proposed Rust deployment shape

- One backend codebase with separate internal modules for accounts, devices,
  sync, object access, and entitlements. Start as a modular monolith.
- Axum/Tokio for HTTP/concurrency; Tower middleware for limits, timeouts, and
  tracing. SQLx for explicit PostgreSQL transactions and migrations.
- Managed PostgreSQL and S3-compatible object storage. Authorize bounded,
  short-lived object transfers; encrypted uploads must satisfy quotas and
  integrity/completion checks before being referenced as available.
- A durable job mechanism with retries and idempotency for email, notifications,
  and housekeeping; worker processes can share the application codebase.
  Select a maintained component rather than inventing queue semantics casually.
- An established identity implementation/provider using standard flows suitable
  for public desktop/mobile/browser clients. Authentication does not substitute
  for ME's own device authorization or vault-key enrollment.
- Container deployment, automated database migrations, backups with restore
  drills, and metadata-only metrics/traces. Hosting provider/region stays open.
- Add caches, brokers, or independently deployed services when measurements or
  reliability requirements justify them. Begin with one authoritative database
  for accounts and entitlements rather than a split Laravel/Rust identity system.

Potential Rust reuse should come from small portable crates for protocol models,
revision validation, and compatibility tests. The existing SQLCipher-bound vault
crate is not automatically a browser/iPhone/server library, and the backend must
not gain client unlocking keys or decryption responsibilities through reuse.

## Validation before production

Define representative sync request sizes, reconnect bursts, expected concurrent
devices, storage growth, and operating budget. Build an authenticated vertical
slice with tenant/device authorization, encrypted batched writes, cursor-based
reads, duplicate suppression, and conflict preservation.

Measure p50/p95/p99 latency, completed operations per unit cost, CPU/RSS, database
pool saturation, backlog recovery, and behavior through restarts/retries. Include
the database and authentication path; keep infrastructure and semantics comparable
if another stack is benchmarked. Do not relax durability or authorization to
make a benchmark faster.

Rust + Axum + Tokio is now selected. Implement the representative vertical slice
in this stack and validate its suitability. Revisit the choice only if evidence
shows a material mismatch; implementing five complete backends is unnecessary.

## Primary sources checked

- [Axum: request handling and Tower/Tokio integration](https://docs.rs/axum/latest/axum/).
- [SQLx: database APIs](https://docs.rs/sqlx/latest/sqlx/).
- [Laravel authentication](https://laravel.com/framework/docs/13.x/authentication),
  [queues](https://laravel.com/framework/docs/13.x/queues),
  [Cashier billing](https://laravel.com/framework/docs/13.x/billing), and
  [Octane worker behavior](https://laravel.com/framework/docs/13.x/octane).
- [Spring Boot production features](https://spring.io/projects/spring-boot/),
  [Spring Security](https://docs.spring.io/spring-security/reference/index.html),
  and [virtual threads](https://docs.spring.io/spring-boot/reference/features/spring-application.html#features.spring-application.virtual-threads).
- [ASP.NET Core overview](https://learn.microsoft.com/en-us/aspnet/core/overview?view=aspnetcore-10.0).
- [Go concurrency](https://go.dev/doc/effective_go#concurrency).
