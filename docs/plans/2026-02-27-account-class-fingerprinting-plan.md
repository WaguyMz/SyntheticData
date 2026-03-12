# Account-Class-Level Fingerprinting — Specification & Integration Plan

**Status:** Planned (specification for implementation)  
**Created:** 2026-02-27  
**Scope:** Specification for per-account-class fingerprinting **to be implemented** (FEC + generic GL): extraction, privacy, consumption by generation, and extension points.

---

## 1. Objectives and Requirements

### 1.1 Goals

- **Per-class marginals:** Capture **amount distributions per account class** (e.g. 601, 701) while preserving privacy.
- **Chart-agnostic:** Support **French GAAP (PCG)** FEC as well as generic GL charts (US GAAP, IFRS) by detecting account/debit/credit columns dynamically.
- **Low-leakage:** Maintain **ε-differential privacy** for the **entire fingerprint**, including per-class stats, using a shared privacy budget.
- **Generation-ready:** Provide per-class parameters in a form that can be **plugged into the JE generator** and, for FEC, into **material cost / margin** generation.

### 1.2 Non-Goals

- Do not store any **row-level** or **PII** information.
- Do not derive or override **fraud/anomaly** rates from the fingerprint (those remain config-driven).
- Do not change document-flow or subledger logic directly; per-class distributions are used mainly for:
  - Standalone JEs.
  - Optional material cost / margin patches (FEC).

---

## 2. Extraction Specification (Fingerprint Side)

### 2.1 Account Class Definition

- **Account class** = first **N ASCII digits** of the account code (default N = 3):
  - Constant: `FEC_ACCOUNT_CLASS_LEVEL = 3` (configurable in a later phase).
  - Example: `"411000"` → `"411"`, `"601200"` → `"601"`.
- Rows with:
  - Fewer than `N` digits, or
  - Non-digit in the first position  
  are **ignored** for per-class stats.

### 2.2 Column Detection (Generic GL)

To implement (e.g. in `stats_extractor.rs` or a dedicated extraction module):

- **Account column detection:**
  - Candidate header names (case/diacritics-insensitive):
    - `account`, `account_number`, `gl_account`, `compte`, `comptenum`, `compte num`, `compte_num`, `compauxnum`, etc.
  - Heuristic: prefer columns containing `"compte"` but **not** `"lib"` (to avoid `CompteLib`).

- **Debit/Credit columns:**
  - Primary matches:
    - `debit` / `credit`, `debit_amount` / `credit_amount`.
  - Localized FEC-style names:
    - `"montant au débit"`, `"montant au crédit"`.
  - Fallback:
    - Any header containing `"debit"` or `"débit"` and any header containing `"credit"` or `"crédit"` when an account column containing `"compte"` is present.

- **Failure mode:**
  - If no suitable `(account, debit, credit)` triple is found, `amount_by_account_class` is set to `null` in the fingerprint.

### 2.3 FEC-Specific Path

When `--accounting-standards french_gaap` (or FEC format detection) is used:

- Account column shall be fixed as:
  - `"Numéro de compte"`.
- Amount columns:
  - `"Montant au débit"`, `"Montant au crédit"`.
- Date-like columns (EcritureDate, PieceDate, etc.) are always **temporal**, never numeric.

### 2.4 Amount Parsing

- Parsing shall support both **standard** and **European** formats:
  - Decimal separators: `.` or `,`.
  - Thousands separators: space or comma (when not used as decimal).
- Non-parsable values (e.g. empty strings, non-numeric text) are treated as **zero** for counting but **omitted** from numeric aggregates.

### 2.5 Minimum Rows per Class

- Configured constant: `FEC_MIN_ROWS_PER_CLASS = 5`.
- For each class:
  - If `row_count(class) < FEC_MIN_ROWS_PER_CLASS`, the class is **omitted** from `amount_by_account_class`.
  - Omitted classes consume **no privacy budget**.

### 2.6 Statistics per Class

For each qualifying class `c`:

- Aggregate debit and credit:
  - Use **signed amounts** at the row level to compute moments.
  - Store at least:
    - `count`, `min`, `max`, `mean`, `std_dev`.
    - Percentiles (p10, p25, p50, p75, p90).
    - **Benford** first-digit histogram / MAD.
    - Fitted **lognormal** parameters (μ, σ) for `ln(|amount|)` where appropriate.

- **Privacy preservation:**
  - All numeric stats are **noised via Laplace** with sensitivity derived from the (winsorized) range.
  - See §3 for privacy budget allocation.

### 2.7 Fingerprint Representation

- Top-level JSON field: `statistics.amount_by_account_class`.
  - Keys: 3-digit classes (`"601"`, `"701"`, etc.).
  - Values: structured stats object per class (`count`, `min`, `max`, `mean`, `std`, `percentiles`, `lognormal_mu`, `lognormal_sigma`, etc.).
- When no data or classes violate constraints:
  - `amount_by_account_class` is set to **`null`**.

---

## 3. Privacy Specification

### 3.1 Global Budget

- **Engine:** Implementation shall use the existing `PrivacyEngine` in `datasynth-fingerprint` (or equivalent).
- **Total budget:** ε (from CLI: `--privacy-level` or `--privacy-epsilon`).
- **Budget subdivision:**
  - Max `MAX_NOISE_QUERIES = 1000` per fingerprint.
  - Each call to `add_noise` spends `ε / MAX_NOISE_QUERIES`.
  - Composition is **naive**: the entire fingerprint, including all per-class stats, stays **ε-DP**.

### 3.2 Noise Application for Per-Class Stats

- For each class and statistic:
  - Compute **raw** statistic over winsorized data.
  - Apply Laplace noise:
    - `noised_stat = raw_stat + Laplace(Δ/stat_budget)` where Δ is sensitivity (e.g. `(max - min) / n` for mean).
- **Benford**:
  - First-digit frequencies are also noised; MAD is computed from the noised distribution.

### 3.3 K-Anonymity and Suppression

- Rare categories (when treated as categorical) are suppressed if they fall below the global **k-anonymity** threshold.
- Per-class stats do **not** expose identities, only aggregated amounts; the main guard is:
  - `FEC_MIN_ROWS_PER_CLASS` + Laplace noise.

### 3.4 Privacy Audit

- Each noise addition and suppression is recorded in the fingerprint’s **privacy audit**:
  - Operation type, target field, `epsilon_spent`.
- CLI logs summarize:
  - Total **epsilon spent**.
  - Whether per-class extraction was enabled and how many classes were included.

---

## 4. Consumption in Generation

### 4.1 Config Patch Structure

When `amount_by_account_class` is present in the fingerprint, the config synthesizer shall emit:

- `transactions.amounts.amounts_by_account_class`:
  - A mapping from:
    - `3-digit class` → distribution parameters.
    - **First-digit** aggregate (1, 2, 3, 6, 7, …) for fallback.
- For FEC, additional patch keys:
  - `master_data.materials.standard_cost_lognormal_mu`, `standard_cost_lognormal_sigma`.
  - `master_data.materials.gross_margin_mean`, `gross_margin_std`.
  - Optional min/max bounds for cost and margin.

### 4.2 JournalEntryGenerator Behaviour

Once per-class data exists, the JE generator (e.g. `je_generator.rs`) shall:

- When `config.transactions.amounts.amounts_by_account_class` is **present**:
  - For each JE:
    - Extract account classes:
      - Primary: first 3-digit class (e.g. `"601"`).
      - Fallback: first digit (e.g. `"6"`).
    - Call:
      - `AmountSampler::sample_summing_to_per_class(&[(class, is_debit), ...], total_amount)`.
  - This ensures:
    - **Per-account-class** marginal distributions approximate the fingerprint.
    - Each JE still remains **balanced** (sum of debits == sum of credits).

- When per-class config is **absent**:
  - `AmountSampler::sample_summing_to` is used with global amount parameters.

### 4.3 Material Cost & Revenue (FEC Only)

For **FEC-origin fingerprints**, the synthesizer also maps:

- 6xx classes → **cost**.
- 7xx classes → **revenue**.

Workflow:

1. Aggregate noised per-class stats for:
   - 6xx (cost) and 7xx (revenue).
2. Derive:
   - Lognormal parameters for **standard cost**.
   - Normal parameters for **gross margin** from 7xx / 6xx.
3. Patch `master_data.materials` accordingly.
4. Material generator uses:
   - LogNormal(μ, σ) for standard cost.
   - Normal(mean, std) for gross margin.

Result:

- P2P and O2C amounts (which derive from material cost × quantity, plus margin) **match** the underlying per-class cost/revenue structure from the fingerprint.

---

## 5. Validation and Evaluation

### 5.1 Internal Validation (to implement)

- **Extraction tests:**
  - Use controlled CSV/FEC fixtures to verify:
    - Correct column detection (account, debit, credit).
    - Expected classes included/excluded by row-count threshold.
    - Numeric stats (before noise) match ground truth.
- **Privacy tests:**
  - Simulate multiple repeated extractions to confirm:
    - Epsilon composition is bounded by configured ε.
    - Budget exhaustion is handled gracefully (error or early stop).

### 5.2 External Evaluation

`datasynth-data fingerprint evaluate` currently focuses on global numeric stats, categorical marginals, and Benford law. As part of (or after) implementing per-class fingerprinting:

- **Extension:** Incorporate `amount_by_account_class` into the fidelity score:
  - Compare per-class distribution between reference fingerprint and re-extracted synthetic fingerprint.
  - Flag classes with high divergence (RMSE or Wasserstein distance).

---

## 6. Extension Ideas (Future Work)

- **Dynamic class level:** Allow configuring account-class granularity (e.g. 2 vs 3 digits) based on row count and privacy budget.
- **Non-PCG mapping:** For US GAAP / IFRS:
  - Provide configurable **class-to-role mapping** (e.g. which prefixes are cost vs revenue) for material patching, similar to PCG’s 6xx/7xx.
- **Graph integration:** Use per-class stats as node features in `datasynth-graph`:
  - Add per-account-class distribution parameters (or SAE-like measures) as `AccountNode` properties or features.
- **SAE baseline:** Treat per-class distributions as a low-entropy baseline; deviations in graph edge distributions per class can contribute to Structural Accounting Entropy.

