# Issue: Implement multipayment behaviour in O2C and P2P

**Status:** Open  
**Created:** 2026-02-27  
**Scope:** Document-flow multipayment (partial + remainder payments, multiple payments per invoice) for both Order-to-Cash (O2C) and Procure-to-Pay (P2P).

---

## 1. Summary

Today, **each invoice receives at most one payment** in both O2C and P2P:

- **O2C:** One customer receipt per chain; partial payments produce a single receipt and `PaymentEvent::PartialPayment` with remainder metadata, but **no second (remainder) receipt** is generated; `PaymentEvent::RemainderPayment(Payment)` exists but is never constructed.
- **P2P:** One vendor payment per chain; `P2PPaymentBehavior.partial_payment_rate` exists in config but **partial/remainder payments are not implemented** — every paid invoice gets exactly one full payment.

This issue tracks implementation of **multipayment behaviour** in both processes: multiple payments per invoice (e.g. first partial + one or more remainder payments), with configurable rates and backward-compatible data shapes.

---

## 2. Goals

| Goal | Description |
|------|-------------|
| **O2C remainder receipts** | When payment type is Partial, generate a second receipt (remainder) for a configurable subset of chains, at a date derived from `avg_days_until_remainder`. |
| **P2P partial + remainder** | Introduce partial vendor payments and remainder payments in P2P (first payment &lt; invoice amount; optional second payment for the remainder). |
| **Single data model pattern** | Use a consistent approach: keep “first” payment in existing field, add `remainder_receipts` / `remainder_payments` (or unified `*_payments: Vec<Payment>`) so existing consumers that only read the first payment remain valid. |
| **JEs and export** | Every payment/receipt (first + remainders) generates its own JE and is written to CSV/JSON/Parquet with correct invoice allocation and document references. |
| **Config** | Add config knobs (e.g. `remainder_payment_rate`, optional `max_remainder_payments`) under O2C and P2P payment behaviour. |

---

## 3. O2C multipayment (detailed)

### 3.1 Current state

- `O2CDocumentChain` has `customer_receipt: Option<Payment>` (single receipt).
- `generate_partial_payment` returns `(Payment, remaining_amount, expected_remainder_date)`; only the first payment is stored; remainder is only in `payment_events` as `PartialPayment { ... }`.
- `PaymentEvent::RemainderPayment(Payment)` is defined but never used.

### 3.2 Required changes

| Area | Change |
|------|--------|
| **Config** | Add under `payment_behavior` (e.g. under `partial_payments`): `remainder_payment_rate: f64` (default e.g. 0.85), optionally `max_remainder_payments: u32` (default 1 for “first + one remainder”). |
| **O2CDocumentChain** | Add `remainder_receipts: Vec<Payment>`. Keep `customer_receipt: Option<Payment>` as the first payment. |
| **O2CGenerator::generate_chain** | For `PaymentType::Partial`, after creating the first partial payment and pushing `PaymentEvent::PartialPayment`, with probability `remainder_payment_rate` call a new method to generate a remainder payment at `expected_remainder_date` (or derived date), append to `remainder_receipts`, push `PaymentEvent::RemainderPayment(remainder)`. |
| **O2CGenerator** | Implement `generate_remainder_payment(invoice, company_code, customer, remaining_amount, payment_date, …)` returning `Payment`, allocating `remaining_amount` to the same invoice, with document reference to invoice (no discount on remainder unless desired). |
| **Runtime / orchestrator** | When building `document_flows.payments` from O2C chains, push `customer_receipt` and each element of `remainder_receipts`. |
| **Document-flow JE generator** | Emit JEs for `customer_receipt` and for each entry in `remainder_receipts` (already one JE per payment; ensure remainder receipts are iterated). |
| **Export** | Ensure all receipts (first + remainder) are written to customer_receipts (or equivalent) with same `invoice_id` and allocation. |

### 3.3 References

- Plan: `docs/plans/2026-02-26-multipayment-o2c-impact-and-fraud-schemes.md`
- Code: `crates/datasynth-generators/src/document_flow/o2c_generator.rs` (`O2CDocumentChain`, `generate_chain`, `generate_partial_payment`, `PaymentEvent`).
- JE: `crates/datasynth-generators/src/document_flow/document_flow_je_generator.rs` (`generate_from_o2c_chain`, `generate_from_ar_receipt`).
- Runtime: `crates/datasynth-runtime/src/enhanced_orchestrator.rs` (collection of `customer_receipt` into `document_flows.payments`).

---

## 4. P2P multipayment (detailed)

### 4.1 Current state

- `P2PDocumentChain` has `payment: Option<Payment>` (single payment).
- `P2PPaymentBehavior` has `partial_payment_rate` but the generator **always** pays the full invoice amount in one payment when it pays; partial/remainder logic is not implemented.

### 4.2 Required changes

| Area | Change |
|------|--------|
| **Config** | Add under P2P `payment_behavior`: e.g. `remainder_payment_rate: f64` (share of partial payments that get a remainder), `avg_days_until_remainder: u32`, optionally `max_remainder_payments: u32`. Ensure `partial_payment_rate` drives “first payment &lt; invoice amount” behaviour. |
| **P2PDocumentChain** | Add `remainder_payments: Vec<Payment>`. Keep `payment: Option<Payment>` as the first payment. |
| **P2PGenerator::generate_chain** | When paying an invoice, with probability `partial_payment_rate` generate a **partial** first payment (e.g. configurable share of invoice amount) and compute `remaining_amount`, `expected_remainder_date`. With probability `remainder_payment_rate`, generate a second payment (remainder) at that date and append to `remainder_payments`. Otherwise a single full payment as today. |
| **P2PGenerator** | Add `generate_partial_payment` (first payment &lt; invoice amount, allocate to invoice) and `generate_remainder_payment(invoice, remaining_amount, payment_date, …)` returning `Payment` allocating remainder to same invoice. |
| **Runtime / orchestrator** | When building `document_flows.payments` from P2P chains, push `payment` and each element of `remainder_payments`. |
| **Document-flow JE generator** | Emit JEs for first payment and for each `remainder_payments` entry (one JE per payment). |
| **Export** | All payments (first + remainder) written to payments output with same invoice reference and allocation. |

### 4.3 References

- Code: `crates/datasynth-generators/src/document_flow/p2p_generator.rs` (`P2PDocumentChain`, `generate_chain`, `generate_payment`).
- JE: `crates/datasynth-generators/src/document_flow/document_flow_je_generator.rs` (`generate_from_p2p_chain`, `generate_from_ap_payment`).
- Runtime: `crates/datasynth-runtime/src/enhanced_orchestrator.rs` (collection of `chain.payment` into `document_flows.payments`).

---

## 5. Shared design and validation

### 5.1 Invariants

- Sum of payment amounts (first + remainders) allocated to an invoice ≤ invoice total (O2C: invoice amount; P2P: payable amount).
- Each payment has correct `allocate_to_invoice` and document reference to the same invoice.
- Fiscal period of each payment is determined by its payment date.

### 5.2 Backward compatibility

- Existing fields `customer_receipt` (O2C) and `payment` (P2P) remain; new remainder lists are additive. Code that only reads the first payment continues to work.
- Default `remainder_payment_rate` can be 0 for P2P if desired to avoid behaviour change until explicitly enabled; O2C plan suggests default 0.85 for remainder after partial.

### 5.3 Testing

- **O2C:** Unit test partial chain with remainder: two receipts, two JEs, sum of receipts = invoice amount; unit test partial without remainder: one receipt, open remainder only in events.
- **P2P:** Unit test partial chain with remainder: two payments, two JEs, sum = invoice amount; unit test partial without remainder: one payment; test full-payment chains unchanged.

---

## 6. Acceptance criteria

- [ ] O2C: Config has `remainder_payment_rate` (and optional `max_remainder_payments`) under payment behaviour; `O2CDocumentChain` has `remainder_receipts`; remainder receipts are generated when partial and rate applies; JEs and export include all receipts.
- [ ] P2P: Config supports partial + remainder (e.g. `remainder_payment_rate`, `avg_days_until_remainder`); `P2PDocumentChain` has `remainder_payments`; partial + remainder payments are generated when config applies; JEs and export include all payments.
- [ ] Runtime/orchestrator collects first + remainder payments into `document_flows.payments` for both O2C and P2P.
- [ ] No regression for chains that do not use partial/remainder (full payment, short payment, on-account, etc. for O2C; full payment for P2P).
- [ ] Unit tests added for O2C and P2P multipayment and non-multipayment cases.

---

## 7. Out of scope (this issue)

- Fraud schemes that *use* multipayment (lapping, partial payment diversion, fictitious remainder, duplicate remainder) are described in `docs/plans/2026-02-26-multipayment-o2c-impact-and-fraud-schemes.md` and can be implemented in follow-up issues.
- AR/AP subledger consistency (e.g. applying multiple receipts to one invoice in AR) should be verified in parallel or in a separate task; document-flow changes should align with subledger where both exist.

---

## 8. References

- O2C impact and fraud schemes: `docs/plans/2026-02-26-multipayment-o2c-impact-and-fraud-schemes.md`
- O2C generator: `crates/datasynth-generators/src/document_flow/o2c_generator.rs`
- P2P generator: `crates/datasynth-generators/src/document_flow/p2p_generator.rs`
- Document-flow JE generator: `crates/datasynth-generators/src/document_flow/document_flow_je_generator.rs`
- Enhanced orchestrator: `crates/datasynth-runtime/src/enhanced_orchestrator.rs`
- Config schema (O2C/P2P payment behaviour): `crates/datasynth-config/src/schema.rs`
