# Output Viewer Tool — Comprehensive Functional & Technical Specification

**Status:** Planned (greenfield implementation)  
**Created:** 2026-02-27  
**Scope:** Fully specify a “Datasynth Output Viewer” that can be reimplemented from scratch to inspect tabular outputs, fingerprints, fraud pathologies, and graph exports (for RIP-GNN) in a single UX.

---

## 1. Objectives

- **Single pane of glass:** Provide auditors, data scientists, and developers with one UI to explore:
  - Generated **tables** (journal entries, subledgers, master data).
  - **Fingerprint** summaries (`.dsf` / JSON).
  - **Fraud/anomaly** labels and multi-stage schemes.
  - **Graph exports** (PyG, Neo4j, RustGraph, RIP-GNN evidence subgraphs).
- **Cross-linking:** Click from:
  - A suspicious JE → its scheme label → its position in the transaction graph → its evidence subgraph.
- **Model-agnostic front-end:** No training/inference code in the viewer:
  - Consumes static exports (CSV, JSON, `.npy`, `metadata.json`).
  - Optionally talks to `datasynth-server` / Python services for convenience APIs.
- **Scalable UX:** Can handle runs with:
  - Millions of rows per table (via pagination/virtualization).
  - Millions of graph edges (viewed through summaries and small evidence subgraphs).

---

## 2. User Stories

### 2.1 Auditor / Domain Expert

- Inspect **journal entries**:
  - Filter by date range, company, account class, amount band, `is_fraud`, `fraud_type`.
  - Drill into **multi-stage schemes** (e.g. all entries for a given `scheme_id`).
- For a flagged entry:
  - See **evidence subgraph** (3-hop) with nodes (accounts, vendors, users, companies) and edges (transactions, approvals, intercompany).
  - View plain-language explanation (e.g. “Circular funding loop between entities A, B, C in 2025-Q3”).

### 2.2 Data Scientist / ML Engineer

- Load a **Pathology Lab** run:
  - See dataset metadata: number of nodes/edges, pathologies per type, class balance.
  - Browse `metadata.json` from `PyGExporter`.
- Visualize:
  - **Degree distributions**, path-length histograms, SCC summaries.
  - **Anomaly distributions** by scheme type, pathology category, and graph centrality.

### 2.3 Developer / Maintainer

- Quickly validate that:
  - Pathology injection produced expected counts per type.
  - Graph exports and labels line up (e.g. no pathologies without graph edges).
  - Fingerprint-derived distributions (per-account-class) match JE populations and graph edge features.

---

## 3. Data Model & Run Layout

The viewer is driven by a **generation run directory** with a well-defined layout.

### 3.1 Run Directory Layout (Static Mode)

Assumed structure (can be parameterized, but this is the default contract):

```text
run_root/
  manifest.json
  fingerprint.json
  tables/
    journal_entries.csv
    acdoca.csv
    ar_subledger.csv
    ap_subledger.csv
    vendors.csv
    customers.csv
    materials.csv
    employees.csv
  schemes/
    multi_stage_labels.csv        # From MultiStageAnomalyLabel
    scheme_statistics.json        # From SchemeStatistics
  graphs/
    transaction/
      metadata.json
      edge_index.npy
      node_features.npy
      edge_features.npy
      node_labels.npy
      edge_labels.npy
    # optional:
    approval/
      metadata.json
      ...
  rip_gnn/
    evidence_subgraphs.jsonl      # One JSON per line
    metrics.json                  # Model-level metrics (AUC, Recall@Top1%, etc.)
```

Behavior:

- If any subdirectory/file is absent, the corresponding navigation entry is disabled:
  - No `fingerprint.json` → fingerprint dashboard tab hidden.
  - No `graphs/` → graph tabs hidden.
  - No `rip_gnn/` → evidence subgraph view hidden.

### 3.2 Table Schemas (Minimum Assumptions)

The viewer acts on generic CSVs but expects **core columns** for richer functionality.

#### 3.2.1 `journal_entries.csv`

Minimum header-level columns:

- `document_id` (string / UUID, unique per JE).
- `company_code` (string).
- `posting_date` (date).
- `document_date` (date).
- `source` (`Manual`, `Automated`, `Recurring`, `Adjustment`).
- `business_process` (`P2P`, `O2C`, `R2R`, `H2R`, `A2R`, …).
- `created_by` (user id).
- `user_persona` (string).
- `header_text` (string).
- `reference` (string; external reference / document number).
- `sox_relevant` (bool).
- Fraud / anomaly fields:
  - `is_fraud` (bool).
  - `fraud_type` (string).
  - `is_anomaly` (bool).
  - `anomaly_type` (string).
  - `scheme_id` (string / UUID).
  - `anomaly_id` (string).

Line-level representation (two options):

- **Flattened** in `journal_entries.csv`:
  - One row per line:
    - Additional columns: `line_number`, `gl_account`, `debit_amount`, `credit_amount`, `cost_center`, `line_text`.
- **Normalized**:
  - `journal_entries.csv` (header-only, one row per JE).
  - `journal_entry_lines.csv` (separate table) with:
    - `document_id`, `line_number`, `gl_account`, `debit_amount`, `credit_amount`, `cost_center`, `line_text`.

The viewer should be configurable as to which layout is used, via a small per-run or global config.

#### 3.2.2 Subledgers & ACDOCA

For `acdoca.csv` and AR/AP subledgers, minimum fields:

- `document_id`, `company_code`, `posting_date`, `gl_account`, `amount`, `debit_credit`, `cost_center`.
- Subledger keys:
  - AR: `customer_id`.
  - AP: `vendor_id`.
  - Inventory: `material_id`, `location_id` (if present).

#### 3.2.3 Master Data Tables

Suggested columns:

- `vendors.csv`:
  - `vendor_id`, `name`, `country`, `is_verified`, `cluster`, `creation_date`.
- `customers.csv`:
  - `customer_id`, `name`, `segment`, `country`, `lifecycle_stage`.
- `materials.csv`:
  - `material_id`, `name`, `standard_cost`, `list_price`, `material_type`.
- `employees.csv`:
  - `employee_id`, `name`, `department`, `role`, `bank_account_id`, `address_region`, `login_region`.

### 3.3 Fingerprint JSON

The viewer primarily consumes:

- `schema`:
  - Table names, columns, inferred types, cardinalities.
- `statistics`:
  - Numeric distributions (min, max, mean, std, percentiles).
  - Categorical stats.
- `statistics.amount_by_account_class`:
  - Per-class (e.g. 601, 701) distribution summaries.
- `privacy`:
  - Epsilon budget, noise query counts.
- `privacy_audit`:
  - Log of privacy operations.

### 3.4 Schemes & Pathologies

#### 3.4.1 `multi_stage_labels.csv`

Columns (derived from `MultiStageAnomalyLabel` plus extensions):

- `anomaly_id` (string).
- `scheme_id` (string / UUID).
- `scheme_type` (string enum).
- `stage_number` (u32).
- `stage_name` (string).
- `total_stages` (u32).
- `perpetrator_id` (string).
- `scheme_detected` (bool).
- Optional:
  - `pathology_name` (string).
  - `pathology_category` (`Sequential`, `Volume`, `Relational`).

#### 3.4.2 `scheme_statistics.json`

JSON object mapping to `SchemeStatistics`:

- `total_schemes`, `active_schemes`, `completed_schemes`, `detected_schemes`.
- `total_impact`.
- Per-type counts (`embezzlement_count`, `revenue_manipulation_count`, `kickback_count`, plus any new types).

### 3.5 Graph Metadata & Evidence Subgraphs

#### 3.5.1 `graphs/transaction/metadata.json`

Mirrors `PyGMetadata`:

- `name`, `num_nodes`, `num_edges`.
- `node_feature_dim`, `edge_feature_dim`.
- `num_node_classes`, `num_edge_classes`.
- `node_types`: `{ "Account": 0, "JournalEntry": 1, ... }`.
- `edge_types`: `{ "Transaction": 0, "Approval": 1, ... }`.
- `statistics`:
  - `density`, `anomalous_node_ratio`, `anomalous_edge_ratio`, plus arbitrary numeric stats.

#### 3.5.2 `rip_gnn/evidence_subgraphs.jsonl`

Each line is a JSON object representing one evidence subgraph:

```json
{
  "run_id": "2026-02-27-pathology-lab-001",
  "graph_type": "transaction",
  "focal_type": "edge",
  "focal_id": 12345,
  "focal_external_id": "JE-0001",
  "severity_score": 0.97,
  "pathology_name": "Smurfing",
  "pathology_category": "Volume",
  "nodes": [
    {
      "id": 1,
      "external_id": "1000",
      "label": "1000 - Cash",
      "type": "Account",
      "properties": { "company_code": "1000", "country": "FR" },
      "is_anomaly": false
    }
  ],
  "edges": [
    {
      "id": 12345,
      "source": 1,
      "target": 2,
      "type": "Transaction",
      "weight": 10500.0,
      "timestamp": "2025-03-30",
      "is_anomaly": true,
      "anomaly_type": "Smurfing",
      "properties": {
        "document_number": "JE-0001",
        "document_type": "PAYMENT",
        "business_process": "P2P"
      }
    }
  ],
  "explanation": "High-density cluster of sub-threshold payments between the same account pair near quarter-end.",
  "generated_at": "2026-02-27T10:15:00Z"
}
```

The viewer must handle missing optional fields gracefully.

---

## 4. Frontend Architecture

### 4.1 Stack (Recommended)

- **Language:** TypeScript.
- **Framework:** React (or equivalent component-based SPA).
- **Build:** Vite or similar.
- **Routing:** `react-router` or equivalent.
- **UI:** Lightweight component library (Chakra/Mantine) or TailwindCSS + headless components.

### 4.2 Layout

- **Left sidebar:**
  - Run selector.
  - Navigation tree:
    - Tables → Journal Entries / Subledgers / Master Data.
    - Fingerprint.
    - Pathologies.
    - Graphs.
    - RIP-GNN.
- **Top bar:**
  - Current run name and stats (rows, period, companies).
  - Global filters (company, date range).
- **Main content:**
  - Route-specific view (table, fingerprint, pathology, graph, evidence).

### 4.3 Routes

- `/runs`:
  - List/selector of available runs.
- `/runs/:runId/tables/:tableName`:
  - Tabular explorer for the given table.
- `/runs/:runId/fingerprint`:
  - Fingerprint dashboard.
- `/runs/:runId/pathologies`:
  - Pathology overview.
- `/runs/:runId/pathologies/:schemeId`:
  - Scheme detail page.
- `/runs/:runId/graph/transaction`:
  - Transaction graph summary.
- `/runs/:runId/rip_gnn/evidence/:focalId`:
  - Evidence subgraph viewer.

---

## 5. Views & Components

### 5.1 Run Selector (`RunSelector`)

- Responsibilities:
  - Discover run directories (local chooser or API).
  - For each run, detect presence of:
    - `fingerprint.json`, `tables/`, `schemes/`, `graphs/`, `rip_gnn/`.
  - Expose a list of `RunInfo` with flags for available features.

### 5.2 Tabular Explorer (`TableView`)

- Input props:
  - `runId`, `tableName`, `schema`, a `dataSource` abstraction.
- Capabilities:
  - Virtualized rendering (for large tables).
  - Column-level:
    - Reordering, show/hide, sorting.
  - Filtering:
    - Text search (contains / starts with).
    - Type-aware filters:
      - Range filters for numeric/date.
      - Multi-select for categorical.
    - Saved filter presets per table.
  - Row selection:
    - Single/multi-row selection.
    - Row click opens detail panel (e.g. `JournalEntryDetail`).

### 5.3 Journal Entry Detail (`JournalEntryDetail`)

- Inputs:
  - `runId`, `document_id`.
- UI sections:
  - **Header:**
    - Posting/document dates, company, source, business process, user, header text, reference, sox_relevant.
  - **Lines:**
    - Table of lines with GL account, debit/credit, cost center, line_text.
  - **Fraud/anomaly:**
    - Flags and types, scheme link (`scheme_id`), `anomaly_id`.
  - **Related entities:**
    - Linked vendor/customer/material/employee records (lookups into master data).
  - **Navigation:**
    - Buttons/links:
      - “View Scheme” → `/pathologies/:schemeId`.
      - “View Evidence Subgraph” → `/rip_gnn/evidence/:focalId` (when mapping exists).

### 5.4 Fingerprint Dashboard (`FingerprintView`)

- Inputs:
  - Parsed fingerprint JSON.
- Panels:
  - **Schema Panel:**
    - Table list with columns, types, null ratios, cardinalities.
  - **Numeric Panel:**
    - Select a numeric column:
      - Show histogram/boxplot.
      - If account-class stats available, show per-class overlays.
  - **Benford Panel:**
    - Observed vs theoretical first-digit distribution.
  - **Privacy Panel:**
    - Epsilon, noise query counts, last few audit log entries.

### 5.5 Pathology & Scheme Explorer (`PathologyView`, `SchemeDetailView`)

- `PathologyView`:
  - Aggregates `multi_stage_labels.csv` and `scheme_statistics.json` into:
    - Total schemes per type and pathology category.
    - Time series of active schemes over time.
    - Table of top impactful schemes (by `total_impact`).
- `SchemeDetailView`:
  - For `scheme_id`:
    - Show scheme metadata:
      - `scheme_type`, optional `pathology_name` and `pathology_category`.
      - `stages_completed`, `final_status`, `detection_status`, `total_impact`.
    - Show anomalies:
      - Table of `anomaly_id`, stages, detection flag.
    - Show related JEs:
      - Table with `document_id`, `posting_date`, accounts, amounts, `is_fraud`/`is_anomaly`.
    - Link to evidence subgraphs tagged with the same `scheme_id` (if present).

### 5.6 Graph Summary & Evidence (`GraphView`, `EvidenceSubgraphView`)

- `GraphView`:
  - Uses `graphs/transaction/metadata.json`.
  - Shows:
    - Node/edge counts.
    - Density, anomaly ratios.
    - Node type and edge type breakdowns.
  - If additional stats JSON is available:
    - Degree histograms.
    - SCC summary metrics.

- `EvidenceSubgraphView`:
  - Inputs:
    - Parsed evidence JSON for a single subgraph.
  - Renders:
    - Force-directed or layered layout.
    - Nodes colored by `type`, highlighted if `is_anomaly`.
    - Edges colored by `type`, with thickness or color intensity by `weight` or importance.
  - Side panel:
    - Focal node/edge information.
    - Explanation text.
    - Raw properties (document numbers, amounts, dates).
  - Interactions:
    - Hover tooltips.
    - Node click → open JE/vendor/customer/employer detail if resolvable.

---

## 6. Data Access Abstraction

Even in static mode, define a `DataSource` abstraction so UI is backend-agnostic.

### 6.1 Conceptual Interface

```ts
interface RunInfo {
  id: string;
  name: string;
  path: string;
  hasFingerprint: boolean;
  hasSchemes: boolean;
  hasGraphs: boolean;
  hasRipGnn: boolean;
}

interface ColumnSchema {
  name: string;
  type: "string" | "number" | "boolean" | "date" | "datetime";
}

interface TableFilters {
  [columnName: string]: {
    type: "string" | "number" | "boolean" | "date";
    op: "eq" | "neq" | "contains" | "range" | "in";
    value: any;
  };
}

interface TablePage<T> {
  rows: T[];
  totalRows: number;
}

interface DataSource {
  listRuns(): Promise<RunInfo[]>;
  listTables(runId: string): Promise<string[]>;
  getTableSchema(runId: string, tableName: string): Promise<ColumnSchema[]>;
  getTablePage(
    runId: string,
    tableName: string,
    page: number,
    pageSize: number,
    filters: TableFilters
  ): Promise<TablePage<Record<string, any>>>;
  getFingerprint(runId: string): Promise<any | null>;
  getSchemeLabels(runId: string): Promise<any | null>;
  getSchemeStats(runId: string): Promise<any | null>;
  getGraphMetadata(runId: string, graphName: string): Promise<any | null>;
  listEvidenceSubgraphs(runId: string): Promise<any[]>;
}
```

Concrete implementations:

- `LocalFileDataSource`:
  - Runs in a Node/Electron/Tauri context, reading from the filesystem.
- `HttpDataSource`:
  - Calls REST endpoints on `datasynth-server` / `viewer-api`.

---

## 7. Non-Functional Requirements

- **Performance:**
  - Support tables with **10M+ rows** via paging/virtualization.
  - Evidence subgraphs up to a few hundred nodes/edges must render in <200ms in modern browsers.
- **Robustness:**
  - Gracefully handle missing files/fields (disable features instead of crashing).
  - Input CSVs may have extra columns; viewer should ignore unknowns by default.
- **Security:**
  - Read-only: no writes back to configs, fingerprints, or output files.
  - In server mode, actual auth/authorization is delegated to the API layer (out of scope here).
- **Extensibility:**
  - New tables/graphs/pathology types should be pluggable via configuration (e.g. mapping table names to “roles”).

---

## 8. Implementation Phases

### Phase 1 — Static Local Viewer

- Build:
  - `RunSelector` (directory-based).
  - `TableView` for `journal_entries.csv`.
  - Minimal `JournalEntryDetail`.
  - Minimal `FingerprintView` (schema + global numeric stats).

### Phase 2 — Fingerprint & Pathology Dashboards

- Add:
  - Full `FingerprintView` (per-account-class, Benford, privacy).
  - `PathologyView` & `SchemeDetailView` with JE linkbacks.

### Phase 3 — Graph Summary & Evidence

- Add:
  - `GraphView` using transaction graph metadata.
  - `EvidenceSubgraphView` with JSONL ingestion.
  - Cross-links:
    - JE → evidence (via `anomaly_id` / `focal_id`).
    - Scheme → representative evidence subgraphs.

### Phase 4 — Optional Server Integration

- Implement `HttpDataSource` and toggle between local/static vs server-backed modes based on configuration.

This specification is intended to be implementation-ready: a new viewer can be built from zero and still integrate cleanly with the Datasynth pipelines and RIP-GNN outputs.

# Output Viewer Tool — Roadmap and Architecture

**Status:** Planned (incremental evolution from current viewer)  
**Created:** 2026-02-27  
**Scope:** Define the feature set and architecture for a “Datasynth Output Viewer” that can inspect tabular outputs, fingerprints, fraud pathologies, and graph exports (for RIP-GNN) in a single UX.

---

## 1. Objectives

- **Single pane of glass:** Provide auditors, data scientists, and developers with one UI to explore:
  - Generated **tables** (journal entries, subledgers, master data).
  - **Fingerprint** summaries (`.dsf` / JSON).
  - **Fraud/anomaly** labels and multi-stage schemes.
  - **Graph exports** (PyG, Neo4j, RustGraph).
- **Cross-linking:** Click from:
  - A suspicious JE → its scheme label → its position in the transaction graph → its evidence subgraph.
- **Model-agnostic:** Frontend should not embed model logic; it consumes:
  - Static exports (CSV, JSON, `.npy`, metadata.json).
  - Optional APIs from `datasynth-server` or a thin Python service.

---

## 2. User Stories

### 2.1 Auditor / Domain Expert

- Inspect **journal entries**:
  - Filter by date, company, account class, amount band, `is_fraud`, `fraud_type`.
  - Drill into **multi-stage schemes** (e.g. all entries for a given `scheme_id`).
- For a flagged entry:
  - See **evidence subgraph** (3-hop) with nodes (accounts, vendors, users) and edges (transactions, approvals).
  - View plain-language explanation (e.g. “Circular funding loop between entities A, B, C in 2025-Q3”).

### 2.2 Data Scientist / ML Engineer

- Load a **Pathology Lab** run:
  - See dataset metadata: number of nodes/edges, pathologies per type, class balance.
  - Browse `metadata.json` from `PyGExporter`.
- Visualize:
  - **Degree distributions**, path-length histograms, SCC summaries.
  - **Anomaly distributions** by scheme type, pathology category, and graph centrality.

### 2.3 Developer / Maintainer

- Quickly validate that:
  - Pathology injection produced expected counts per type.
  - Graph exports and labels line up (e.g. no pathologies without graph edges).
  - Fingerprint-derived distributions (per-account-class) match JE populations.

---

## 3. High-Level Architecture

### 3.1 Frontend

- **Tech stack:** Reuse / extend existing `datasynth-output-viewer`:
  - React + TypeScript + Vite (currently present).
  - Component library: lightweight (e.g. Chakra UI or Tailwind) or custom.
- **Key modules:**
  - `DataTableView`: virtualized grid for large CSVs (journal_entries, subledgers, master data).
  - `FingerprintView`: renders fingerprint JSON (schema, stats, `amount_by_account_class`, privacy audit) with charts.
  - `PathologyView`: aggregated views of schemes, labels, counts, timelines.
  - `GraphView`:
    - Basic graph visualization (D3, Cytoscape, or similar).
    - Evidence subgraph display.
  - `ModelRunSelector`:
    - Selects an output directory (local path or server-provided run).

### 3.2 Backend / Data Sources

Two deployment modes:

1. **Local / static:**
   - Viewer reads:
     - CSV / Parquet via browser file APIs (or server-side helper).
     - Fingerprint JSON (`*.json` companion to `.dsf`).
     - Graph export artifacts:
       - `metadata.json`, `.npy` files (loaded via small Python/Node helper, not directly in browser).
   - For local development, a small Node/Express (or Rust) file server can provide:
     - JSON endpoints for summarized graph / scheme info.

2. **Server-integrated:**
   - Extend `datasynth-server` (or a separate microservice) with endpoints:
     - `/api/runs` → list of completed generation runs with metadata.
     - `/api/run/{id}/tables` → paginated table data.
     - `/api/run/{id}/fingerprint` → JSON fingerprint.
     - `/api/run/{id}/schemes` → scheme summaries and labels.
     - `/api/run/{id}/graph/summary` → node/edge counts, degree dist, SCC stats.
     - `/api/run/{id}/graph/evidence?edge_id=...` → 3-hop evidence subgraph (precomputed or on-demand).

**Recommendation:** Start with **static/local** mode (no backend changes), then optionally add a server mode for integrated deployments.

---

## 4. Feature Breakdown

### 4.1 Tabular Explorer

- **Sources:**
  - `journal_entries.csv`
  - `acdoca.csv` or equivalent transaction export.
  - Subledgers (AR/AP/Inventory/Payroll).
  - Master data (vendors, customers, materials, employees).
- **Capabilities:**
  - Column-based filtering and sorting.
  - Saved filter presets (e.g. “High-value, quarter-end entries”, “Fraud-labelled only”).
  - Basic aggregations:
    - Sums by account class, company, business process.

### 4.2 Fingerprint Dashboard

- Input: fingerprint JSON (companion to `.dsf`).
- Panels:
  - Schema overview:
    - Tables, columns, inferred types, cardinalities.
  - Numeric stats:
    - Global vs per-account-class distributions.
    - Benford deviation charts.
  - Privacy:
    - Epsilon used, noise query count, audit log entries.
  - For FEC:
    - PCG-style view: classes 1–7 with cost/revenue breakdown.

### 4.3 Pathology & Scheme Explorer

- Input:
  - `MultiStageAnomalyLabel` exports (either CSV or JSON).
  - Scheme statistics (`SchemeStatistics`).
- Views:
  - **By pathology:**
    - Bar chart: counts per pathology type (Sequential/Volume/Relational).
    - Time series: scheme start/end dates by type.
  - **By scheme:**
    - Detail view for a `scheme_id`:
      - Type, stages completed, total impact, detection status.
      - Linked anomaly IDs and affected JEs.

### 4.4 Graph Overview & Evidence Subgraphs

- Inputs:
  - `datasynth-graph` exports:
    - `metadata.json`
    - (Optional) precomputed JSON for:
      - Node degree histograms.
      - SCC summaries.
      - Node/edge anomaly distributions.
  - Evidence subgraphs produced by the RIP-GNN pipeline:
    - Each as a small JSON describing:
      - Nodes (id, type, label, properties).
      - Edges (source, target, type, weight, timestamp, anomaly flag).
      - Focal node/edge and explanation text.

- Views:
  - **Graph summary:**
    - Node/edge counts.
    - Density, anomalous node/edge ratios.
    - Distribution of node degrees.
  - **Evidence subgraph viewer:**
    - Force-directed or layered layout.
    - Node color by type (Account, Vendor, User, Company, CostCenter, JournalEntry).
    - Edge color by edge type (Transaction, Approval, Intercompany, DocumentReference).
    - Side panel showing explanation text and raw properties.

---

## 5. Implementation Phases

### Phase 1 — Minimal Viable Viewer Upgrade

- **Goal:** Turn current `datasynth-output-viewer` into a reliable tabular explorer.
- Tasks:
  - Standardize **directory layout** assumptions (e.g. `output/journal_entries.csv`, `output/fingerprint.json`).
  - Implement high-performance `DataTable` with:
    - Server-side or client-side pagination.
    - Basic filtering (account, date, company, amount band, `is_fraud`).
  - Add simple **fingerprint summary**:
    - Load `fingerprint.json` from a selected run.
    - Show tables, column types, and basic numeric stats.

### Phase 2 — Fingerprint & Pathology Dashboards

- Add dedicated pages:
  - `/fingerprint`:
    - Charts for per-account-class distributions, Benford deviations.
  - `/pathologies`:
    - Aggregations over `MultiStageAnomalyLabel` and `SchemeStatistics`.
    - Drill-down into a single scheme with links to affected JEs.

### Phase 3 — Graph & Evidence Integration

- Implement minimal graph support:
  - Load `metadata.json` from graph exports.
  - Show overall graph statistics and anomaly ratios.
- Add **evidence subgraph viewer**:
  - Accepts precomputed evidence subgraph JSON (produced by RIP-GNN Python pipeline).
  - Renders 3-hop neighborhood around focal node/edge.
- Add **cross-linking**:
  - From JE row → open evidence subgraph (when available).
  - From scheme detail → open representative evidence subgraph.

### Phase 4 — Server Integration (Optional)

- Extend `datasynth-server` or a new `viewer-api` service:
  - API endpoints for:
    - Listing runs.
    - Streaming table data.
    - Serving fingerprint JSON and graph summaries.
    - Serving evidence subgraphs dynamically (optional).
- Adapt frontend to support:
  - Local-file mode **or** server mode via a simple switch.

---

## 6. Non-Goals and Constraints

- The viewer is **read-only**; it does not modify fingerprints or configs.
- No heavy GPU-based graph rendering in-browser; keep graph visualization scoped to:
  - Small evidence subgraphs.
  - Aggregate statistics for the full graph.
- Model training and evaluation (e.g. training RIP-GNN) remain in:
  - Separate Python notebooks / scripts, not in the viewer.

