# Agentic LLM Forensic Investigation Benchmark – Technical Plan

**Status:** Planned  
**Created:** 2026-03-05  
**Scope:** Design a reproducible benchmark where modern LLMs act as autonomous forensic auditors over EY-ASU SyntheticData FEC / journal entry datasets, using tool-augmented reasoning (SQL, grep, graph traversal) under explicit token budgets.

---

## 1. Problem Framing and Objectives

### 1.1 Motivation

Most prior work on LLMs in audit focuses on **prompted Q&A** or **document summarization**. Real forensic work is different:

- The auditor must **search, hypothesize, refine, and pivot** across large ledgers.
- Evidence is **distributed** across time, accounts, entities, and document flows.
- Many pathologies are **relational** (e.g., ghost employees sharing bank accounts, triad bypass on reused invoices).

The goal of this benchmark is to answer:

- Can state-of-the-art LLMs behave as **agentic forensic auditors** when given tools and a realistic synthetic ledger?
- How does performance scale with **token budgets** and **tooling richness**?
- Do stronger models learn **human-like investigative strategies** (e.g., focusing on period-end, high-risk accounts, shared identifiers)?

### 1.2 High-Level Task

Given a generated dataset (FEC + master data + hidden anomaly labels):

- The model runs as an **agent** with access to:
  - SQL over the JE database.
  - Grep-like text search over raw files (configs, descriptions, labels in “oracle” mode).
  - Optional graph traversals over a transaction / entity graph.
  - Simple plotting tools (e.g., histogram / time-series).
- The agent is given a **global mission**:
  - *“Investigate this ledger for material frauds, control failures, and process anomalies. Produce a report summarizing suspected schemes, supporting evidence, and affected entities.”*
- The agent operates under a **fixed token budget** (e.g., 50M tokens, inclusive of tool outputs and internal reasoning).

Output artefacts:

- A **ranked suspicion list** of `document_id`s / schemes with confidence scores and hypothesized pathology types.
- A **narrative report** explaining the investigation path and key evidence.
- The full **tool-use trace** for qualitative analysis.

---

## 2. Data Export Format from the Generator

### 2.1 Core Tables / Files

We assume a single run of `datasynth-data generate` with a config such as `config_multischemes.yaml`.

- **Journal Entries (FEC / JE)**
  - `journal_entries.csv` (and/or Parquet):
    - Header-level: `document_id` (UUID), `company_code`, `posting_date`, `document_date`, `fiscal_period`, `currency`, `document_type`, `source`, `is_fraud`, `sod_violation`, `header_text`.
    - Line-level: `line_number`, `gl_account`, `auxiliary_account_number`, `debit_amount`, `credit_amount`, `local_amount`, `cost_center`, `segment`, `project_code`, tax fields.
  - `fec.csv`:
    - French FEC export (18+ columns) for realism and downstream regulatory tooling.

- **Master Data**
  - `master_data/vendors.json`:
    - `vendor_id`, `name`, `country`, `currency`, `payment_terms`, `bank_accounts`, `is_fraud_actor`, `auxiliary_gl_account`.
  - `master_data/customers.json`:
    - `customer_id`, `name`, `credit_rating`, `payment_terms`, `bank_accounts`, `is_fraud_actor`, `auxiliary_gl_account`.
  - `master_data/employees.json`:
    - `employee_id`, `user_id`, `display_name`, `department_id`, `company_code`, `hire_date`, `creation_date`, `bank_account`, `is_fraud_actor`.
    - Shadow payroll ghost employees share the **same bank account** as the perpetrator.

- **Anomaly Labels (Oracle, Hidden from Agent)**
  - `labels/anomaly_labels.jsonl`:
    - `anomaly_id`, `anomaly_type` (Fraud/Error/Process/Statistical/Relational), `document_id`, `company_code`, `anomaly_date`, `severity`, `description`.
    - `metadata` JSON: `scheme_type`, `stage`, `perpetrator_id`, `counterparty`, `reused_invoice`, `action_amount`, etc.
  - Used only for **evaluation**, never exposed to the agent during the run.

- **Config & Provenance**
  - The exact generator config (e.g., `config_multischemes.yaml`).
  - `run_manifest.json`, `prov.json`, and `lineage_graph.json` to track which fraud schemes were enabled and how outputs were produced.

### 2.2 Recommended Database Schema

The benchmark expects the JE corpus to be queryable via SQL.

- `je_header(document_id PK, company_code, posting_date, document_date, fiscal_period, document_type, currency, source, is_fraud, sod_violation, header_text, ...)`
- `je_line(document_id FK, line_number, gl_account, auxiliary_account_number, debit_amount, credit_amount, local_amount, cost_center, segment, project_code, tax_code, tax_amount, ...)`
- `anomaly_label(document_id, anomaly_type, scheme_type, stage, metadata JSONB, anomaly_date, severity, ...)`
- `vendor(vendor_id PK, name, country, currency, is_fraud_actor, auxiliary_gl_account, ...)`
- `customer(customer_id PK, name, country, currency, is_fraud_actor, auxiliary_gl_account, ...)`
- `employee(employee_id PK, user_id, display_name, company_code, department_id, hire_date, creation_date, is_fraud_actor, ...)`

Implementation options:

- **PostgreSQL** (full SQL, JSONB, indexing).
- **DuckDB** for local experiments with in-process SQL.

---

## 3. Benchmark Methodology

### 3.1 Agent Environment and Tools

For each LLM model (e.g., Gemini 3 Pro, Claude 4.6 Opus, Qwen 3.5), define a consistent **tool API**:

- `sql(query: string) -> table`  
  - Executes a read-only SQL query over the JE database.
  - Enforce timeout and row limits; agent must learn to aggregate rather than dump entire tables.

- `grep(pattern: string, file: string) -> matches`  
  - For searching descriptions, configs, and (in ablations) oracle label files.

- `graph_query(query: string) -> subgraph` (optional, advanced)
  - Simple DSL to traverse transaction or entity graphs (e.g., “neighbors of employee X via bank accounts”).

- `plot(query_or_series) -> image_ref`
  - Minimal API that produces histograms / time-series and returns a reference the model can inspect.

Additionally:

- A **scratchpad memory** (short-term notes) persisted across tool calls for each run.
- A strict **token meter** for each run, visible to the orchestrator (not to the model), to allow early stopping and uniform budgets.

### 3.2 Task Definition and Instructions

Canonical high-level prompt:

- You are a **forensic auditor** investigating a synthetic but realistic accounting ledger.
- You have read-only access to:
  - Journal entries (FEC/JE), including headers and line items.
  - Master data for vendors/customers/employees.
- Your goal is to identify and explain **suspicious transactions, fraud schemes, or control failures**.
- You may call tools (`sql`, `grep`, optional `graph_query`, `plot`) multiple times, but your total token budget is limited.
- At the end, you must produce:
  1. A **machine-readable suspicion list** (JSON) describing:
     - Suspected `document_id`s.
     - Hypothesized `scheme_type` (e.g., shadow_payroll, triad_bypass, embezzlement, etc.).
     - Confidence (0–1) and short rationale.
  2. A **narrative report** in markdown explaining your investigative process and key evidence.

Optional sub-tasks (for ablations):

- Focused tasks like:
  - “Identify possible **shadow payroll** (ghost employees) in this ledger.”
  - “Identify **triad bypass** payments that reuse invoice IDs.”
  - “Identify **gradual embezzlement** via small recurring expenses.”

### 3.3 Evaluation Protocol

For each model:

- Run **N independent agents** (different seeds) on the same dataset and tools.
- Capture:
  - Tool call traces.
  - Intermediate reasoning (where allowed).
  - Final suspicion list and report.

Offline evaluation:

- Join suspicion lists with `anomaly_labels` on `document_id`:
  - **Entry-level metrics**:
    - Precision, recall, F1 per `anomaly_type` and per `scheme_type`.
    - ROC-AUC / PR-AUC by score threshold on confidence.
  - **Scheme-level metrics**:
    - A scheme is “detected” if the agent flags at least one of its actions with sufficient confidence.
    - Report scheme detection rate per multi-stage scheme type (shadow_payroll, triad_bypass, expense_laundering, etc.).

Qualitative evaluation:

- Aggregate **reasoning patterns**:
  - How often do agents inspect high-risk accounts (e.g., 6411xx / 6451xx / 471xxx / 401xxx / 512xxx)?
  - Do they correlate employees and bank accounts for ghost payroll?
  - Do they reuse FEC structure (journal number, auxiliary account, lettrage) appropriately?

Budget vs performance:

- Run the same model at different **token budgets** (e.g., 10M, 25M, 50M).
- Plot detection metrics vs budget to study efficiency and diminishing returns.

---

## 4. Recommended Visualization and Analysis Tools

- **SQL frontend**: Metabase or Apache Superset on top of Postgres/DuckDB.
- **FEC / JE viewer**: existing `datasynth-output-viewer` for quick inspection of suspicious entries.
- **Graphs**:
  - Use `datasynth-graph` + Neo4j or PyTorch Geometric to visualize:
    - Payment graphs for triad bypass.
    - Employee–bank account networks for shadow payroll.
- **Agent traces**:
  - Minimal web UI or notebook to browse:
    - Tool calls over time.
    - Partial results (SQL tables, subgraphs).
    - Link back to JE viewer and anomaly labels for analyst review.

---

## 5. Practical Considerations and Core Constraints

1. **No label leakage**
   - Anomaly labels must **never** be available to the agent during investigation.
   - Labels are only used for offline scoring and ablation studies.

2. **Dataset size and performance**
   - Target ~30k–100k JEs per dataset, with multi-stage schemes enabled.
   - Use indexing on key columns (`document_id`, `gl_account`, `company_code`, `posting_date`, `auxiliary_account_number`).

3. **Reproducibility**
   - Fix `global.seed` and record `run_manifest.json` for each benchmark dataset.
   - Version-control:
     - Configs (`config_multischemes.yaml` variants).
     - Exact model versions and tool definitions.

4. **Fairness across models**
   - Ensure:
     - Same dataset and database schema.
     - Same set of tools and constraints.
     - Similar decoding settings where possible (temperature, top-p).

This plan defines the artefacts, methodology, and constraints needed to turn EY-ASU SyntheticData into a rigorous benchmark for agentic LLM forensic investigation over journal entries and FEC exports.

