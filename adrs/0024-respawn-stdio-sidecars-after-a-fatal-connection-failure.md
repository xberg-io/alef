---
status: accepted
date: 2026-11-08
decision-makers:
  - Alef maintainers
amends: ["0017"]
---

# Respawn stdio sidecars after a fatal connection failure

## Context and Problem Statement

ADR-0017 gives a stdio connection no second chance: a timeout, a broken frame or a dead child kills and reaps the child and invalidates the connection, and every later call fails with `SidecarStopped` until the host exits. Hosts that call a long-lived sidecar from a thread pool (poly's Go `sh` component, called from a rayon pool) lose the component for the rest of the process because of one slow or crashed call, and they cannot restart the child themselves without reimplementing the manager's verified install and spawn path.

## Decision Drivers

* One slow or crashing call must not stop a sidecar for the rest of the host process.
* A host must not reimplement the verified install and spawn path to recover.
* A host's session (an `Init` call, per-config registrations) must survive a respawn.
* A permanently broken executable must not cycle start-and-fail rounds forever.
* Hosts must be able to observe that a connection was rebuilt.

## Considered Options

* Respawn inside the runtime from the already-verified executable, within a per-component budget, replaying describe validation and a host-provided session setup.
* Expose a public spawn API so the host replaces the process itself.
* Keep the permanent stop; hosts that need recovery create a new `ComponentManager`.

## Decision Outcome

Chosen option: "Respawn inside the runtime", because recovery needs exactly the verified artifact and spawn path the manager already owns, and only the runtime can replay it without duplication or a wider public API.

`SidecarProcess` stores its executable, manifest and policy. After a fatal error stops a connection, the next gated call respawns it: kill and reap the old child, start a fresh one, re-run the describe validation against the verified manifest, and replay the host's session setup (`ComponentManager::with_sidecar_session`), which also runs after the first spawn. The call that failed still returns its error; the respawn happens on a later call, which is the first one the new child serves. At most `with_sidecar_restart_budget` respawn *attempts* are made per component per manager (default 1; 0 restores the permanent stop), so a broken executable cannot loop: failed attempts, including a failed session setup, consume the budget. Once the budget is exhausted the connection stays stopped, as ADR-0017 provides. Calls stay serialized: a respawn waits for the sender guard an in-flight call holds, and nothing is dispatched to the new child before its session setup has finished. Restarts are observable through `with_sidecar_restart_event` and `sidecar_restart_count`; status reporting is passive and never respawns or consumes the budget. A session setup must send its calls through the `SidecarSession` it is given; calling back into the manager would re-enter the respawn it is part of.

This amends ADR-0017: the transport, framing, handshake validation and trust model are unchanged, but a fatal error invalidates the connection only until the budget allows a respawn.

### Consequences

* Good, because a single slow or crashed call no longer disables a sidecar for the rest of the process run.
* Good, because the host keeps one API surface: no restart, respawn or re-verification code leaves the runtime.
* Bad, because the session setup is replayed on every respawn, so a host whose setup is not safe to repeat gets repeated side effects on the child.
* Bad, because a respawn attempt that fails (a broken executable, a failing setup) consumes the budget even if the failure was transient.
* Bad, because a restart handler or session setup that bypasses `SidecarSession` can deadlock on the respawn mutex.

### Confirmation

`crates/alef-component-runtime/tests/stdio_sidecar.rs` drives the acceptance behavior over a scripted child: `should_respawn_after_a_crashed_call_and_replay_the_session_setup` (first call errors, second succeeds on a fresh child, setup ran on both), `should_reap_a_timed_out_child_and_serve_the_next_call_from_a_fresh_one`, `should_stop_after_the_restart_budget_is_exhausted`, `should_consume_the_budget_when_a_respawn_fails_and_fire_no_event` (attempts consume the budget; failed respawns fire no event), `should_report_restarts_through_the_event_callback` and `should_report_status_without_respawning_a_stopped_connection` (status is passive). The no-dispatch-before-setup invariant is enforced by the gated re-check in `SidecarProcess::exchange_gated`.
