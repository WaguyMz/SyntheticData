<!-- GitHub issue title: Implement account-class-level fingerprinting -->

## Description

Per-account-class fingerprinting **is specified but not yet implemented**. The goal is to:

- Detect `(account, debit, credit)` columns (with FEC shortcuts).
- Aggregate amounts by **account class** (first N digits, initially 3) into `statistics.amount_by_account_class`.
- Enforce privacy with ε-differential privacy and k-anonymity thresholds.
- Feed per-class distributions into the JE generator and, for FEC, into material cost / margin generation.

This issue tracks the implementation of account-class-level fingerprinting, plus the tests and hooks needed so it can be safely extended later.

---

## Goals

| Goal | Description |
|------|-------------|
| **Implement extraction** | Implement column detection and per-account-class aggregation as per the plan. |
| **Implement privacy layer** | Apply ε-DP noise and suppression for per-class stats under the global privacy engine. |
| **Wire into generation** | Make JE generation and (for FEC) material cost/margin able to consume per-class parameters. |
| **Add evaluation hooks** | Prepare `datasynth-data fingerprint evaluate` to compare per-class distributions once implemented. |
| **Human-readable JSON export** | Generate an optional, stable, human-readable JSON representation of each fingerprint (including per-account-class stats) for debugging and inspection. |
| **Future extensibility** | Leave clear extension points for configurable class granularity, non-PCG mappings, and graph features. |

---

## Target behaviour (once implemented)

- **Account class**
  - Class = first `N` ASCII digits of the account code (configurable, default `FEC_ACCOUNT_CLASS_LEVEL = 3`).
  - Rows with too few digits or a non-digit first char are excluded from per-class stats.

- **Generic GL column detection**
  - Heuristics to find `account` (e.g. `account`, `gl_account`, `compte` but not `CompteLib`) and `debit`/`credit` (`debit`, `credit`, localized forms including `montant au débit` / `montant au crédit`).
  - If no `(account, debit, credit)` triple is found, `amount_by_account_class` is set to `null`.

- **FEC path**
  - Fixed columns: `Numéro de compte`, `Montant au débit`, `Montant au crédit`.
  - Date-like columns (e.g. `EcritureDate`, `PieceDate`) are always treated as temporal.

- **Per-class stats**
  - For each class with at least `FEC_MIN_ROWS_PER_CLASS` rows:
    - `count`, `min`, `max`, `mean`, `std_dev`, percentiles (p10–p90).
    - Benford first-digit histogram and MAD.
    - Fitted lognormal parameters (μ, σ) on `ln(|amount|)` where applicable.
  - All stats are noised via Laplace under a global ε-DP budget; rare categories can be suppressed.

- **Fingerprint representation and consumption**
  - Stored under `statistics.amount_by_account_class` as `{ "601": { ... }, "701": { ... }, ... }`, or `null` when unavailable.
  - Config synthesizer maps this to `transactions.amounts.amounts_by_account_class` and, for FEC, to material cost / margin parameters.
  - `JournalEntryGenerator` uses per-class parameters when present, falling back to global amount configuration.

---

## Proposed work

### Extraction & parsing

- [ ] Add fixtures and tests for:
  - Multiple header variants (EN/FR, abbreviations, extra tokens) for account/debit/credit.
  - Mixed numeric formats (dot/comma decimals, spaces/thousands separators) and non-numeric noise.
  - Edge cases: missing columns, short or malformed account codes.
- [ ] Test `FEC_MIN_ROWS_PER_CLASS` behaviour: classes below the threshold are omitted and do not consume privacy budget.
- [ ] Add regression tests verifying that `amount_by_account_class` is `null` when detection fails.

### Privacy & auditability

- [ ] Ensure all per-class stats go through the `PrivacyEngine` (no raw stats leaked).
- [ ] Add tests that:
  - Total ε spent never exceeds the configured ε, even with many per-class stats.
  - Budget exhaustion is handled gracefully (clear error or early termination with `amount_by_account_class = null`).
- [ ] Verify privacy audit entries for per-class stats (operation, field, ε spent) and ensure CLI logs summarize:
  - Total ε spent.
  - Whether per-class stats were enabled and how many classes were included.

### Evaluation integration

- [ ] Extend `datasynth-data fingerprint evaluate` to:
  - Load `statistics.amount_by_account_class` for reference and synthetic fingerprints.
  - Compute per-class divergence (e.g. RMSE or Wasserstein distance) on selected stats (mean, std, quantiles, Benford MAD).
  - Aggregate into an “account-class fidelity” score and highlight worst-divergent classes.
- [ ] Surface these metrics in CLI output and reports.

### Human-readable fingerprint JSON

- [ ] Define a stable, human-readable JSON schema for the fingerprint, including `statistics.amount_by_account_class` and related privacy audit metadata.
- [ ] Implement serialization from the internal fingerprint model to pretty-printed JSON following this schema.
- [ ] Add a CLI option to write this JSON file alongside other fingerprint artifacts, with a clear naming convention and output location.
- [ ] Add tests to ensure the JSON is parseable, matches the schema, contains no row-level or PII data, and reflects the noised statistics actually used by generation.

### Configuration & extensions

- [ ] Add a config option for `account_class_level` (e.g. 2 vs 3 digits) with validation against row counts and privacy constraints.
- [ ] For non-PCG charts (US GAAP / IFRS), introduce a configurable mapping that labels prefixes as cost vs revenue for material/cost patching.
- [ ] Optionally expose per-class parameters into `datasynth-graph`:
  - Attach per-class features (e.g. lognormal μ/σ, Benford MAD) to account nodes.
  - Integrate into graph-level metrics (e.g. Structural Accounting Entropy baseline) if/when available.

### Documentation

- [ ] Add short user docs describing:
  - How account-class fingerprinting works conceptually.
  - How to interpret new evaluation metrics.
  - How to configure `account_class_level` and non-PCG mappings.

---

## Acceptance criteria

- [ ] Extraction is robust across tested header/localization variants; detection failures are explicit and result in `amount_by_account_class = null`.
- [ ] All per-class stats are provably within the global ε budget, with complete privacy audit entries.
- [ ] `datasynth-data fingerprint evaluate` reports per-class fidelity metrics and flags divergent classes.
- [ ] CLI can emit a human-readable fingerprint JSON file that matches the defined schema and is free of row-level/PII data.
- [ ] `account_class_level` and non-PCG mappings are configurable, validated, and documented.
- [ ] Optional integration with `datasynth-graph` is implemented or explicitly deferred with a design note.
- [ ] Documentation is updated and linked from relevant CLI/README sections.

---

## References

- Extraction & privacy: `crates/datasynth-fingerprint/src/extraction/`, `crates/datasynth-fingerprint/src/privacy/`
- Generation: JE and material generators in `crates/datasynth-generators/`
- Evaluation: `crates/datasynth-eval/`, `datasynth-data fingerprint evaluate`
- Graph: `crates/datasynth-graph/`

