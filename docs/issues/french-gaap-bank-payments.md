<!-- GitHub issue title: Implement French GAAP (PCG/FEC) bank payments and multipayment-linked fraud schemes -->

## Description

Bank payments under **French GAAP / PCG 2024** are not yet fully wired through the generation pipeline. The goal is to:

- Ensure **Class 5 bank accounts** (`512*`, `514*`) exist in the French GAAP chart-of-accounts preset and are tagged as bank accounts.
- Generate proper **bank payment JEs** (DR/CR `512*` vs `411*`/`401*`/`421`/`445*`) for O2C receipts, P2P disbursements, payroll payouts, and tax payments.
- Produce **FEC-compliant export** for bank JEs (journal code `BQ`, correct `CompteNum`, `DateReglement`).
- Implement **bank statement generation** and **bank reconciliation** that mirror GL payments with realistic timing, fees, and reconciling items.
- Hook bank payment flows into **fraud/pathology injection** (Smurfing, Lapping, Partial Payment Diversion, Circular Funding, etc.) so that scheme-generated JEs carry correct anomaly labels through GL, bank statements, and graph exports.

This issue also covers the **multipayment-dependent fraud schemes** described in the O2C multipayment plan — Lapping, Partial Payment Diversion, Fictitious Remainder Payment, and Duplicate Remainder Posting — which require multipayment infrastructure (tracked separately in the multipayment O2C/P2P issue) plus bank-level visibility.

---

## Goals

| Goal | Description |
|------|-------------|
| **PCG bank accounts** | French GAAP preset includes `512*` bank accounts per company, tagged `is_bank_account = true`. |
| **Bank JE generation** | Map `Payment` objects to balanced JEs with correct PCG accounts (`512*` vs counterparty) and deterministic document IDs. |
| **FEC export** | Bank JEs exported with journal code `BQ`, correct `CompteNum`, `EcritureDate`, `DateReglement`. |
| **Bank statement generation** | Per-bank-account time series of `BankStatementLine` entries mirroring GL payments, plus noise (timing differences, fees, interest). |
| **Bank reconciliation** | `BankReconciliation` / `ReconcilingItem` linking GL payments to bank statement lines, with timing/FX/missing-entry reconciling items. |
| **Multipayment fraud schemes** | Implement Lapping, Partial Payment Diversion, Fictitious Remainder, and Duplicate Remainder as fraud schemes operating over bank payment flows. |
| **Anomaly labeling** | Bank-related JEs and bank statement lines carry `is_anomaly`, `anomaly_type`, `scheme_id` when part of a scheme. |

---

## Phase 1 — Baseline French GAAP bank flows

### Chart of accounts

- [ ] Confirm / adjust `ChartOfAccountsGenerator` for PCG 2024:
  - `512*` bank accounts per company (e.g. `512000`, `512001` for multiple banks).
  - Vendor (`401*`), customer (`411*`), payroll (`421`, `43*`), tax (`445*`) accounts as per PCG.
- [ ] Add metadata: `is_bank_account = true` for `512*` / `514*` accounts; optional bank name, currency, country.

### Payment generators (logical layer)

- [ ] **O2C:** For each customer payment event, generate `Payment` with `bank_account_id` resolvable to GL `512xxx`.
- [ ] **P2P:** For each vendor payment, generate `Payment` with vendor, amount, bank account.
- [ ] **Payroll & Tax:** Based on payroll runs and tax due balances, generate payments with bank counterpart.

### Bank JE generation

- [ ] For each `Payment`, resolve bank GL account (`512xxx`) and counterparty GL account(s) (`411*`, `401*`, `421`/`43*`, `445*`).
- [ ] Generate balanced JE with deterministic `document_id`, correct `posting_date`, and `business_process` tag (O2C/P2P/H2R/Tax).
- [ ] For French GAAP presets, configure dedicated bank journal code (`BQ`) and label (`Banque`).

### FEC export

- [ ] Ensure every bank-related JE is exported exactly once to FEC.
- [ ] Correct `JournalCode` (`BQ`), `CompteNum` (`512xxx` / `411xxx` / `401xxx` etc.), `EcritureDate`, `DateReglement`.

**Code refs:** `crates/datasynth-generators/src/coa_generator.rs`, `crates/datasynth-generators/src/document_flow/`, `crates/datasynth-standards/src/`

---

## Phase 2 — Bank statements & reconciliation

- [ ] Implement `BankStatementGenerator` (in `datasynth-banking`):
  - Per-company, per-bank-account time series of `BankStatementLine` entries.
  - Mirror GL payment JEs with realistic `value_date` / `booking_date` offsets.
  - Inject extra lines: bank fees (`627`), interest (`661`/`761`), timing-difference entries.
- [ ] Implement `BankReconciliationGenerator`:
  - Link 0/1/N GL payments ↔ 0/1/N bank statement lines.
  - Produce `ReconcilingItem`s for timing differences, FX differences, GL-only, bank-only entries.
- [ ] Export: `bank_statement_lines.csv`, `bank_reconciliations.csv`, `reconciling_items.csv`.

**Code refs:** `crates/datasynth-banking/src/`, `crates/datasynth-core/src/models/`

---

## Phase 3 — Multipayment-linked fraud schemes

These schemes depend on multipayment infrastructure (tracked in the multipayment O2C/P2P issue) and bank-level visibility.

### Lapping (Teeming and Lading)

- [ ] Implement `SchemeType::Lapping` / `FraudType::Lapping`:
  - Misappropriate Customer A's payment; apply Customer B's payment to A's invoice, C's to B's, etc.
  - Requires multiple customers, multiple payments, and allocation of one customer's payment to another's invoice.
  - Action type: `MisapplyPayment` (receipt R allocated to invoice I where R's customer ≠ I's customer).
  - Labels: anomaly on the allocation (wrong `invoice_id` for receipt).

### Partial Payment Diversion

- [ ] Implement `SchemeType::PartialPaymentDiversion` / `FraudType::PartialPaymentDiversion`:
  - Customer makes partial payment; remainder is never posted or is stolen.
  - "Missing remainder" where the model would normally generate one.
  - Stages: (1) partial payment received and posted, (2) remainder stolen/not posted, (3) concealment (aging, write-off).

### Fictitious Remainder Payment

- [ ] Implement `SchemeType::FictitiousRemainder` / `FraudType::FictitiousRemainderPayment`:
  - Fake second payment (remainder) posted to clear the receivable and hide prior theft.
  - Remainder receipt is fictitious — no matching bank/cash movement.
  - Anomaly label on the fictitious remainder receipt.

### Duplicate Remainder Posting

- [ ] Implement `FraudType::DuplicateRemainderPosting` (or extend `DuplicatePayment` to AR context):
  - Remainder received once but posted twice — creates overpayment or wrong allocation.
  - Coherence check: sum of allocations > invoice amount.

### Cross-cutting

- [ ] Ensure all fraud-injected bank JEs carry `is_anomaly`, `anomaly_type`, `scheme_id`, `anomaly_id`.
- [ ] Ensure bank statement lines also carry anomaly flags where appropriate (e.g. unposted bank lines vs GL).
- [ ] Validate graph signatures for these schemes are visible in `datasynth-graph` exports.

**Code refs:** `crates/datasynth-generators/src/anomaly/schemes/`, `crates/datasynth-core/src/models/anomaly.rs`, `crates/datasynth-generators/src/document_flow/o2c_generator.rs`

---

## Acceptance criteria

- [ ] French GAAP preset includes properly tagged `512*` bank accounts.
- [ ] Bank payment JEs are generated for O2C, P2P, payroll, and tax flows with correct PCG accounts.
- [ ] FEC export for bank JEs is compliant (journal code, account numbers, dates).
- [ ] Bank statements and reconciliations are generated per bank account per period.
- [ ] Lapping, Partial Payment Diversion, Fictitious Remainder, and Duplicate Remainder schemes are implemented and produce labeled anomalies.
- [ ] All fraud-injected bank JEs and bank statement lines carry correct anomaly metadata.
- [ ] Fingerprint extraction exposes per-class-5 bank account movement distributions.

---

## References

- French GAAP bank payments plan: `docs/plans/2026-02-27-french-gaap-bank-payments.md`
- Multipayment & fraud schemes plan: `docs/plans/2026-02-26-multipayment-o2c-impact-and-fraud-schemes.md`
- Multipayment O2C/P2P issue: `docs/issues/multipayment-o2c-p2p.md`
- O2C generator: `crates/datasynth-generators/src/document_flow/o2c_generator.rs`
- P2P generator: `crates/datasynth-generators/src/document_flow/p2p_generator.rs`
- Fraud schemes: `crates/datasynth-generators/src/anomaly/schemes/`
- Fraud types: `crates/datasynth-core/src/models/anomaly.rs`
- Banking module: `crates/datasynth-banking/src/`
