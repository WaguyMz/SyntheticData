# Why most fraud scheme types show no (or few) instances

## Summary

Labels with **scheme type** (Gradual Embezzlement, Triad Bypass, Smurfing, etc.) and `scenario_id` come from the **SchemeAdvancer**. That component is only used when the runtime (or another caller) explicitly:

1. Calls **`maybe_start_scheme()`** once per period with available users, accounts, and counterparties.
2. Calls **`advance_schemes()`** (e.g. once per day) to generate scheme actions.
3. Converts scheme actions into labels and merges them into the anomaly output.

Currently, the **orchestrator only calls `injector.process_entries(entries)`**. It never calls `maybe_start_scheme` or `advance_schemes`. So:

- **Multi-stage scheme labels** (with `scenario_id`, `pathology_name`, scheme instance grouping) are **not produced** in the default pipeline.
- The fraud you do see is from the **generic anomaly injector**: `select_anomaly_category()` → fraud type (e.g. FictitiousTransaction, RevenueManipulation) → strategy application. Those labels do not have `scenario_id` and are not grouped by the 10 RIP-GNN scheme types.

So “no fraud for most schemes” is expected until the scheme advancer is driven by the runtime.

## What was changed (code)

- **Injector**: When multi-stage schemes are enabled, all **10 scheme probabilities** are now taken from config. Previously only embezzlement, revenue_manipulation, and kickback were wired; the other seven used `Default::default()` (0.5%). They now all use the same base probability (e.g. 1% when `scheme_probability` / per-scheme config is 0.01). So when the advancer *is* integrated, all 10 types will have the configured rate.
- **Config**: `config_with_10_schemes.yaml` sets the three configurable scheme probabilities to 1%; the injector applies that same level to the other seven when building `SchemeAdvancerConfig`.

## What is left (future work)

To actually get labels for all 10 scheme types:

1. **Orchestrator** (or a dedicated phase) should:
   - Build lists of available users, accounts, and counterparties (from master data / entries).
   - For each period (e.g. first day of each month), call `injector.maybe_start_scheme(date, company_code, users, accounts, counterparties)`.
   - For each distinct (date, company) in the entry set, call `injector.advance_schemes(date, company_code)`.
2. **Scheme actions** returned by `advance_schemes` must be turned into **labels** (and optionally linked to journal entries by matching date/company/user/account), then merged into the anomaly labels written to `labels/anomaly_labels` / `labels/fraud_labels` / multi_stage export.

Until that integration exists, the output viewer’s “Scheme taxonomy” will show mostly empty counts for most scheme types, and the label table will be dominated by non-scheme (generic) fraud and other anomaly types.
