<!-- GitHub issue title: Implement additional fraud schemes and pathology lab for Graph RIP-GNN -->

## Description

The Graph RIP-GNN integration plan identifies **10 fraud pathologies** needed for the paper *"Graph RIP-GNN: Relational Integrity-Preserving Graph Neural Networks for Risk-Proportional Audit Sampling"*. Three schemes already exist (`GradualEmbezzlementScheme`, `RevenueManipulationScheme`, `VendorKickbackScheme`), but **seven additional scheme implementations** are needed, along with extensions to `SchemeType`, `SchemeActionType`, `SchemeAdvancer`, and labeling infrastructure.

This issue tracks the implementation of the missing fraud schemes, the Pathology Lab benchmark configuration, and the `SchemeAdvancer` extensions required to orchestrate all 10 pathologies with taxonomy-aligned labels.

Graph construction enrichment and the Python-side RIP-GNN / SAE / ERA implementation are out of scope here (tracked separately).

---

## Goals

| Goal | Description |
|------|-------------|
| **7 new fraud schemes** | Implement Triad Bypass, Shadow Payroll, Expense Laundering, Smurfing, Circular Funding, Phantom Warehousing, and Intercompany Wash Trades as `FraudScheme` structs. |
| **Extend `SchemeType` / `SchemeActionType`** | Add variants for each new scheme and any missing action primitives. |
| **Extend `SchemeAdvancer`** | Support per-pathology probabilities, sampling across all scheme types, and taxonomy-aligned labels. |
| **Taxonomy labels** | Extend `MultiStageAnomalyLabel` with `pathology_name` and `pathology_category` (Sequential / Volume / Relational). |
| **Pathology Lab config** | Create a benchmark preset (`pathology_lab_rip_gnn.yaml`) that generates ~10M JEs with ~100k labeled pathologies across all 10 types. |

---

## New fraud schemes

### Triad Bypass (Process Bypass)

- [ ] Implement `TriadBypassScheme` (`SchemeType::TriadBypass`):
  - Uses O2C/P2P document flows.
  - Stages: (1) legitimate invoice + payment flows to establish history, (2) fraudulent payment **reusing an old invoice ID** without a new invoice, (3) concealment via adjustments/write-offs.
  - New action type: `ReuseDocumentId`.
  - Topological signature: breaks the normal three-way match (PO/GR/Invoice) by injecting a payment against a previously-cleared document.

### Shadow Payroll (Ghost Worker)

- [ ] Implement `ShadowPayrollScheme` (`SchemeType::ShadowPayroll`):
  - Stages: (1) create ghost employee in HR master data, (2) repeated payroll postings to ghost employee bank account, (3) optional concealment (reclassification, write-off).
  - New action types: `CreateGhostEmployee`, `CreateGhostBankAccount`.
  - Integration: HR/payroll generator creates ghost employees and payroll entries; JE generator posts related expense/cash flows.
  - Identity features: `bank_account_id`, optional `address_region` / `login_region` on employee/user models for latent identity collision detection.

### Expense Laundering (Entropy Fan-out)

- [ ] Implement `ExpenseLaunderingScheme` (`SchemeType::ExpenseLaundering`):
  - Stages: (1) create network of low-centrality, unverified vendors, (2) generate micro-expenses from a single cash node to these vendors, (3) concealment via misclassification / timing.
  - New action types: `CreateShellVendor`, `CreateMicroExpense`.
  - Requires vendor master data attributes: `is_verified`, `creation_date`, optional `owner_user_id` or related-party flags.
  - Topological signature: high fan-out from single source to many low-centrality sinks.

### Smurfing (Threshold Evasion)

- [ ] Implement `SmurfingScheme` (`SchemeType::Smurfing`):
  - Builds on existing `FraudType::SplitTransaction` and `FraudAmountPattern::ThresholdAdjacent`.
  - Stages: (1) identify target path (specific vendor / GL account), (2) generate many small, just-below-threshold payments between the same nodes, (3) optional spreading across dates/entities.
  - Topological signature: many parallel edges with amounts clustering just below approval/reporting thresholds.

### Circular Funding (Round-Tripping)

- [ ] Implement `CircularFundingScheme` (`SchemeType::CircularFunding`):
  - Operates over intercompany / banking modules.
  - Stages: (1) set up A→B, B→C, C→A loans, (2) orchestrate cash flows forming SCCs with net-zero consolidated impact.
  - New action type: `IntercompanyRoundTrip`.
  - Topological signature: strongly connected components in intercompany flow graph with cancelling net effects.

### Phantom Warehousing (Inventory Isolate)

- [ ] Implement `PhantomWarehousingScheme` (`SchemeType::PhantomWarehousing`):
  - Uses manufacturing/inventory modules.
  - Stages: (1) create ghost locations, (2) move inventory in cycles among non-productive locations, (3) never connect to Sales or Cash sinks.
  - New action type: `InventoryTransferToGhostLocation`.
  - Topological signature: isolated subgraph of inventory movements disconnected from revenue/cash nodes.

### Intercompany Wash Trades

- [ ] Implement `IntercompanyWashTradeScheme` (`SchemeType::IntercompanyWashTrades`):
  - Generates symmetric intercompany trades between subsidiaries.
  - Parallel edges with cancelling effects in the consolidated trial balance.
  - Topological signature: symmetric paired edges between entity nodes with net-zero impact.

---

## Extend `SchemeType` and `SchemeActionType`

- [ ] Add to `SchemeType`:
  - `TriadBypass`, `ShadowPayroll`, `ExpenseLaundering`, `Smurfing`, `CircularFunding`, `PhantomWarehousing`, `IntercompanyWashTrades`.
- [ ] Add to `SchemeActionType`:
  - `ReuseDocumentId`, `CreateGhostEmployee`, `CreateGhostBankAccount`, `CreateMicroExpense`, `CreateShellVendor`, `IntercompanyRoundTrip`, `InventoryTransferToGhostLocation`.

**Code refs:** `crates/datasynth-generators/src/anomaly/schemes/scheme.rs`, `crates/datasynth-core/src/models/anomaly.rs`

---

## Extend `SchemeAdvancer` and labeling

### `SchemeAdvancerConfig`

- [ ] Add per-pathology probabilities:
- `triad_bypass_probability`, `shadow_payroll_probability`, `expense_laundering_probability`, `circular_funding_probability`, `phantom_warehousing_probability`, `intercompany_wash_trade_probability`.
- [ ] Normalize probabilities across all scheme types (existing + new).

### Scheme selection

- [ ] Update `maybe_start_scheme` to sample among **all** supported schemes and instantiate the correct struct based on the sampled type and available actors (users, vendors, companies, employees, entities).

### Taxonomy-aligned labels

- [ ] Extend `MultiStageAnomalyLabel` with:
  - `pathology_name: String` (e.g. `"Smurfing"`, `"CircularFunding"`, `"ShadowPayroll"`).
  - `pathology_category: String` — one of `"Sequential"`, `"Volume"`, `"Relational"`.
- [ ] Map existing schemes to the taxonomy:
  - `GradualEmbezzlement` → Sequential/Volume.
  - `RevenueManipulation` → Volume.
  - `VendorKickback` → Relational.

**Code refs:** `crates/datasynth-generators/src/anomaly/schemes/scheme_advancer.rs`

---

## Pathology Lab benchmark config

- [ ] Create `configs/pathology_lab_rip_gnn.yaml` with:
  - `period_months: 36` (3 years).
  - Volume tuned to ~10M JEs.
  - Temporal patterns enabled (business days, period-end dynamics).
  - Generic fraud types dialed down so the 10 target pathologies dominate.
  - `anomaly_injection.scheme_advancer` with per-pathology probabilities targeting ~100k total labeled pathologies (~10k per type).
  - `graph_export.enabled: true` for PyG export at end of generation.

---

## Pathology taxonomy summary

| # | Pathology | Category | SchemeType | Status |
|---|-----------|----------|------------|--------|
| 1 | Gradual Embezzlement | Sequential/Volume | `GradualEmbezzlement` | **Exists** |
| 2 | Revenue Manipulation (Channel Stuffing) | Volume | `RevenueManipulation` | **Exists** |
| 3 | Vendor Kickbacks | Relational | `VendorKickback` | **Exists** |
| 4 | Triad Bypass (Process Bypass) | Relational | `TriadBypass` | To implement |
| 5 | Shadow Payroll (Ghost Worker) | Sequential | `ShadowPayroll` | To implement |
| 6 | Expense Laundering (Entropy Fan-out) | Volume | `ExpenseLaundering` | To implement |
| 7 | Smurfing (Threshold Evasion) | Volume | `Smurfing` | To implement |
| 8 | Circular Funding (Round-Tripping) | Relational | `CircularFunding` | To implement |
| 9 | Phantom Warehousing (Inventory Isolate) | Relational | `PhantomWarehousing` | To implement |
| 10 | Intercompany Wash Trades | Relational | `IntercompanyWashTrades` | To implement |

---

## Acceptance criteria

- [ ] All 7 new `FraudScheme` structs are implemented with multi-stage `SchemeStage` definitions, `SchemeAction` emission, and `SchemeTransactionRef` population.
- [ ] `SchemeType` and `SchemeActionType` enums include all new variants; serialization/deserialization is tested.
- [ ] `SchemeAdvancer` samples across all 10 scheme types with configurable per-pathology probabilities.
- [ ] `MultiStageAnomalyLabel` includes `pathology_name` and `pathology_category` for all schemes (existing and new).
- [ ] Each scheme integrates with its relevant generator(s): O2C/P2P for Triad Bypass, HR/payroll for Shadow Payroll, vendor master data for Expense Laundering, intercompany for Circular Funding and Wash Trades, inventory for Phantom Warehousing.
- [ ] `configs/pathology_lab_rip_gnn.yaml` preset generates ~10M JEs with labeled pathologies and produces PyG-ready graph exports.
- [ ] Unit tests cover each scheme's stage progression, action generation, and label output.

---

## References

- Graph RIP-GNN integration plan: `docs/plans/2026-02-27-graph-rip-gnn-integration.md`
- Existing schemes: `crates/datasynth-generators/src/anomaly/schemes/embezzlement.rs`, `revenue_manipulation.rs`, `kickback.rs`
- Scheme orchestration: `crates/datasynth-generators/src/anomaly/schemes/scheme_advancer.rs`
- Scheme types: `crates/datasynth-generators/src/anomaly/schemes/scheme.rs`
- Fraud types: `crates/datasynth-core/src/models/anomaly.rs`
- Graph export: `crates/datasynth-graph/src/`
- HR/payroll: `crates/datasynth-generators/src/hr/`
- Manufacturing/inventory: `crates/datasynth-generators/src/manufacturing/`
- Intercompany: `crates/datasynth-generators/src/intercompany/`
