# Fraud Scheme Generation Hardening — Specification & Roadmap

**Status:** Proposed  
**Created:** 2026-03-12  
**Depends on:** `technical-report-scheme-generation-flow.md` (v2.0), `2026-03-12-forensic-llm-tool-calibration-improvements.md`  
**Scope:** Fix spec-vs-implementation gaps in the 6 core schemes, add cross-scheme structural realism, and introduce 4 new fraud typologies.

---

## 1. Motivation

The current 6-scheme generator produces multi-stage fraud actions that are
materialised into journal entries. An audit of the generation pipeline against
the normative spec (v2.0) and against real forensic investigation expectations
reveals three categories of issues:

1. **Spec gaps** — forensic signals described in the technical report are not
   produced by the materialiser or by the scheme logic (e.g. after-hours
   timestamps, perpetrator `user_id`, lettrage, split payments).
2. **Structural limitations** — schemes are generated independently with no
   cross-scheme interaction, shared perpetrators, or realistic concealment
   patterns. This makes detection easier than intended.
3. **Typological coverage** — all 6 schemes are income-statement / P2P focused.
   Balance-sheet fraud, negative-signal fraud (missing expected transactions),
   and cross-domain (JE + master data) fraud are absent.

Addressing these issues is necessary both for benchmark validity (the agent
should not be solving a task that is artificially easy due to implementation
shortcuts) and for research novelty (richer fraud typologies make the benchmark
more discriminating across models).

---

## 2. Tier 1 — Spec-vs-Implementation Gaps

### 2.1 Propagate `user_id` into `JournalEntryHeader.created_by`

**Problem.** Every scheme assigns a `perpetrator_id` and sets `action.user_id`
on each `SchemeAction`. The materialiser (`materialize_scheme_actions`) ignores
this field and creates every header with `created_by: "SYSTEM"`. The forensic
signal of segregation-of-duties violation — a single user posting all shell
vendor invoices — is completely absent from the generated data.

**Required change.**

| Component | Change |
|-----------|--------|
| `enhanced_orchestrator.rs` → `materialize_scheme_actions` | After constructing `JournalEntryHeader`, set `header.created_by = action.user_id.clone().unwrap_or("SYSTEM".into())` and propagate `header.user_persona` accordingly. |
| All 6 scheme `advance()` methods | Verify that every emitted `SchemeAction` carries `.with_user(self.perpetrator_id.clone())`. Audit and fix any actions that omit this. |

**Validation.** After generation, run:

```sql
SELECT created_by, COUNT(*), COUNT(DISTINCT document_id)
FROM journal_entries
WHERE is_fraud = true
GROUP BY created_by;
```

Each scheme instance should show a single `created_by` value across all its
fraudulent JEs.

**Priority:** Critical — without this, segregation-of-duties analysis is
impossible for all 6 schemes.

---

### 2.2 Add `target_time` to `SchemeAction` for After-Hours Timestamps

**Problem.** The embezzlement spec (§2.4) mandates that Stage 0/1 JEs are
timestamped between 22:00–06:00 CET. The `SchemeAction` struct only carries
`target_date: NaiveDate` — there is no time-of-day field. The materialiser
sets `created_at: Utc::now()` (wall-clock time of generation).

**Required changes.**

| Component | Change |
|-----------|--------|
| `scheme.rs` → `SchemeAction` | Add `pub target_time: Option<NaiveTime>`. Default `None`. |
| `embezzlement.rs` → `advance()` | For Stage 0 and Stage 1 actions, set `target_time` to a random time in `[22:00, 06:00)` (wrapping midnight). For Stage 2/3, set to normal business hours `[08:00, 18:00)` or leave `None`. |
| `enhanced_orchestrator.rs` → `materialize_scheme_actions` | When `action.target_time.is_some()`, combine `target_date` and `target_time` into `entry_timestamp`. When `None`, use the existing deterministic timestamp logic. |
| PostgreSQL export / CSV export | Ensure `entry_timestamp` is exported with full datetime precision (not date-only). |

**Validation.** After generation, the agent should be able to run:

```sql
SELECT EXTRACT(HOUR FROM entry_timestamp) AS hour, COUNT(*)
FROM journal_entries
WHERE is_fraud = true AND anomaly_type = 'GradualEmbezzlement'
GROUP BY hour ORDER BY hour;
```

and observe a cluster in the 22–06 range for early-stage actions.

**Priority:** High — temporal analysis is a stated forensic channel.

---

### 2.3 Implement Lettrage (Matching) for Embezzlement

**Problem.** The spec (§2.3) requires that the invoice AP credit (Part A) and
the payment AP debit (Part B) share a unique matching code (`lettrage`). Without
this, a simple AP ageing query reveals unmatched balances on the shell vendor
sub-ledger — the fraud is trivially detectable.

**Required changes.**

| Component | Change |
|-----------|--------|
| `JournalEntryLine` model | Add `pub lettrage_code: Option<String>`. |
| `embezzlement.rs` → `advance()` | When emitting a payment action (Part B), set `action.reference` to the same value as the corresponding invoice action's `action_id` (or a deterministic lettrage code like `L-{scheme_id_short}-{seq}`). |
| `materialize_scheme_actions` | For embezzlement payment actions, look up the corresponding invoice JE line on `401000` and assign a shared `lettrage_code` to both the Cr line (invoice) and the Dr line (payment). |
| CSV / PostgreSQL export | Export `lettrage_code` as a column. |

**Forensic impact.** With lettrage, the shell vendor's AP sub-ledger appears
clean (all invoices are matched to payments). The agent must find the fraud
through other signals (shell vendor identity, amount patterns, after-hours
posting, single-user posting). Without lettrage, a one-line SQL query on
unmatched AP balances solves the problem.

**Priority:** Critical — without it, embezzlement detection is artificially easy.

---

### 2.4 Split Kickback Payment into Fragments

**Problem.** The spec (§4.2, Action 2) requires that the inflated invoice
payment is split into 2–3 separate JEs on different dates (e.g. 60%/40% or
50%/30%/20%). Currently the kickback scheme emits one `InflateInvoice` action
and one `MakeKickbackPayment` action with no intermediate payment splitting.

**Required changes.**

| Component | Change |
|-----------|--------|
| `scheme.rs` → `SchemeActionType` | Add `PayInflatedInvoicePartial`. |
| `kickback.rs` → `advance()` | After `InflateInvoice`, schedule 2–3 `PayInflatedInvoicePartial` actions at T+10, T+15, T+20 (configurable jitter). Each carries a fraction of the inflated amount. All share the same `reference` (the invoice ID). |
| `materialize_scheme_actions` | Map `PayInflatedInvoicePartial` → Dr `401000`, Cr `512000` with partial amount. Assign `lettrage_code` that partially matches the invoice: partial lettrage until final payment, then full match. |

**Forensic impact.** The agent must reconstruct that multiple partial payments
sum to the inflated invoice amount — a pattern matching task that tests
arithmetic reasoning. The shared `reference` field across the split payments is
the linking signal.

**Priority:** High — removes a major missing forensic pattern.

---

### 2.5 Insert Ghost Employee into Master Data

**Problem.** The shadow payroll scheme generates a `ghost_employee_id`
(e.g. `EMP-00042`) and monthly payroll JEs crediting `421000`. But the ghost
employee record is never inserted into the exported `employees` table. The agent
cannot correlate the payroll counterparty to an employee record to discover
anomalies (no manager, no cost centre, hire date coincides with first payroll,
bank account shared with real employee).

**Required changes.**

| Component | Change |
|-----------|--------|
| `shadow_payroll.rs` → `advance()` (Stage 0 / `GhostHire`) | Emit a side-effect struct `GhostEmployeeRecord` containing: `employee_id`, `display_name` (generated), `hire_date` (= scheme start), `cost_center` (= perpetrator's cost centre), `payroll_account_number` (= perpetrator's bank account or new account), `manager_id` (= `None` or self-referential), `termination_date` (= `None`). |
| `enhanced_orchestrator.rs` | After scheme advancement, collect `GhostEmployeeRecord`s and merge them into the `employees` master data table before export. |
| `SchemeAction` or `SchemeAdvancer` | Add a mechanism to return master-data mutations alongside JE actions (e.g. `Vec<MasterDataMutation>` alongside `Vec<SchemeAction>`). |

**Forensic signals enabled.**

| Signal | Query the Agent Could Use |
|--------|--------------------------|
| No manager | `SELECT * FROM employees WHERE manager_id IS NULL` |
| Shared bank account | `graph_query(ghost_emp_id, edge_types=["shares_bank_account"])` |
| Hire date = first payroll | Cross-reference `employees.hire_date` with first JE on `421000` for that aux account |
| No cost centre / odd cost centre | `SELECT * FROM employees WHERE cost_center IS NULL OR cost_center NOT IN (SELECT DISTINCT cost_center FROM employees WHERE ...)` |

**Priority:** High — without this, shadow payroll detection via employee
analysis is impossible.

---

### 2.6 Align `FRAUD_CATALOGUE` Prompt with Actual Generator Output

**Problem.** The `FRAUD_CATALOGUE` in `prompts.py` describes revenue
manipulation as a simple forward/reverse cycle. The Rust implementation
generates 4 distinct stages: early revenue recognition (Q4), expense deferral
(Q1), reserve release (Q2), and channel stuffing (Q4). The agent is not told
about expense deferral, reserve release, or channel stuffing, so it may not
investigate them.

**Required change.** Update `FRAUD_CATALOGUE` entry for Revenue Manipulation:

> The perpetrator inflates reported financial performance using several
> techniques that may appear across different quarters: recognising revenue
> before it is earned, deferring expenses to later periods, releasing
> previously established reserves to boost income, and pushing excess
> inventory or sales onto customers near period-end ("channel stuffing").
> The ledger reflects artificial revenue, deferred charges, released
> provisions, and possibly matching reversals in subsequent periods.

Keep the description agnostic to detection methods (consistent with the
catalogue's design philosophy).

**Priority:** Medium — the agent may still discover these patterns
independently, but the asymmetry between prompt and data is a confound.

---

## 3. Tier 2 — Cross-Scheme Structural Improvements

### 3.1 Cross-Scheme Perpetrator Reuse

**Problem.** Schemes are started with independent perpetrator assignment. In
real forensic investigations, discovering that the same employee is involved in
both a kickback and an embezzlement scheme is the breakthrough moment. Currently
the benchmark cannot test this capability.

**Design.**

```yaml
# config.yaml addition
anomaly_injection:
  multi_stage_schemes:
    perpetrator_reuse:
      enabled: true
      reuse_probability: 0.30   # P(new scheme assigned to existing perpetrator)
      max_schemes_per_perpetrator: 3
```

| Component | Change |
|-----------|--------|
| `SchemeAdvancer` | Maintain a `perpetrator_pool: Vec<(String, Vec<SchemeType>)>` tracking active perpetrators and their scheme types. |
| `maybe_start_scheme()` | When starting a new scheme, draw from `perpetrator_pool` with probability `reuse_probability` instead of sampling a fresh user. Respect `max_schemes_per_perpetrator`. |
| Labels / export | Add `perpetrator_id` to `anomaly_labels` so ground truth supports perpetrator-level evaluation. |

**Forensic impact.** The agent must cross-reference findings across scheme types:
"User X appears as the poster for shell vendor invoices (embezzlement) AND as
the approver for the colluding vendor (kickback). These are two distinct fraud
schemes by the same perpetrator."

**Evaluation impact.** Enables a new metric: **Perpetrator Identification Rate**
(already defined in `models.py` but currently untestable because perpetrators
don't overlap).

---

### 3.2 Scheme Co-Occurrence Matrix

**Problem.** Scheme starts are independent Bernoulli draws. In reality, fraud
types cluster: an embezzler who controls a shell vendor may also run expense
laundering through the same vendor network.

**Design.**

```yaml
anomaly_injection:
  multi_stage_schemes:
    co_occurrence:
      enabled: true
      # When scheme A starts, P(scheme B also starts with same perpetrator)
      matrix:
        embezzlement:
          expense_laundering: 0.25
          kickback: 0.10
        kickback:
          triad_bypass: 0.15
        shadow_payroll:
          embezzlement: 0.10
```

| Component | Change |
|-----------|--------|
| `SchemeAdvancer` → `maybe_start_scheme()` | After starting scheme A, iterate the co-occurrence row for A. For each entry, draw Bernoulli and conditionally start scheme B with the same perpetrator (subject to `max_concurrent_schemes`). |
| Config schema | Add `CoOccurrenceConfig` with a `HashMap<SchemeType, HashMap<SchemeType, f64>>`. |

**Forensic impact.** Creates multi-scheme fraud networks that share entities
(perpetrators, vendors, bank accounts). The agent must discover that seemingly
independent anomalies are part of a coordinated campaign.

---

### 3.3 Concrete Concealment JE Patterns

**Problem.** `CoverUp` and `Conceal` action types are materialised as generic
JEs with default accounts. Real concealment has specific accounting patterns.

**Design.** Implement three concealment strategies:

#### 3.3.1 Journal Entry Splitting

The perpetrator breaks a large suspicious JE into N smaller ones posted across
different dates.

| Field | Value |
|-------|-------|
| Action type | `ConcealSplit` |
| Trigger | Original amount > configurable threshold (e.g. 2× stage average) |
| Pattern | Replace 1 action of amount A with N actions of amount A/N (N ∈ [2, 5]), posted over N consecutive days |
| Materialisation | Same accounts as original, reduced amounts |

#### 3.3.2 Reclassification

Move a suspicious balance from a scrutinised account to a less-scrutinised one.

| Field | Value |
|-------|-------|
| Action type | `ConcealReclassify` |
| Trigger | Configurable probability per stage (e.g. 0.15 in Stage 2+) |
| Pattern | Dr target account (e.g. `471000` Suspense or `486000` Deferred Charges), Cr original expense account. Net effect: the expense disappears from Class 6 and hides in the balance sheet. |
| Materialisation | Two-line JE with specific accounts per concealment target |

#### 3.3.3 Contra-Entry Netting

Create offsetting entries within a period so the net impact on a given account
is zero, then reverse the offset next period.

| Field | Value |
|-------|-------|
| Action type | `ConcealContraEntry` |
| Trigger | End-of-month, when cumulative fraud on an account exceeds a threshold |
| Pattern | (1) Dr `6xxxxx`, Cr `471000` — offset within period. (2) Next month: Dr `471000`, Cr `6xxxxx` — reverse the offset. |
| Materialisation | Two paired JEs with matching `reference` fields |

**Changes required.**

| Component | Change |
|-----------|--------|
| `scheme.rs` → `SchemeActionType` | Add `ConcealSplit`, `ConcealReclassify`, `ConcealContraEntry`. |
| Each scheme's `advance()` | After emitting a primary action, probabilistically emit a concealment action. |
| `materialize_scheme_actions` | Add materialisation branches for the 3 new action types with specific account mappings. |

---

### 3.4 Fingerprint-Based Amount Sampling for All Stages

**Problem.** Scheme stages use hardcoded amount ranges (e.g. `(dec!(100), dec!(500))`
for embezzlement Stage 0). If the dataset fingerprint says typical Class 6
transactions are €2,000–€15,000, then €100–€500 is a trivial statistical
outlier.

**Design.** Replace hardcoded ranges with fingerprint-relative percentiles.

| Stage Character | Percentile Band | Multiplier |
|-----------------|-----------------|------------|
| Testing / Setup | P5 – P15 | 0.05 – 0.15 of distribution |
| Escalation | P15 – P50 | 0.15 – 0.50 |
| Acceleration | P50 – P85 | 0.50 – 0.85 |
| Desperation | P85 – P99 | 0.85 – 0.99 |

| Component | Change |
|-----------|--------|
| `SchemeStage` | Add `fingerprint_percentile_band: Option<(f64, f64)>`. |
| Each scheme's `advance()` | When `context.fingerprint_amount_configs` is available and the stage has a percentile band, use `context.sample_amount_from_fingerprint()` with clamping to the percentile band. Fall back to hardcoded range otherwise. |

---

### 3.5 Business Calendar Awareness

**Problem.** `SchemeContext` has `audit_in_progress` and `detection_activity`
fields, but no scheme actually reads them. Real fraudsters modulate activity
around audits and holidays.

**Design.**

| Component | Change |
|-----------|--------|
| `SchemeContext` | Add `pub is_holiday_period: bool`, `pub days_to_fiscal_year_end: i32`. |
| `enhanced_orchestrator.rs` | Populate these fields from the business calendar config. |
| All schemes → `advance()` | When `audit_in_progress`, reduce action emission probability by 80% (the perpetrator goes quiet during audits). When `is_holiday_period`, increase emission probability by 30% (reduced oversight). When `days_to_fiscal_year_end < 30`, embezzlement Stage 3 (desperation) triggers regardless of time-in-stage. |

---

## 4. Tier 3 — New Fraud Typologies

### 4.1 Payroll Tax Diversion (Negative-Signal Fraud)

**Category:** HR / Payroll Cycle.

**Objective.** The perpetrator collects employee payroll deductions (social
charges, tax withholdings) but does not remit them to the tax authority. The
company's liability grows silently.

**Why it matters.** This is a **negative-signal** fraud — the agent must detect
the *absence* of expected transactions rather than the *presence* of anomalous
ones. This tests a fundamentally different reasoning capability.

**Lifecycle.**

| Stage | Duration | Behaviour |
|-------|----------|-----------|
| 0 — Setup | 1 month | Normal payroll + normal tax remittance to establish baseline |
| 1 — Diversion | 6–12 months | Payroll JEs are posted normally (Dr `641100`, Dr `645100`, Cr `421000`, Cr `431000`). The expected remittance (Dr `431000`, Cr `512000`) is either missing entirely or delayed by 60–90+ days. |
| 2 — Cover-up | 2 months | The perpetrator posts fictitious remittance entries (Dr `431000`, Cr `471000` suspense) to make the liability appear settled, without actual bank outflow. |

**JE patterns.**

Normal payroll (present in both clean and fraud data):

| Line | Account | Side | Amount |
|------|---------|------|--------|
| 1 | `641100` (Wages) | Dr | Salary |
| 2 | `645100` (Social Charges) | Dr | Salary × 0.42 |
| 3 | `421000` (Personnel Payable) | Cr | Salary |
| 4 | `431000` (Social Security Payable) | Cr | Salary × 0.42 |

Expected remittance (present in clean data, **missing** in fraud):

| Line | Account | Side | Amount |
|------|---------|------|--------|
| 1 | `431000` (Social Security Payable) | Dr | Accumulated charges |
| 2 | `512000` (Bank) | Cr | Accumulated charges |

Cover-up fictitious remittance (present only in fraud):

| Line | Account | Side | Amount |
|------|---------|------|--------|
| 1 | `431000` (Social Security Payable) | Dr | Accumulated charges |
| 2 | `471000` (Suspense) | Cr | Accumulated charges |

**Detection signals.**

- Growing balance on `431000` over time (liability not cleared).
- No corresponding Dr `431000` / Cr `512000` entries (missing bank outflow).
- In the cover-up phase: `431000` cleared via suspense (`471000`) instead of
  bank (`512000`).

**Implementation.**

| Component | Change |
|-----------|--------|
| `SchemeType` enum | Add `PayrollTaxDiversion`. |
| New file: `payroll_tax_diversion.rs` | Implement `FraudScheme` trait with 3-stage lifecycle. Stage 1 emits no fraudulent JE — it *suppresses* the expected remittance action. Stage 2 emits `ConcealRemittance` actions. |
| `SchemeAdvancer` | Register the new scheme type with its config probability. |
| `materialize_scheme_actions` | Add branch for `ConcealRemittance` → Dr `431000`, Cr `471000`. |
| Normal data generator | Must generate regular tax remittance JEs for clean companies so the agent can compare. |

---

### 4.2 Asset Misappropriation via Inventory Manipulation

**Category:** Balance Sheet / Inventory.

**Objective.** The perpetrator overstates inventory value by creating fictitious
inventory receipts, then writes off the "missing" inventory as spoilage or
shrinkage.

**Why it matters.** This is a balance-sheet fraud. All 6 existing schemes
operate on income-statement flows (P2P, payroll, revenue). Adding an inventory
scheme tests whether the agent can reason about stock movements and book-vs-
physical discrepancies.

**Lifecycle.**

| Stage | Duration | Behaviour |
|-------|----------|-----------|
| 0 — Inflation | 4–8 months | Fictitious goods receipts: Dr `3xxxxx` (Inventory), Cr `603000` (COGS reversal). Amounts are modest (P25–P50 of normal receipt values). |
| 1 — Siphoning | 2–4 months | Physical inventory removed (no JE — this is an off-books theft). Book inventory remains inflated. |
| 2 — Write-off | 1–2 months | The perpetrator writes off the discrepancy as spoilage: Dr `6xxxxx` (Inventory Write-off / Shrinkage), Cr `3xxxxx` (Inventory). |

**JE patterns.**

Inflation:

| Line | Account | Side | Amount |
|------|---------|------|--------|
| 1 | `3xxxxx` (Inventory) | Dr | Amount |
| 2 | `603000` (COGS / Goods Received) | Cr | Amount |

Write-off concealment:

| Line | Account | Side | Amount |
|------|---------|------|--------|
| 1 | `685000` (Exceptional Charges / Shrinkage) | Dr | Amount |
| 2 | `3xxxxx` (Inventory) | Cr | Amount |

**Detection signals.**

- Inventory balance grows faster than COGS (unusual inventory turnover ratio).
- Goods receipts without matching purchase orders.
- Cluster of write-offs following a period of inflation.
- Single user posting both the receipts and the write-offs (SoD violation).

---

### 4.3 Related-Party Transaction Abuse

**Category:** Procurement / Master Data.

**Objective.** A manager approves transactions with a vendor they have an
undisclosed financial interest in. Transactions may be at market price (amounts
are not anomalous), but the approval chain is compromised.

**Why it matters.** This fraud cannot be detected from JE amounts or timing
alone. It requires cross-referencing JE metadata (`created_by`, `approved_by`)
with master data (vendor ownership, bank accounts) and entity graph traversal.
It tests the agent's ability to combine structured data from multiple sources.

**Lifecycle.**

| Stage | Duration | Behaviour |
|-------|----------|-----------|
| 0 — Setup | 1 month | The perpetrator (an employee with approval authority) registers a new vendor or takes over an existing one. The vendor's bank account or registered address links to the perpetrator. |
| 1 — Operation | 6–18 months | Normal-looking procurement JEs (Dr `6xxxxx`, Cr `401000`) posted and approved by the same user or by the perpetrator's direct report. Amounts are within normal ranges for the expense category. |
| 2 — Escalation | 3–6 months | Transaction volume or amount to the related vendor increases, but individual JEs remain within approval thresholds. |

**JE patterns.** Standard procurement — indistinguishable from legitimate JEs
based on amounts and accounts alone.

**Detection signals.**

- Entity graph: vendor's bank account matches an employee's payroll account.
- Approval analysis: a single user both creates and approves JEs to this vendor
  (or the approval chain is: perpetrator → perpetrator's direct report).
- Vendor concentration: an increasing share of a cost centre's spend goes to
  one vendor.
- Master data: vendor registered recently, with address or contact details
  matching an employee's HR record.

**Implementation notes.** Requires master data mutations:

- Insert a related vendor with bank account matching the perpetrator's payroll
  account (triggers `shares_bank_account` edge in the entity graph).
- Set `created_by` = perpetrator on all JEs.
- Optionally set `approved_by` = perpetrator or perpetrator's manager
  (requires the `approval_chain` field on `JournalEntryHeader`).

---

### 4.4 Circular Cash Flow (Single-Entity Journal Variant)

**Category:** Cash / Receivables.

**Objective.** The perpetrator creates a cycle of manual journal entries that
move cash through suspense accounts, making it appear that a receivable has been
collected when it hasn't.

**Why it matters.** This creates a **temporal chain** of 3+ JEs across different
dates that form a logical cycle. The agent must track money flows through
intermediary accounts and reconstruct the circular path. This tests graph-level
reasoning over the accounting flow.

**Lifecycle.**

| Stage | Duration | Behaviour |
|-------|----------|-----------|
| 0 — Fake collection | 1 day | Dr `512000` (Bank), Cr `471000` (Suspense) — simulate incoming cash. |
| 1 — AR clearance | 1–3 days later | Dr `471000` (Suspense), Cr `411000` (AR) — clear the customer receivable using the fake cash. |
| 2 — Reversal / write-off | 15–30 days later | Dr `654000` (Bad Debt Expense), Cr `512000` (Bank) — reverse the fake cash receipt, hidden as a customer write-off. |

**JE patterns.**

Step 1 — Fake cash receipt:

| Line | Account | Side | Amount |
|------|---------|------|--------|
| 1 | `512000` (Bank) | Dr | Amount |
| 2 | `471000` (Suspense) | Cr | Amount |

Step 2 — AR clearance:

| Line | Account | Side | Amount |
|------|---------|------|--------|
| 1 | `471000` (Suspense) | Dr | Amount |
| 2 | `411000` (AR) | Cr | Amount |

Step 3 — Concealed reversal:

| Line | Account | Side | Amount |
|------|---------|------|--------|
| 1 | `654000` (Bad Debt) | Dr | Amount |
| 2 | `512000` (Bank) | Cr | Amount |

**Detection signals.**

- Suspense account (`471000`) used as intermediary between Bank and AR — unusual
  for normal O2C flow.
- Same amount appears in Bank Dr, Suspense Cr, Suspense Dr, AR Cr, Bad Debt Dr,
  Bank Cr within a 30-day window.
- Same `created_by` across all 3 JEs.
- AR cleared without a corresponding customer payment (no matching `reference`
  to a sales invoice).
- `654000` (Bad Debt) entry follows shortly after an AR clearance — unusual
  timing for a genuine write-off.

---

## 5. Implementation Roadmap

### Phase 1: Critical Fixes (Tier 1, items 2.1–2.5)

These must be completed before any experimental runs, as they affect the
validity of evaluation results.

| # | Item | Effort | Files Changed |
|---|------|--------|---------------|
| 1 | `user_id` → `created_by` propagation | S | `enhanced_orchestrator.rs`, all 6 scheme `.rs` files |
| 2 | `target_time` + after-hours timestamps | M | `scheme.rs`, `embezzlement.rs`, `enhanced_orchestrator.rs`, export |
| 3 | Lettrage for embezzlement | M | `JournalEntryLine`, `embezzlement.rs`, `enhanced_orchestrator.rs`, export |
| 4 | Split kickback payment | M | `scheme.rs`, `kickback.rs`, `enhanced_orchestrator.rs` |
| 5 | Ghost employee in master data | M | `shadow_payroll.rs`, `enhanced_orchestrator.rs`, `SchemeAdvancer` |

**Estimated effort:** 2–3 days.

### Phase 2: Structural Improvements (Tier 2, items 3.1–3.5)

| # | Item | Effort | Files Changed |
|---|------|--------|---------------|
| 6 | Cross-scheme perpetrator reuse | M | `SchemeAdvancer`, config schema |
| 7 | Scheme co-occurrence matrix | M | `SchemeAdvancer`, config schema |
| 8 | Concrete concealment patterns (3 types) | L | `scheme.rs`, all scheme files, `enhanced_orchestrator.rs` |
| 9 | Fingerprint-based amount sampling | M | `SchemeStage`, all scheme files |
| 10 | Business calendar awareness | S | `SchemeContext`, `enhanced_orchestrator.rs`, all scheme files |

**Estimated effort:** 4–5 days.

### Phase 3: New Typologies (Tier 3, items 4.1–4.4)

| # | Item | Effort | Files Changed |
|---|------|--------|---------------|
| 11 | Payroll Tax Diversion | L | New scheme file, `SchemeType`, `SchemeAdvancer`, materialiser, config |
| 12 | Inventory Manipulation | L | New scheme file, `SchemeType`, `SchemeAdvancer`, materialiser, config |
| 13 | Related-Party Transaction | L | New scheme file, master data mutations, `SchemeType`, materialiser |
| 14 | Circular Cash Flow | M | New scheme file, `SchemeType`, `SchemeAdvancer`, materialiser, config |

**Estimated effort:** 5–7 days.

### Phase 4: Benchmark Integration

| # | Item | Effort |
|---|------|--------|
| 15 | Update `FRAUD_CATALOGUE` in `prompts.py` to cover all scheme types | S |
| 16 | Update `evaluator.py` scheme matching for new scheme types | M |
| 17 | Update `models.py` `SchemeType` enum and `CORE_SCHEMES` set | S |
| 18 | Add ground-truth perpetrator graph to evaluation data | M |
| 19 | Regenerate benchmark dataset with all fixes applied | S |

**Estimated effort:** 2–3 days.

**Total estimated effort:** 13–18 days.

---

## 6. Impact on Benchmark Difficulty

| Improvement | Effect on Detection Difficulty |
|-------------|-------------------------------|
| `user_id` propagation | Enables SoD analysis (new detection channel) but also makes perpetrator-level evaluation meaningful |
| Lettrage | Closes the trivial AP-ageing shortcut for embezzlement |
| Split payments | Requires arithmetic reconstruction for kickback |
| Ghost employee in master data | Enables employee-level anomaly analysis for shadow payroll |
| Perpetrator reuse | Creates multi-scheme networks requiring cross-scheme correlation |
| Concealment patterns | Adds a deception layer the agent must pierce |
| Fingerprint amounts | Eliminates trivial statistical outlier detection |
| Payroll Tax Diversion | Tests negative-signal (absence) reasoning |
| Related-Party Transaction | Requires JE + master data + graph cross-referencing |
| Circular Cash Flow | Tests temporal chain reconstruction |

These improvements collectively move the benchmark from "can the agent find
obvious anomalies in JE patterns" to "can the agent conduct a realistic forensic
investigation that combines temporal reasoning, entity analysis, graph
traversal, and multi-source evidence synthesis."

---

## 7. Summary Priority Table

| Priority | Item | Section |
|----------|------|---------|
| **Critical** | Propagate `user_id` to `created_by` | §2.1 |
| **Critical** | Implement lettrage for embezzlement | §2.3 |
| **High** | Add `target_time` + after-hours timestamps | §2.2 |
| **High** | Insert ghost employee into master data | §2.5 |
| **High** | Split kickback payment into fragments | §2.4 |
| **High** | Cross-scheme perpetrator reuse | §3.1 |
| **Medium** | Scheme co-occurrence matrix | §3.2 |
| **Medium** | Concrete concealment JE patterns | §3.3 |
| **Medium** | Payroll Tax Diversion scheme | §4.1 |
| **Medium** | Align `FRAUD_CATALOGUE` with generator | §2.6 |
| **Medium** | Fingerprint-based amount sampling | §3.4 |
| **Medium** | Business calendar awareness | §3.5 |
| **Lower** | Inventory Manipulation scheme | §4.2 |
| **Lower** | Related-Party Transaction scheme | §4.3 |
| **Lower** | Circular Cash Flow scheme | §4.4 |
