# VAT in O2C Document-Flow Journal Entries — Impact & Severity

**Status:** Planned (implementation deferred)  
**Created:** 2026-02-25  
**Scope:** Order-to-Cash (O2C) document-flow JEs when tax/VAT is enabled.

---

## 1. Executive Summary

When **tax** (and VAT/GST) is enabled, the **AR subledger** path posts customer invoice JEs with correct VAT treatment: **DR AR (gross), CR Revenue (net), CR VAT Payable (tax)**. The **O2C document-flow** path instead posts **DR AR (gross), CR Revenue (gross)** — no separate VAT credit line. This document describes the **impact** of adding VAT management to O2C document-flow lines and the **severity** of the current gap. Implementation is planned for a later release.

---

## 2. Current State

### 2.1 Two O2C JE Paths

| Path | Component | Customer invoice → JE logic | VAT in credit lines? |
|------|------------|-----------------------------|----------------------|
| **Document flow** | `DocumentFlowJeGenerator::generate_from_customer_invoice` | DR AR = `total_gross_amount`<br>CR Revenue = `total_gross_amount` | **No** |
| **AR subledger** | `ARGenerator::generate_invoice_je` | DR AR = gross<br>CR Revenue = net<br>CR VAT Payable = tax (when `tax_amount > 0`) | **Yes** |

### 2.2 Data Already Available

- **`CustomerInvoice`** (document-flow model) already has:
  - `total_net_amount`
  - `total_tax_amount`
  - `total_gross_amount` (= net + tax)
- **`DocumentFlowJeConfig`** currently has: `ar_account`, `revenue_account`, `cogs_account`, etc. **No** VAT/VAT Payable account (would need to be added or sourced from a shared tax-account constant).
- **Tax accounts:** `datasynth_core::accounts::tax_accounts::VAT_PAYABLE` exists and is used in the AR subledger.

### 2.3 When Each Path Is Used

- **Document-flow JEs** are produced by the main pipeline when O2C document flows are enabled; they tie JEs to document chains (Sales Order → Delivery → Customer Invoice → Customer Receipt).
- **AR subledger JEs** are produced when the AR subledger generates its own invoices and corresponding JEs (separate from the document-flow Customer Invoice objects).

So in runs that rely on **document-flow** O2C for GL postings, all customer-invoice JEs currently show **revenue at gross** and **no VAT Payable credit**.

---

## 3. Impact of Adding VAT to O2C Document-Flow Lines

### 3.1 Functional Impact

| Area | Impact |
|------|--------|
| **Accounting correctness** | Revenue will be posted at **net**; VAT collected will appear as **VAT Payable** (liability). Aligns document-flow JEs with standard treatment and with the AR subledger. |
| **VAT reporting & reconciliation** | Outputs that aggregate by account (e.g. trial balance, FEC, analytics) will show VAT Payable movement on O2C invoices when tax is enabled, improving consistency with tax returns and tax_lines. |
| **Audit & compliance** | Simulated data will better reflect real ERP behaviour (net revenue + separate tax line), improving usefulness for audit testing and SOX/ISA scenarios. |
| **Consistency** | Single, consistent rule: “when tax is enabled and invoice has tax, post net revenue + VAT Payable” across both document-flow and AR subledger. |

### 3.2 Scope of Code Changes

| Location | Change |
|----------|--------|
| **`DocumentFlowJeConfig`** | Add optional `vat_payable_account: Option<String>` (or always use `tax_accounts::VAT_PAYABLE` and no config if preferred). |
| **`DocumentFlowJeGenerator::generate_from_customer_invoice`** | When `invoice.total_tax_amount > 0`: post DR AR (gross), CR Revenue (`total_net_amount`), CR VAT Payable (`total_tax_amount`); set `tax_code` (e.g. `"VAT"`) on the VAT line. When `total_tax_amount == 0`, keep current behaviour (DR AR, CR Revenue, both gross = net). |
| **French GAAP (PCG)** | If a PCG VAT Payable account exists, use it in `DocumentFlowJeConfig::french_gaap()`; otherwise reuse same logic with PCG account constant. |
| **Config schema / presets** | No strict requirement; config can remain optional if VAT account is derived from a constant. Optionally expose `document_flows.vat_payable_account` for overrides. |

### 3.3 Dependencies and Assumptions

- **Tax enabled:** Change only affects runs with `tax.enabled: true` (and typically `vat_gst.enabled: true`). When tax is disabled, `total_tax_amount` is typically zero; behaviour stays as today (single credit to revenue at gross).
- **CustomerInvoice population:** O2C document-flow must already set `total_net_amount` and `total_tax_amount` on `CustomerInvoice` (e.g. from order/item-level tax). No change to upstream document generation assumed in this impact note.
- **Line numbering:** Adding a third line (VAT) may require explicit line numbers (e.g. 1: AR, 2: Revenue, 3: VAT Payable) for consistency with AR subledger and for FEC/export.

### 3.4 Downstream / Export Impact

- **Journal entry exports (CSV, JSON, Parquet):** One extra line per customer-invoice JE when tax > 0; `tax_code` and `tax_amount` populated on that line.
- **FEC:** Extra lines with correct Compte, Libellé, Débit/Crédit, and optional tax-related columns if present in the FEC schema.
- **Trial balance / analytics:** VAT Payable will show higher credits (and revenue lower) for O2C when VAT is enabled — expected and desired.
- **Process mining (OCEL):** No structural change; same events, more lines per JE where applicable.

### 3.5 Risks of Implementing

| Risk | Mitigation |
|------|------------|
| **Double-counting VAT** | Only add VAT line when `total_tax_amount > 0`; use same account as AR subledger (`VAT_PAYABLE`). Ensure no other component posts the same invoice’s VAT again. |
| **Config drift** | Use shared constant or single config field for VAT Payable account so document-flow and AR subledger stay aligned. |
| **Backward compatibility** | Behaviour change only when tax is enabled; existing runs with tax disabled unchanged. Document in release notes. |

---

## 4. Severity of the Current Gap

### 4.1 Severity Level: **Medium**

- **Not critical:** Pipeline runs successfully; no crash or data corruption. Revenue and AR totals are consistent at gross level.
- **Not low:** When VAT is enabled, document-flow JEs are **accounting-incorrect** (revenue at gross, no VAT liability), which affects realism, reconciliation, and any use case that relies on document-flow JEs for tax or audit.

### 4.2 When the Gap Matters

| Scenario | Severity |
|----------|----------|
| **Tax disabled** | **N/A** — gap does not apply. |
| **Tax enabled, AR subledger only** | **Low** — if JEs are taken only from AR subledger, behaviour is already correct. |
| **Tax enabled, document-flow JEs used for GL** | **Medium** — revenue and VAT are wrong in GL; trial balance and VAT reporting will not match tax returns / tax_lines. |
| **Tax enabled + FEC / French GAAP** | **Medium** — FEC will show revenue at gross and no VAT Payable for O2C document-flow invoices; French statutory view is inaccurate. |
| **Audit / SOX / ISA testing** | **Medium** — auditors or tests comparing document flow to GL will see a mismatch (document has net+tax, GL has gross revenue only). |

### 4.3 Who It Affects

- **Users enabling tax and using O2C document flows** for reporting, reconciliation, or audit-style analytics.
- **Implementations that compare document-level tax to GL** (e.g. tax_lines vs journal_entries).
- **French GAAP / FEC users** who enable VAT and expect VAT Payable to move on sales.

### 4.4 When It Can Be Deferred

- Tax is not enabled, or VAT is not used.
- Only AR subledger JEs are consumed for GL (document-flow JEs ignored or not generated).
- Use case is purely volume/performance or non-tax (e.g. process mining structure only, no GL reconciliation).

---

## 5. Implementation Outline (For Later)

1. **Config (optional):** Add `vat_payable_account` to `DocumentFlowJeConfig` (default: `tax_accounts::VAT_PAYABLE`). Update `DocumentFlowJeConfig::french_gaap()` if a PCG VAT account exists.
2. **Logic in `generate_from_customer_invoice`:**
   - If `invoice.total_tax_amount > Decimal::ZERO`:  
     - Line 1: DR AR = `invoice.total_gross_amount`.  
     - Line 2: CR Revenue = `invoice.total_net_amount`.  
     - Line 3: CR VAT Payable = `invoice.total_tax_amount`, `tax_code: Some("VAT")`, `tax_amount: Some(invoice.total_tax_amount)`.
   - Else: keep current behaviour (DR AR, CR Revenue, both `total_gross_amount`).
3. **Tests:** Unit test with `total_tax_amount > 0` (three lines, correct amounts and tax_code); regression with `total_tax_amount == 0` (unchanged two-line JE).
4. **Docs:** Update `docs/src/advanced/tax-accounting.md` and any O2C/document-flow docs to state that O2C document-flow JEs post net revenue + VAT Payable when tax is enabled.

---

## 6. References

- **Document-flow JE generator:** `crates/datasynth-generators/src/document_flow/document_flow_je_generator.rs` — `generate_from_customer_invoice`.
- **AR subledger (reference implementation):** `crates/datasynth-generators/src/subledger/ar_generator.rs` — `generate_invoice_je` (DR AR, CR Revenue net, CR VAT Payable).
- **Tax accounting overview:** `docs/src/advanced/tax-accounting.md`.
- **CustomerInvoice model:** `crates/datasynth-core/src/models/documents/customer_invoice.rs` — `total_net_amount`, `total_tax_amount`, `total_gross_amount`.
