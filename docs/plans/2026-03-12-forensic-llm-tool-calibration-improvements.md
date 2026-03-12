# Forensic LLM – Tool Calibration Improvement Note

**Status:** In Progress  
**Created:** 2026-03-12  
**Scope:** Define the forensic task precisely (what is a scheme, how to score it), calibrate token limits and memory management, and select a SOTA agentic strategy grounded in a literature review.

---

## 1. Task Definition: What Is a Fraud Scheme?

### 1.1 Problem

The current system treats fraud detection as a **document-level classification** problem: each `SuspicionItem` is keyed by a single `document_id`, and evaluation joins on `document_id` against `anomaly_labels`. There is no formal notion of a **scheme** as a composite forensic entity.

Real fraud schemes span multiple journal entries, involve identifiable perpetrators, operate over bounded time windows, and produce quantifiable monetary damage. The evaluator ignores `entity_id`, `related_document_ids`, `period`, and `monetary_impact` — fields already present in `SuspicionItem` but never exploited for scoring.

### 1.2 Scheme Definition

A fraud scheme is a structured object with five attributes:

| Attribute | Type | Description |
|-----------|------|-------------|
| **Perpetrator** | `entity_id` + `entity_type` | The person or entity orchestrating the fraud (employee, vendor, customer). |
| **Scheme type** | `SchemeType` enum | One of the 6 core types (shadow payroll, triad bypass, embezzlement, revenue manipulation, kickback, expense laundering) or extended taxonomy. |
| **Time window** | `[start_date, end_date]` | The fiscal period(s) during which the scheme was active. |
| **Concerned JEs** | `Set[document_id]` | All journal entries constituting the scheme (the entry set). |
| **Monetary impact** | `float` | Aggregate loss: sum of individual JE-level impacts. |

A scheme is **not** a single suspicious entry. It is the full constellation of entries, entities, and temporal scope that together constitute the fraud.

### 1.3 Composite Scheme Score

A single score must rank schemes for auditor attention. The score should be intuitive (higher = worse / more urgent) and decomposable into interpretable components.

**Proposed formula:**

```
scheme_score = w_c · avg_confidence
             + w_s · (max_severity / 5)
             + w_v · coverage_ratio
             + w_m · log_monetary_normalized
```

Where:

- `avg_confidence` — mean `confidence` of constituent `SuspicionItem`s (agent's belief strength).
- `max_severity / 5` — worst-case severity across constituent items, normalised to [0, 1].
- `coverage_ratio` — fraction of the scheme's JE set that the agent actually flagged: `|flagged ∩ scheme_JEs| / |scheme_JEs|`. Measures investigation thoroughness.
- `log_monetary_normalized` — `log(1 + monetary_impact) / log(1 + max_impact_in_dataset)`. Captures materiality on a compressed scale.

**Default weights:** `w_c = 0.35`, `w_s = 0.20`, `w_v = 0.25`, `w_m = 0.20` (sum = 1.0). Weights are hyperparameters tunable via ablation.

### 1.4 Ground-Truth Scheme Construction

The `anomaly_labels` table contains `metadata` JSONB with `scheme_type`, `stage`, and `perpetrator_id`. Ground-truth schemes are built by:

1. Grouping labels by `(perpetrator_id, scheme_type)`.
2. For each group, extract:
   - `start_date = min(anomaly_date)`, `end_date = max(anomaly_date)`.
   - `je_set = {document_id for each label in group}`.
   - `total_impact = sum(monetary_impact)`.
3. A scheme is **detected** if coverage_ratio ≥ τ (default τ = 0.3) and at least one JE in the set is flagged with confidence ≥ 0.5.

### 1.5 Evaluation Metrics (Scheme-Level)

In addition to the existing document-level P/R/F1 and AUC metrics, add:

| Metric | Definition |
|--------|------------|
| **Scheme Detection Rate (SDR)** | Fraction of ground-truth schemes detected (per the rule in §1.4). |
| **Scheme Precision** | Of all scheme-clusters the agent reported, how many correspond to a real ground-truth scheme. |
| **Scheme F1** | Harmonic mean of SDR and Scheme Precision. |
| **Mean Coverage** | Average coverage_ratio across detected schemes — measures depth of investigation per scheme. |
| **Perpetrator Identification Rate** | Fraction of schemes where the agent correctly identified the perpetrator `entity_id`. |

### 1.6 Implementation Changes

| Component | Change |
|-----------|--------|
| `models.py` | Add `SchemeReport` model that groups `SuspicionItem`s by `(entity_id, scheme_type)` and computes `scheme_score`. |
| `evaluator.py` | Add `evaluate_schemes()` that builds ground-truth schemes from `anomaly_labels.metadata`, matches them to predicted scheme clusters, and computes SDR, scheme precision, mean coverage, perpetrator identification rate. |
| `prompts.py` | Update the output contract to instruct the agent to group findings by scheme, not just list individual documents. |
| `run.py` | Wire scheme-level evaluation into the output pipeline. |

---

## 2. Token Limits and Memory Management Calibration

### 2.1 Current Configuration and Observed Usage

| Parameter | Value | Observation |
|-----------|-------|-------------|
| `max_tokens_per_step` | 16,384 | Adequate for most models. |
| `max_tokens` (budget) | 50,000,000 | Runs consume ~2.6M (5.3%). Budget is never the binding constraint. |
| `warn_threshold` | 0.80 | Never triggered in practice. |
| `stop_threshold` | 0.95 | Never triggered in practice. |
| Min SQL guardrail | `10 + 30 × n_core` | This is the *de facto* depth controller. |
| Context management | Full history, unbounded | Context grows monotonically with every step. |
| Scratchpad | Free-text append only | No eviction, no structure, no summarisation. |

The 50M token budget is essentially decorative. The SQL guardrail and model willingness to call `finish_investigation` are what actually determine investigation depth.

### 2.2 Token Budget Calibration

**Option A — Tighten the budget for realistic experiments:**

Reduce `max_tokens` to a range that is actually binding (e.g., 3M, 5M, 10M), and run budget-vs-performance ablations as already planned in the benchmark spec (§3.3). This produces the most scientifically useful data.

**Option B — Keep 50M but add per-phase budgets:**

Allocate the total budget across investigation phases:

| Phase | Share | Purpose |
|-------|-------|---------|
| Orientation | 5% | DB overview, schema exploration, row counts. |
| Per-scheme investigation (×6) | 12.5% each (75% total) | Deep dive per scheme type. |
| Reflection + synthesis | 10% | Cross-scheme analysis, deduplication, report writing. |
| Reserve | 10% | Overflow, retries, forced finish. |

The planner (see §3) allocates these at the start and the budget tracker enforces them.

### 2.3 `max_tokens_per_step` Calibration

| Model class | Recommended `max_tokens_per_step` |
|-------------|-----------------------------------|
| API models with per-token billing (GPT-4o, Claude, Gemini) | 16,384 (current) |
| Self-hosted large-context models (Qwen 122B, LLaMA 405B) | 32,768 |
| Models with 200k+ context windows | 32,768, with context compaction |

The per-step limit controls generation length. 16k is safe for most SQL-heavy reasoning; 32k gives headroom for complex multi-tool steps.

### 2.4 Memory Management: Context Compaction

**Problem:** By step 40, the message history contains ~40 SQL results (each 1–5k tokens), ~40 reasoning blocks, and ~40 tool call records. Estimated context: 120–200k tokens. This approaches or exceeds the context window of most models (128k for Qwen, 200k for Claude/Gemini).

**Solution: Sliding-window summarisation.**

After every N steps (recommended N = 10):

1. Take the oldest K messages (K = N − 2, keeping the 2 most recent).
2. Feed them to the LLM with the prompt: *"Summarise the key findings, hypotheses, and evidence from these investigation steps in ≤ 500 tokens."*
3. Replace the K messages with a single `user` message containing the summary.
4. Keep the full scratchpad intact (it serves as persistent structured memory).

**Cost:** One extra LLM call per compaction. At 4 compactions per 40-step run, this adds ~4 calls (negligible vs. the 40 investigation calls).

**Alternative — structured scratchpad as primary memory:**

Instead of free-text append, make the scratchpad a JSON object:

```json
{
  "orientation": {
    "n_entries": 45000,
    "date_range": ["2024-01-01", "2024-12-31"],
    "companies": ["1000", "2000"],
    "document_types": ["SA", "KR", "KZ", "RV", "AB"]
  },
  "hypotheses": [
    {
      "scheme": "shadow_payroll",
      "status": "investigating",
      "key_entities": ["EMP-0042", "EMP-0099"],
      "key_evidence": ["Shared bank account IBAN-xxx", "Both hired same day"],
      "document_ids": ["abc-123", "def-456"]
    }
  ],
  "leads_to_follow": [
    "Check vendor V-00312 for kickback pattern — unusually high invoice amounts"
  ]
}
```

The agent updates this structured scratchpad each step (via the `scratchpad` tool with a `replace` mode). The system prompt injects only the structured scratchpad, not the full message history, for "what I know so far" context. Raw messages can then be aggressively compacted.

### 2.5 Context Size Monitoring

Add a pre-call check in `_main_loop`:

1. Before each LLM call, estimate the token count of `full_messages` using tiktoken.
2. If estimated context exceeds 80% of the model's context window, trigger compaction immediately.
3. Log context size at every step for diagnostics.

This prevents silent truncation or API errors at high step counts.

---

## 3. Agentic Strategy: Literature Review and SOTA Selection

### 3.1 Literature Review

#### 3.1.1 ReAct — Reasoning + Acting (Yao et al., 2023)

- **Paper:** *"ReAct: Synergizing Reasoning and Acting in Language Models"*, ICLR 2023.
- **Mechanism:** Interleaved thought–action–observation traces. The LLM generates a reasoning step, picks a tool, observes the result, and repeats.
- **Strengths:** Simple, robust, broadly supported by all LLM providers.
- **Weaknesses:** No explicit planning horizon, no backtracking, no budget allocation. Susceptible to local-optima traps (over-investigating one lead while ignoring others).
- **Current status:** This is what the forensic agent implements today.

#### 3.1.2 Reflexion (Shinn et al., 2023)

- **Paper:** *"Reflexion: Language Agents with Verbal Reinforcement Learning"*, NeurIPS 2023.
- **Mechanism:** After each episode (or phase), the agent generates a self-reflection on its performance: what worked, what was missed, what to try next. The reflection is injected as context for the next episode.
- **Strengths:** Enables learning-within-episode without weight updates. Particularly effective for iterative tasks where early decisions constrain later options.
- **Weaknesses:** Requires a clear notion of "episode boundary" and a quality signal for reflection. Adds one LLM call per reflection.
- **Relevance:** High. After each scheme investigation phase, a reflection step can identify under-explored hypotheses and redirect effort.

#### 3.1.3 Plan-and-Solve (Wang et al., 2023)

- **Paper:** *"Plan-and-Solve Prompting: Improving Zero-Shot Chain-of-Thought Reasoning"*, ACL 2023.
- **Mechanism:** The LLM first generates an explicit plan (decomposition into sub-problems), then solves each sub-problem sequentially.
- **Strengths:** Produces structured, auditable investigation plans. Naturally decomposes the 6-scheme task.
- **Weaknesses:** Plans may be brittle; no mechanism for mid-execution plan revision.
- **Relevance:** High for the planning phase; should be combined with adaptive re-planning.

#### 3.1.4 ADaPT — Adaptive Decomposition of Plans into Traces (Prasad et al., 2024)

- **Paper:** *"ADaPT: As-Needed Decomposition and Planning with Language Models"*, NAACL 2024.
- **Mechanism:** Hierarchical planning where a high-level planner decomposes the task into sub-tasks, a low-level executor handles each sub-task via ReAct, and the planner adapts based on intermediate results. Failed or inconclusive sub-tasks trigger re-planning.
- **Strengths:** Combines structured planning with adaptive execution. Budget-aware decomposition is straightforward to implement.
- **Weaknesses:** More complex orchestration; requires defining sub-task success/failure criteria.
- **Relevance:** Very high. The forensic task naturally decomposes into: orientation → per-scheme investigation → cross-scheme synthesis. ADaPT's adaptive re-planning handles the common case where one scheme investigation reveals leads for another.

#### 3.1.5 LATS — Language Agent Tree Search (Zhou et al., 2024)

- **Paper:** *"Language Agent Tree Search Unifies Reasoning Acting and Planning"*, ICML 2024.
- **Mechanism:** Monte Carlo Tree Search over reasoning traces. Multiple candidate actions are explored, evaluated by a value function, and the best path is selected. Supports backtracking.
- **Strengths:** Can explore multiple hypotheses in parallel; principled exploration-exploitation tradeoff.
- **Weaknesses:** Extremely expensive — requires multiple rollouts per decision point. Impractical under tight token budgets.
- **Relevance:** Low for full investigations. Potentially useful for high-stakes sub-tasks (e.g., confirming a suspected embezzlement scheme) where exploring 2–3 SQL strategies is worth the cost.

#### 3.1.6 Multi-Agent Debate and Specialisation (Du et al., 2023; Hong et al., 2024)

- **Papers:** *"Improving Factuality and Reasoning via Debate"* (Du et al., 2023); *"MetaGPT: Meta Programming for Multi-Agent Collaboration"* (Hong et al., 2024).
- **Mechanism:** Multiple LLM agents with distinct roles collaborate. Variants include: parallel investigation with synthesis (ensemble), sequential handoff (pipeline), adversarial debate (verification).
- **Strengths:** Natural fit for the 6-scheme forensic task — each agent specialises in one scheme type. A coordinator synthesises findings. Cross-agent communication enables discovery of cross-scheme links.
- **Weaknesses:** High total token cost (N agents × budget). Coordination complexity. May produce redundant work.
- **Relevance:** High for the benchmark comparison axis (single-agent vs. multi-agent). The existing `run_ensemble()` is a starting point but currently runs independent full-task agents, not specialised ones.

#### 3.1.7 Cognitive Architectures: CoALA Framework (Sumers et al., 2024)

- **Paper:** *"Cognitive Architectures for Language Agents"*, TMLR 2024.
- **Mechanism:** Unifying framework decomposing agents into: perception → memory (working + long-term + episodic) → reasoning (planning + reflection + decision) → action. Provides a taxonomy for design choices.
- **Relevance:** Useful as a design lens. The forensic agent's memory system (§2.4) maps directly to CoALA's working memory (scratchpad) and episodic memory (compacted history).

### 3.2 Comparison Matrix

| Approach | Planning | Reflection | Backtracking | Budget-aware | Multi-scheme | Token cost | Complexity |
|----------|----------|------------|--------------|--------------|--------------|------------|------------|
| ReAct (current) | None | None | None | No | Implicit | Low | Low |
| Reflexion | None | Per-phase | None | No | Implicit | Low+ | Low+ |
| Plan-and-Solve | Static | None | None | Possible | Explicit | Low | Medium |
| **ADaPT** | **Adaptive** | **Built-in** | **Re-plan** | **Yes** | **Explicit** | **Medium** | **Medium** |
| LATS | Tree search | Value fn | Full | No | Implicit | Very high | High |
| Multi-Agent | Per-agent | Optional | Per-agent | Per-agent | Explicit | High (×N) | High |

### 3.3 Recommended Architecture: Plan-Execute-Reflect (ADaPT-inspired)

The recommended architecture combines ADaPT's adaptive planning with Reflexion's self-critique, applied to the forensic domain:

```
┌──────────────────────────────────────────────────────────────────┐
│                        PLANNER PHASE                             │
│                                                                  │
│  1. Orientation: DB overview (row counts, date ranges,           │
│     document types, company codes, distinct users)               │
│  2. Generate investigation plan:                                 │
│     - Ordered list of scheme-specific sub-tasks                  │
│     - Per-scheme token/SQL budget allocation                     │
│     - Priority ranking (most likely schemes first)               │
│     - Entity focus areas per scheme                              │
│                                                                  │
│  Output: InvestigationPlan (structured JSON)                     │
└──────────────────────────┬───────────────────────────────────────┘
                           │
            ┌──────────────┼──────────────────┐
            ▼              ▼                  ▼
     ┌────────────┐ ┌────────────┐     ┌────────────┐
     │  Scheme 1  │ │  Scheme 2  │ ... │  Scheme N  │
     │  Executor  │ │  Executor  │     │  Executor  │
     │  (ReAct    │ │  (ReAct    │     │  (ReAct    │
     │   sub-loop)│ │   sub-loop)│     │   sub-loop)│
     └─────┬──────┘ └─────┬──────┘     └─────┬──────┘
           │               │                  │
           │   ┌───────────┘                  │
           │   │   ┌──────────────────────────┘
           ▼   ▼   ▼
     ┌─────────────────────────────┐
     │       REFLECT PHASE         │
     │                             │
     │  - Self-assessment: what    │
     │    was found, what missed   │
     │  - Coverage gap analysis    │
     │  - Cross-scheme link check  │
     │  - Confidence recalibration │
     │  - Re-plan if needed        │
     │    (adaptive loop back      │
     │     to executor phase)      │
     └─────────────┬───────────────┘
                   │
     ┌─────────────▼───────────────┐
     │     SYNTHESISE + REPORT     │
     │                             │
     │  - Merge suspicions across  │
     │    scheme executors         │
     │  - Deduplicate entries      │
     │  - Compute scheme_score     │
     │  - Generate narrative       │
     │  - Call finish_investigation│
     └─────────────────────────────┘
```

### 3.4 Phase Details

#### Phase 1: Planning (1 LLM call + 3–5 SQL calls)

The agent receives the system prompt and immediately performs orientation queries (row counts, date ranges, schema sampling). It then produces a structured plan:

```json
{
  "phases": [
    {
      "scheme": "shadow_payroll",
      "priority": 1,
      "budget_sql_calls": 35,
      "budget_tokens": 800000,
      "initial_hypotheses": [
        "Check for employees sharing bank accounts",
        "Look for terminated employees still receiving payroll"
      ]
    },
    ...
  ],
  "total_budget_sql": 210,
  "reflection_after_schemes": [1, 3, 6]
}
```

The plan is stored in the structured scratchpad and guides execution.

#### Phase 2: Per-Scheme Execution (ReAct sub-loops)

Each scheme investigation runs as a bounded ReAct sub-loop within the main loop:

- Entry criterion: the planner assigns this scheme next.
- Budget: capped at the allocated SQL calls and token share.
- Exit criterion: budget for this scheme exhausted, or agent signals scheme-complete.
- Output: tentative `SuspicionItem`s appended to the scratchpad.

The message history from prior schemes is **compacted** before starting the next scheme (sliding-window summarisation from §2.4), keeping the structured scratchpad as persistent context.

#### Phase 3: Reflection (1–2 LLM calls)

After completing all scheme executors (or after a configurable subset), the agent performs a structured reflection:

1. **Coverage check:** For each scheme type, did the agent investigate at least N SQL patterns? If not, flag for follow-up.
2. **Cross-scheme links:** Do any entities appear in multiple scheme investigations? (e.g., a vendor flagged for both kickback and expense laundering.)
3. **Confidence recalibration:** Given the full picture, should any confidence scores be adjusted?
4. **Re-planning (adaptive):** If the reflection reveals a gap (e.g., "I never checked for triad bypass"), loop back to the executor phase for that scheme. This is the ADaPT-style adaptive re-planning.

#### Phase 4: Synthesis + Report (1–2 LLM calls)

- Group `SuspicionItem`s into `SchemeReport`s by `(entity_id, scheme_type)`.
- Compute `scheme_score` for each.
- Deduplicate entries that appear in multiple scheme investigations.
- Generate the narrative report.
- Call `finish_investigation`.

### 3.5 Implementation Roadmap

| Step | Component | Change | Priority |
|------|-----------|--------|----------|
| 1 | `models.py` | Add `InvestigationPlan`, `SchemePhase`, `SchemeReport` models. | P0 |
| 2 | `prompts.py` | Add planning prompt and reflection prompt templates. | P0 |
| 3 | `agent.py` | Refactor `_main_loop` into `_plan()`, `_execute_scheme()`, `_reflect()`, `_synthesise()` phases. | P0 |
| 4 | `token_budget.py` | Add per-phase budget tracking and enforcement. | P1 |
| 5 | `agent.py` | Implement sliding-window context compaction between scheme phases. | P1 |
| 6 | `tools.py` | Extend scratchpad tool with `mode: "replace"` for structured updates. | P1 |
| 7 | `evaluator.py` | Add `evaluate_schemes()` with SDR, scheme precision, mean coverage, perpetrator ID rate. | P1 |
| 8 | `config.py` | Add `AgentStrategy` config (plan-execute-reflect vs. flat ReAct) for ablation. | P2 |
| 9 | `agent.py` | Multi-agent variant: parallel scheme executors with coordinator synthesis. | P2 |
| 10 | `run.py` | Add `--strategy` CLI flag to select flat-react vs. plan-execute-reflect. | P2 |

### 3.6 Ablation Axes for the Benchmark

The architecture supports controlled ablation along the following axes:

| Axis | Variants |
|------|----------|
| **Strategy** | Flat ReAct (baseline) vs. Plan-Execute-Reflect vs. Multi-Agent |
| **Token budget** | 3M, 5M, 10M, 25M, 50M |
| **Memory** | Full history (baseline) vs. Sliding-window compaction vs. Structured scratchpad |
| **Reflection** | None (baseline) vs. After each scheme vs. After all schemes |
| **Planning** | None (baseline) vs. Static plan vs. Adaptive re-planning |
| **Model** | Qwen 3.5 122B vs. Claude 4.6 Opus vs. Gemini 3 Pro |

---

## 4. References

1. Yao, S. et al. (2023). *ReAct: Synergizing Reasoning and Acting in Language Models.* ICLR 2023.
2. Shinn, N. et al. (2023). *Reflexion: Language Agents with Verbal Reinforcement Learning.* NeurIPS 2023.
3. Wang, L. et al. (2023). *Plan-and-Solve Prompting: Improving Zero-Shot Chain-of-Thought Reasoning by Large Language Models.* ACL 2023.
4. Prasad, A. et al. (2024). *ADaPT: As-Needed Decomposition and Planning with Language Models.* NAACL 2024.
5. Zhou, A. et al. (2024). *Language Agent Tree Search Unifies Reasoning Acting and Planning in Language Models.* ICML 2024.
6. Du, Y. et al. (2023). *Improving Factuality and Reasoning in Language Models through Multiagent Debate.* ICML 2023.
7. Hong, S. et al. (2024). *MetaGPT: Meta Programming for a Multi-Agent Collaborative Framework.* ICLR 2024.
8. Sumers, T. et al. (2024). *Cognitive Architectures for Language Agents.* TMLR 2024.
