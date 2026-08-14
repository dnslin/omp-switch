## Agent skills

### Issue tracker

Issues are tracked in GitHub Issues using the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Triage uses the canonical labels `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, and `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Domain documentation uses the single-context layout. See `docs/agents/domain.md`.


## Architecture Design

- Do not add compatibility layers, fallback paths, or migrations for unconfirmed
  compatibility requirements. If the task involves public APIs, persisted data,
  external consumers, or rolling deployments, first identify the actual
  compatibility constraints. If none exist, remove obsolete paths directly.

- Choose the simplest implementation that fully satisfies the current
  requirements and acceptance criteria. Avoid speculative abstractions,
  configuration, extension points, and indirection.

- Grow the system in layers. Start from the smallest version that works
  end to end, and add each new capability on top of a product that already
  works. Never trade a working product for unfinished complexity.

- Keep components modular and concerns clearly separated.

- Introduce an abstraction only when it solves a concrete problem already
  present in the code, such as repeated behavior or multiple real
  implementations.

- Prefer established, well-maintained libraries when they reduce overall
  complexity or improve reliability. Do not reimplement common functionality
  without a clear reason.

- Lean on dependencies already present in the project before writing a custom
  implementation or adding new packages. Do not assume a library lacks a
  capability without checking its documentation, types, and existing usage
  in the codebase.

- For module boundaries, data models, and dependency directions that must be
  decided now, choose designs that can remain valid long term. Do not accept
  stopgap implementations that are knowingly meant to be replaced later, but
  do not build extension frameworks for hypothetical future requirements.

- Before designing a new product interaction, public interface, protocol, or
  architectural pattern, study how established products solve the same
  problem. Prefer proven patterns and conventions over inventing an approach
  from scratch. Do not perform unrelated research for local fixes or problems
  already covered by clear project conventions.

- Simplicity must not come at the expense of correctness, security,
  testability, or explicitly required operational behavior.


## Working Method

- Before modifying code, read the relevant implementation, tests, type
  definitions, configuration, and call paths. Do not start implementing based
  only on file names, isolated snippets, or assumptions.

- Follow the project's existing directory structure, naming conventions,
  error-handling patterns, and testing conventions. Introduce a new convention
  only when the existing ones cannot satisfy the requirement.

- Change only the code required to complete the current task. Do not
  opportunistically refactor unrelated modules, rename unrelated symbols,
  reformat unrelated files, or solve problems outside the requested scope.

- If you discover a problem outside the task scope, report the problem and its
  impact, but do not modify it unless it blocks the requested work.

- When a requirement is ambiguous, first determine whether the ambiguity affects
  external behavior, persisted data, public interfaces, or architectural
  boundaries. Ask the user when the decision has meaningful consequences.
  Otherwise, use the smallest reasonable assumption and state it explicitly.


## Verification

- When behavior changes, add or update tests that verify the changed behavior.
  Prefer testing externally observable behavior over internal implementation
  details.

- After making changes, run the tests, type checks, static analysis, and build
  commands directly relevant to the modification.

- Never claim that tests pass, the build succeeds, or an issue is fixed unless
  the corresponding verification was actually run.

- Report the commands that were actually run, their results, and anything that
  remains unverified.
:
- Do not make checks pass by hardcoding test data, bypassing validation,
  weakening assertions, suppressing errors, or deleting failing tests.

- Do not swallow errors or use silent fallbacks to hide failures. Preserve
  enough error context for the problem to be diagnosed.


## Communication Style

When explaining work to the user:

- Use natural, direct Chinese by default.

- Give the conclusion first, followed by the reasons and relevant details.

- Do not explain one abstract concept using another abstract concept.

- Keep each sentence focused on one main judgment whenever possible.

- Keep each paragraph focused on one purpose.

- When introducing a technical term for the first time, immediately explain it
  in plain Chinese.

- Prefer concrete examples involving files, commands, data flows, or operations
  over purely theoretical explanations.

- Do not repeat context merely for completeness.

- Do not expand the user's request without a clear reason.

- When a process is complex, clearly explain:
  1. what step is currently being performed;
  2. why this step is necessary;
  3. what result this step will produce;
  4. what the user needs to do next.

- Unless explicitly requested, avoid stiff academic language, marketing
  language, and unnatural translated phrasing.
