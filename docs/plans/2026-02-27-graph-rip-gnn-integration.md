# Graph RIP-GNN — Integration Plan with EY-ASU SyntheticData

**Status:** Planned  
**Created:** 2026-02-27  
**Scope:** Extend EY-ASU SyntheticData to support the paper *“Graph RIP-GNN: Relational Integrity-Preserving Graph Neural Networks for Risk-Proportional Audit Sampling”* — including pathology generation, graph construction, and ML export.

---

## 1. Executive Summary

The current EY-ASU SyntheticData stack already contains most of the primitives needed for Graph RIP-GNN:

- **Multi-stage fraud schemes** (`GradualEmbezzlementScheme`, `RevenueManipulationScheme`, `VendorKickbackScheme`) and a `SchemeAdvancer` orchestration layer.
- **Graph export** via `datasynth-graph`, including transaction graphs and a PyTorch Geometric (`PyGExporter`) pipeline.
- **Strong accounting physics**: `JournalEntry` enforces debits = credits, with realistic temporal patterns and document flows.

To make the paper fully implementable, we need to:

1. Extend the anomaly / scheme layer to cover the **10 fraud pathologies** in the paper (Sequential, Volume, Relational).
2. Enrich graph construction to a **heterogeneous temporal multigraph** (accounts, users, vendors, cost centers, entities) with relation-aware features.
3. Provide a **Pathology Lab benchmark**: configs + CLI that generate a 10M-transaction dataset with labeled pathologies and PyG-ready graph exports.
4. Implement the **RIP layer, Structural Accounting Entropy (SAE), and Edge-Relational Attention (ERA)** in Python on top of existing exports.

---

## 2. Current Capabilities vs Paper Requirements

### 2.1 Current Capabilities

- **Fraud schemes and orchestration**
  - `crates/datasynth-generators/src/anomaly/schemes/`:
    - `GradualEmbezzlementScheme` (slow-burn embezzlement).
    - `RevenueManipulationScheme` (revenue/channel-related).
    - `VendorKickbackScheme` (buyer–vendor–cash triad).
  - `SchemeStage` encodes amounts, duration, transaction counts, and concealment techniques.
  - `SchemeAdvancer` coordinates multiple schemes, including:
    - `CompletedScheme` summaries.
    - `MultiStageAnomalyLabel` with `scheme_id`, `scheme_type`, `stage_number`, `perpetrator_id`, etc.

- **Journal entry generation and fraud flags**
  - `crates/datasynth-generators/src/je_generator.rs`:
    - Enforces **double-entry** (debits = credits) for all JEs.
    - Uses `FraudConfig` and `FraudType` distributions to inject point anomalies.
    - Adds temporal behavior (seasonality, business days, period-end spikes).
    - Uses `user_persona`, approval workflows, batching behavior, and master-data pools.

- **Graph construction and export**
  - `crates/datasynth-graph/src/models/`:
    - `Graph`, `GraphNode`, `GraphEdge`, `HeterogeneousGraph`.
    - `NodeType` (`Account`, `JournalEntry`, `Vendor`, `Customer`, `User`, `Company`, etc.).
    - `EdgeType` (`Transaction`, `Approval`, `Ownership`, `Intercompany`, `DocumentReference`, …).
  - `TransactionGraphBuilder`:
    - Builds **Account-level transaction networks**:
      - Nodes: accounts (and optionally document nodes).
      - Edges: debit→credit or document→account with timestamps and features.
    - Propagates `entry.header.is_anomaly` and `anomaly_type` to edges.
  - `PyGExporter`:
    - Exports `edge_index.npy`, `node_features.npy`, `edge_features.npy`, `node_labels.npy`, `edge_labels.npy`, masks, and `metadata.json`.
    - Provides an auto-generated `load_graph.py` loader script.

### 2.2 Paper Requirements

**Graph RIP-GNN paper needs:**

- A **heterogeneous temporal multigraph**:
  - Nodes: GL accounts, cost centers, vendors, customers, users, companies, inventory locations.
  - Edges: JEs, approvals, intercompany flows, document references, inventory movements.
  - Temporal dimension for sequential / drift modeling (TGN-style).
- A **Relational Integrity-Preserving (RIP) layer**:
  - Double-entry constraint as a structural bias in message passing.
  - Conservation-aware penalties (“logical friction” in value flows).
- **10 fraud pathologies** across Sequential, Volume, Relational categories, each with:
  - Distinct topological signatures (SCCs, fan-out, cycles, latent drift).
  - Supervisory labels for benchmarking detection models.
- **SAE (Structural Accounting Entropy)**:
  - Metric for logical disorder in local subgraphs.
- **Explainability**:
  - Evidence subgraphs (3-hop) via GNNExplainer-like tools.

The gap is therefore not foundational, but in **coverage, graph richness, and ML glue code**.

---

## 3. Pathology Lab: Extending Multi-Stage Schemes

### 3.1 Extend `SchemeType` and `SchemeActionType`

**Goal:** Map the paper’s 10 pathologies onto explicit `SchemeType` variants and `SchemeActionType` primitives.

- In `datasynth_core::models::SchemeType` (not shown here but already referenced):
  - Add variants for each topology-driven scheme, e.g.:
    - `TriadBypass` (Process Bypass / re-used invoice ID).
    - `ShadowPayroll` (Ghost employees).
    - `ExpenseLaundering`.
    - `Smurfing`.
    - `CircularFunding`.
    - `PhantomWarehousing`.
    - `IntercompanyWashTrades`.

- In `SchemeActionType` (`scheme.rs`):
  - We already have `CreateFraudulentEntry`, `CreateFictitiousVendor`, `InflateInvoice`, `MakeKickbackPayment`, `ChannelStuff`, etc.
  - Extend with any missing building blocks:
    - `ReuseDocumentId` (for Triad Bypass).
    - `CreateGhostEmployee`, `CreateGhostBankAccount`.
    - `CreateMicroExpense`, `CreateShellVendor`.
    - `IntercompanyRoundTrip`, `InventoryTransferToGhostLocation`.

This preserves the current multi-stage scheme API while expanding the pathology vocabulary.

### 3.2 Implement Additional `FraudScheme` Structs

For each paper pathology, implement a dedicated `FraudScheme` in `crates/datasynth-generators/src/anomaly/schemes/`, similar to `GradualEmbezzlementScheme` and `VendorKickbackScheme`:

- **Bypassing the Triad (Process Bypass)**
  - `TriadBypassScheme`:
    - Uses O2C P2P/O2C document flows.
    - Stages:
      - Setup: legitimate invoice + payment flows to establish history.
      - Bypass: second fraudulent payment that **reuses an old invoice ID** without a new invoice.
      - Concealment: adjustments/write-offs.
    - Actions:
      - `CreateFraudulentPayment` + `ReuseDocumentId`.

- **Shadow Payroll (Ghost Worker)**
  - `ShadowPayrollScheme`:
    - Stages:
      - Create ghost employee (HR master data).
      - Repeated payroll postings to ghost employee bank account.
      - Optional concealment (reclassification, write-off).
    - Integration:
      - HR/payroll generator creates ghost employees and payroll entries.
      - JE generator posts related expense/cash flows.

- **Expense Laundering (Entropy Fan-out)**
  - `ExpenseLaunderingScheme`:
    - Stages:
      - Create network of low-centrality, unverified vendors.
      - Generate micro-expenses from a single cash node to these vendors.
      - Concealment via misclassification / timing.
    - Requires vendor master data attributes:
      - `is_verified`, `creation_date`, and optional `owner_user_id` or related-party flags.

- **Smurfing (Threshold Evasion)**
  - ~~`SmurfingScheme`~~ (removed):
    - Builds on `FraudType::SplitTransaction` and `FraudAmountPattern::ThresholdAdjacent`.
    - Stages:
      - Identify target path (e.g. specific vendor / GL account).
      - Generate many small, just-below-threshold payments between the same nodes.
      - Optional concealment via spreading across dates/entities.

- **Gradual Embezzlement (Slow Burn)**
  - Already implemented as `GradualEmbezzlementScheme`.
  - Align labels to paper taxonomy (`pathology = "GradualEmbezzlement"`; category `Volume/Sequential`).

- **Revenue Manipulation (Channel Stuffing)**
  - Ensure `RevenueManipulationScheme` covers:
    - Quarter-end spikes in revenue disconnected from inventory/shipping events.
    - Use `BusinessProcess::O2C` and period-end dynamics.

- **Circular Funding (Round-Tripping)**
  - `CircularFundingScheme`:
    - Operates over intercompany / banking modules.
    - Stages:
      - Set up A→B, B→C, C→A loans.
      - Orchestrate cash flows that form SCCs with net-zero consolidated impact.

- **Vendor Kickbacks (Relational Triad)**
  - Already implemented as `VendorKickbackScheme`.
  - Ensure edge-level features include price / volume proxies where available.

- **Inventory Phantom Warehousing (Isolate)**
  - `PhantomWarehousingScheme`:
    - Uses manufacturing/inventory modules.
    - Stages:
      - Create ghost locations.
      - Move inventory in cycles among non-productive locations.
      - Never connect to `Sales` or `Cash` sinks.

- **Intercompany Wash Trades (Wash)**
  - `IntercompanyWashTradeScheme`:
    - Generates symmetric intercompany trades between subsidiaries.
    - Parallel edges with cancelling effects in the consolidated trial balance.

Each scheme should:

- Use `SchemeStage` to define temporal structure and intensity.
- Emit `SchemeAction`s that downstream generators can consume.
- Populate `SchemeTransactionRef`s so we can later map graph edges to specific scheme stages.

### 3.3 Extend `SchemeAdvancer` and Labeling

In `scheme_advancer.rs`:

- **Configuration:**
  - Extend `SchemeAdvancerConfig` to include per-pathology probabilities (e.g. `triad_bypass_probability`, `shadow_payroll_probability`, etc.).
  - Normalize probabilities across all scheme types similar to current embezzlement/revenue/kickback handling.

- **Scheme selection:**
  - Update `maybe_start_scheme` to:
    - Sample among **all** supported schemes.
    - Instantiate the correct scheme struct based on the sampled type and available actors (users, vendors, companies).

- **Labels:**
  - Extend `MultiStageAnomalyLabel` with:
    - `pathology_name: String` (e.g. `"Smurfing"`, `"CircularFunding"`).
    - `pathology_category: String` (e.g. `"Sequential"`, `"Volume"`, `"Relational"`).

This creates a Pathology Lab with clear supervision aligned with the paper’s taxonomy.

### 3.4 Impact of fraud-actor design on Graph RIP

The **fraud-actor design** (top-level tagging of a fixed subset of vendors, customers, and employees as `is_fraud_actor`) directly benefits Graph RIP-GNN:

1. **Stable relational subgraphs** — Schemes always use the same tagged entities for a given seed/config. Fraud-related edges consistently connect the same Vendor/Customer/User nodes (e.g. circular funding A→B→C→A reuses the same B and C). The graph exhibits coherent fraud participant subgraphs instead of random entity sampling per row.
2. **RIP layer and mass conservation** — Cycle detection and value-flow conservation (double-entry, SCCs) align with stable node identity; cycles map to the same entities across the simulation, making structural patterns (e.g. round-trips) detectable by the RIP layer.
3. **Node-level signal** — `is_fraud_actor` is exported as a node feature and in node properties in graph builders that consume master data (hypergraph `add_vendors` / `add_customers` / `add_employees`). The Python RIP-GNN stack can use it as an input feature or as a supervision signal for node-level fraud-participant prediction/benchmarking.
4. **Explainability** — Evidence subgraphs (e.g. 3-hop) will repeatedly highlight the same fraud-actor nodes across scheme instances.

**Implementation:** The hypergraph builder includes `is_fraud_actor` in node features and properties so exports (PyG, DGL, RustGraph) carry it through to the Pathology Lab and RIP-GNN code.

---

## 4. Enterprise Graph: From Transaction Network to Heterogeneous Temporal Multigraph

### 4.1 Enrich Transaction Graph Builder

Current `TransactionGraphBuilder` builds:

- Nodes:
  - `AccountNode` for each `(company_code, account_code)`.
- Edges:
  - `TransactionEdge` between debit and credit accounts (or document→account).
  - Features: amount (log), debit/credit flag, weekday, day, month, month-end, year-end, Benford probability.
  - Properties: `document_number`, `posting_date`, `is_debit`.
  - Anomaly propagation from `JournalEntry.header.is_anomaly`.

To match the **Enterprise manifold** described in the paper:

- Extend `TransactionGraphConfig` with flags:
  - `include_users: bool`
  - `include_vendors: bool`
  - `include_customers: bool`
  - `include_cost_centers: bool`
  - (Existing `include_document_nodes` stays as-is.)

- In `TransactionGraphBuilder`:
  - When `include_document_nodes` is true:
    - Already creates `NodeType::JournalEntry` for each document.
  - When `include_vendors` / `include_customers` is true:
    - Create `NodeType::Vendor` / `NodeType::Customer` nodes for IDs attached to JEs or lines.
    - Add `EdgeType::DocumentReference` or `EdgeType::Custom("VendorLink")` edges linking docs/accounts to vendors/customers.
  - When `include_users` is true:
    - Create `UserNode`s from `JournalEntry.header.created_by`.
    - Add edges:
      - `EdgeType::Custom("CreatedBy")` from user to document.
      - Or reuse `ApprovalEdge` when modeled via approval workflow.
  - When `include_cost_centers` is true:
    - Create `NodeType::CostCenter` nodes from `line.cost_center`.
    - Add `EdgeType::CostAllocation` edges from cost centers to accounts or documents.

This gives us an ERP-style heterogeneous multigraph (Accounts, Users, Vendors, Cost Centers, Companies) on top of existing infrastructure.

### 4.2 Temporal and Relational Features for ERA

For **Edge-Relational Attention (ERA)** we need explicit relation types per edge. Suggested changes:

- Extend `TransactionEdge` to capture:
  - `document_type` (e.g. `INVOICE`, `PAYMENT`, `CORRECTION`, `PAYROLL`, `INVENTORY`), derived from `BusinessProcess` and document metadata.
  - `source_type` from `TransactionSource` (`Manual`, `Automated`, `Recurring`, `Adjustment`).

- In `TransactionGraphBuilder::add_journal_entry*`:
  - After `tx_edge.compute_features()`, add:
    - `edge.properties["document_type"] = EdgeProperty::String(document_type)`.
    - `edge.properties["source_type"] = EdgeProperty::String(source_type)`.
  - Optionally add categorical encodings to `edge.features` (e.g. integers or one-hots in Python).

ERA in the RIP-GNN implementation can then:

- Group and attend over edges by (`document_type`, `source_type`) or by full triplets (`source_node_type`, `edge_type`, `target_node_type`).

### 4.3 Signed Amounts and Double-Entry Constraints

The generator already enforces double entry at the **JE level**. For the RIP layer:

- **Data side:**
  - Maintain `TransactionEdge` with:
    - `is_debit` flag.
    - `edge.weight` as absolute or log-amount (as it is today).
  - Optionally add:
    - A derived **signed weight** feature in Python: `signed_amount = amount * (+1 for debit, -1 for credit)`.

- **Model side (Python / PyG):**
  - Implement the RIP layer so that:
    - For each account node, incoming/outgoing signed flows around a time step satisfy (or are penalized for violating) conservation.
    - Messages are weighted by adherence to the accounting equation (e.g. residual net flow at that node / time).

No change to the `JournalEntry` invariants is required; the core “accounting physics” is already encoded by construction.

---

## 5. Pathology Lab Benchmark & CLI

### 5.1 Config Preset

Add a dedicated configuration preset, e.g. `configs/pathology_lab_rip_gnn.yaml`, with:

- **Global / transactions:**
  - `period_months: 36` (3 years).
  - Volume tuned to ~10M JEs across configured companies.
  - Temporal patterns enabled (business days, period-end dynamics).
- **Fraud / anomaly / schemes:**
  - `fraud.enabled: true` but with most generic fraud types dialed down so the 10 target pathologies dominate.
  - `anomaly_injection.scheme_advancer` extended with probabilities per pathology (`circular_funding_probability`, etc.).
  - Target of ~100k labeled pathologies (10k per type), controlled via scheme start probabilites and durations.
- **Graph export:**
  - Flag to enable graph building and PyG export at the end of generation, e.g.:
    - `graph_export.enabled: true`
    - `graph_export.type: "rip_gnn_transaction_network"` (transaction + heterogeneous options).

### 5.2 CLI / Runtime Wiring

In `datasynth-cli` / `datasynth-runtime`:

- Add a high-level entry point, e.g.:

```bash
datasynth-data generate \
  --config configs/pathology_lab_rip_gnn.yaml \
  --output ./output/pathology_lab_rip_gnn
```

Implementation outline:

1. Run the existing orchestration to generate JE + master data + document flows + anomalies and schemes.
2. After tabular output is written:
   - Construct a `TransactionGraphBuilder` (with extended config for heterogeneous nodes).
   - Ingest JEs (and optionally approvals, intercompany flows) into `Graph` or `HeterogeneousGraph`.
   - Call `PyGExporter` to write `edge_index.npy`, `node_features.npy`, `edge_features.npy`, `node_labels.npy`, `edge_labels.npy`, masks, and `metadata.json`.

This yields exactly the “Pathology Lab” dataset described in the paper: large-scale graph data with 10 labeled pathologies.

---

## 6. SAE and RIP-GNN Implementation (Python Side)

Most of the model-specific logic for RIP-GNN is best implemented in Python on top of exported graphs:

- **Structural Accounting Entropy (SAE):**
  - Train on 8M “clean” transactions:
    - Estimate distributions of neighbor types, edge attributes (amount, doc type, time), and path patterns.
  - For each edge (or local subgraph), compute:
    - Negative log-likelihood under the learned distribution, or
    - Entropy of local transition distributions.
  - Add SAE either as:
    - An additional edge feature in PyG (Python only), or
    - A post-hoc diagnostic metric.

- **RIP layer & ERA:**
  - Use exported PyG data and metadata to construct `HeteroData`:
    - Split nodes/edges by type using `node_types` and `edge_types` from `metadata.json`.
  - Implement:
    - RIP layer that enforces / penalizes violations of conservation at nodes over time.
    - Edge-Relational Attention using `document_type`, `source_type`, and relation triplets.

No Rust code change is strictly necessary here; we should keep RIP-GNN as a separate Python package (e.g. `rip_gnn`) consuming the outputs of `datasynth-graph`.

---

## 7. Explainability and Evidence Subgraphs

For ISA 315-style explainability, we need to map detected anomalies back to human-readable enterprise artifacts.

Current support:

- `GraphNode` tracks `external_id`, `label`, `node_type`, `properties`.
- `GraphEdge` and `TransactionEdge` carry:
  - `document_number`, `posting_date`, `is_debit`, `weight`, `edge_type`, `anomaly_type`.
- `PyGExporter` maintains a fixed node/edge ordering for features and labels.

Proposed additions (mostly Python utilities):

- Build a mapping from PyG indices back to:
  - `GraphNode.external_id`, `NodeType`, and properties (e.g. account code, company, cost center).
  - `GraphEdge.properties` (document numbers, dates, doc types).
- When RIP-GNN flags a high-risk edge or node:
  - Extract a 3-hop induced subgraph around it.
  - Serialize as:
    - A small JSON graph (nodes, edges, labels, properties), and/or
    - A textual explanation (e.g. “Payment node connected to Invoice node with 0 remaining capacity; multiple parallel edges to same vendor under threshold.”).

This aligns directly with the paper’s “Evidence Subgraph” concept without requiring additional Rust changes.

---

## 8. Shadow Payroll Identity Features (Comment Clarification)

In the paper draft, Shadow Payroll detection mentions **address** and **login IP**. The core requirement is **identity linkage**, not raw PII.

Recommended approach:

- In `datasynth-core` employee / user models:
  - Add:
    - `bank_account_id: String` (already present in banking domain; reuse).
    - Optional `address_region: String` (e.g. country/region, not full address).
    - Optional `login_region` or `auth_channel` (e.g. `"VPN_US"`, `"OnPrem_FR"`).

- In `UserNode` (graph model):
  - Add these as:
    - Categorical features (`address_region`, `login_region`).
    - Node properties for inspection.

This is sufficient for detecting **latent identity collisions** (ghost worker sharing bank account or region patterns with supervisor) while avoiding explicit IP address or full address storage.

---

## 9. Implementation Order (Practical Roadmap)

1. **Pathology coverage**
   - Implement missing `FraudScheme` structs and extend `SchemeAdvancer` and `SchemeType`.
   - Wire schemes into relevant generators (O2C, HR, intercompany, inventory).
2. **Graph enrichment**
   - Extend `TransactionGraphConfig` and `TransactionGraphBuilder` for heterogeneous nodes and richer edge metadata.
   - Optionally add a `HeterogeneousGraphBuilder` that organizes per-relation `Graph`s.
3. **Benchmark plumbing**
   - Add `pathology_lab_rip_gnn.yaml` preset and CLI path that:
     - Generates JE + master data + pathologies.
     - Produces PyG exports in a stable directory layout.
4. **Python RIP-GNN library**
   - Build HeteroData loaders from current `metadata.json` and `.npy` files.
   - Implement RIP layer, ERA, and SAE computation and evaluation scripts.
5. **Explainability**
   - Implement Python utilities for mapping model outputs back to ledger artifacts and evidence subgraphs suitable for auditors.

Once these steps are implemented, the codebase will fully support the Graph RIP-GNN paper as an executable benchmark: from synthetic pathology generation to graph construction, model training, and risk-proportional audit sampling experiments.

---

## 10. Codebase Readiness Audit — Per-Scheme Implementation Specifications

This section provides an exhaustive, file-level assessment of the codebase's readiness to implement each of the 10 fraud pathologies. For every scheme we identify: (a) what already exists and can be reused, (b) what is partially present but needs extension, and (c) what must be built from scratch. Each specification lists every Rust file that needs modification or creation, the exact structs/enums/fields to add, configuration changes, graph integration requirements, and labeling hooks.

---

### 10.1 Scheme 1 — Triad Bypass (Process Bypass)

**Paper description:** A fraudulent payment that **reuses an existing invoice document ID**, bypassing the PO → GR → Invoice → Payment three-way match. The topological signature is a broken triad: the payment edge exists, but the expected intermediate document chain is absent or reused.

#### 10.1.1 Readiness Assessment

| Layer | Status | Detail |
|-------|--------|--------|
| **Document flow models** | ✅ Ready | `P2PDocumentChain` (PO → GR → Invoice → Payment) fully implemented in `document_flow/p2p_generator.rs`. `ThreeWayMatchResult` and `MatchVariance` in `document_flow/three_way_match.rs` validate PO–GR–Invoice consistency. |
| **Payment model** | ✅ Ready | Payment struct has `allocations` with `invoice_reference` and `discount_taken`. Supports partial payments via `remainder_payments`. |
| **Document reference tracking** | ✅ Ready | `DocumentReference` model and `DocumentChainManager` in `document_flow/document_chain_manager.rs` track chains by document number. |
| **Invoice reuse detection** | ⚠️ Partial | `DocumentFlowAnomalyType` in `anomaly/document_flow_anomalies.rs` has `InvoiceWithoutPO` and `InvoiceWithoutGR` but no `ReusedInvoiceId` variant. |
| **SchemeActionType** | ❌ Missing | No `ReuseDocumentId` variant in `SchemeActionType` enum. |
| **FraudScheme struct** | ❌ Missing | No `TriadBypassScheme` implementation. |
| **SchemeType variant** | ❌ Missing | `SchemeType` has no `TriadBypass` variant (only `GradualEmbezzlement`, `RevenueManipulation`, `VendorKickback`, `RoundTripping`, `GhostEmployee`, `ExpenseReimbursement`, `InventoryTheft`, `Custom`). |
| **SchemeAction → JE conversion** | ❌ Missing | `SchemeAction`s are produced but **never converted to concrete `JournalEntry`s**. There is no interpreter layer between scheme actions and the JE generator. This is a cross-cutting gap affecting all 7 unimplemented schemes. |

#### 10.1.2 Required Changes

**File: `crates/datasynth-core/src/models/anomaly.rs`**

- Add `TriadBypass` to `SchemeType` enum.
- Add `FraudType::TriadBypass` variant (or reuse `InvoiceManipulation` with metadata).

**File: `crates/datasynth-generators/src/anomaly/schemes/scheme.rs`**

- Add `ReuseDocumentId` to `SchemeActionType`.
- Add `CreateFraudulentPayment` if not already present (✅ exists).
- Extend `SchemeAction` with optional `reused_document_id: Option<String>` field to carry the invoice ID being reused.

**File: `crates/datasynth-generators/src/anomaly/schemes/triad_bypass.rs` (NEW)**

```rust
pub struct TriadBypassScheme {
    scheme_id: Uuid,
    perpetrator_id: String,
    target_vendor_id: String,
    reused_invoice_id: Option<String>,   // set after setup stage
    stages: Vec<SchemeStage>,
    current_stage: usize,
    status: SchemeStatus,
    // ...
}
```

- **Stage 1 — Establishment (3 months):** Generate 3–5 legitimate P2P chains (PO→GR→Invoice→Payment) to establish a normal relationship with the vendor. Actions: `CreateFraudulentEntry` (legitimate entries for cover).
- **Stage 2 — Bypass (6 months):** Generate fraudulent payments that reference an existing, already-paid invoice ID. Actions: `CreateFraudulentPayment` + `ReuseDocumentId`. Amount range $5K–$50K. The key: the document chain check (three-way match) would show a payment against an invoice that was already fully settled.
- **Stage 3 — Concealment (2 months):** Generate adjusting entries, write-offs, or reclassifications to hide the duplicate payment. Actions: `Conceal`.

**File: `crates/datasynth-generators/src/anomaly/schemes/mod.rs`**

- Add `pub mod triad_bypass;` and re-export `TriadBypassScheme`.

**File: `crates/datasynth-generators/src/anomaly/scheme_advancer.rs`**

- Add `triad_bypass_probability: f64` to `SchemeAdvancerConfig`.
- In `maybe_start_scheme()`: add sampling branch for `TriadBypassScheme`. Requires `available_counterparties` (vendors) and access to existing `P2PDocumentChain` history (or at minimum, a list of previously used invoice IDs).

**File: `crates/datasynth-generators/src/anomaly/document_flow_anomalies.rs`**

- Add `ReusedInvoiceId` variant to `DocumentFlowAnomalyType`.

**File: `crates/datasynth-config/src/schema.rs`**

- Add `triad_bypass: TriadBypassSchemeConfig` to `MultiStageSchemeConfig`.
- Define `TriadBypassSchemeConfig { probability: f64, setup_stage: SchemeStageConfig, bypass_stage: SchemeStageConfig, concealment_stage: SchemeStageConfig }`.

**Critical gap — SchemeAction → JE materialization:**

The `SchemeAdvancer` produces `Vec<SchemeAction>`, but there is **no component that converts these actions into actual `JournalEntry` records**. The existing `advance_schemes()` in `injector.rs` collects actions into `self.scheme_actions` but does not feed them back to the JE generator. This requires a new **`SchemeActionMaterializer`** component (see §10.11 for the cross-cutting specification).

**Graph integration:**

- The triad bypass creates a topological anomaly: a payment edge from Cash → AP without a corresponding invoice→GR chain. In the `TransactionGraphBuilder`, this will appear as a direct edge between the AP account and Cash account for the same vendor, but the document node (if `include_document_nodes: true`) will show a reused `document_number`. The `document_number` property on `TransactionEdge` already carries this data.
- The `PyGExporter` label propagation already works via `entry.header.is_anomaly`.

---

### 10.2 Scheme 2 — Shadow Payroll (Ghost Worker)

**Paper description:** A ghost employee is created in HR master data, and recurring payroll postings divert salary to a bank account controlled by the perpetrator. The topological signature is a `User` node with no manager chain, connected to a `BankAccount` node that shares attributes (region, account) with an existing employee.

#### 10.2.1 Readiness Assessment

| Layer | Status | Detail |
|-------|--------|--------|
| **Employee model** | ✅ Ready | `Employee` has `employee_id`, `user_id`, `manager_id`, `direct_reports`, `status`, `department_id`, `cost_center`, `hire_date`, `termination_date`, `job_level`, `approval_limit`. |
| **Payroll model** | ✅ Ready | `PayrollRun` + `PayrollLineItem` with `employee_id`, `gross_pay`, `net_pay`, `cost_center`, `department`. |
| **PayrollGenerator** | ✅ Ready | Generates per-employee payroll line items per pay period. |
| **FraudType** | ✅ Ready | `FraudType::GhostEmployee` and `FraudType::GhostEmployeePayroll` already exist in the enum. |
| **SchemeType** | ⚠️ Partial | `SchemeType::GhostEmployee` exists as an enum variant but has **no implementing struct**. |
| **SchemeActionType** | ❌ Missing | No `CreateGhostEmployee` or `CreateGhostBankAccount` variants. |
| **Employee bank account** | ❌ Missing | `Employee` has no `bank_account_id` field. The banking domain has `BankAccount` but it is not linked to `Employee`. |
| **Identity linkage features** | ❌ Missing | No `address_region`, `login_region`, or `auth_channel` on `Employee` or `User`. Required for the GNN to detect shared-identity signals. |
| **FraudScheme struct** | ❌ Missing | No `ShadowPayrollScheme` implementation despite `SchemeType::GhostEmployee` existing. |

#### 10.2.2 Required Changes

**File: `crates/datasynth-core/src/models/user.rs`**

- Add to `Employee`:
  ```rust
  pub bank_account_id: Option<String>,
  pub address_region: Option<String>,
  pub login_region: Option<String>,
  ```
- Add to `User`:
  ```rust
  pub bank_account_id: Option<String>,
  pub address_region: Option<String>,
  pub login_region: Option<String>,
  ```

**File: `crates/datasynth-generators/src/master_data/employee_generator.rs`**

- Populate `bank_account_id` for all employees (format: `"BA-{employee_id}"`).
- Populate `address_region` from a region pool matching company locale.
- Populate `login_region` (e.g. `"OnPrem_{country}"` or `"VPN_{country}"`).

**File: `crates/datasynth-generators/src/anomaly/schemes/scheme.rs`**

- Add to `SchemeActionType`:
  ```rust
  CreateGhostEmployee,
  CreateGhostBankAccount,
  CreatePayrollEntry,
  ```
- Extend `SchemeAction` with:
  ```rust
  pub ghost_employee_id: Option<String>,
  pub ghost_bank_account_id: Option<String>,
  ```

**File: `crates/datasynth-generators/src/anomaly/schemes/shadow_payroll.rs` (NEW)**

```rust
pub struct ShadowPayrollScheme {
    scheme_id: Uuid,
    perpetrator_id: String,
    ghost_employee_id: String,
    ghost_bank_account_id: String,
    perpetrator_bank_account_id: String,  // shares with ghost
    stages: Vec<SchemeStage>,
    current_stage: usize,
    status: SchemeStatus,
    total_diverted: Decimal,
    transactions: Vec<SchemeTransactionRef>,
}
```

- **Stage 1 — Setup (1 month):** Create ghost employee record with `manager_id` pointing to perpetrator, `bank_account_id` matching perpetrator's bank account (the key identity collision signal), and plausible `address_region` / `login_region` overlapping with perpetrator. Actions: `CreateGhostEmployee`, `CreateGhostBankAccount`.
- **Stage 2 — Recurring diversion (12 months):** Each pay period, generate a `PayrollLineItem` for the ghost employee. The payroll posting creates JEs: DR Salary Expense (5xxx) / CR Cash (1xxx). Amount: $3K–$8K per period (realistic salary range). Actions: `CreatePayrollEntry`. Detection difficulty: Hard (blends with legitimate payroll).
- **Stage 3 — Concealment (2 months):** If detection risk rises, reclassify the ghost employee or terminate them with a severance payout. Actions: `Conceal`, `CoverUp`.

**File: `crates/datasynth-generators/src/anomaly/scheme_advancer.rs`**

- Add `shadow_payroll_probability: f64` to `SchemeAdvancerConfig`.
- In `maybe_start_scheme()`: add branch for `ShadowPayrollScheme`. Requires that the perpetrator has payroll authority (check `Employee.can_approve_je` or a new `can_modify_payroll` flag).

**File: `crates/datasynth-generators/src/anomaly/schemes/mod.rs`**

- Add `pub mod shadow_payroll;` and re-export.

**File: `crates/datasynth-config/src/schema.rs`**

- Add `shadow_payroll: ShadowPayrollSchemeConfig` to `MultiStageSchemeConfig`.
- Define config with probability, salary range, diversion duration.

**File: `crates/datasynth-graph/src/models/nodes.rs`**

- Extend `UserNode` features with `bank_account_id`, `address_region`, `login_region` as categorical features. Currently `UserNode` has `persona`, `department`, `approval_limit`, `working_hours` — add 3 new categorical entries.

**Graph integration:**

- Ghost employee appears as a `UserNode` with `manager_id` edge to perpetrator.
- The payroll JE edges flow from Salary Expense → Cash, tagged to the ghost employee.
- The GNN can detect the identity collision: ghost `UserNode.bank_account_id` == perpetrator `UserNode.bank_account_id`.
- Need `include_users: true` in `TransactionGraphConfig` (currently this flag does **not exist** — see §10.12 for graph builder extension spec).

---

### 10.3 Scheme 3 — Expense Laundering (Entropy Fan-out)

**Paper description:** A single cash source fans out micro-expenses to a network of newly-created, low-centrality, unverified vendors. The topological signature is high fan-out from one account to many loosely-connected leaf vendor nodes, with unusually high local entropy.

#### 10.3.1 Readiness Assessment

| Layer | Status | Detail |
|-------|--------|--------|
| **Vendor model** | ⚠️ Partial | `Vendor` has `vendor_type`, `is_active`, `country`, `payment_terms`, `bank_accounts`, `tax_id`. But no `is_verified`, `creation_date`, `created_by_user_id`, or `related_party_flag` fields. |
| **Vendor network** | ✅ Ready | `VendorRelationship` has `lifecycle_stage` (Onboarding, RampUp, etc.), `quality_score`, `strategic_importance`, `cluster` (Problematic, Transactional). |
| **FraudType** | ✅ Ready | `FraudType::ShellCompanyPayment`, `FraudType::FictitiousVendor` exist. |
| **SchemeType** | ⚠️ Partial | `SchemeType::ExpenseReimbursement` exists but is semantically different from Expense Laundering. Need a distinct variant. |
| **Micro-expense generation** | ⚠️ Partial | `FraudAmountPattern::ThresholdAdjacent` exists for amounts near thresholds. But no dedicated micro-expense pattern (many small amounts spread across vendors). |
| **SchemeActionType** | ❌ Missing | No `CreateShellVendor` or `CreateMicroExpense` variants. |
| **FraudScheme struct** | ❌ Missing | No `ExpenseLaunderingScheme`. |

#### 10.3.2 Required Changes

**File: `crates/datasynth-core/src/models/master_data.rs`**

- Add to `Vendor`:
  ```rust
  pub is_verified: bool,         // default true for legitimate vendors
  pub creation_date: Option<NaiveDate>,
  pub created_by_user_id: Option<String>,
  pub is_related_party: bool,    // default false
  ```

**File: `crates/datasynth-core/src/models/anomaly.rs`**

- Add `ExpenseLaundering` to `SchemeType` enum.

**File: `crates/datasynth-generators/src/anomaly/schemes/scheme.rs`**

- Add to `SchemeActionType`:
  ```rust
  CreateShellVendor,
  CreateMicroExpense,
  ```
- Extend `SchemeAction` with:
  ```rust
  pub shell_vendor_ids: Option<Vec<String>>,
  pub expense_category: Option<String>,   // e.g. "Office Supplies", "Consulting"
  ```

**File: `crates/datasynth-generators/src/anomaly/schemes/expense_laundering.rs` (NEW)**

```rust
pub struct ExpenseLaunderingScheme {
    scheme_id: Uuid,
    perpetrator_id: String,
    shell_vendor_ids: Vec<String>,
    source_account: String,       // cash or expense account used as hub
    stages: Vec<SchemeStage>,
    current_stage: usize,
    status: SchemeStatus,
    total_laundered: Decimal,
    transactions: Vec<SchemeTransactionRef>,
}
```

- **Stage 1 — Vendor network creation (2 months):** Create 5–15 shell vendors. Each has `is_verified: false`, `creation_date` within the scheme window, `lifecycle_stage: Onboarding`, `cluster: Transactional`, `quality_score` all zeros, minimal `bank_accounts`. Actions: `CreateShellVendor` (repeated). The vendors should have similar `tax_id` patterns or shared `bank_accounts` (collusion signal).
- **Stage 2 — Micro-expense fan-out (9 months):** Generate 20–50 small expense payments per month, each $50–$500, from a single expense account (e.g. 6xxx Office Supplies) to the shell vendors. JE pattern: DR Expense / CR Cash (or AP). The amounts should individually be unremarkable but aggregate to $10K–$50K/month. Actions: `CreateMicroExpense`. Each payment references a different shell vendor.
- **Stage 3 — Escalation (3 months):** Increase per-transaction amounts to $500–$2K. Some vendors become dormant, new ones are created. Actions: `CreateMicroExpense`, `CreateShellVendor`.
- **Stage 4 — Concealment (2 months):** Reclassify expenses, create adjusting entries, or terminate shell vendors. Actions: `Conceal`.

**File: `crates/datasynth-generators/src/master_data/vendor_generator.rs`**

- When producing shell vendors for schemes: set `is_verified = false`, `creation_date = scheme_start_date`, `created_by_user_id = perpetrator_id`, `is_related_party = true`, `lifecycle_stage = Onboarding`.
- Add method `generate_shell_vendor(perpetrator_id: &str, date: NaiveDate) -> Vendor`.

**File: `crates/datasynth-config/src/schema.rs`**

- Add `expense_laundering: ExpenseLaunderingSchemeConfig` to `MultiStageSchemeConfig`.
- Fields: `probability`, `min_shell_vendors`, `max_shell_vendors`, `micro_expense_range`, `escalation_threshold`.

**Graph integration:**

- With `include_vendors: true` in `TransactionGraphConfig`, shell vendors appear as `VendorNode`s.
- The fan-out topology (one expense account → many vendor nodes) is directly visible in the transaction graph.
- Vendor nodes with `is_verified: false` should be exported as a node feature (0/1).
- `creation_date` relative to scheme start can be an additional node feature.
- The SAE computation in Python will detect the high local entropy at the expense account node.

---

### 10.4 Scheme 4 — Smurfing (Threshold Evasion)

**Paper description:** Many transactions deliberately structured just below approval or reporting thresholds. The topological signature is a dense cluster of parallel edges between the same node pair, all with amounts in a narrow band below a threshold.

#### 10.4.1 Readiness Assessment

| Layer | Status | Detail |
|-------|--------|--------|
| **Threshold-adjacent amounts** | ✅ Ready | `FraudAmountPattern::ThresholdAdjacent` already exists. `FraudConfig.approval_thresholds` defines thresholds `[1000, 5000, 10000, 25000, 50000, 100000]`. |
| **Split transaction fraud** | ✅ Ready | `FraudType::SplitTransaction` and `FraudType::JustBelowThreshold` both exist. |
| **InjectionStrategy** | ✅ Ready | `InjectionStrategy::ThresholdAvoidance { threshold, adjusted_amount }` and `InjectionStrategy::SplitTransaction { original_amount, split_count, split_doc_ids }` both exist. |
| **SchemeType** | ❌ Missing | No `Smurfing` variant. Could map to `Custom` but a dedicated variant is preferred. |
| **SchemeActionType** | ⚠️ Partial | `CreateFraudulentPayment` exists. Need to carry threshold and split info. |
| **FraudScheme struct** | ❌ Missing | No `SmurfingScheme`. |
| **Multi-path spreading** | ❌ Missing | Current `SplitTransaction` is a single-point anomaly in `je_generator.rs`. The scheme needs to generate many transactions over time across multiple accounts/dates, not just split a single entry. |

#### 10.4.2 Required Changes

**File: `crates/datasynth-core/src/models/anomaly.rs`**

- Add `Smurfing` to `SchemeType`.

**File: `crates/datasynth-generators/src/anomaly/schemes/scheme.rs`**

- Add to `SchemeActionType`:
  ```rust
  CreateStructuredPayment,  // payment deliberately below threshold
  ```
- Extend `SchemeAction` with:
  ```rust
  pub threshold_target: Option<Decimal>,   // the threshold being evaded
  pub structuring_count: Option<u32>,      // how many splits in this batch
  ```

**(Smurfing scheme was an earlier design sketch and has been removed from the implementation and config.)**

---

### 10.5 Scheme 5 — Gradual Embezzlement (Slow Burn)

**Paper description:** An employee slowly escalates fraudulent entries over months, starting small and increasing. The topological signature is a slowly increasing edge weight trend from a specific user to specific accounts.

#### 10.5.1 Readiness Assessment

| Layer | Status | Detail |
|-------|--------|--------|
| **FraudScheme struct** | ✅ Implemented | `GradualEmbezzlementScheme` in `schemes/embezzlement.rs` with 4 stages: Testing (2mo, $100–500), Escalation (6mo, $500–2K), Acceleration (3mo, $2K–10K), Desperation (1mo, $10K–50K). |
| **SchemeType** | ✅ Ready | `SchemeType::GradualEmbezzlement`. |
| **SchemeAdvancer** | ✅ Ready | Samples and advances embezzlement schemes. `embezzlement_probability: 0.02` default. |
| **Concealment** | ✅ Ready | Stages use `ConcealmentTechnique::TransactionSplitting`, `TimingManipulation`, `DocumentManipulation`, `AccountMisclassification`. |
| **Labeling** | ⚠️ Partial | `MultiStageAnomalyLabel` has `scheme_type: SchemeType` but no `pathology_name` or `pathology_category` fields for paper taxonomy alignment. |
| **SchemeAction → JE** | ❌ Missing | Actions produced but not materialized into entries (same cross-cutting gap). |

#### 10.5.2 Required Changes

**File: `crates/datasynth-generators/src/anomaly/scheme_advancer.rs`**

- Add to `MultiStageAnomalyLabel`:
  ```rust
  pub pathology_name: String,        // "GradualEmbezzlement"
  pub pathology_category: String,    // "Sequential"
  ```
- In `record_label()`: populate `pathology_name` from `SchemeType` display name, `pathology_category` from a lookup (`GradualEmbezzlement → "Sequential"`, etc.).

**File: `crates/datasynth-generators/src/anomaly/schemes/embezzlement.rs`**

- No structural changes needed. Validate that `advance()` produces actions with sufficient frequency to generate labeled JEs.
- Verify that the `detection_probability()` logic correctly accumulates risk across stages.

**File: `crates/datasynth-config/src/schema.rs`**

- The existing `EmbezzlementSchemeConfig` is sufficient. No changes needed.

**Graph integration:**

- The embezzlement signature appears as a temporal trend in edge weights from user → expense accounts.
- Requires `include_users: true` in `TransactionGraphConfig` (missing, see §10.12).
- Edge timestamps (`posting_date`) already exported allow temporal trend detection.
- The Python SAE computation can measure increasing local disorder over time windows.

---

### 10.6 Scheme 6 — Revenue Manipulation (Channel Stuffing)

**Paper description:** Premature or fictitious revenue recognition, especially at quarter-end, disconnected from physical delivery/inventory flows. The topological signature is revenue edges concentrated at period-end with no corresponding inventory/COGS edges.

#### 10.6.1 Readiness Assessment

| Layer | Status | Detail |
|-------|--------|--------|
| **FraudScheme struct** | ✅ Implemented | `RevenueManipulationScheme` in `schemes/revenue_manipulation.rs` with 4 quarterly stages: Early Recognition, Expense Deferral, Reserve Release, Channel Stuffing. |
| **O2C document flow** | ✅ Ready | `O2CDocumentChain` (SO → Delivery → CustomerInvoice → CustomerReceipt) in `document_flow/o2c_generator.rs`. |
| **FraudType** | ✅ Ready | `FraudType::ChannelStuffing`, `FraudType::PrematureRevenue`, `FraudType::RevenueManipulation`, `FraudType::RevenueTimingManipulation`. |
| **Period-end dynamics** | ✅ Ready | `PeriodEndDynamics` in `distributions/period_end.rs` models month-end, quarter-end, year-end spikes. |
| **Revenue–inventory disconnect** | ⚠️ Partial | The scheme generates revenue JEs but does not currently verify or intentionally break the SO→Delivery→Invoice chain. Revenue entries are standalone, not linked to the O2C flow. |
| **Labeling** | ⚠️ Partial | Same `pathology_name`/`pathology_category` gap as Embezzlement. |
| **SchemeAction → JE** | ❌ Missing | Same cross-cutting gap. |

#### 10.6.2 Required Changes

**File: `crates/datasynth-generators/src/anomaly/schemes/revenue_manipulation.rs`**

- Enhance `advance()` to:
  - Generate actions that explicitly target O2C accounts (AR, Revenue).
  - Include metadata indicating whether the revenue entry has a corresponding delivery document. Field: `has_delivery_support: bool` on `SchemeAction` or in `description`.
  - For Channel Stuffing (Stage 4): generate sales entries without corresponding `Delivery` records, creating a revenue–inventory disconnect visible in the document flow.

**File: `crates/datasynth-generators/src/anomaly/scheme_advancer.rs`**

- Add `pathology_name: "RevenueManipulation"`, `pathology_category: "Volume"` in label generation.

**File: `crates/datasynth-config/src/schema.rs`**

- The existing `RevenueManipulationSchemeConfig` is sufficient but could benefit from a `disconnected_delivery_rate: f64` field controlling how often channel-stuffed revenue lacks delivery documents.

**Graph integration:**

- In the transaction graph: revenue manipulation appears as DR AR / CR Revenue edges at period-end without corresponding DR COGS / CR Inventory edges.
- The Python RIP layer can compute conservation violations: revenue recognized without matching expense/inventory flows.
- Need `document_type` property on edges (see §10.12) to distinguish `INVOICE` vs `DELIVERY` vs `PAYMENT` edges, enabling the ERA module to attend differently by relation type.

---

### 10.7 Scheme 7 — Circular Funding (Round-Tripping)

**Paper description:** Cash flows A→B→C→A forming a strongly connected component (SCC) with net-zero consolidated impact. Used to inflate revenue or disguise loans. The topological signature is a cycle in the intercompany/entity graph.

#### 10.7.1 Readiness Assessment

| Layer | Status | Detail |
|-------|--------|--------|
| **Intercompany models** | ✅ Ready | `IntercompanyRelationship`, `ICTransactionType` (GoodsSale, ServiceProvided, ManagementFee, Royalty, LoanInterest, etc.), `ICMatchedPair` in core models. |
| **IC generator** | ✅ Ready | `ICGenerator` produces seller/buyer JE pairs with matched references. |
| **IC matching** | ✅ Ready | `ICMatchingEngine` tracks receivables/payables and matches. |
| **IC elimination** | ✅ Ready | `EliminationGenerator` eliminates IC balances, revenue/expense, unrealized profit. |
| **SchemeType** | ⚠️ Partial | `SchemeType::RoundTripping` exists as enum variant but has **no implementing struct**. |
| **Banking module** | ✅ Ready | `BankingOrchestrator` with AML typologies including `round_tripping`. |
| **FraudType** | ⚠️ Partial | `RelationalAnomalyType::CircularTransaction` exists. `FraudType::RoundTripping` does not exist (would need to add, or use `Custom`). |
| **Multi-company context** | ⚠️ Partial | `SchemeContext` has `company_code: String` (singular). Circular funding requires 3+ companies. Need `available_companies: Vec<String>`. |
| **FraudScheme struct** | ❌ Missing | No `CircularFundingScheme`. |

#### 10.7.2 Required Changes

**File: `crates/datasynth-core/src/models/anomaly.rs`**

- Verify `SchemeType::RoundTripping` is present (✅ it is). No change needed.
- Add `FraudType::CircularFunding` or reuse `FraudType::RoundTripping` (currently missing from `FraudType` enum — must add).

**File: `crates/datasynth-generators/src/anomaly/schemes/scheme.rs`**

- Add to `SchemeActionType`:
  ```rust
  IntercompanyRoundTrip,    // IC transaction forming part of a cycle
  CreateIntercompanyLoan,   // set up IC loan
  ```
- Extend `SchemeContext` with:
  ```rust
  pub available_companies: Vec<String>,  // for multi-entity schemes
  pub available_ic_relationships: Vec<(String, String)>,  // (seller, buyer) pairs
  ```

**File: `crates/datasynth-generators/src/anomaly/schemes/circular_funding.rs` (NEW)**

```rust
pub struct CircularFundingScheme {
    scheme_id: Uuid,
    perpetrator_id: String,
    cycle_entities: Vec<String>,    // e.g. ["C001", "C002", "C003"]
    cycle_amount: Decimal,
    ic_type: ICTransactionType,     // ManagementFee, LoanInterest, etc.
    stages: Vec<SchemeStage>,
    current_stage: usize,
    status: SchemeStatus,
    total_cycled: Decimal,
    transactions: Vec<SchemeTransactionRef>,
}
```

- **Stage 1 — Relationship setup (2 months):** Establish IC relationships between 3+ entities. Generate a few legitimate IC transactions to create a baseline. Actions: `CreateIntercompanyLoan` (legitimate).
- **Stage 2 — Cycle activation (6 months):** Generate round-trip cash flows: C001→C002 (ManagementFee), C002→C003 (ServiceProvided), C003→C001 (LoanInterest). Each leg produces a matched IC JE pair. The consolidated net impact is zero but each entity shows inflated revenue. Actions: `IntercompanyRoundTrip`. Amount range: $100K–$1M per cycle.
- **Stage 3 — Escalation (6 months):** Increase cycle frequency and amounts. May add a 4th entity to obscure the cycle. Actions: `IntercompanyRoundTrip`.
- **Stage 4 — Wind-down (2 months):** Reduce and settle balances. Actions: `Conceal`.

**File: `crates/datasynth-generators/src/anomaly/scheme_advancer.rs`**

- Add `circular_funding_probability: f64` to `SchemeAdvancerConfig`.
- In `maybe_start_scheme()`: requires `context.available_companies.len() >= 3`. Select 3 companies forming the cycle.

**File: `crates/datasynth-config/src/schema.rs`**

- Add `circular_funding: CircularFundingSchemeConfig` to `MultiStageSchemeConfig`.
- Fields: `probability`, `min_cycle_entities: usize` (default 3), `max_cycle_entities: usize` (default 5), `cycle_amount_range`, `ic_type`.

**Graph integration:**

- Circular funding creates a topological SCC in the entity/intercompany graph.
- `EntityGraphBuilder` already builds `CompanyNode`s and `EdgeType::Intercompany` edges. The cycle A→B→C→A will be directly visible.
- For the transaction-level graph: IC JEs create account-level edges (DR IC Receivable / CR Revenue in seller, DR Expense / CR IC Payable in buyer). The cycle appears as a closed loop through IC clearing accounts across entities.
- The Python SCC detection can identify the cycle in the exported graph.
- Edge property `is_intercompany: bool` should be added to `TransactionEdge` to flag IC edges (currently derivable from account codes but not explicit).

---

### 10.8 Scheme 8 — Vendor Kickbacks (Relational Triad)

**Paper description:** A procurement employee colludes with a vendor to inflate invoices, and the vendor returns a portion as a kickback. The topological signature is a triad: Employee → Vendor → Cash flow back to Employee (or related party).

#### 10.8.1 Readiness Assessment

| Layer | Status | Detail |
|-------|--------|--------|
| **FraudScheme struct** | ✅ Implemented | `VendorKickbackScheme` in `schemes/kickback.rs` with 4 stages: Setup (3mo), Price Inflation (12mo, 10–25%), Kickback Payments (6mo, 30–70% of inflation), Concealment (3mo). |
| **SchemeType** | ✅ Ready | `SchemeType::VendorKickback`. |
| **SchemeAdvancer** | ✅ Ready | `kickback_probability: 0.01` default. Tracks `active_vendors` to avoid reuse. |
| **SchemeActionType** | ✅ Ready | `CreateFictitiousVendor`, `InflateInvoice`, `MakeKickbackPayment`, `CoverUp`. |
| **Vendor model** | ✅ Ready | Full vendor with `bank_accounts`, `payment_terms`, etc. |
| **Price/volume features** | ⚠️ Partial | `TransactionEdge` has `debit_amount`, `credit_amount`, `cost_center`, `business_process`. But no explicit `unit_price` or `quantity` features on edges. These exist on document flow models (`PurchaseOrderItem.unit_price`, `GoodsReceiptItem.quantity`) but are not propagated to graph edges. |
| **Labeling** | ⚠️ Partial | Same `pathology_name`/`pathology_category` gap. |
| **SchemeAction → JE** | ❌ Missing | Same cross-cutting gap. |

#### 10.8.2 Required Changes

**File: `crates/datasynth-generators/src/anomaly/schemes/kickback.rs`**

- Add `pathology_name()` and `pathology_category()` helper methods returning `"VendorKickback"` and `"Relational"`.
- Ensure that `InflateInvoice` actions carry the `inflation_percent` in `SchemeAction.description` or a new field, so the materializer can compute the actual inflated amount.

**File: `crates/datasynth-generators/src/anomaly/scheme_advancer.rs`**

- Add `pathology_name: "VendorKickback"`, `pathology_category: "Relational"` in label generation.

**File: `crates/datasynth-graph/src/builders/transaction_graph.rs`**

- When building edges from document flow data (future enhancement): propagate `unit_price` and `quantity` as edge features. This enables the ERA module to detect price inflation patterns.
- For now: the `TransactionEdge.edge.weight` (amount) is sufficient. The inflation shows as increasing edge weights between the same vendor and expense accounts over time.

**File: `crates/datasynth-graph/src/models/edges.rs`**

- Add optional `TransactionEdge` fields:
  ```rust
  pub unit_price: Option<f64>,
  pub quantity: Option<f64>,
  pub inflation_percent: Option<f64>,   // for labeled kickback edges
  ```

**Graph integration:**

- The kickback triad requires `include_vendors: true` and `include_users: true` in the graph.
- Visible as: `UserNode(perpetrator)` → `VendorNode(colluding vendor)` via inflated invoice edges, and `VendorNode` → back-channel payment to `UserNode` (or related cash account).
- Existing `ApprovalGraphBuilder` can overlay approval edges showing the perpetrator approved their own inflated invoices.

---

### 10.9 Scheme 9 — Phantom Warehousing (Inventory Isolate)

**Paper description:** Inventory is moved to ghost storage locations that never connect to sales or cash flows. The topological signature is an isolated subgraph of inventory movement edges disconnected from revenue/COGS/cash sinks.

#### 10.9.1 Readiness Assessment

| Layer | Status | Detail |
|-------|--------|--------|
| **Inventory movement model** | ✅ Ready | `InventoryMovement` in `manufacturing_models.rs` with `movement_type` (GoodsReceipt, GoodsIssue, Transfer, Return, Scrap, Adjustment), `storage_location`, `material_code`, `quantity`, `value`. |
| **Storage locations** | ⚠️ Partial | Currently hardcoded as `STORAGE_LOCATIONS = ["WH01-A01", "WH01-A02", "WH01-B01", "WH02-A01", "WH02-B01", "WH03-A01"]` in `inventory_movement_generator.rs`. No concept of "ghost" or "non-productive" locations. No location master data model. |
| **Inventory position tracking** | ✅ Ready | `InventoryPosition` in `subledger/inventory_generator.rs` tracks `quantity_on_hand`, `quantity_in_transit`, `quantity_blocked` per `(material_id, plant, storage_location)`. |
| **Manufacturing JE generation** | ✅ Ready | Manufacturing flow generates JEs for inventory movements (DR Inventory / CR GR/IR, DR COGS / CR Inventory, etc.). |
| **FraudType** | ✅ Ready | `FraudType::InventoryTheft` and `FraudType::AssetMisappropriation` exist. |
| **SchemeType** | ✅ Ready (partial) | `SchemeType::InventoryTheft` exists but no implementing struct. Phantom warehousing is a variant. |
| **SchemeActionType** | ❌ Missing | No `InventoryTransferToGhostLocation` or `CreateGhostLocation` variants. |
| **FraudScheme struct** | ❌ Missing | No `PhantomWarehousingScheme`. |
| **Location master data** | ❌ Missing | No `StorageLocation` master data model. Locations are strings, not entities with attributes (active/inactive, productive/non-productive). |

#### 10.9.2 Required Changes

**File: `crates/datasynth-core/src/models/manufacturing_models.rs`**

- Add a new model:
  ```rust
  pub struct StorageLocation {
      pub location_id: String,
      pub plant: String,
      pub description: String,
      pub location_type: StorageLocationType,
      pub is_productive: bool,        // connected to sales/shipping
      pub is_active: bool,
      pub created_date: Option<NaiveDate>,
  }

  pub enum StorageLocationType {
      Warehouse,
      ShippingDock,
      ReceivingDock,
      QualityInspection,
      Production,
      Ghost,           // scheme-injected
  }
  ```

**File: `crates/datasynth-core/src/models/anomaly.rs`**

- Add `PhantomWarehousing` to `SchemeType` (or reuse `InventoryTheft`; a dedicated variant is better for labeling).

**File: `crates/datasynth-generators/src/anomaly/schemes/scheme.rs`**

- Add to `SchemeActionType`:
  ```rust
  CreateGhostLocation,
  InventoryTransferToGhostLocation,
  CyclicalInventoryTransfer,   // transfer between ghost locations
  ```
- Extend `SchemeAction` with:
  ```rust
  pub ghost_location_id: Option<String>,
  pub material_id: Option<String>,
  pub quantity: Option<f64>,
  ```

**File: `crates/datasynth-generators/src/anomaly/schemes/phantom_warehousing.rs` (NEW)**

```rust
pub struct PhantomWarehousingScheme {
    scheme_id: Uuid,
    perpetrator_id: String,
    ghost_locations: Vec<String>,
    target_materials: Vec<String>,
    stages: Vec<SchemeStage>,
    current_stage: usize,
    status: SchemeStatus,
    total_diverted_value: Decimal,
    transactions: Vec<SchemeTransactionRef>,
}
```

- **Stage 1 — Location creation (1 month):** Create 2–4 ghost storage locations with `StorageLocationType::Ghost`, `is_productive: false`. Actions: `CreateGhostLocation`.
- **Stage 2 — Initial diversion (3 months):** Transfer inventory from productive locations to ghost locations via `Transfer` movements. JE: DR Inventory-Ghost / CR Inventory-Productive. Amount: $10K–$100K worth of materials per month. Actions: `InventoryTransferToGhostLocation`.
- **Stage 3 — Circular cycling (6 months):** Move inventory between ghost locations to create activity and obscure the diversion. JE: DR Inventory-Ghost2 / CR Inventory-Ghost1. Actions: `CyclicalInventoryTransfer`. The inventory never flows to COGS or Sales.
- **Stage 4 — Adjustment/write-off (2 months):** Write off or adjust inventory to close the loop. JE: DR Scrap/Loss / CR Inventory-Ghost. Actions: `Conceal`.

**File: `crates/datasynth-generators/src/manufacturing/inventory_movement_generator.rs`**

- Replace hardcoded `STORAGE_LOCATIONS` with a configurable location pool.
- Add method `generate_transfer(from_location: &str, to_location: &str, material: &str, quantity: f64, date: NaiveDate) -> InventoryMovement`.
- Support `MovementType::Transfer` with explicit from/to locations.

**File: `crates/datasynth-config/src/schema.rs`**

- Add `phantom_warehousing: PhantomWarehousingSchemeConfig` to `MultiStageSchemeConfig`.
- Fields: `probability`, `ghost_location_count`, `target_material_count`, `diversion_value_range`.

**Graph integration:**

- Ghost storage locations should appear as `NodeType::Custom("StorageLocation")` or a new `NodeType::StorageLocation` in the graph.
- Inventory transfer edges connect storage location nodes. Ghost locations form an isolated subgraph: they have no outgoing edges to Revenue, COGS, or Cash nodes.
- The Python RIP layer can detect conservation violations: inventory value enters the ghost subgraph but never exits to legitimate sinks.
- `TransactionGraphConfig` needs `include_storage_locations: bool` flag.

---

### 10.10 Scheme 10 — Intercompany Wash Trades (Wash)

**Paper description:** Symmetric intercompany trades between subsidiaries that cancel out in consolidation but inflate individual entity revenue/expense. The topological signature is parallel edges A→B and B→A with matching amounts and types.

#### 10.10.1 Readiness Assessment

| Layer | Status | Detail |
|-------|--------|--------|
| **IC models** | ✅ Ready | `ICMatchedPair` (seller JE + buyer JE), `ICTransactionType`, `IntercompanyRelationship`. |
| **IC generator** | ✅ Ready | `ICGenerator` creates matched seller/buyer pairs with proper GL mappings. |
| **IC elimination** | ✅ Ready | `EliminationGenerator` can detect and eliminate IC balances. |
| **IC matching engine** | ✅ Ready | `ICMatchingEngine` matches receivables/payables by reference and amount. |
| **FraudType** | ⚠️ Partial | `RelationalAnomalyType::UnmatchedIntercompany` and `RelationalAnomalyType::TransferPricingAnomaly` exist. But no `IntercompanyWashTrade` variant. |
| **SchemeType** | ❌ Missing | No `IntercompanyWashTrade` variant. |
| **Multi-entity context** | ⚠️ Partial | Same `SchemeContext.available_companies` gap as Circular Funding (§10.7). |
| **FraudScheme struct** | ❌ Missing | No `IntercompanyWashTradeScheme`. |

#### 10.10.2 Required Changes

**File: `crates/datasynth-core/src/models/anomaly.rs`**

- Add `IntercompanyWashTrade` to `SchemeType`.
- Add `FraudType::IntercompanyWashTrade` to `FraudType`.

**File: `crates/datasynth-generators/src/anomaly/schemes/scheme.rs`**

- Add to `SchemeActionType`:
  ```rust
  CreateWashTrade,          // symmetric IC trade
  CreateCounterWashTrade,   // the return leg
  ```

**File: `crates/datasynth-generators/src/anomaly/schemes/intercompany_wash_trade.rs` (NEW)**

```rust
pub struct IntercompanyWashTradeScheme {
    scheme_id: Uuid,
    perpetrator_id: String,
    entity_a: String,
    entity_b: String,
    wash_type: ICTransactionType,
    stages: Vec<SchemeStage>,
    current_stage: usize,
    status: SchemeStatus,
    total_washed: Decimal,
    transactions: Vec<SchemeTransactionRef>,
}
```

- **Stage 1 — Establish relationship (2 months):** Create legitimate IC transactions between A and B to establish a baseline. Actions: `IntercompanyRoundTrip` (legitimate).
- **Stage 2 — Symmetric wash trades (9 months):** For each wash trade, generate two matched IC transactions:
  - A sells to B: DR IC Receivable (A) / CR Revenue (A) and DR Expense (B) / CR IC Payable (B).
  - B sells to A: DR IC Receivable (B) / CR Revenue (B) and DR Expense (A) / CR IC Payable (A).
  - Same amount, same period. Net consolidated effect: zero. But each entity shows inflated revenue.
  - Actions: `CreateWashTrade`, `CreateCounterWashTrade`. Amount: $50K–$500K per pair.
- **Stage 3 — Escalation (3 months):** Increase amounts and frequency. May involve 3+ entities for obfuscation. Actions: `CreateWashTrade`.
- **Stage 4 — Wind-down (2 months):** Reduce and settle. Actions: `Conceal`.

**File: `crates/datasynth-generators/src/anomaly/scheme_advancer.rs`**

- Add `wash_trade_probability: f64` to `SchemeAdvancerConfig`.
- In `maybe_start_scheme()`: requires `context.available_companies.len() >= 2` and existing IC relationships.

**File: `crates/datasynth-config/src/schema.rs`**

- Add `intercompany_wash_trade: IntercompanyWashTradeSchemeConfig` to `MultiStageSchemeConfig`.
- Fields: `probability`, `wash_amount_range`, `wash_duration_months`, `ic_type`.

**Graph integration:**

- In the entity graph: parallel bidirectional edges between A and B with matching amounts. `EntityGraphBuilder` already creates `EdgeType::Intercompany` edges.
- In the transaction graph: symmetric pairs of IC clearing account edges visible as parallel edges with matching weights.
- The `ICMatchingEngine` will show these as perfectly matched — the fraud is that the trades are fictitious, not unmatched.
- The label on the IC JEs (`is_anomaly: true`, `anomaly_type: IntercompanyWashTrade`) propagates to graph edges for supervised training.

---

### 10.11 Cross-Cutting Specification: SchemeAction Materializer

**This is the single largest gap** affecting all 7 unimplemented schemes (and improving the 3 existing ones). Currently, `SchemeAdvancer.advance_all()` returns `Vec<SchemeAction>` but no component converts these into `JournalEntry` records.

#### Current Flow (broken)

```
SchemeAdvancer.advance_all() → Vec<SchemeAction>
  ↓
AnomalyInjector.advance_schemes() stores in self.scheme_actions
  ↓
(dead end — actions are never turned into JEs)
```

#### Required Flow

```
SchemeAdvancer.advance_all() → Vec<SchemeAction>
  ↓
SchemeActionMaterializer.materialize(actions, context) → Vec<JournalEntry>
  ↓
Each JE has: header.is_anomaly=true, header.is_fraud=true,
  header.fraud_type=Some(FraudType::*), header.anomaly_type=Some(AnomalyType::Fraud(*))
  ↓
JEs are merged into main entry stream
  ↓
Labels recorded via SchemeAdvancer.record_label()
```

#### Implementation

**File: `crates/datasynth-generators/src/anomaly/scheme_materializer.rs` (NEW)**

```rust
pub struct SchemeActionMaterializer {
    rng: ChaCha8Rng,
    accounts: AccountLookup,         // GL account pool
    company_code: String,
}

impl SchemeActionMaterializer {
    pub fn materialize(
        &mut self,
        action: &SchemeAction,
        scheme_type: SchemeType,
    ) -> Option<JournalEntry> {
        match action.action_type {
            SchemeActionType::CreateFraudulentEntry => { /* DR expense / CR cash */ }
            SchemeActionType::CreateFraudulentPayment => { /* DR AP / CR cash */ }
            SchemeActionType::InflateInvoice => { /* DR expense / CR AP with inflated amount */ }
            SchemeActionType::MakeKickbackPayment => { /* DR misc expense / CR cash */ }
            SchemeActionType::ManipulateRevenue => { /* DR AR / CR revenue */ }
            SchemeActionType::DeferExpense => { /* DR prepaid / CR expense */ }
            SchemeActionType::ReleaseReserves => { /* DR reserve / CR revenue */ }
            SchemeActionType::ChannelStuff => { /* DR AR / CR revenue (no delivery) */ }
            SchemeActionType::CreateStructuredPayment => { /* DR expense / CR cash, below threshold */ }
            SchemeActionType::CreateMicroExpense => { /* DR expense / CR cash, small amount */ }
            SchemeActionType::CreatePayrollEntry => { /* DR salary / CR cash */ }
            SchemeActionType::IntercompanyRoundTrip => { /* DR IC recv / CR revenue + DR expense / CR IC pay */ }
            SchemeActionType::CreateWashTrade => { /* symmetric IC pair */ }
            SchemeActionType::InventoryTransferToGhostLocation => { /* DR inv-ghost / CR inv-prod */ }
            SchemeActionType::CyclicalInventoryTransfer => { /* DR inv-ghost2 / CR inv-ghost1 */ }
            SchemeActionType::Conceal | SchemeActionType::CoverUp => { /* adjusting/reclassification JE */ }
            SchemeActionType::ReuseDocumentId => { /* payment with reused doc number */ }
            SchemeActionType::CreateGhostEmployee |
            SchemeActionType::CreateGhostBankAccount |
            SchemeActionType::CreateShellVendor |
            SchemeActionType::CreateGhostLocation => {
                // Master data mutations — return None (handled by master data generators)
                return None;
            }
            _ => return None,
        }
    }
}
```

Each materialized JE must:
1. Enforce double-entry (debits = credits).
2. Set `header.is_anomaly = true`, `header.is_fraud = true`.
3. Set `header.fraud_type = Some(FraudType::*)` based on scheme type.
4. Set `header.anomaly_type = Some(AnomalyType::Fraud(*))`.
5. Set `header.created_by = action.user_id` (perpetrator).
6. Set `header.document_number` uniquely (or reuse existing for `ReuseDocumentId`).
7. Record `SchemeTransactionRef` on the scheme.

**File: `crates/datasynth-generators/src/anomaly/mod.rs`**

- Add `pub mod scheme_materializer;`.

**File: `crates/datasynth-runtime/src/enhanced_orchestrator.rs`**

- In the anomaly injection phase (or as a new sub-phase): after `advance_schemes()`, call `SchemeActionMaterializer.materialize()` for each action and append the resulting JEs to the main entry vector.
- Ensure the materialized JEs pass through `RunningBalanceTracker` for balance coherence.

---

### 10.12 Cross-Cutting Specification: Graph Builder Extensions

Several schemes require graph builder capabilities that don't yet exist.

#### 10.12.1 TransactionGraphConfig Extensions

**File: `crates/datasynth-graph/src/builders/transaction_graph.rs`**

Add to `TransactionGraphConfig`:
```rust
pub include_users: bool,              // default: false
pub include_cost_centers: bool,       // default: false
pub include_storage_locations: bool,  // default: false
```

In `TransactionGraphBuilder`:

- **When `include_users` is true:**
  - For each JE, create `NodeType::User` node keyed by `entry.header.created_by`.
  - Add `EdgeType::Custom("CreatedBy")` edge from User node to document/account node.
  - `UserNode` features: `persona` (categorical), `department`, `approval_limit`, `bank_account_id`, `address_region`, `login_region`.

- **When `include_cost_centers` is true:**
  - For each JE line with `cost_center.is_some()`, create `NodeType::CostCenter` node.
  - Add `EdgeType::CostAllocation` edge from cost center to account node.

- **When `include_storage_locations` is true:**
  - For inventory movement JEs, create `NodeType::Custom("StorageLocation")` node.
  - Add edges representing inventory transfers between locations.

#### 10.12.2 TransactionEdge Extensions

**File: `crates/datasynth-graph/src/models/edges.rs`**

Add to `TransactionEdge`:
```rust
pub document_type: Option<String>,     // "INVOICE", "PAYMENT", "PAYROLL", "INVENTORY", etc.
pub source_type: Option<String>,       // "Manual", "Automated", "Recurring", "Adjustment"
pub is_intercompany: bool,             // whether this edge is part of an IC transaction
pub scheme_id: Option<String>,         // if part of a fraud scheme
pub pathology_name: Option<String>,    // e.g. "Smurfing", "CircularFunding"
pub pathology_category: Option<String>, // "Sequential", "Volume", "Relational"
```

These become:
- Edge properties in `GraphEdge.properties`.
- Categorical features for ERA attention grouping.
- Supervision labels for pathology-specific detection.

#### 10.12.3 PyGExporter Metadata Extensions

**File: `crates/datasynth-graph/src/exporters/pytorch_geometric.rs`**

Extend `metadata.json` with:
```json
{
  "node_types": { "Account": [0, 150], "User": [150, 180], "Vendor": [180, 230], ... },
  "edge_types": { "Transaction": [0, 5000], "CreatedBy": [5000, 5200], ... },
  "pathology_labels": {
    "TriadBypass": { "edge_indices": [...], "category": "Relational" },
    "ShadowPayroll": { "edge_indices": [...], "category": "Sequential" },
    ...
  }
}
```

This enables the Python side to:
- Build `HeteroData` by slicing node/edge arrays by type ranges.
- Create per-pathology binary labels for multi-task training.
- Compute SAE per pathology category.

---

### 10.13 Cross-Cutting Specification: Configuration Schema

**File: `crates/datasynth-config/src/schema.rs`**

The `MultiStageSchemeConfig` must be extended to include all 10 schemes:

```rust
pub struct MultiStageSchemeConfig {
    pub enabled: bool,
    // Existing schemes
    pub embezzlement: EmbezzlementSchemeConfig,
    pub revenue_manipulation: RevenueManipulationSchemeConfig,
    pub kickback: KickbackSchemeConfig,
    // New schemes
    pub triad_bypass: TriadBypassSchemeConfig,
    pub shadow_payroll: ShadowPayrollSchemeConfig,
    pub expense_laundering: ExpenseLaunderingSchemeConfig,
    pub circular_funding: CircularFundingSchemeConfig,
    pub phantom_warehousing: PhantomWarehousingSchemeConfig,
    pub intercompany_wash_trade: IntercompanyWashTradeSchemeConfig,
}
```

Each new config struct should follow the pattern of `EmbezzlementSchemeConfig`:
```rust
pub struct <Scheme>SchemeConfig {
    pub probability: f64,
    // Per-stage configs
    pub stage_1: SchemeStageConfig,
    pub stage_2: SchemeStageConfig,
    // ...
    // Scheme-specific parameters
    // ...
}
```

**File: `crates/datasynth-generators/src/anomaly/scheme_advancer.rs`**

`SchemeAdvancerConfig` must be extended with per-scheme probabilities:

```rust
pub struct SchemeAdvancerConfig {
    // Existing
    pub embezzlement_probability: f64,
    pub revenue_manipulation_probability: f64,
    pub kickback_probability: f64,
    // New
    pub triad_bypass_probability: f64,
    pub shadow_payroll_probability: f64,
    pub expense_laundering_probability: f64,
    pub circular_funding_probability: f64,
    pub phantom_warehousing_probability: f64,
    pub wash_trade_probability: f64,
    // Existing
    pub max_concurrent_schemes: usize,
    pub allow_repeat_perpetrators: bool,
    pub seed: u64,
}
```

The `maybe_start_scheme()` method must be refactored from the current 3-branch if-else to a weighted sampling approach across all 10 scheme types.

---

### 10.14 Cross-Cutting Specification: Labeling and MultiStageAnomalyLabel

**File: `crates/datasynth-generators/src/anomaly/scheme_advancer.rs`**

Extend `MultiStageAnomalyLabel`:

```rust
pub struct MultiStageAnomalyLabel {
    // Existing fields
    pub anomaly_id: String,
    pub scheme_id: Uuid,
    pub scheme_type: SchemeType,
    pub stage_number: u32,
    pub stage_name: String,
    pub total_stages: u32,
    pub perpetrator_id: String,
    pub scheme_detected: bool,
    // New fields for paper taxonomy
    pub pathology_name: String,           // e.g. "Smurfing"
    pub pathology_category: String,       // "Sequential", "Volume", "Relational"
    pub topological_signature: String,    // e.g. "fan_out", "scc", "parallel_edges"
    pub affected_entity_ids: Vec<String>, // companies, vendors, employees involved
    pub monetary_impact: Option<Decimal>,
}
```

Add a helper function for the pathology taxonomy mapping:

```rust
fn pathology_metadata(scheme_type: &SchemeType) -> (String, String, String) {
    match scheme_type {
        SchemeType::GradualEmbezzlement   => ("GradualEmbezzlement", "Sequential", "temporal_drift"),
        SchemeType::RevenueManipulation   => ("RevenueManipulation", "Volume", "period_end_spike"),
        SchemeType::VendorKickback        => ("VendorKickback", "Relational", "triad"),
        SchemeType::RoundTripping         => ("CircularFunding", "Relational", "scc"),
        SchemeType::GhostEmployee         => ("ShadowPayroll", "Sequential", "identity_collision"),
        SchemeType::InventoryTheft        => ("PhantomWarehousing", "Relational", "isolate"),
        SchemeType::ExpenseReimbursement  => ("ExpenseLaundering", "Volume", "fan_out"),
        SchemeType::Custom                => ("Custom", "Custom", "custom"),
        // New variants to add:
        // SchemeType::TriadBypass        => ("TriadBypass", "Relational", "broken_triad"),
        // SchemeType::Smurfing           => ("Smurfing", "Volume", "parallel_edges"),
        // SchemeType::IntercompanyWashTrade => ("IntercompanyWashTrade", "Relational", "symmetric_parallel"),
    }
}
```

---

### 10.15 Summary: Readiness Matrix

| # | Scheme | SchemeType | FraudScheme Struct | SchemeActionType | Domain Models | Config | Graph Support | Materializer | Overall |
|---|--------|-----------|-------------------|-----------------|---------------|--------|---------------|-------------|---------|
| 1 | Triad Bypass | ❌ Add | ❌ New file | ❌ Add `ReuseDocumentId` | ✅ P2P flow ready | ❌ Add | ⚠️ Need `document_type` on edges | ❌ Blocked | 🔴 |
| 2 | Shadow Payroll | ⚠️ Exists (`GhostEmployee`) | ❌ New file | ❌ Add 3 variants | ⚠️ Need `bank_account_id` on Employee | ❌ Add | ⚠️ Need `include_users` | ❌ Blocked | 🔴 |
| 3 | Expense Laundering | ❌ Add | ❌ New file | ❌ Add 2 variants | ⚠️ Need `is_verified` on Vendor | ❌ Add | ⚠️ Need `include_vendors` wiring | ❌ Blocked | 🔴 |
| 4 | Smurfing | ❌ Add | ❌ New file | ❌ Add 1 variant | ✅ Thresholds ready | ❌ Add | ✅ Existing features | ❌ Blocked | 🟡 |
| 5 | Gradual Embezzlement | ✅ Ready | ✅ Implemented | ✅ Ready | ✅ Ready | ✅ Ready | ⚠️ Need `include_users` | ❌ Blocked | 🟡 |
| 6 | Revenue Manipulation | ✅ Ready | ✅ Implemented | ✅ Ready | ✅ O2C ready | ✅ Ready | ⚠️ Need `document_type` on edges | ❌ Blocked | 🟡 |
| 7 | Circular Funding | ⚠️ Exists (`RoundTripping`) | ❌ New file | ❌ Add 2 variants | ✅ IC ready | ❌ Add | ⚠️ Need `is_intercompany` on edges | ❌ Blocked | 🔴 |
| 8 | Vendor Kickbacks | ✅ Ready | ✅ Implemented | ✅ Ready | ✅ Ready | ✅ Ready | ⚠️ Need vendor graph wiring | ❌ Blocked | 🟡 |
| 9 | Phantom Warehousing | ⚠️ Exists (`InventoryTheft`) | ❌ New file | ❌ Add 3 variants | ⚠️ Need `StorageLocation` model | ❌ Add | ❌ Need `include_storage_locations` | ❌ Blocked | 🔴 |
| 10 | IC Wash Trades | ❌ Add | ❌ New file | ❌ Add 2 variants | ✅ IC ready | ❌ Add | ⚠️ Need `is_intercompany` on edges | ❌ Blocked | 🔴 |

**Legend:** 🟢 Ready | 🟡 Partially ready (minor changes) | 🔴 Significant work needed

#### Cross-Cutting Blockers (affect all schemes)

| Blocker | Severity | Description |
|---------|----------|-------------|
| **SchemeAction → JE materialization** | 🔴 Critical | No component converts scheme actions into journal entries. All 10 schemes are blocked. |
| **`include_users` graph config** | 🟡 High | User nodes not supported in `TransactionGraphBuilder`. Affects schemes 2, 5, 8. |
| **`document_type` edge property** | 🟡 High | ERA attention grouping requires explicit document type on edges. Affects schemes 1, 6, 7, 10. |
| **`pathology_name`/`pathology_category` labels** | 🟡 Medium | Paper taxonomy alignment missing from `MultiStageAnomalyLabel`. Affects all 10 schemes. |
| **`SchemeContext` multi-entity** | 🟡 Medium | `SchemeContext` only has one `company_code`. Affects schemes 7 and 10. |

#### Recommended Implementation Order

1. **SchemeActionMaterializer** (§10.11) — unblocks all schemes.
2. **Labeling extensions** (§10.14) — `pathology_name`, `pathology_category`, `topological_signature`.
3. **SchemeContext multi-entity** — add `available_companies` field.
4. **Graph builder extensions** (§10.12) — `include_users`, `include_cost_centers`, `include_storage_locations`, `document_type`, `source_type`, `is_intercompany` on edges.
5. **Smurfing** (§10.4) — simplest new scheme, reuses existing threshold infrastructure.
6. **Shadow Payroll** (§10.2) — reuses existing payroll infrastructure, needs `Employee.bank_account_id`.
7. **Triad Bypass** (§10.1) — reuses existing P2P document flow infrastructure.
8. **Expense Laundering** (§10.3) — needs `Vendor.is_verified` and shell vendor generation.
9. **Circular Funding** (§10.7) — needs multi-company context and IC generator integration.
10. **IC Wash Trades** (§10.10) — similar to Circular Funding but bilateral.
11. **Phantom Warehousing** (§10.9) — needs `StorageLocation` model and inventory generator changes.
12. **Revenue Manipulation enhancement** (§10.6) — delivery disconnect for existing scheme.
13. **Vendor Kickback enhancement** (§10.8) — price/volume features for existing scheme.
14. **Gradual Embezzlement enhancement** (§10.5) — label taxonomy alignment only.

---

### 10.16 New Files Summary

| File | Crate | Purpose |
|------|-------|---------|
| `anomaly/schemes/triad_bypass.rs` | datasynth-generators | TriadBypassScheme |
| `anomaly/schemes/shadow_payroll.rs` | datasynth-generators | ShadowPayrollScheme |
| `anomaly/schemes/expense_laundering.rs` | datasynth-generators | ExpenseLaunderingScheme |
| (removed)                     |                       |                |
| `anomaly/schemes/circular_funding.rs` | datasynth-generators | CircularFundingScheme |
| `anomaly/schemes/phantom_warehousing.rs` | datasynth-generators | PhantomWarehousingScheme |
| `anomaly/schemes/intercompany_wash_trade.rs` | datasynth-generators | IntercompanyWashTradeScheme |
| `anomaly/scheme_materializer.rs` | datasynth-generators | SchemeActionMaterializer |
| (StorageLocation model additions) | datasynth-core | StorageLocation, StorageLocationType |

### 10.17 Modified Files Summary

| File | Changes |
|------|---------|
| `crates/datasynth-core/src/models/anomaly.rs` | Add `SchemeType` variants: `TriadBypass`, `Smurfing`, `ExpenseLaundering`, `PhantomWarehousing`, `IntercompanyWashTrade`. Add `FraudType` variants: `CircularFunding`, `IntercompanyWashTrade`, `TriadBypass`. |
| `crates/datasynth-core/src/models/user.rs` | Add `bank_account_id`, `address_region`, `login_region` to `Employee` and `User`. |
| `crates/datasynth-core/src/models/master_data.rs` | Add `is_verified`, `creation_date`, `created_by_user_id`, `is_related_party` to `Vendor`. |
| `crates/datasynth-core/src/models/manufacturing_models.rs` | Add `StorageLocation` struct and `StorageLocationType` enum. |
| `crates/datasynth-generators/src/anomaly/schemes/scheme.rs` | Add ~12 new `SchemeActionType` variants. Extend `SchemeAction` with optional fields for ghost IDs, threshold targets, material IDs, etc. Extend `SchemeContext` with `available_companies`, `available_ic_relationships`. |
| `crates/datasynth-generators/src/anomaly/schemes/mod.rs` | Add `pub mod` for 7 new scheme files. Re-export all new scheme structs. |
| `crates/datasynth-generators/src/anomaly/scheme_advancer.rs` | Add 7 probability fields to `SchemeAdvancerConfig`. Refactor `maybe_start_scheme()` to weighted sampling across all 10 types. Extend `MultiStageAnomalyLabel` with `pathology_name`, `pathology_category`, `topological_signature`, `affected_entity_ids`, `monetary_impact`. |
| `crates/datasynth-generators/src/anomaly/injector.rs` | Integrate `SchemeActionMaterializer`: after `advance_schemes()`, call materializer and append resulting JEs to output. |
| `crates/datasynth-generators/src/anomaly/mod.rs` | Add `pub mod scheme_materializer;`. |
| `crates/datasynth-generators/src/anomaly/document_flow_anomalies.rs` | Add `ReusedInvoiceId` to `DocumentFlowAnomalyType`. |
| `crates/datasynth-generators/src/master_data/vendor_generator.rs` | Populate new `Vendor` fields. Add `generate_shell_vendor()` method. |
| `crates/datasynth-generators/src/master_data/employee_generator.rs` | Populate `bank_account_id`, `address_region`, `login_region` for employees. |
| `crates/datasynth-generators/src/manufacturing/inventory_movement_generator.rs` | Replace hardcoded `STORAGE_LOCATIONS` with configurable pool. Add `generate_transfer()` method. |
| `crates/datasynth-config/src/schema.rs` | Extend `MultiStageSchemeConfig` with 7 new scheme configs. Add new config structs for each scheme. |
| `crates/datasynth-graph/src/builders/transaction_graph.rs` | Add `include_users`, `include_cost_centers`, `include_storage_locations` to `TransactionGraphConfig`. Implement user/cost-center/location node creation and edge wiring. |
| `crates/datasynth-graph/src/models/edges.rs` | Add `document_type`, `source_type`, `is_intercompany`, `scheme_id`, `pathology_name`, `pathology_category` to `TransactionEdge`. |
| `crates/datasynth-graph/src/models/nodes.rs` | Extend `UserNode` features with `bank_account_id`, `address_region`, `login_region`. |
| `crates/datasynth-graph/src/exporters/pytorch_geometric.rs` | Extend `metadata.json` with `node_types`, `edge_types` ranges and `pathology_labels` mapping. |
| `crates/datasynth-runtime/src/enhanced_orchestrator.rs` | Wire `SchemeActionMaterializer` into anomaly injection phase. Pass `available_companies` to `SchemeContext`. Enable graph config flags for heterogeneous graph export. |

