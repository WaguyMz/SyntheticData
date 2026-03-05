# RIP-GNN – Unsupervised Journal Entry Representation for Pathology Detection

**Status:** Planned  
**Created:** 2026-03-05  
**Scope:** Design a graph-based ML export and modeling plan for RIP-GNN – a Conditional Temporal Graph Network that learns unsupervised representations of journal entries and flags structural, temporal, and relational “pathologies” in EY-ASU SyntheticData.

---

## 1. Problem Framing and Objectives

### 1.1 Motivation

Journal entries live at the intersection of:

- **Double-entry structure** (debits = credits, GL classes).
- **Business processes** (P2P, O2C, payroll, treasury, etc.).
- **Relational context** (vendors, customers, employees, bank accounts).
- **Temporal behavior** (period-end spikes, lags, regime changes).

Many fraud and error patterns are fundamentally **graph phenomena**:

- Shadow payroll: ghost employees sharing **bank accounts** with perpetrators.
- Triad bypass: payments reusing an existing **invoice ID** without a new invoice.
- Expense laundering: flows through **suspense** and **shell vendors**.

The goal of RIP-GNN is to:

- Learn a **latent space of “normal” accounting activity** over a **heterogeneous temporal graph**.
- Score JEs or subgraphs for **pathological deviation** without using labels during training.
- Provide a flexible export that supports multiple GNN architectures (TGN, TGAT, GraphSAGE + time).

### 1.2 Core Objectives

1. Define a **graph schema** (nodes, edges, attributes) derived from SyntheticData outputs.
2. Implement a **PyTorch Geometric export pipeline** suitable for temporal GNNs.
3. Specify **self-supervised objectives** (link prediction, masked-context prediction, contrastive learning).
4. Design **evaluation metrics** using anomaly labels as ground truth, without leaking them into training.
5. Document **practical constraints** for large graphs (sampling, batching, hardware).

---

## 2. Graph Data Format from the Generator

### 2.1 Node Types and Attributes

We build a heterogeneous graph with at least the following node types:

- **Journal Entry (`je`)**
  - `id`: `document_id` (UUID).
  - Attributes:
    - `posting_date`, `document_date`, `fiscal_period`, `company_code`, `currency`.
    - Aggregated structure:
      - GL histogram (e.g., counts per GL class 1XXX/2XXX/…/7XXX).
      - Total debit/credit, number of lines.
      - Flags: `is_fraud` (for evaluation only), `sod_violation`, `has_suspense_posting`.
    - Optional embeddings:
      - One-hot for `document_type` (JE, AP invoice, payment, etc.).

- **Account (`account`)**
  - `id`: `gl_account`.
  - Attributes:
    - PCG class (e.g., 60/61/62/64/65/70/71).
    - Account type (asset/liability/equity/revenue/expense).
    - Industry-specific tags (e.g., payroll, tax, suspense).

- **Counterparty / Entity nodes**
  - **Vendor (`vendor`)**: `vendor_id`, `country`, `is_fraud_actor`, `auxiliary_gl_account`.
  - **Customer (`customer`)**: `customer_id`, `country`, `credit_rating`, `is_fraud_actor`.
  - **Employee (`employee`)**: `employee_id`, `user_id`, `company_code`, `department_id`, `is_fraud_actor`.
  - **BankAccount (`bank_account`)**:
    - Synthetic ID (e.g., IBAN), `bank_country`, type (payroll/vendor/customer).
    - Note: for shadow payroll, **ghost employees** share the same bank account as the perpetrator.

- Optional:
  - **Company (`company`)**: company code, country, industry.
  - **Process (`doc`)**: P2P/O2C document nodes (PO, GR, Invoice, Payment).

### 2.2 Edge Types and Attributes

Edges capture structural, relational, and temporal links:

- **JE–Account**: `je` → `account`
  - Type: `posts_to`.
  - Attributes:
    - `amount_signed` (debit positive, credit negative).
    - Line-level context (cost center, project code, segment).

- **JE–Counterparty**
  - `je` → `vendor`: `involves_vendor` (from AP lines).
  - `je` → `customer`: `involves_customer` (from AR lines).
  - `je` → `employee`: `involves_employee` (e.g. payroll).
  - `je` → `bank_account`: `paid_from_bank` / `paid_to_bank`.

- **Temporal edges**
  - `je_t` → `je_{t+1}` within:
    - Same `company_code`.
    - Optional per process chain (PO→GR→Invoice→Payment).
  - Attributes: `delta_days`, flags for period-end proximity.

- **Process edges (optional but recommended)**
  - From `document_flows/*`:
    - `po` → `gr` → `vendor_invoice` → `payment`.
  - These edges let the model distinguish **process-conformant** vs **bypassing** paths.

### 2.3 Export Container

Implementation target:

- Extend `datasynth-graph` and `PyGExporter` to produce:
  - `graph.pt` containing:
    - Node feature tensors per type (`x_je`, `x_account`, `x_vendor`, …).
    - Edge indices and attributes per edge type (`edge_index_posts_to`, `edge_attr_posts_to`, …).
    - Timestamp tensors (`t_je`, `t_edge`) suitable for temporal GNNs.
  - `metadata.json`:
    - Node/edge type names.
    - Feature dimensionalities and encodings.
    - Mappings from IDs to indices.
  - Separate **CSV/JSON** versions for debugging:
    - `je_nodes.csv`, `account_nodes.csv`, `edges_posts_to.csv`, etc.

Anomaly labels:

- Export `labels/anomaly_labels.jsonl` unchanged.
- Provide an additional `node_labels_je.npy` or CSV mapping `je_index` → `label` for evaluation only.

---

## 3. Modeling Methodology

### 3.1 Graph Construction Pipeline

1. **Ingest FEC / JE data** from `journal_entries.json` / `fec.csv`.
2. Build **JE nodes** with:
   - Structured features (GL histogram, total amount, number of lines).
   - Temporal metadata (posting_date, fiscal_period).
3. Ingest **master data** for vendors/customers/employees/bank accounts.
4. Construct **edges**:
   - `posts_to` from JE lines.
   - `involves_*` from auxiliary GL and partner IDs.
   - Temporal next-edges by sorting per company/time.
   - Process edges using `document_flows/*`.
5. Serialize to PyG format for training.

### 3.2 RIP-GNN Architecture (Baseline)

We target a **temporal heterogeneous GNN** with the following structure:

- **Encoder**:
  - Per node type MLPs / embeddings for categorical fields:
    - GL bucket embeddings for accounts.
    - Country / industry embeddings for entities.
    - Process-type embeddings for JEs.

- **Message passing**:
  - Use a Temporal Graph Network (TGN) or TGAT-style layer:
    - Messages flow along edges with time-stamps.
    - Separate attention weights per edge type (`posts_to`, `involves_vendor`, etc.).
  - Incorporate **double-entry structure** as a soft constraint:
    - Encourage symmetry between debit/credit flows via loss terms or architectural constraints (e.g., paired messages across debit/credit edges).

- **Decoder / objectives** (unsupervised):
  - **Temporal link prediction**:
    - Predict future neighbors or edge existence for a JE given history.
  - **Masked context prediction**:
    - Mask GL class, counterparty type, or time-bucket and predict from neighbors.
  - **Contrastive anomaly objective**:
    - Positive pairs: normal JEs in similar process contexts.
    - Negative pairs: corrupted / shuffled neighbors or pathologically labeled JEs in ablations.

The result is an embedding \(h_{je}(t)\) for each JE node, plus embeddings for accounts and counterparties.

### 3.3 Pathology Scoring

After training:

- **Reconstruction-based score**:
  - For each JE, compute reconstruction error on:
    - Its GL histogram.
    - Contextual features (counterparty type, process stage).
  - Higher reconstruction error ⇒ more “pathological”.

- **Likelihood-based score**:
  - For link prediction objectives, use:
    - Negative log-likelihood of observed neighbors.
  - Flag edges / nodes with low likelihood as anomalies.

- **Aggregation to scheme-level**:
  - Combine JE-level scores for:
    - All JEs in a document chain (PO→GR→Invoice→Payment).
    - All actions in a multi-stage scheme (shadow_payroll, triad_bypass, etc.) using `anomaly_labels`.

Evaluation:

- Join scores with `anomaly_labels` on `document_id` and compute:
  - ROC-AUC / PR-AUC by `anomaly_type` and `scheme_type`.
  - Scheme detection rate (was at least one action in the scheme given a high score?).

---

## 4. Visualization and Tooling

### 4.1 Graph Exploration

- Use **Neo4j** or **Gephi** with a subset of the graph:
  - Visualize:
    - Shadow payroll: employee–bank account–payment triangles linking ghosts and perpetrators.
    - Triad bypass: invoice–payment subgraphs where invoices are reused without new invoices.
  - Filter by high RIP-GNN anomaly score to see “evidence subgraphs”.

### 4.2 Embedding Space Analysis

- Export JE embeddings \(h_{je}\) to `je_embeddings.npy` / CSV.
- Use **UMAP** or **t-SNE** to:
  - Show clusters for normal P2P/O2C/payroll activity.
  - Highlight outliers coloured by scheme_type (for evaluation).

### 4.3 Temporal Behavior

- Plot time-series of average or max anomaly score:
  - Per company.
  - Per account class (e.g., 60/61/62/64/65/70/71).
  - Around known scheme windows from config.

---

## 5. Practical Considerations and Constraints

### 5.1 Scale and Sampling

- Target graph size for initial experiments:
  - ~30k–100k JE nodes.
  - O(10^5–10^6) edges.
- Use:
  - **Neighbor sampling** (GraphSAGE-style) for training.
  - **Subgraph batching** by time window or by process chain.

### 5.2 Label Usage and Leakage

- Training must remain **unsupervised / self-supervised**:
  - Anomaly labels are **not** used during parameter updates.
- Labels are reserved for:
  - Hyperparameter tuning (via validation scores).
  - Final benchmark evaluation and ablations.

### 5.3 Reproducibility

- Fix seeds in both:
  - Data generation (via `global.seed` and scheme configs).
  - GNN training (PyTorch seeds, DataLoader ordering).
- Version control:
  - `pcg_2024.json` and relevant generator configs.
  - RIP-GNN code (model architecture, training loop).

This plan specifies the graph schema, export format, modeling approach, and evaluation metrics needed to build RIP-GNN on top of EY-ASU SyntheticData as an independent contribution.

