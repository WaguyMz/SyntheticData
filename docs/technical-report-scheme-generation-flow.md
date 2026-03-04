# Technical Report: Generation Flow of Multi-Stage Fraud Schemes

**Document version:** 2.0
**Date:** 2026-03-04
**Status:** Revised Implementation Spec – 6 Core Schemes
**Scope:** datasynth-generators (anomaly/schemes), datasynth-runtime (anomaly injection phase), datasynth-config (multi_stage_schemes schema).

---

## 1. Executive Summary

The DataSynth pipeline generates **multi-stage fraud scheme** labels by driving a **SchemeAdvancer** from the runtime: once per simulated month it may start new schemes (by type and probability), and once per calendar day per company it advances all active schemes. Each advancement yields **SchemeAction**s, which are turned into **MultiStageAnomalyLabel**s and then into exported **LabeledAnomaly** records.

This document is the **normative implementation spec** for the **6 fraud schemes** retained after v1.5 scope reduction. It supersedes the descriptive sections of v1.5 with **precise behavioural requirements** derived from the gap analysis in the former Section 12. Each scheme section states what the implementation **must** do, including required JE patterns, account constraints, label semantics, and stage behaviour.

Removed in v2.0 scope: CircularFunding, PhantomWarehousing, IntercompanyWashTrades (all required multi-entity topology not available in the single-FEC scope).

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
│  • maybe_start_scheme(context): independent Bernoulli draw per scheme type;  │
│    instantiates the scheme struct; assigns perpetrator + vendor (kickback)    │
│  • advance_all(context): calls scheme.advance(ctx, rng) per active scheme    │
│  • record_label(anomaly_id, action): pushes MultiStageAnomalyLabel           │
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
| `entries_per_start_attempt` | u64 | Volume gating (default 5000). |
| `max_starts_per_company_per_month` | u32 | Hard cap per company-month (default 10). |
| `max_concurrent_schemes` | usize | Per-type concurrency cap (default 5). |
| `allow_repeat_perpetrators` | bool | Default false. |

Example:

```yaml
anomaly_injection:
  multi_stage_schemes:
    enabled: true
    embezzlement:       { probability: 0.03 }
    revenue_manipulation: { probability: 0.02 }
    kickback:           { probability: 0.015, amount_shift_percent: 0.15,
                          vendor_typical_amount_min: 5000, vendor_typical_amount_max: 50000 }
    triad_bypass:       { probability: 0.008 }
    shadow_payroll:     { probability: 0.008 }
    expense_laundering: { probability: 0.015 }
    entries_per_start_attempt: 10000
    max_starts_per_company_per_month: 2
```

### 3.2 Recommended probabilities

For a single-company FEC with ~10k JEs/year:

| Scheme | Probability | Notes |
|--------|-------------|-------|
| Embezzlement | 0.03 | Most common; long lifecycle |
| Revenue manipulation | 0.02 | Quarterly-bounded |
| Kickback | 0.015 | Requires available vendor |
| Expense laundering | 0.015 | Fan-out; needs multiple counterparties |
| Triad bypass | 0.008 | Short scheme, high severity |
| Shadow payroll | 0.008 | Long cycle; needs employee-naming conventions |

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
* **Action: `GhostHire` (Admin):** No JE.
* **Action: `MonthlyPayroll` (Financial):** Recurring every month-end.
* **Action: `GhostTermination` (Admin/Financial).**

### 6.2 Precise JE Pattern (`MonthlyPayroll`)

| Line | Account | Side | Amount |
| :--- | :--- | :--- | :--- |
| 1 | `641100` (Wages) | Dr | Salary |
| 2 | `645100` (Charges) | Dr | Salary * 0.42 |
| 3 | `421000` (Personnel) | Cr | Salary * 1.42 |

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

