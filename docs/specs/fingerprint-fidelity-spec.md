## Fingerprint Fidelity Score – Specification

> **Status:** Draft  
> **Scope:** Definition of the confidentiality / fidelity score used when generating from a fingerprint (`datasynth-fingerprint` + `datasynth-cli`).

---

### 1. Purpose and Scope

- **Goal:** Quantify how closely **synthetic data** matches a **source fingerprint** while respecting privacy constraints.
- **Where used:** Printed as the "Confidentiality score (fidelity to fingerprint)" at the end of `datasynth-data generate --fingerprint ...` and written to `fingerprint_fidelity_report.json` in the output directory.
- **Current focus:** For journal-entry–style data, the fidelity score is intentionally focused on:
  - **Account-class mix** (share of rows per class, e.g. 1XXX/2XXX/6XXX/7XXX)
  - **Amount distributions per account class**, under a **log-normal hypothesis**

Other components (schema, correlations, rules, anomalies) are still computed for diagnostics, but **do not currently contribute** to the aggregate score.

---

### 2. Data Sources

The evaluator compares:

- **Original fingerprint**: Loaded from the `.dsf` file passed via `--fingerprint` (or a corresponding JSON sidecar).
- **Synthetic fingerprint**: Extracted on the fly from the generated CSV (typically `journal_entries.csv`) using `FingerprintExtractor`.

Both fingerprints expose a `StatisticsFingerprint` which includes:

- `numeric_columns`: global numeric stats (distribution type, parameters, percentiles, etc.)
- `categorical_columns`: per-column category frequencies
- `benford_analysis`: first-digit distribution for amounts
- `account_class_stats`: **per-account-class statistics** (row counts, optional numeric stats)

---

### 3. Overall Score and Component Weights

The evaluator is configured by `FidelityConfig`:

- **Threshold**:  
  - `threshold: 0.8` → `passes = overall_score >= 0.8`
- **Weights (current default):**
  - `statistical_weight = 1.0`
  - `correlation_weight = 0.0`
  - `schema_weight = 0.0`
  - `rule_weight = 0.0`
  - `anomaly_weight = 0.0`

Therefore:

- **Overall score** = **Statistical fidelity** (0.0–1.0)  
- CLI output:
  - `Overall:        {overall_score * 100:.1f}%`
  - `Statistical:   {statistical_fidelity * 100:.1f}%`
  - `Account-class mix MAD: {account_class_proportion_mad:.4f}` (if available)
  - `Pass:          yes|no`

The JSON report (`fingerprint_fidelity_report.json`) still includes the other component scores, but they are **not used in `overall_score`** with this weighting.

---

### 4. Account-Class Mix – MAD (Mean Absolute Deviation)

**Goal:** Measure how similar the **row-count share per account class** is between original and synthetic data.

Definitions:

- Let `account_class_stats` be grouped by **first digit** of the class pattern (e.g., `6XXX → '6'`, `701 → '7'`).
- For each digit \\( d \\), in the **original**:
  - \\( \\text{orig\\_count}\_d = \\sum\\_{s \\in \\text{classes with digit } d} s.\\text{row\\_count} \\)
- For each digit \\( d \\), in the **synthetic**:
  - \\( \\text{syn\\_count}\_d = \\sum\\_{s \\in \\text{classes with digit } d} s.\\text{row\\_count} \\)

Proportions:

\\[
p^{\\text{orig}}_d = \\frac{\\text{orig\\_count}_d}{\\sum\\_k \\text{orig\\_count}_k}, \\quad
p^{\\text{syn}}_d = \\frac{\\text{syn\\_count}_d}{\\sum\\_k \\text{syn\\_count}_k}
\\]

Mean Absolute Deviation over digits present in the original:

\\[
\\Delta_d = \\left| p^{\\text{orig}}_d - p^{\\text{syn}}_d \\right| \\\\
\\text{MAD} = \\frac{1}{N} \\sum\\_d \\Delta_d
\\]

- Stored in `details.account_class_proportion_mad`.
- Reported in CLI as `Account-class mix MAD: {mad:.4f}`.
- A **lower MAD** means **closer account-class mix** (e.g., 0.0 = perfect, 0.17 ≈ 17 percentage points average deviation).

The associated **class-mix score** is:

\\[
\\text{class\\_mix\\_score} = 1.0 - \\min(\\text{MAD}, 1.0)
\\]

and is used as one of the statistical components (see §5).

---

### 5. Statistical Fidelity – Account-Class–Focused

When **both** fingerprints expose `account_class_stats`, statistical fidelity is computed from **per-account-class similarity**:

#### 5.1 Digit-level aggregation

1. Compute total row counts:

\\[
\\text{orig\\_total} = \\sum\\_s s.\\text{row\\_count}, \\quad
\\text{syn\\_total} = \\sum\\_s s.\\text{row\\_count}
\\]

2. Group classes by **first digit** \\( d \\in \\{0,\\dots,9\\} \\), gathering:
   - Total row count per digit (used for proportions).
   - Any per-class `NumericStats` (amount distributions) in that digit.

#### 5.2 Proportion similarity per digit

From §4:

\\[
\\text{proportion\\_sim}_d = 1 - \\left| p^{\\text{orig}}_d - p^{\\text{syn}}_d \\right|
\\]

#### 5.3 Amount-distribution similarity per digit (log-normal aware)

For each digit \\( d \\):

- Let `orig_numerics` = list of `NumericStats` for original classes in digit \\( d \\).
- Let `syn_numerics` = list of `NumericStats` for synthetic classes in digit \\( d \\).

If **both lists are non-empty**:

1. For every pair \\( (o, s) \\in \\text{orig\\_numerics} \\times \\text{syn\\_numerics} \\):
   - Compute `ColumnFidelityMetrics`:
     - `ks_statistic` – KS-like distance from percentiles (scale-invariant).
     - `mean_diff`, `std_dev_diff` – see §6 (log-normality handling).
   - Per-pair amount score:

\\[
\\text{amount\\_score} = \\max\\left(0,\\; 1 - \\frac{\\text{KS} + \\min(\\text{mean\\_diff}, 1) + \\min(\\text{std\\_diff}, 1)}{3}\\right)
\\]

2. Average all `amount_score`s within digit \\( d \\):

\\[
\\text{amount\\_sim}_d = \\frac{1}{M} \\sum\\_{i=1}^{M} \\text{amount\\_score}_i
\\]

If either side has no numeric stats in that digit, set \\( \\text{amount\\_sim}_d = 1.0 \\) (only mix is evaluated).

#### 5.4 Digit-level class similarity

For each digit \\( d \\):

\\[
\\text{class\\_sim}_d =
  \\begin{cases}
  \\dfrac{\\text{proportion\\_sim}_d + \\text{amount\\_sim}_d}{2}, & \\text{if both sides have numeric stats for } d \\\\
  \\text{proportion\\_sim}_d, & \\text{otherwise}
  \\end{cases}
\\]

#### 5.5 Aggregate statistical fidelity

If at least one digit has stats:

\\[
\\text{statistical\\_fidelity} = \\frac{1}{N} \\sum\\_d \\text{class\\_sim}_d
\\]

This value is clamped to \\([0,1]\\) and becomes the **overall fidelity score** with the current weighting.

If `account_class_stats` are missing on either side, the evaluator falls back to:

- Benford MAD comparison (first-digit distributions).  
- Account-class mix and amount fidelity using alternative paths.  
- Global numeric/categorical column matching (less relevant for the JE fingerprint path).

---

### 6. Log-Normality and Amount Metrics

Amounts in journal-entry data are often modeled as **log-normal**, especially after outlier handling and Benford alignment. The evaluator explicitly accounts for this:

#### 6.1 Extracting log-normal parameters

For each `NumericStats` \\( s \\):

1. If:
   - `distribution == LogNormal`, **and**
   - `distribution_params` contain valid \\( \\mu, \\sigma > 0 \\),

   then use those directly.

2. Otherwise, derive \\( \\mu, \\sigma \\) via **method of moments** from mean and variance:

\\[
\\sigma^2 = \\ln\\left(1 + \\frac{\\text{Var}}{\\text{mean}^2}\\right), \\quad
\\mu = \\ln(\\text{mean}) - \\frac{\\sigma^2}{2}
\\]

If the mean is non-positive, fall back to normalized mean/std differences.

#### 6.2 Log-normal penalty

When both sides have valid log-normal parameters:

- Let \\( (\\mu_o, \\sigma_o) \\) and \\( (\\mu_s, \\sigma_s) \\) be the log-space parameters.
- Compute:

\\[
\\text{mu\\_penalty} = \\frac{|\\mu_o - \\mu_s|}{1 + |\\mu_o|}, \\quad
\\text{sigma\\_penalty} = \\frac{|\\sigma_o - \\sigma_s|}{1 + \\sigma_o}
\\]

- Combine and bound:

\\[
\\text{ln\\_penalty} = \\min(\\text{mu\\_penalty} + \\text{sigma\\_penalty}, 2.0)
\\]

- Set:

\\[
\\text{mean\\_diff} = \\text{ln\\_penalty} / 2, \\quad
\\text{std\\_diff} = \\text{ln\\_penalty} / 2
\\]

These `mean_diff` and `std_dev_diff` feed into the **amount score** definition in §5.3.

If log-normal parameters are not available, the evaluator falls back to:

- Normalized absolute difference of means.
- Normalized difference of standard deviations (bounded).

#### 6.3 Percentile-based KS-like statistic

To complement log-normal parameters, the evaluator also computes a **KS-like statistic** from percentile grids:

- Let `percentiles` arrays \\( P_o, P_s \\) be aligned for original/synthetic columns.
- Let:

\\[
\\text{range} = \\max(\\text{max}_o - \\text{min}_o,\\; \\text{max}_s - \\text{min}_s,\\; 1.0)
\\]

- Define:

\\[
\\text{KS} = \\max\\_i \\left| \\frac{P_{o,i} - P_{s,i}}{\\text{range}} \\right|
\\]

This KS-like statistic is used alongside `mean_diff` and `std_diff` in the amount score formula.

---

### 7. Pass / Fail Semantics

- **Input:** `overall_score` \\( \\in [0,1] \\), `threshold` (default 0.8).
- **Pass condition:** `passes = (overall_score >= threshold)`.

Interpretation:

- `overall_score >= 0.8`:
  - Synthetic account-class mix and amount distributions are **close** to the fingerprint.
- `overall_score` in the 0.6–0.8 range:
  - Acceptable but with noticeable drift (potentially good for higher confidentiality).
- `overall_score < 0.6`:
  - Synthetic data deviates substantially from fingerprinted statistics.

---

### 8. CLI Output and JSON Report

For fingerprint-based runs (`--fingerprint`), the CLI prints:

```text
Confidentiality score (fidelity to fingerprint)
=================================================
  Overall:        {overall_score * 100:.1f}%
  Statistical:   {statistical_fidelity * 100:.1f}%
  Account-class mix MAD: {account_class_proportion_mad:.4f}
  Pass:          {yes|no}
```

The corresponding JSON (`fingerprint_fidelity_report.json`) contains:

- `overall_score`
- `statistical_fidelity`
- `correlation_fidelity` (currently weight 0)
- `schema_fidelity` (weight 0)
- `rule_compliance` (weight 0)
- `anomaly_fidelity` (weight 0)
- `passes`
- `details`:
  - `account_class_proportion_mad`
  - `benford_mad` (if computed)
  - `correlation_rmse` (if computed)
  - `row_count_ratio`
  - per-column metrics and additional diagnostics

---

### 9. Future Extensions (Non-Normative)

Potential future changes (not implemented yet):

- Reintroduce non-zero weights for correlation, schema, rules, or anomaly fidelity for use cases that require tighter structural matching.
- Add configurable weighting profiles:
  - **Training data mode:** higher weight on statistical and correlation fidelity.
  - **Confidentiality mode:** cap fidelity to enforce minimum deviation.
- Extend account-class fidelity to **sub-classes** (e.g., 411/412 splits) where available.

