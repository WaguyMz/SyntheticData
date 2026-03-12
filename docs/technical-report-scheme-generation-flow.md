# Technical Report: Generation Flow of Multi-Stage Fraud Schemes

**Document version:** 3.0
**Date:** 2026-03-12
**Status:** Revised Implementation Spec – 10 Core Schemes
**Scope:** datasynth-generators (anomaly/schemes), datasynth-runtime (anomaly injection phase), datasynth-config (multi_stage_schemes schema).

---

## 1. Executive Summary

The DataSynth pipeline generates **multi-stage fraud scheme** labels by driving a **SchemeAdvancer** from the runtime: once per simulated month it may start new schemes (by type and probability), and once per calendar day per company it advances all active schemes. Each advancement yields **SchemeAction**s, which are turned into **MultiStageAnomalyLabel**s and then into exported **LabeledAnomaly** records.

This document is the **normative implementation spec** for the **10 fraud schemes** in the current scope. It supersedes v2.0 (6 schemes) with 4 new typologies added in v3.0, along with structural improvements (perpetrator reuse, co-occurrence, lettrage, split payments, after-hours timestamps, ghost employee master data, concealment patterns).

Removed from scope: CircularFunding, PhantomWarehousing, IntercompanyWashTrades (all require multi-entity topology not available in single-FEC scope).

### v3.0 Additions (2026-03-12)
- **4 new schemes:** Payroll Tax Diversion, Inventory Manipulation, Related-Party Transaction Abuse, Circular Cash Flow.
- **Embezzlement:** Now emits paired Invoice (`EmbezzleInvoice`) + Payment (`EmbezzlePayment`) with lettrage matching on AP lines. After-hours timestamps (22:00–06:00) on Stage 0/1.
- **Kickback:** Split payments (2–3 partial `PayInflatedInvoicePartial` at T+10, T+15, T+20).
- **Shadow Payroll:** Ghost employee master data record via `ghost_employees()` trait method.
- **All schemes:** `user_id` → `created_by` propagation, `target_time` → `created_at` in materialiser.
- **SchemeAdvancer:** Perpetrator reuse probability, co-occurrence matrix, new scheme type draws.

---

## 2. Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  Config (YAML)                                                                │
│  anomaly_injection.multi_stage_schemes.enabled + per-scheme probability      │
└───────────────────────────────────────┬─────────────────────────────────────┘
                                        │
                                        ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  Runtime (EnhancedOrchestrator)                                               │
│  • inject_anomalies(entries, coa)                                            │
│  • Build EnhancedInjectionConfig from config (all configured scheme probs)   │
│  • Build AnomalyInjector(AnomalyInjectorConfig)                              │
│  • Scheme phase: for each (month_start × company) → maybe_start_scheme();    │
│                 for each (date × company) in range → advance_schemes()        │
│  • process_entries(entries) → injects entry-level anomalies + merges         │
│    scheme labels into result.labels                                           │
└───────────────────────────────────────┬─────────────────────────────────────┘
                                        │
                                        ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  AnomalyInjector (datasynth-generators)                                       │
│  • Holds optional SchemeAdvancer(SchemeAdvancerConfig)                        │
│  • maybe_start_scheme(date, company, users, accounts, counterparties)         │
│  • advance_schemes(date, company) → Vec<SchemeAction>; for each action        │
│    calls advancer.record_label(anomaly_id, action)                            │
│  • process_entries(): merges advancer.get_labels() into labels via            │
│    multi_stage_label_to_labeled_anomaly()                                     │
└───────────────────────────────────────┬─────────────────────────────────────┘
                                        │
                                        ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  SchemeAdvancer                                                               │
│  • maybe_start_schemes(context): independent Bernoulli draw per scheme type; │
│    instantiates scheme struct; assigns perpetrator (with reuse logic) +       │
│    vendor; co-occurrence pass starts linked schemes with same perpetrator     │
│  • advance_all(context): calls scheme.advance(ctx, rng) per active scheme    │
│  • record_label(anomaly_id, action): pushes MultiStageAnomalyLabel           │
│  • ghost_employees(): collects GhostEmployeeRecord from active schemes       │
│  • flush_completed_schemes(context): removes Completed/Terminated/Detected   │
└───────────────────────────────────────┬─────────────────────────────────────┘
                                        │
                                        ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  Export                                                                       │
│  • LabeledAnomaly records → output/labels/anomaly_labels.{json,jsonl}        │
│  • Scheme JEs → journal_entries.csv with document_id = action_id (UUID)      │
│  • Label document_id = "scheme-{action_id}" joins 1:1 to JE document_id      │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Configuration

### 3.1 YAML schema (datasynth-config)

Under `anomaly_injection.multi_stage_schemes`:

| Field | Type | Purpose |
|-------|------|---------|
| `enabled` | bool | Master switch. |
| `embezzlement` | EmbezzlementSchemeConfig | Probability + per-stage overrides. |
| `revenue_manipulation` | RevenueManipulationSchemeConfig | Probability + inflation targets. |
| `kickback` | KickbackSchemeConfig | Probability + inflation %, vendor typical range. |
| `triad_bypass` | SchemeProbabilityOnlyConfig | Single `probability`. |
| `shadow_payroll` | SchemeProbabilityOnlyConfig | Single `probability`. |
| `expense_laundering` | SchemeProbabilityOnlyConfig | Single `probability`. |
| `payroll_tax_diversion` | SchemeProbabilityOnlyConfig | Single `probability`. |
| `inventory_manipulation` | SchemeProbabilityOnlyConfig | Single `probability`. |
| `related_party_abuse` | SchemeProbabilityOnlyConfig | Single `probability`. |
| `circular_cash_flow` | SchemeProbabilityOnlyConfig | Single `probability`. |
| `entries_per_start_attempt` | u64 | Volume gating (default 5000). |
| `max_starts_per_company_per_month` | u32 | Hard cap per company-month (default 10). |
| `max_concurrent_schemes` | usize | Per-type concurrency cap (default 5). |
| `allow_repeat_perpetrators` | bool | Default false. |
| `perpetrator_reuse_probability` | f64 | P(new scheme assigned to existing perpetrator). Default 0. |
| `max_schemes_per_perpetrator` | usize | Cap on schemes per perpetrator. Default 3. |

Example:

```yaml
anomaly_injection:
  multi_stage_schemes:
    enabled: true
    embezzlement:             { probability: 0.03 }
    revenue_manipulation:     { probability: 0.02 }
    kickback:                 { probability: 0.015, amount_shift_percent: 0.15,
                                vendor_typical_amount_min: 5000, vendor_typical_amount_max: 50000 }
    triad_bypass:             { probability: 0.008 }
    shadow_payroll:           { probability: 0.008 }
    expense_laundering:       { probability: 0.015 }
    payroll_tax_diversion:    { probability: 0.008 }
    inventory_manipulation:   { probability: 0.005 }
    related_party_abuse:      { probability: 0.005 }
    circular_cash_flow:       { probability: 0.005 }
    entries_per_start_attempt: 10000
    max_starts_per_company_per_month: 2
    allow_repeat_perpetrators: true
    perpetrator_reuse_probability: 0.30
    max_schemes_per_perpetrator: 3
```

### 3.2 Recommended probabilities

For a single-company FEC with ~10k JEs/year:

| Scheme | Probability | Category | Notes |
|--------|-------------|----------|-------|
| Embezzlement | 0.03 | Sequential | Most common; long lifecycle |
| Revenue manipulation | 0.02 | Volume | Quarterly-bounded |
| Kickback | 0.015 | Relational | Requires available vendor |
| Expense laundering | 0.015 | Volume | Fan-out; needs multiple counterparties |
| Triad bypass | 0.008 | Relational | Short scheme, high severity |
| Shadow payroll | 0.008 | Sequential | Long cycle; needs employee-naming conventions |
| Payroll tax diversion | 0.008 | Negative-Signal | Negative-signal fraud (absence of remittances) |
| Inventory manipulation | 0.005 | Balance-Sheet | Balance-sheet fraud via fictitious GRs + write-off |
| Related-party abuse | 0.005 | Cross-Domain | Requires master data + entity graph correlation |
| Circular cash flow | 0.005 | Temporal-Chain | 3-step temporal chain (fake receipt → AR clear → bad debt) |

Probabilities are **independent** Bernoulli draws — they do not need to sum to 1.

---

## 4. Runtime Scheme Phase

1. **Inputs:** `entries`, `coa`, `master_data` (employees, vendors, customers).
2. **Context per advance:** `users` from `master_data.employees[*].user_id`, `accounts` from `coa.get_postable_accounts()`, `counterparties` from vendor + customer IDs.
3. **Maybe-start:** For each `(month_start_date, company_code)`, call `maybe_start_scheme(context)`.
4. **Advance:** For each `(date, company_code)` in the date range, call `advance_schemes(date, company_code)`.
5. **Labels merge:** After `process_entries`, `advancer.get_labels()` is converted to `LabeledAnomaly` and merged into the injector's label list.
6. **No JEs are written inside the scheme logic.** The runtime materialises `SchemeAction`s into journal entries in a separate `materialize_scheme_actions` pass.

---

## 5. Label and Document ID Semantics

This is the **normative** mapping. All implementations must conform.

| Field | Value | Meaning |
|-------|-------|---------|
| `scenario_id` | `scheme_id` (UUID) | Groups all labels for one scheme instance. |
| `document_id` | `"scheme-{action_id}"` | One per SchemeAction; unique across the dataset. |
| `document_type` | `"scheme"` | |
| Materialized JE `document_id` | `action_id` (UUID, bare) | Joins 1:1 to label via `document_id = "scheme-{action_id}"`. |

**Consequence:** The viewer resolves "Concerned transaction" by stripping the `"scheme-"` prefix from `label.document_id` to obtain the bare `action_id`, then looking up `journal_entries` where `document_id = action_id`. This is a 1:1 join: one action = one JE header = one label.

To group all JEs for a scheme instance, use `scenario_id` (scheme_id): all JEs whose `document_id` appears in a label with that `scenario_id` belong to the instance.

---

## 6. Per-Scheme Specification

Each scheme implements the `FraudScheme` trait. The sections below are **implementation requirements**, not descriptions of the current code.

---
# Implementation Specification: Multi-Stage Fraud Schemes (v2.0)

## 1. Global Implementation Constants
To ensure consistency across all generators, the following defaults must be applied unless overridden by specific scheme logic:

* **VAT Rate:** 20% (Standard French rate).
* **Account Formatting:** Use French Plan Comptable Général (PCG) standards.
* **ID Generation:** 
  * `scheme_id`: Persistent UUID for the instance.
  * `action_id`: Unique UUID per generated JE.
  * `scenario_id`: The `scheme_id` string.
  * `document_id`: `"scheme-{action_id}"` for labels; `action_id` for JEs.

---

## 2. Scheme 1: Gradual Embezzlement (Salami Slicing)

* **Process Category:** End-to-End P2P (Invoice + Payment + Reconciliation).
* **Objective:** Slow drain of cash via high-volume, low-value fictitious invoices, obfuscated through account rotation and after-hours activity.

### 2.1 Lifecycle & Stages
* **Stage 0 (Testing):** Sub-threshold amounts (< €300).
* **Stage 1 (Escalation):** Increase in frequency using a "Shell" vendor.
* **Stage 2 (Acceleration):** Larger amounts, multiple entries per week.
* **Stage 3 (Desperation):** Rapid drain, rushed clusters near month-end.

### 2.2 Precise JE Pattern (Per Action)
Each "Action" in this scheme now consists of two distinct JEs: the Invoice and the Payment, which must be reconciled via Lettrage.

**Part A: The Fictitious Invoice (Action Type: `EmbezzleInvoice`)**

| Line | Account | Description | Side | Amount |
| :--- | :--- | :--- | :--- | :--- |
| 1 | `6xxxxx` | Rotating Expense Account (See 2.4) | Dr | Amount / 1.20 |
| 2 | `445660` | Deductible VAT | Dr | Amount - (Amount / 1.20) |
| 3 | `401000` | Vendor AP (Aux: `SHELL_VND_{scheme_id}`) | Cr | Amount |

**Part B: The Payment (Action Type: `EmbezzlePayment`)**
*Scheduled T+15 to T+30 days after Part A.*

| Line | Account | Description | Side | Amount |
| :--- | :--- | :--- | :--- | :--- |
| 1 | `401000` | Vendor AP (Aux: `SHELL_VND_{scheme_id}`) | Dr | Amount |
| 2 | `512000` | Bank | Cr | Amount |

### 2.3 Lettrage (Matching) Requirement
The implementation must perform "Lettrage" (Matching). Once Part B is posted:
* Both lines on account `401000` (the Cr from Invoice and Dr from Payment) must be tagged with a unique matching code (e.g., `L-101`, `AA`, `B2`).
* This simulates a clean auxiliary ledger, making the fraud harder to detect via "unpaid balance" reports.

### 2.4 Implementation Constraints & Behavioral Details
* **Expense Account Rotation:** To avoid "spikes" in a single account, the generator must rotate the Class 6 account used in Part A:
  * **Stage 0:** `628000` (Misc Services), `614200` (Stationery).
  * **Stage 1+:** `622000` (Subcontracting), `627000` (Travel), `606700` (Energy).
* **Metadata & Forensic Markers:**
  * `user_id`: Fixed perpetrator UUID across all actions (Violates Segregation of Duties).
  * `timestamps`: Stage 0/1 JEs must be timestamped between 22:00-06:00 CET (After-hours).
  * `reference`: Fictitious ID like `"INV-SHELL-{random:6}"`.
  * `approval_flags`: Set to `"AUTO_APPROVED"` (due to < €300 threshold).
  * `is_synthetic_shell`: Must be set to `true` in `master_data.vendors`.

---

## 3. Scheme 2: Revenue Manipulation (Window Dressing)

* **Process Category:** Order-to-Cash (O2C) - Accrual Fraud.
* **Objective:** Inflate quarterly performance by recognizing revenue early and reversing it in the next period.

### 3.1 Lifecycle & Stages
**The Cycle:** Operates on a Quarterly basis (Q1-Q4).
* **Forward Phase:** Last 3 days of Month 3, 6, 9, or 12.
* **Reversal Phase:** First 5 days of the following month.

### 3.2 Precise JE Pattern

**Part A: The Inflation (Action Type: `ForwardRevenue`)**

| Line | Account | Description | Side | Amount |
| :--- | :--- | :--- | :--- | :--- |
| 1 | `411000` | Customer AR (Aux: `DUM-CUST-{scheme_id}`) | Dr | Amount |
| 2 | `70xxxx` | Revenue Account (e.g., `706000`) | Cr | Amount |

**Part B: The Reversal (Action Type: `ReverseRevenue`)**
*Must be scheduled 1-5 days after Part A.*

| Line | Account | Description | Side | Amount |
| :--- | :--- | :--- | :--- | :--- |
| 1 | `70xxxx` | Revenue Account (Mirror of Part A) | Dr | Amount |
| 2 | `411000` | Customer AR (Mirror of Part A) | Cr | Amount |

---

## 4. Scheme 3: Vendor Kickback

* **Process Category:** End-to-End P2P (Invoice + Multi-Payment + Bribe).
* **Objective:** Collude with a real vendor to overpay; hide the overpayment via fragmented payments.

### 4.1 Amount Logic
* `UsualPrice` = Baseline.
* `InflatedPrice` = `UsualPrice` * (1 + random(0.10, 0.25)).
* `Bribe` = (`InflatedPrice` - `UsualPrice`) * 0.15.

### 4.2 Precise JE Pattern

**Action 1: `InflateInvoice` (Invoice Step)**

| Line | Account | Description | Side | Amount |
| :--- | :--- | :--- | :--- | :--- |
| 1 | `622600` | Consulting Fees | Dr | InflatedPrice / 1.20 |
| 2 | `445660` | VAT | Dr | InflatedPrice - Net |
| 3 | `401000` | Vendor AP (Aux: `COLLUDING_VND_ID`) | Cr | InflatedPrice |

**Action 2: `PayInflatedInvoice` (The Multi-Payment/Split)**
To hide the overpayment, the generator must split the payment of the `InflatedPrice` into 2 or 3 separate JEs on different dates (e.g., T+10, T+15, T+20).
* **Payment JE 1:** Dr `401000` (60% of amount), Cr `512000`.
* **Payment JE 2:** Dr `401000` (40% of amount), Cr `512000`.
* **Requirement:** All payments must share the same reference field (the invoice ID) to allow partial lettrage until the final payment is made.

**Action 3: `BribePayout` (The Kickback)**
Uses a DIFFERENT shell vendor.

---

## 5. Scheme 4: Triad Bypass

* **Process Category:** P2P - Payment Circumvention.
* **Objective:** Directly pay a vendor without an invoice/PO, referencing a real historic Invoice ID.

### 5.1 Logic Flow
* **Search:** Find a real `document_id` (`401xxx` Credit).
* **Reuse:** Save `reused_invoice_id` and `vendor_id`.
* **Bypass:** Emit a direct Bank -> Vendor payment JE.

### 5.2 Precise JE Pattern

| Line | Account | Description | Side | Amount |
| :--- | :--- | :--- | :--- | :--- |
| 1 | `401000` | Vendor AP (Aux: `REUSED_VND_ID`) | Dr | Amount |
| 2 | `512000` | Bank | Cr | Amount |

---

## 6. Scheme 5: Shadow Payroll (Ghost Employees)

* **Process Category:** Human Resources / Payroll Cycle.
* **Objective:** Add a fake employee; collect monthly salary.

### 6.1 Lifecycle
* **Action: `GhostHire` (Admin):** No JE. Creates a `GhostEmployeeRecord` in master data via `ghost_employees()` trait method (v3.0).
* **Action: `MonthlyPayroll` (Financial):** Recurring every month-end.
* **Action: `GhostTermination` (Admin/Financial).**

### 6.2 Precise JE Pattern (`MonthlyPayroll`)

| Line | Account | Side | Amount |
| :--- | :--- | :--- | :--- |
| 1 | `641100` (Wages) | Dr | Salary |
| 2 | `645100` (Charges) | Dr | Salary * 0.42 |
| 3 | `421000` (Personnel) | Cr | Salary * 1.42 |

### 6.3 Master Data Mutation (v3.0)
When `GhostHire` fires, the scheme returns a `GhostEmployeeRecord { employee_id, display_name, hire_date, perpetrator_id, scheme_id }`. The `SchemeAdvancer.ghost_employees()` method collects these for downstream master data injection.

---

## 7. Scheme 6: Expense Laundering

* **Process Category:** P2P - Multi-hop Wash.
* **Objective:** Obfuscate large outflows via a "Suspense" account bounce across multiple shell vendors.

### 7.1 Precise JE Pattern (The "Suspense Bounce")

**Step 1: Inbound**

| Line | Account | Side | Amount |
| :--- | :--- | :--- | :--- |
| 1 | `628000` | Dr | Amount / 1.20 |
| 2 | `445660` | Dr | VAT |
| 3 | `471000` (Suspense) | Cr | Amount |

**Step 2: Outbound**
*2 days later*

| Line | Account | Side | Amount |
| :--- | :--- | :--- | :--- |
| 1 | `471000` (Suspense) | Dr | Amount |
| 2 | `401000` (Vendor AP) | Cr | Amount |

---

## 8. Scheme 7: Payroll Tax Diversion (v3.0)

* **Process Category:** HR / Payroll → Treasury.
* **Objective:** Perpetrator diverts payroll tax remittances; the fraud signal is the **absence** of expected outflows (negative-signal fraud).
* **Detection Challenge:** Requires the agent to reason about what *should* have happened but did not.

### 8.1 Lifecycle & Stages

* **Stage 0 (Baseline, 2 months):** Normal payroll processing — taxes withheld and remitted on schedule. Establishes the "expected" pattern.
* **Stage 1 (Diversion, 3 months):** Tax remittance JEs are **suppressed** (action `SuppressRemittance` — no JE emitted). The withheld amount remains in liability account `431000`.
* **Stage 2 (Cover-up, 2 months):** Fictitious remittance via suspense account to conceal the growing liability (`ConcealRemittance`: Dr `431000`, Cr `471000`).

### 8.2 Precise JE Patterns

**`SuppressRemittance` (Stage 1):** No JE produced — this is the signal. The expected monthly remittance (Dr `431000` Social Security Payable, Cr `512000` Bank) is absent.

**`ConcealRemittance` (Stage 2):**

| Line | Account | Description | Side | Amount |
| :--- | :--- | :--- | :--- | :--- |
| 1 | `431000` | Social Security Payable | Dr | Amount |
| 2 | `471000` | Suspense | Cr | Amount |

### 8.3 Forensic Markers
* Growing balance on `431000` with no matching bank outflow.
* Concealment JEs to suspense have perpetrator's `user_id` and no external reference.

---

## 9. Scheme 8: Inventory Manipulation (v3.0)

* **Process Category:** Supply Chain / Balance Sheet.
* **Objective:** Inflate inventory value via fictitious goods receipts, siphon physical inventory, then write off as shrinkage.
* **Detection Challenge:** Balance-sheet fraud; requires reasoning about inventory flow consistency.

### 9.1 Lifecycle & Stages

* **Stage 0 (Inflation, 3 months):** Fictitious goods receipt notes (`FictitiousGoodsReceipt`) increase inventory without corresponding POs.
* **Stage 1 (Siphoning, 2 months):** Physical theft — no JE emitted. Inventory physically leaves but the books still show it.
* **Stage 2 (Write-off, 1 month):** `InventoryWriteOff` as spoilage/shrinkage to reconcile books with physical count.

### 9.2 Precise JE Patterns

**`FictitiousGoodsReceipt` (Stage 0):**

| Line | Account | Description | Side | Amount |
| :--- | :--- | :--- | :--- | :--- |
| 1 | `370000` | Inventory | Dr | Amount |
| 2 | `603000` | COGS Reversal / Purchases | Cr | Amount |

**`InventoryWriteOff` (Stage 2):**

| Line | Account | Description | Side | Amount |
| :--- | :--- | :--- | :--- | :--- |
| 1 | `685000` | Exceptional Charges (Shrinkage) | Dr | Amount |
| 2 | `370000` | Inventory | Cr | Amount |

### 9.3 Forensic Markers
* Goods receipts without matching PO references.
* Large write-off amounts at fiscal year-end clustered under a single perpetrator.
* Inventory account balance inflation followed by sudden reduction.

---

## 10. Scheme 9: Related-Party Transaction Abuse (v3.0)

* **Process Category:** P2P — Cross-Domain / Entity Network.
* **Objective:** Register a fictitious vendor controlled by the perpetrator and route procurement through it at inflated prices.
* **Detection Challenge:** Requires cross-referencing JE metadata with vendor master data (shared bank accounts, addresses) and entity relationship graphs.

### 10.1 Lifecycle & Stages

* **Stage 0 (Setup, 1 month):** Register a fictitious vendor (`CreateFictitiousVendor`). The vendor shares bank account or address details with the perpetrator or another existing vendor.
* **Stage 1 (Operation, 4 months):** Normal-looking procurement JEs routed to the related vendor (`RelatedPartyProcurement`). Amounts start conservative.
* **Stage 2 (Escalation, 2 months):** Volume and amounts increase. Multiple JEs per advance.

### 10.2 Precise JE Pattern

**`RelatedPartyProcurement` (Stages 1–2):**

| Line | Account | Description | Side | Amount |
| :--- | :--- | :--- | :--- | :--- |
| 1 | `622600` | Consulting / Service Fees | Dr | Amount |
| 2 | `401000` | Vendor AP (Aux: `REL_VND_{scheme_id}`) | Cr | Amount |

### 10.3 Forensic Markers
* Vendor created shortly before first transaction.
* Shared bank account between vendor and employee (or another vendor).
* Concentration of spend with a single new vendor.
* Perpetrator is both the approver and the vendor contact.

---

## 11. Scheme 10: Circular Cash Flow (v3.0)

* **Process Category:** Treasury / AR — Temporal Chain.
* **Objective:** Create the illusion of cash collections by cycling funds through suspense accounts. A 3-step temporal chain of interdependent JEs.
* **Detection Challenge:** The agent must reconstruct the temporal chain across 3 separate JEs that, individually, may appear routine.

### 11.1 Lifecycle

Multi-cycle scheme. Each cycle produces 3 JEs at staggered dates:
1. **`FakeCashReceipt`** (T+0): Dr Bank, Cr Suspense.
2. **`ClearARViaSuspense`** (T+5): Dr Suspense, Cr AR.
3. **`ConcealAsBadDebt`** (T+10): Dr Bad Debt Expense, Cr Bank.

Cycles repeat every ~30 days with escalating amounts.

### 11.2 Precise JE Patterns

**`FakeCashReceipt`:**

| Line | Account | Description | Side | Amount |
| :--- | :--- | :--- | :--- | :--- |
| 1 | `512000` | Bank | Dr | Amount |
| 2 | `471000` | Suspense | Cr | Amount |

**`ClearARViaSuspense`:**

| Line | Account | Description | Side | Amount |
| :--- | :--- | :--- | :--- | :--- |
| 1 | `471000` | Suspense | Dr | Amount |
| 2 | `411000` | Accounts Receivable | Cr | Amount |

**`ConcealAsBadDebt`:**

| Line | Account | Description | Side | Amount |
| :--- | :--- | :--- | :--- | :--- |
| 1 | `654000` | Bad Debt Expense | Dr | Amount |
| 2 | `512000` | Bank | Cr | Amount |

### 11.3 Forensic Markers
* Suspense account acts as a bridge between otherwise unrelated bank and AR movements.
* The 3 JEs share the same `user_id` and approximately equal amounts.
* Bad debt write-offs consistently match recent "cash receipts."
* Temporal pattern: receipt → clearance → write-off at regular intervals.

---

## 12. Cross-Scheme Structural Features (v3.0)

### 12.1 Perpetrator Reuse
When `allow_repeat_perpetrators: true` and `perpetrator_reuse_probability > 0`, a newly started scheme may be assigned to an existing perpetrator rather than picking a fresh employee. This models the real-world pattern where a single fraudster operates multiple schemes. Controlled by `max_schemes_per_perpetrator`.

### 12.2 Scheme Co-Occurrence
The `SchemeAdvancerConfig.co_occurrence` matrix maps `(SchemeType, SchemeType)` pairs to conditional probabilities. When a scheme starts, the advancer checks whether linked schemes should also start with the same perpetrator (e.g., embezzlement often co-occurs with expense laundering).

### 12.3 Concealment Patterns
All schemes may emit concealment JEs in later stages:
* **`ConcealReclassify`:** Reclassify suspicious amounts from suspense to an expense account.
* **`ConcealContraEntry` + `ConcealContraReverse`:** Paired contra-entries that net to zero but obscure the trail.

### 12.4 Business Calendar Awareness
`SchemeContext` now includes `is_holiday_period` and `days_to_fiscal_year_end`. Schemes can use these to:
* Accelerate near fiscal year-end (desperation behaviour).
* Exploit reduced oversight during holiday periods.

---

