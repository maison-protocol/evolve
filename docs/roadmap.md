<img src="../assets/logo/evolve-mark.svg" width="72" align="right" alt="Evolve logo"/>

# 🗺️ Evolve: Development Roadmap and Possibilities

> **Status:** planning draft (November 2026). Dates are targets for a small core team (3–5 people) and will move as we learn. Each phase ships something people can use and ends with a demo that proves a hypothesis.

---

## Contents

1. [Strategy: engine first, language second](#1-strategy-engine-first-language-second)
2. [Phase plan](#2-phase-plan) · [Mission track](#-mission-track-ai-models-ai-native-apps-and-agent-systems)
3. [Release milestones](#3-release-milestones)
4. [EvoBench](#4-evobench)
5. [Success metrics](#5-success-metrics)
6. [Risks and mitigations](#6-risks-and-mitigations)
7. [Rust beta policy](#7-rust-beta-policy)
8. [Team and governance](#8-team-and-governance)
9. [Possibilities: where Evolve could go](#9-possibilities-where-evolve-could-go)
10. [How to help right now](#10-how-to-help-right-now)

---

## 1. Strategy: engine first, language second

Building a new language takes years. Validating the *idea* shouldn't. So development runs on **two tracks** that converge:

```
              2027 Q1        Q2          Q3          Q4        2028 Q1      Q2        H2
Track A  ───[ Engine MVP ]──[ Py bindings ]──┐
 Engine      fossil · fitness · mutation     │
                                             ├──[ Converge ]──[ JIT + sandbox ]──[ Agents ]──[ 1.0 ]
Track B  ───[ Parser + interpreter ]──[ Data-native core ]─┘
 Language
```

- **Track A, the Evolution Engine**, is a set of Rust crates (`evo-fossil`, `evo-search`, `evo-sandbox`) with **Python bindings**. Python developers can use `@gene`, `fitness`, and Lessons *before the Evolve language exists*. This tests the core hypothesis with real users and real failures early.
- **Track B, the Language**, covers the parser, interpreter, type system, and data-native core. It's slower and more ambitious.
- **Convergence (Phase 3):** the language uses the same engine natively, so `gene def` in Evolve and `@gene` in Python share one fossil record and one lineage format.

**Why this order:** if the evolution loop doesn't help real users in Python, a new syntax won't save it. If it does help, we'll have users, data, and a benchmark before the compiler is finished.

---

## 2. Phase plan

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../assets/diagrams/roadmap-dark.svg">
  <img alt="Roadmap timeline from Seed v0.0 (Feb 2027) to Forest v1.0 (H2 2028)" src="../assets/diagrams/roadmap-light.svg" width="100%">
</picture>

### Phase 0: Seed 🌱 (Nov 2026 – Feb 2027)
**Goal:** agree on what we're building.

| Deliverable | Detail |
|---|---|
| Vision, use cases, roadmap | ✅ this repo |
| RFC process | ✅ [`rfcs/`](../rfcs/) folder, template, statuses, review rules |
| Core RFCs | 🌱 drafted: [0001](../rfcs/0001-syntax-core.md) Syntax core · [0002](../rfcs/0002-lesson-schema.md) Lesson schema · [0003](../rfcs/0003-gene-fitness-semantics.md) `gene`/`fitness` semantics · [0004](../rfcs/0004-promotion-and-safety.md) Promotion & safety model · [0005](../rfcs/0005-data-model.md) Data model. Next: comment period |
| Grammar draft | 🌱 [Grammar revision 2](../spec/grammar.ebnf) (PEG notation, reviewed against the Python 3.16 reference) done; tree-sitter grammar for syntax highlighting still to do |
| EvoBench v0 spec | 🌱 [Case format v0 and scorer](../bench/README.md) done; 1 of 50 cases (see [EvoBench](#4-evobench)) |
| Project hygiene | ✅ Apache-2.0 license, ✅ Code of Conduct, ✅ CONTRIBUTING, ✅ [CI](../.github/workflows/ci.yml) on beta and stable (nightly informational) |

**Exit criteria:** the five core RFCs are accepted, and Lesson schema v0 is frozen for the engine to build on.

---

### Phase 1: Sprout 🌿 (Feb – Jun 2027) → `v0.1`
**Goal:** the loop works end to end, in Python first.

| Track A: Engine | Track B: Language |
|---|---|
| `evo-fossil`: append-only Lesson + lineage store (Parquet + SQLite index) | 🌱 `evo-syntax`: lexer + lossless CST (rowan), error-tolerant parser: **prototype done** against draft RFC 0001 revision 2 ([crate](../crates/evo-syntax/)) |
| `evo-search`: fitness runner, **parametric** mutation, selection, seeds | Tree-walking interpreter: Python-like core (functions, classes, control flow, f-strings) |
| `evo-engine` Python package (PyO3): `@gene`, `fitness`, `attempt/learn` context manager | `attempt / learn` with Lessons written to the same fossil store |
| CLI: `evo lessons`, `evo lineage`, `evo evolve`, `evo revert` | `evo run`, `evo repl` |

**Exit demo:** a **Python** pipeline using `evo-engine` hits a date-format drift, records a Lesson, evolves a parser parameter, and proposes a fix that a human approves. Separately, `.evo` scripts run with `attempt/learn`.

---

### Phase 2: Root 🌳 (May – Oct 2027) → `v0.2`
**Goal:** Evolve becomes data-native.

- N-dimensional arrays with broadcasting (`f.(xs)`, `.+`), slicing, and linear algebra (`A \ b`)
- **Multiple dispatch** with method tables (also the hook for evolved variants)
- Arrow-backed **DataFrames** and the pipe operator (`|>`), with `read"..."` / `write"..."` loaders for CSV, Parquet, and JSON
- **Units of measure** and **uncertainty** (`±`) as types, with propagation
- Gradual type checker (`evo-types`): optional hints, inference, dispatch resolution
- 🧠 **Tensors and automatic differentiation** on a Rust backend (candidates: Burn, Candle), the foundation for the AI-model mission
- Track A: **structural** mutation operators for Python ASTs (guards, alternate parses, type widening)

**Exit demo:** the [scientific computing use case](use-cases.md#4-scientific-computing-with-honest-uncertainty) runs in Evolve, with units and uncertainty flowing through correctly.

---

### Phase 3: Branch 🌲 (Sep 2027 – Jan 2028) → `v0.3`
**Goal:** the tracks converge, and evolution becomes native syntax.

- `gene def`, `fitness`, `invariant`, `population`, `@conserved`, `@heal` in the language
- Structural mutation directly on the Evolve CST, rendering back to idiomatic source
- Promotion workflow: `evo promote`, `evo diff`, and approval gates (`human` / `tests` / `auto`)
- Holdout sets and **adversarial examples generated from Lessons** to resist fitness gaming
- **EvoBench v1** published with baseline heal rates
- 🧩 `module` construct with typed interfaces, and `gene model` to evolve architectures on **small proxy models** first

**Exit demo:** the [self-healing data pipeline](use-cases.md#1-self-healing-data-pipelines) runs end to end in pure Evolve, with lineage, quarantine, and human promotion.

---

### Phase 4: Bloom 🌸 (Dec 2027 – Apr 2028) → `v0.4`
**Goal:** fast and safe enough for real services.

- **Cranelift JIT** with type specialization (Julia-style) for dispatch and numeric hot paths
- **WASM sandbox** (wasmtime) for every variant under evaluation
- **Capabilities** (`@capabilities(none | fs | net | clock)`) enforced at the sandbox boundary
- **Budgets** for generations, CPU, memory, wall time, and money/tokens
- `@heal` in long-running services, with **shadow populations** that run variants on a slice of real traffic and discard their outputs
- Lesson **PII redaction** on by default
- 🌍 **World-interaction capabilities** for models and agents (tools, sensors, actuators), sandboxed and budgeted
- 🛡️ Safety eval suites usable as hard `invariant`s (jailbreak, harmful-output, and robustness suites)

**Exit demo:** a running HTTP service heals a broken third-party adapter in a sandbox, under budget, without touching production until a human approves.

---

### Phase 5: Pollinate 🐝 (Mar – Aug 2028) → `v0.5`
**Goal:** Evolve becomes the best language for AI agents.

- `evo serve --mcp`: tools for `lessons.query`, `gene.mutate`, `fitness.run`, `lineage.show`, `promote.request`
- **Agentic mutation providers**: plug in hosted or local LLMs. Every prompt and response is recorded in the lineage for reproducibility.
- Recombination (crossover) between successful variants
- 🕸️ **`agent`, `protocol`, and `society`** constructs: typed, versioned, conserved contracts between agents; system-level fitness
- 🌐 **Open agent interoperability**: A2A-style agent-to-agent messaging alongside MCP
- 📱 **`app` and `gene prompt`** constructs for AI-native applications, with Lessons flowing into regression evals
- **Python interop** both ways (call Python libraries from Evolve, embed Evolve in Python)
- **LSP + VS Code extension** with a lineage tree view, Lesson explorer, and fitness diff
- Agent uplift study on EvoBench (see [metrics](#5-success-metrics))

**Exit demo:** an off-the-shelf coding agent connected over MCP fixes measurably more EvoBench cases, and repeats fewer mistakes, than the same agent working in plain Python.

---

### Phase 6: Forest 🌍 (H2 2028) → `v1.0`
**Goal:** a stable, documented, production-ready language.

- **LLVM AOT** backend for release binaries
- Package manager and registry (`evo add`, `evo publish`) with Rust crate interop
- Standard library written in Evolve (`std.data`, `std.units`, `std.http`, `std.test`, `std.evolve`)
- *The Evolve Book*, a language reference, and a cookbook
- Stability guarantees: language spec 1.0, Lesson and lineage formats versioned with migrations
- External security audit of the sandbox and promotion pipeline
- 🎯 Flagship examples: an AI-native app and a multi-agent society, both built entirely in Evolve

**Exit:** 1.0 released, with a public commitment to backward compatibility.

---

## 🎯 Mission track: AI models, AI-native apps, and agent systems

The phases above build the foundation. This track shows how each [mission](vision.md#-mission) grows on top of it.

| Mission | Foundation (Phases 1–3) | First usable (Phases 4–5) | Mature (post-1.0) |
|---|---|---|---|
| 🧠 **Next-gen AI models** | Tensors, autodiff, `module`, `gene model` on small proxy models | Safety suites as invariants, world-interaction capabilities, context `species` | Population-based training at scale, evolved architectures transferred from proxies to full models, safety-invariant certification |
| 📱 **AI-native apps** | Lessons, `attempt / learn`, prompt genes in the Python engine | `app` construct, typed tools, Lessons as regression evals | App templates, hosted evolution with team approval UI |
| 🕸️ **Agent-to-agent systems** | Typed messages, capabilities, lineage | `agent`, `protocol`, `society`; A2A + MCP interoperability | Large societies, cross-organization protocols, collusion-resistant system fitness |

---

## 3. Release milestones

| Version | Phase | Headline |
|---|---|---|
| `v0.0` | Seed | Docs, RFCs, grammar |
| `v0.1` | Sprout | Evolution engine for Python + Evolve interpreter |
| `v0.2` | Root | Arrays, dispatch, DataFrames, units, uncertainty |
| `v0.3` | Branch | Native `gene` / `fitness` / `population`, EvoBench v1 |
| `v0.4` | Bloom | JIT, WASM sandbox, capabilities, `@heal` in services |
| `v0.5` | Pollinate | MCP, agentic mutation, agents/protocols/societies, AI-native apps, Python interop, LSP |
| `v1.0` | Forest | AOT, package manager, stdlib, stability |

---

## 4. EvoBench

**A public benchmark of real-world software failures and how well a system heals them.** It's useful beyond Evolve: any agent or repair tool can be scored on it.

Each case contains:
- a small program that used to work,
- an **environment change** that breaks it (schema drift, API change, new edge case, dependency upgrade, workload shift),
- visible examples (for fitness) and a **hidden holdout** set (for honest scoring),
- invariants that must never be violated.

| Category | Example |
|---|---|
| Schema drift | Column renamed, type changed, new nullable field |
| Format drift | Date, number, or currency formats from new locales |
| API contract change | Field moved, enum value added, pagination changed |
| Edge cases | Empty input, Unicode, huge values, time zones |
| Dependency upgrade | Deprecated function removed, default changed |
| Performance drift | Workload grows 10×; heuristic needs retuning |

Targets: 50 cases (Phase 0), 300 (Phase 3), 1,000 (Phase 5), with community contributions.

---

## 5. Success metrics

| Metric | Definition | Target by 1.0 |
|---|---|---|
| **Heal rate** | % of EvoBench cases where a promoted variant passes the hidden holdout | > 60% |
| **False promotion rate** | % of promoted variants that later regress or violate an invariant | < 1% |
| **Mean time to heal** | From first Lesson to an approval-ready variant | < 10 min for parametric, < 1 h for structural |
| **Agent uplift** | Same agent's heal rate in Evolve vs. plain Python | Statistically significant improvement |
| **Repeat-mistake rate** | How often an agent re-introduces a failure kind already in its Lessons | Falls over time within a project |
| **Numeric performance** | Evolve vs. Julia on a standard numeric benchmark suite | Within 2× |
| **Unapproved prod changes** | With default settings | Exactly 0 |

---

## 6. Risks and mitigations

| Risk | Likelihood | Mitigation |
|---|---|---|
| **Scope too large** for a small team | High | Engine-first strategy; each phase ships standalone value; no feature joins the plan without an RFC |
| **Fitness gaming / overfitting** (Goodhart's law) | High | Hidden holdouts, adversarial examples from Lessons, invariants, human gates, false-promotion tracking |
| **Security of self-modifying code** | Medium | WASM sandbox, capabilities, `@conserved`, budgets, no auto-promotion by default, external audit before 1.0 |
| **Performance of a dynamic language** | Medium | Julia-style type specialization in the JIT; AOT for release; Rust FFI for hot spots |
| **LLM cost and nondeterminism** | Medium | Parametric and structural mutation first (no LLM needed); cache and record agent proposals; local-model support |
| **Rust beta churn** | Low–Medium | See [Rust beta policy](#7-rust-beta-policy) |
| **Evolution optimizing around safety** (models or agents finding loopholes in safety evals) | Medium | Safety modules are `@conserved`; safety suites are hard invariants, not score terms; hidden red-team suites; human gates for every model promotion |
| **Compute cost at model scale** | High | Evolve small proxy models and adapters first; budgets on every run; transfer winning designs upward |
| **Adoption: "why learn a new language?"** | High | The Python engine delivers value without switching; Python interop; a killer demo (self-healing pipelines) |
| **Privacy of Lessons** (inputs may hold PII) | Medium | Redaction on by default, configurable retention, never ship Lessons off-machine without opt-in |

---

## 7. Rust beta policy

Evolve tracks Rust's **beta channel** to adopt upcoming language, `std`, and codegen improvements one release cycle early.

- **Development:** `rust-toolchain.toml` uses `channel = "beta"`.
- **Releases:** each Evolve release pins a dated beta toolchain (for example `beta-2027-06-01`) so builds are reproducible.
- **CI matrix:** beta (required), stable (required: the code must also compile on stable, so we never depend on a feature that gets pulled), nightly (informational).
- **Rule:** if a feature exists only on nightly, we don't use it.

---

## 8. Team and governance

**Core roles needed:**

| Role | Focus |
|---|---|
| Compiler engineer | Parser, HIR, type system, Cranelift and LLVM backends |
| Runtime and data engineer | Memory model, arrays, Arrow DataFrames, units and uncertainty |
| Evolution and ML engineer | Mutation operators, fitness, selection, EvoBench |
| AI/ML research engineer | Tensors and autodiff, `module`/`model`, safety eval suites, agent societies |
| Security engineer (part-time) | Sandbox, capabilities, promotion pipeline, audits |
| Developer relations and docs | The Book, examples, community, Python-engine adoption |

**Governance:** RFCs for any language or format change, a small core team with lazy consensus, a public roadmap (this file), and monthly progress notes in GitHub Discussions.

---

## 9. Possibilities: where Evolve could go

Beyond 1.0, these are directions worth exploring. They're ideas, not commitments.

### Near horizon (2028–2029)
- **Collective immune system:** an opt-in, privacy-preserving **Lesson registry**. When one project learns that a vendor changed its date format, every subscribed project gets the antibody (an adversarial test case) automatically.
- **Gene registry:** publish evolvable functions with their fitness specs and lineage, so others can fork them into their own environments (*speciation as a package*).
- **Notebook-native evolution:** Jupyter-style notebooks where cells evolve and the lineage is visible inline.
- **Prompt and agent genes:** treat prompts, tool definitions, and agent policies as genes with fitness, extending the DSPy idea to whole programs.

### Mid horizon (2029–2031)
- **Verified invariants:** discharge `invariant` clauses with an SMT solver (such as Z3), so some mutations are *proven* safe and don't just pass tests.
- **Evolving the compiler itself:** inlining, specialization, and scheduling heuristics in the Evolve compiler as genes, tuned on real workloads.
- **GPU kernel evolution:** evolve tiling and fusion strategies for array code on specific hardware.
- **Federated evolution at the edge:** fleets of devices evolve controllers locally (parametric only, inside `@conserved` envelopes) and share winning variants centrally.
- **Evolve Garden (hosted service):** managed fossil storage, sandboxed evolution runners, a team approval UI, and a dashboard for lineage. This could also make the project sustainable.

### Far horizon (research)
- **Open-ended evolution:** populations that keep generating novel, useful behavior rather than converging, inspired by artificial-life research.
- **Self-hosting:** Evolve written in Evolve, with its own core evolving under strict gates.
- **A science of software evolution:** the public fossil record as a dataset for studying how real software fails and adapts over time.

---

## 10. How to help right now

| If you are… | You can… |
|---|---|
| A language designer | Comment on or draft the core RFCs (syntax, gene semantics) |
| A data or platform engineer | Contribute **EvoBench cases** from real failures you've fixed (anonymized) |
| An ML or agents person | Propose mutation operators and fitness designs; critique the anti-gaming plan |
| A Rust developer | Prototype `evo-fossil`, or help specify the constructs in [`tests/syntax/docs-allowlist.txt`](../tests/syntax/docs-allowlist.txt) so `evo-syntax` can parse them |
| A teacher | Tell us what a "failure-friendly" first programming course would need |

Open an issue titled `RFC: <idea>`, `Bench: <failure case>`, or `Question: <topic>`.
