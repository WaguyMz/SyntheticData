# French GAAP (PCG/FEC) Bank Payments — Design & Implementation Plan

**Status:** Planned (derived from current design, some parts speculative)  
**Created:** 2026-02-27  
**Scope:** Document and refine how Datasynth should handle **bank payments** under **French GAAP / PCG 2024**, including:
- How payments are represented in GL (bank accounts, subledgers, document flows).  
- How they appear in the **FEC** export and fingerprint.  
- How banking flows interact with **O2C/P2P**, **AR/AP**, and **bank reconciliation**.  
- Hooks for **fraud/pathology** injection (Smurfing, Lapping, Remainder Diversion, Circular Funding, etc.).

This plan is written to be implementable even if parts of the current behavior are only partially present; where needed, we call out **“Existing” vs “To Implement”**.

---

## 1. Conceptual Model (French GAAP / PCG)

### 1.1 Bank-Related Accounts (PCG 2024)

- **Class 5 – Comptes financiers**:
  - `512*` – **Banques** (bank current accounts).
  - `514*` – Chèques postaux.
  - `53*` – Caisse, etc. (cash on hand).
- Bank-related **counterparties**:
  - AR: `411*` (Clients) vs bank accounts (`512*`) for **customer receipts**.
  - AP: `401*` (Fournisseurs) vs bank accounts (`512*`) for **vendor payments**.
  - Payroll: `421` (Personnel – rémunérations dues), `43*` (sécurité sociale), `512*` for net payroll disbursements.
  - Taxes: `445*` (TVA), other tax accounts, vs `512*`.

### 1.2 Payment Types

We distinguish **logical payment types** (business semantics) from **instrument types** (cheque, virement, CB, prélèvement).

- **Logical payment types:**
  - Customer receipts (O2C).
  - Vendor disbursements (P2P).
  - Payroll payouts.
  - Tax payments (VAT/TVA, corporate tax).
  - Intercompany transfers.
  - Bank fees and interest.

- **Instrument types:**
  - `CHEQUE`, `VIREMENT`, `CARTE`, `PRELEVEMENT`, `ESPECES`, etc.
  - French FEC itself does not encode the instrument type directly, but **bank statement lines** and internal docs should.

---

## 2. Current / Target Data Flow

### 2.1 GL & Document Flow Perspective

From the docs and architecture:

- O2C document flow:
  - Sales order → delivery → **invoice** → **customer receipt (Payment)**.
  - **JE** for payment:
    - DR `512*` (Bank), CR `411*` (Client).
- P2P document flow:
  - Purchase order → goods receipt → vendor invoice → **vendor payment (Payment)**.
  - JE for payment:
    - DR `401*` (Fournisseur), CR `512*` (Bank).
- Bank reconciliation module:
  - `BankStatementLine`, `BankReconciliation`, `ReconcilingItem` models already exist.
  - Goal: link bank statement lines (what the bank sees) to FEC/GL entries (what the books show).

### 2.2 FEC Export Perspective

- FEC requires one line per accounting entry line, with:
  - `JournalCode`, `JournalLib`, `EcritureNum`, `EcritureDate`, `CompteNum`, `CompteLib`, `Debit`, `Credit`, `DateReglement`, `CodeDevise`, etc.
- For bank payments under PCG:
  - `JournalCode` may be `BQ` (banque) or another bank-specific journal.
  - `CompteNum` for:
    - Bank line: `512xxx`, amount in Debit (incoming) or Credit (outgoing).
    - Counterparty line: `411xxx`, `401xxx`, `421xxx`, `445xxx`, etc.
  - `DateReglement` matches the payment date (value date vs booking date can be modeled optionally).

### 2.3 Bank Reconciliation & Banking Module

The **banking** crate (`datasynth-banking`) introduces:

- `BankStatementLine`: synthetic bank statement entries:
  - Date, amount, description, instrument type, bank account.
- `BankReconciliation` / `ReconcilingItem`:
  - Link FEC JEs (payments, interest, fees) to `BankStatementLine`s.
  - Model timing differences, partial matches, and reconciling items.

**Target behavior:** For every GL payment JE, there is:

- A corresponding or near-corresponding bank statement line.
- Bank reconciliation status (matched/unmatched/partially matched) and reconciling reasons (timing difference, FX, rounding, missing transaction).

---

## 3. Core GL Patterns for Bank Payments

### 3.1 Customer Receipts (O2C)

For each customer payment (full, partial, remainder):

- **JE structure (simplified):**
  - Full payment:
    - DR `512xxx` (Bank)  
    - CR `411xxx` (Customer).
  - Partial payment:
    - DR `512xxx` (amount paid).  
    - CR `411xxx` (same amount).
  - Remainder payment (multipayment):
    - Second JE with DR `512xxx`, CR `411xxx` for remaining amount.

- **FEC representation:**
  - Journal: `BQ` or AR-related bank journal.
  - `EcritureNum`: unique per JE; lines share same number.
  - `EcritureDate`: posting date.
  - `DateReglement`: payment date (could equal posting date or bank value date).

### 3.2 Vendor Payments (P2P)

For each vendor disbursement:

- JE:
  - DR `401xxx` (Fournisseur).
  - CR `512xxx` (Bank).

### 3.3 Payroll Payments

- JE:
  - DR `421` (Personnel – rémunérations dues) and/or `43*` (charges sociales).
  - CR `512xxx` (Bank).

### 3.4 Taxes & Miscellaneous

- VAT (TVA) payments:
  - DR `44551` or relevant TVA account.
  - CR `512xxx`.
- Bank fees:
  - DR `627` (services bancaires).
  - CR `512xxx`.
- Interest:
  - DR `661` (charges d’intérêts) or CR `761` (produits d’intérêts), with contra to `512xxx`.

---

## 4. Implementation Plan in Datasynth

### 4.1 Chart of Accounts & French GAAP Preset

- Ensure French GAAP preset (`pcg_2024.json`) includes:
  - Proper `512*` bank accounts per company (e.g. `512000`, `512001` for multiple banks).
  - Vendor/customer/payroll/tax accounts as per PCG.
- Add metadata:
  - Tag accounts as `is_bank_account = true` (for `512*`, `514*`).
  - Optional: bank name, currency, country.

### 4.2 Payment Generators (Logical Layer)

Extend / verify payment generation modules:

- **O2C Payment Generator:**
  - For each invoice and payment event:
    - Generate `Payment` domain object with:
      - `payment_id`, `customer_id`, `company_code`, `bank_account_id`, `amount`, `payment_date`, `instrument_type`.
      - Allocation to invoices (partial, remainder, on-account).
- **P2P Payment Generator:**
  - For each vendor invoice:
    - Generate `Payment` with vendor, amount, due date vs payment date.
- **Payroll & Tax Payment Generators:**
  - Based on payroll runs and tax due balances.

These generators should be **agnostic** of French GAAP, but must:

- Receive or be able to resolve a `bank_account_id` → GL `512xxx` account.

### 4.3 JE Generation for Bank Payments

In the JE generator or a dedicated **Bank JE generator**:

- For each `Payment`:
  - Resolve:
    - Bank GL account (`512xxx`) for the given `bank_account_id` and company.
    - Counterparty GL account(s):
      - `411*` for customers.
      - `401*` for vendors.
      - `421` / `43*` for payroll.
      - `445*` for taxes.
  - Generate a balanced JE:
    - With deterministic `document_id`, `document_date`, `posting_date`, reference (e.g. `PAY-YYYYMM-SEQ`).
    - Header:
      - `business_process`: `O2C` for customer receipts, `P2P` for vendor, `H2R` for payroll, etc.
      - `source`: `Automated` (from payment engine) or `Manual` (for adjustments).

For **French GAAP** presets:

- Configure:
  - A dedicated **bank journal code** (`BQ`) and label (`Banque`).
  - Ensure FEC export maps bank payment JEs to this journal by default.

### 4.4 Bank Statement Generation

Implement or verify a `BankStatementGenerator` (likely in `datasynth-banking`):

- For each company and each `bank_account_id`:
  - Generate a time series of `BankStatementLine` entries:
    - For each GL payment JE:
      - Create a corresponding bank statement line:
        - `value_date`, `booking_date`, `amount`, `balance_after`, `description`, `instrument_type`.
    - Inject **noise**:
      - Extra lines not yet posted in GL (timing differences).
      - Bank fees and interest lines (may or may not have GL JEs, depending on config).

### 4.5 Bank Reconciliation Logic

Implement or wire up a **Bank Reconciliation Generator**:

- Input:
  - GL payment JEs (filtered on bank accounts).
  - Bank statement lines.
- Output:
  - `BankReconciliation` objects linking:
    - 0/1/N GL payments ↔ 0/1/N bank statement lines.
  - `ReconcilingItem`s for:
    - Timing differences.
    - FX differences.
    - Missing GL (bank-only) or missing bank (GL-only) entries.

For French GAAP FEC:

- We do **not** encode reconciliation directly in FEC, but:
  - Reconciliation summaries can be exported to separate CSV/JSON for the viewer and evaluation.

---

## 5. French GAAP-Specific Concerns

### 5.1 FEC Compliance

- Ensure:
  - Every bank-related JE is exported exactly once to FEC.
  - Lines:
    - Bank accounts: `CompteNum` like `512xxx`, correct debit/credit.
    - Counterparties: `411*`, `401*`, etc.
  - `JournalCode` is consistent (`BQ` or integrated AR/AP/HR journals, but stable per run).
  - Dates:
    - `EcritureDate`: posting date.
    - `DateReglement`: payment date (may equal `EcritureDate`).

### 5.2 PCG Class Usage & Fingerprint

- Fingerprint extraction:
  - Per-account-class stats already use PCG classes:
    - Class 5 (`5xx`) for financial accounts.
    - Classes 6/7 for expenses/revenue.
- For bank payments:
  - Evaluate distribution of **bank account movement**:
    - Volume per `512xxx` class.
    - Typical payment sizes (per instrument and per business process).
  - Expose these as part of `amount_by_account_class` and higher-level stats for French GAAP footprints.

---

## 6. Fraud & Pathology Hooks (Bank-Focused)

Bank payments are central to several fraud schemes:

- **Smurfing (Threshold Evasion)**:
  - Many small payments through `512*` accounts.
- **Lapping**:
  - Customer receipts misapplied; bank flow is legitimate, AR allocation is not.
- **Partial Payment Diversion / Fictitious Remainders**:
  - Missing or fake bank receipts vs expected AR state.
- **Circular Funding / Intercompany Wash Trades**:
  - Intercompany loans and repayments cycling via `512*` at different entities.
- **Vendor Kickbacks / Expense Laundering**:
  - Bank flows between perpetrator-controlled entities and suspicious vendors.

**Plan:**

- Ensure each bank-related JE:
  - Carries:
    - `is_anomaly`, `anomaly_type` when part of a scheme.
    - `scheme_id`, `anomaly_id`.
  - Appears in:
    - Transaction graph (edges of type `Transaction` between `Account` nodes).
    - Banking-specific graph or hypergraph (optional) if we model bank accounts as their own node type.
- Ensure bank statement lines:
  - Also have anomaly flags where appropriate (e.g. unposted bank lines vs GL).

---

## 7. Implementation Phases

### Phase 1 — Baseline French GAAP Bank Flows

1. Confirm / adjust **ChartOfAccountsGenerator** for PCG:
   - Ensure `512*` bank accounts exist per company.
2. Implement or verify:
   - O2C / P2P / Payroll / Tax payment generators creating `Payment` objects with bank account IDs.
3. Implement **Bank JE generator**:
   - Map `Payment` objects to JEs with correct PCG accounts (`512*` vs `411*`, `401*`, `421`, `445*`).
4. Ensure FEC export correctly outputs these bank JEs.

### Phase 2 — Bank Statements & Reconciliation

1. Implement `BankStatementGenerator`:
   - One statement per bank account, per period.
   - Mirror GL payments with realistic timing and extra bank-only entries (fees, interest).
2. Implement `BankReconciliationGenerator`:
   - Generate `BankReconciliation` and `ReconcilingItem`s for each bank account.
3. Export:
   - `bank_statement_lines.csv`, `bank_reconciliations.csv`, `reconciling_items.csv`.

### Phase 3 — Fraud-Ready Integration

1. Wire Smurfing, Lapping, Remainder, Circular Funding, etc. schemes into:
   - Payment generators and Bank JE generator.
   - Banking/graph exports (with flags).
2. Ensure:
   - Pathology-aware labels for:
     - GL bank JEs.
     - Bank statement lines.
3. Validate:
   - Graph signatures for these schemes exist and are visible in `datasynth-graph` exports.

### Phase 4 — Viewer & RIP-GNN

1. Extend the Output Viewer:
   - Dedicated **Banking view**:
     - Per-bank-account movement, reconcile status, reconciling items.
2. Extend RIP-GNN pipelines:
   - Bank-specific node/edge features:
     - Bank account centrality, average transaction size, instrument composition.
   - Use these for:
     - Smurfing detection.
     - Circular funding loops.
     - Unusual reconciliation patterns.

---

## 8. Summary

This plan describes the intended behavior for **French GAAP bank payments** and provides an implementation roadmap that connects:

- PCG-based **chart of accounts**,  
- O2C/P2P/Payroll/Tax **payment generators** and their **bank JEs**,  
- FEC-compliant **export**,  
- **Bank statements** and **reconciliation**, and  
- **Fraud/pathology injection** and graph-based detection (RIP-GNN).  

The design is incremental: we can first solidify French GAAP-compliant bank flows, then layer on reconciliation and fraud schemes, and finally integrate with the viewer and RIP-GNN tooling.

