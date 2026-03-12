<!-- GitHub issue title: Implement multipayment behaviour in O2C and P2P document flows -->

## Description

Today, **each invoice receives at most one payment** in both O2C and P2P:

- **O2C:** One customer receipt per chain. Partial payments produce a single receipt and `PaymentEvent::PartialPayment` with remainder metadata, but **no second (remainder) receipt** is generated; `PaymentEvent::RemainderPayment(Payment)` exists but is never constructed.
- **P2P:** One vendor payment per chain. `P2PPaymentBehavior.partial_payment_rate` exists in config but **partial/remainder payments are not implemented** — every paid invoice gets exactly one full payment.

This issue is to implement **multipayment behaviour** in both processes: multiple payments per invoice (e.g. first partial + one or more remainder payments), with configurable rates and backward-compatible data shapes. Fraud schemes that build on multipayment will be addressed in a separate issue.

---

## Goals

| Goal | Description |
|------|-------------|
| **O2C remainder receipts** | When payment type is Partial, generate a second receipt (remainder) for a configurable subset of chains, at a date derived from `avg_days_until_remainder`. |
| **P2P partial + remainder** | Introduce partial vendor payments and remainder payments in P2P (first payment &lt; invoice amount; optional second payment for the remainder). |
| **Consistent data model** | Keep “first” payment in existing field; add `remainder_receipts` / `remainder_payments` so existing consumers that only read the first payment remain valid. |
| **JEs and export** | Every payment/receipt (first + remainders) generates its own JE and is written to CSV/JSON/Parquet with correct invoice allocation and document references. |
| **Config** | Add config knobs (e.g. `remainder_payment_rate`, optional `max_remainder_payments`) under O2C and P2P payment behaviour. |

---

## O2C multipayment

**Current state**

- `O2CDocumentChain` has `customer_receipt: Option<Payment>` (single receipt).
- `generate_partial_payment` returns `(Payment, remaining_amount, expected_remainder_date)`; only the first payment is stored; remainder is only in `payment_events` as `PartialPayment { ... }`.
- `PaymentEvent::RemainderPayment(Payment)` is defined but never used.

**Required changes**

| Area | Change |
|------|--------|
| Config | Add under `payment_behavior` (e.g. under `partial_payments`): `remainder_payment_rate: f64` (default e.g. 0.85), optionally `max_remainder_payments: u32` (default 1). |
| `O2CDocumentChain` | Add `remainder_receipts: Vec<Payment>`. Keep `customer_receipt: Option<Payment>` as the first payment. |
| `O2CGenerator::generate_chain` | For `PaymentType::Partial`, after creating the first partial payment and pushing `PaymentEvent::PartialPayment`, with probability `remainder_payment_rate` generate a remainder payment at `expected_remainder_date`, append to `remainder_receipts`, push `PaymentEvent::RemainderPayment(remainder)`. |
| `O2CGenerator` | Implement `generate_remainder_payment(invoice, company_code, customer, remaining_amount, payment_date, …)` returning `Payment`, allocating `remaining_amount` to the same invoice, with document reference to invoice. |
| Runtime / orchestrator | When building `document_flows.payments` from O2C chains, push `customer_receipt` and each element of `remainder_receipts`. |
| Document-flow JE generator | Emit JEs for `customer_receipt` and for each entry in `remainder_receipts`. |
| Export | Ensure all receipts (first + remainder) are written to customer_receipts output with same `invoice_id` and allocation. |

**Code refs:** `crates/datasynth-generators/src/document_flow/o2c_generator.rs`, `document_flow_je_generator.rs`, `crates/datasynth-runtime/src/enhanced_orchestrator.rs`

---

## P2P multipayment

**Current state**

- `P2PDocumentChain` has `payment: Option<Payment>` (single payment).
- `P2PPaymentBehavior` has `partial_payment_rate` but the generator always pays the full invoice amount in one payment; partial/remainder logic is not implemented.

**Required changes**

| Area | Change |
|------|--------|
| Config | Add under P2P `payment_behavior`: `remainder_payment_rate: f64`, `avg_days_until_remainder: u32`, optionally `max_remainder_payments: u32`. Ensure `partial_payment_rate` drives “first payment &lt; invoice amount”. |
| `P2PDocumentChain` | Add `remainder_payments: Vec<Payment>`. Keep `payment: Option<Payment>` as the first payment. |
| `P2PGenerator::generate_chain` | With probability `partial_payment_rate` generate a partial first payment and compute `remaining_amount`, `expected_remainder_date`. With probability `remainder_payment_rate`, generate a second payment (remainder) at that date and append to `remainder_payments`. Otherwise single full payment as today. |
| `P2PGenerator` | Add `generate_partial_payment` (first payment &lt; invoice amount, allocate to invoice) and `generate_remainder_payment(invoice, remaining_amount, payment_date, …)` returning `Payment` allocating remainder to same invoice. |
| Runtime / orchestrator | When building `document_flows.payments` from P2P chains, push `payment` and each element of `remainder_payments`. |
| Document-flow JE generator | Emit JEs for first payment and for each `remainder_payments` entry. |
| Export | All payments (first + remainder) written to payments output with same invoice reference and allocation. |

**Code refs:** `crates/datasynth-generators/src/document_flow/p2p_generator.rs`, `document_flow_je_generator.rs`, `crates/datasynth-runtime/src/enhanced_orchestrator.rs`

---

## Design and validation

**Invariants**

- Sum of payment amounts (first + remainders) allocated to an invoice ≤ invoice total.
- Each payment has correct `allocate_to_invoice` and document reference to the same invoice.
- Fiscal period of each payment is determined by its payment date.

**Backward compatibility**

- Existing fields `customer_receipt` (O2C) and `payment` (P2P) remain; new remainder lists are additive.
- Default `remainder_payment_rate` can be 0 for P2P if desired until explicitly enabled; O2C default e.g. 0.85 for remainder after partial.

