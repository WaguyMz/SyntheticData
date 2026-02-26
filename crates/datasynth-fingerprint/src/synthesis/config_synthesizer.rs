//! Config synthesizer - converts fingerprints to generator configs.

use std::collections::HashMap;

use super::CopulaGenerator;
use crate::error::FingerprintResult;
use crate::models::{
    AccountClassAmountStats, CorrelationMatrix, DistributionType, Fingerprint, GaussianCopula,
    NumericStats,
};

/// Options for config synthesis.
#[derive(Debug, Clone)]
pub struct SynthesisOptions {
    /// Scale factor for row counts (1.0 = same size, 2.0 = double).
    pub scale: f64,
    /// Random seed for generation.
    pub seed: Option<u64>,
    /// Whether to preserve correlations.
    pub preserve_correlations: bool,
    /// Whether to inject anomalies based on fingerprint.
    pub inject_anomalies: bool,
}

impl Default for SynthesisOptions {
    fn default() -> Self {
        Self {
            scale: 1.0,
            seed: None,
            preserve_correlations: true,
            inject_anomalies: true,
        }
    }
}

/// Synthesizer that converts fingerprints to generator configurations.
pub struct ConfigSynthesizer {
    options: SynthesisOptions,
}

impl ConfigSynthesizer {
    /// Create a new config synthesizer.
    pub fn new() -> Self {
        Self {
            options: SynthesisOptions::default(),
        }
    }

    /// Create with custom options.
    pub fn with_options(options: SynthesisOptions) -> Self {
        Self { options }
    }

    /// Synthesize a partial config from a fingerprint.
    ///
    /// Returns a ConfigPatch that can be merged with a base configuration.
    pub fn synthesize(&self, fingerprint: &Fingerprint) -> FingerprintResult<ConfigPatch> {
        let mut patch = ConfigPatch::new();

        // Extract row count with scaling
        let total_rows: u64 = fingerprint
            .schema
            .tables
            .values()
            .map(|t| t.row_count)
            .sum();
        let scaled_rows = (total_rows as f64 * self.options.scale) as u64;

        patch.set(
            "transactions.count",
            ConfigValue::Integer(scaled_rows as i64),
        );

        // Set seed if specified
        if let Some(seed) = self.options.seed {
            patch.set("global.seed", ConfigValue::Integer(seed as i64));
        }

        // Prefer per-account-class (level 3) amount distribution when present (e.g. from FEC)
        if let Some(ref by_class) = fingerprint.statistics.amount_by_account_class {
            if let Some(mixture) = self.mixture_amount_stats(by_class) {
                let amount_config = self.map_numeric_distribution(&mixture);
                for (k, v) in amount_config {
                    patch.set(&format!("transactions.amounts.{}", k), v);
                }
                // Expose per-class params so generators can mimic per account class
                if let Ok(json) = serde_json::to_string(&self.amount_class_params(by_class)) {
                    patch.set(
                        "transactions.amounts_by_account_class",
                        ConfigValue::String(json),
                    );
                }
            }
            // FEC: derive material price ranges from 6xx (achats) and 7xx (ventes) for document-flow alignment
            self.apply_fec_material_patch(by_class, &mut patch);
        }

        // Fallback: map global numeric columns to amount config
        if patch.get("transactions.amounts.lognormal_mu").is_none() {
            for (key, stats) in &fingerprint.statistics.numeric_columns {
                if key.contains("amount") || key.contains("value") || key.contains("price") {
                    let amount_config = self.map_numeric_distribution(stats);
                    for (k, v) in amount_config {
                        patch.set(&format!("transactions.amounts.{}", k), v);
                    }
                    break;
                }
            }
        }

        // Do not patch fraud/anomaly from fingerprint: keep base config's anomaly and fraud
        // injection so that fraud schemes and anomaly injection remain as configured (e.g. preset).

        Ok(patch)
    }

    /// Map numeric statistics to amount distribution config.
    fn map_numeric_distribution(&self, stats: &NumericStats) -> HashMap<String, ConfigValue> {
        let mut config = HashMap::new();

        config.insert("min_amount".to_string(), ConfigValue::Float(stats.min));
        config.insert("max_amount".to_string(), ConfigValue::Float(stats.max));

        match stats.distribution {
            DistributionType::LogNormal => {
                if let (Some(mu), Some(sigma)) = (
                    stats.distribution_params.param1,
                    stats.distribution_params.param2,
                ) {
                    config.insert("lognormal_mu".to_string(), ConfigValue::Float(mu));
                    config.insert("lognormal_sigma".to_string(), ConfigValue::Float(sigma));
                }
            }
            DistributionType::Normal => {
                // Convert normal to log-normal approximation for amounts
                if stats.mean > 0.0 {
                    let variance = stats.std_dev.powi(2);
                    let sigma_sq = (1.0 + variance / stats.mean.powi(2)).ln();
                    let mu = stats.mean.ln() - sigma_sq / 2.0;

                    config.insert("lognormal_mu".to_string(), ConfigValue::Float(mu));
                    config.insert(
                        "lognormal_sigma".to_string(),
                        ConfigValue::Float(sigma_sq.sqrt()),
                    );
                }
            }
            _ => {
                // Use empirical parameters based on percentiles
                if stats.percentiles.p50 > 0.0 {
                    let mu = stats.percentiles.p50.ln();
                    let sigma = (stats.percentiles.p75 / stats.percentiles.p25).ln() / 1.349;
                    config.insert("lognormal_mu".to_string(), ConfigValue::Float(mu));
                    config.insert(
                        "lognormal_sigma".to_string(),
                        ConfigValue::Float(sigma.abs()),
                    );
                }
            }
        }

        // Round number bias
        if let Some(benford) = stats.benford_first_digit {
            // Higher digit 1 frequency suggests round number bias
            let round_bias = if benford[0] < 0.25 { 0.3 } else { 0.15 };
            config.insert(
                "round_number_probability".to_string(),
                ConfigValue::Float(round_bias),
            );
        }

        config
    }

    /// Build row-count-weighted mixture of per-class debit stats for global amount config.
    fn mixture_amount_stats(
        &self,
        by_class: &HashMap<String, AccountClassAmountStats>,
    ) -> Option<NumericStats> {
        let total_rows: u64 = by_class.values().map(|s| s.row_count).sum();
        if total_rows == 0 {
            return None;
        }
        let total = total_rows as f64;
        // Weight by row_count; use debit_stats (primary for amount magnitude)
        let mut sum_mean = 0.0;
        let mut sum_var_plus_mean_sq = 0.0;
        let mut min_val = f64::MAX;
        let mut max_val = f64::MIN;
        for s in by_class.values() {
            let n = s.row_count as f64;
            let m = s.debit_stats.mean;
            let v = s.debit_stats.std_dev.powi(2);
            sum_mean += m * n;
            sum_var_plus_mean_sq += (v + m * m) * n;
            min_val = min_val.min(s.debit_stats.min);
            max_val = max_val.max(s.debit_stats.max);
        }
        let mean = sum_mean / total;
        let variance = (sum_var_plus_mean_sq / total) - (mean * mean);
        let std_dev = variance.max(0.0).sqrt();
        if mean <= 0.0 || !std_dev.is_finite() {
            return None;
        }
        Some(NumericStats {
            count: total_rows,
            min: min_val,
            max: max_val,
            mean,
            std_dev,
            percentiles: crate::models::Percentiles::default(),
            distribution: DistributionType::LogNormal,
            distribution_params: {
                let sigma_sq = (1.0 + variance / (mean * mean)).ln();
                let mu = mean.ln() - sigma_sq / 2.0;
                crate::models::DistributionParams::log_normal(mu, sigma_sq.sqrt())
            },
            zero_rate: 0.0,
            negative_rate: 0.0,
            benford_first_digit: None,
        })
    }

    /// Serializable per-class params for amounts_by_account_class (JSON in patch).
    /// Includes distributional stats (mean, std, lognormal μ/σ) per class and adds first-digit
    /// (1, 2, 3, 6, 7, etc.) aggregated stats so standalone JEs use distributional params.
    fn amount_class_params(
        &self,
        by_class: &HashMap<String, AccountClassAmountStats>,
    ) -> HashMap<String, serde_json::Value> {
        let mut out = HashMap::new();
        for (class, s) in by_class {
            let (d_mu, d_sigma) = self.stats_to_lognormal_params(&s.debit_stats);
            let (c_mu, c_sigma) = self.stats_to_lognormal_params(&s.credit_stats);
            let obj = serde_json::json!({
                "row_count": s.row_count,
                "debit": { "lognormal_mu": d_mu, "lognormal_sigma": d_sigma },
                "credit": { "lognormal_mu": c_mu, "lognormal_sigma": c_sigma },
                "debit_mean": s.debit_stats.mean,
                "debit_std": s.debit_stats.std_dev,
                "credit_mean": s.credit_stats.mean,
                "credit_std": s.credit_stats.std_dev,
            });
            out.insert(class.clone(), obj);
        }
        // First-digit (accounts 1, 2, 3, 6, 7, etc.) distributional stats for standalone JEs
        for digit in '1'..='9' {
            let prefix = digit.to_string();
            if let Some((debit_mix, credit_mix)) = self.mixture_for_prefix(by_class, digit) {
                let (d_mu, d_sigma) = self.stats_to_lognormal_params(&debit_mix);
                let (c_mu, c_sigma) = self.stats_to_lognormal_params(&credit_mix);
                let obj = serde_json::json!({
                    "debit": { "lognormal_mu": d_mu, "lognormal_sigma": d_sigma },
                    "credit": { "lognormal_mu": c_mu, "lognormal_sigma": c_sigma },
                    "debit_mean": debit_mix.mean,
                    "debit_std": debit_mix.std_dev,
                    "credit_mean": credit_mix.mean,
                    "credit_std": credit_mix.std_dev,
                });
                out.insert(prefix, obj);
            }
        }
        out
    }

    /// Row-count-weighted mixture of debit and credit stats for classes whose key starts with `prefix`.
    fn mixture_for_prefix(
        &self,
        by_class: &HashMap<String, AccountClassAmountStats>,
        prefix: char,
    ) -> Option<(NumericStats, NumericStats)> {
        let subset: HashMap<_, _> = by_class
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if subset.is_empty() {
            return None;
        }
        let debit_only: HashMap<_, _> = subset
            .iter()
            .map(|(k, s)| {
                (
                    k.clone(),
                    AccountClassAmountStats {
                        account_class: k.clone(),
                        row_count: s.row_count,
                        debit_stats: s.debit_stats.clone(),
                        credit_stats: s.debit_stats.clone(), // dummy, we only use debit in mixture
                    },
                )
            })
            .collect();
        let credit_only: HashMap<_, _> = subset
            .iter()
            .map(|(k, s)| {
                (
                    k.clone(),
                    AccountClassAmountStats {
                        account_class: k.clone(),
                        row_count: s.row_count,
                        debit_stats: s.credit_stats.clone(),
                        credit_stats: s.credit_stats.clone(),
                    },
                )
            })
            .collect();
        let debit_mix = self.mixture_amount_stats(&debit_only)?;
        let credit_mix = self.mixture_amount_stats(&credit_only)?;
        Some((debit_mix, credit_mix))
    }

    fn stats_to_lognormal_params(&self, stats: &NumericStats) -> (f64, f64) {
        if let (Some(mu), Some(sigma)) = (
            stats.distribution_params.param1,
            stats.distribution_params.param2,
        ) {
            return (mu, sigma);
        }
        if stats.percentiles.p50 > 0.0 {
            let mu = stats.percentiles.p50.ln();
            let sigma = (stats.percentiles.p75 / stats.percentiles.p25).ln() / 1.349;
            return (mu, sigma.abs());
        }
        if stats.mean > 0.0 {
            let variance = stats.std_dev.powi(2);
            let sigma_sq = (1.0 + variance / stats.mean.powi(2)).ln();
            let mu = stats.mean.ln() - sigma_sq / 2.0;
            return (mu, sigma_sq.sqrt());
        }
        (0.0, 1.0)
    }

    /// FEC: derive material standard_cost and gross_margin as distributional stats (mean, std, lognormal mu/sigma) from 6xx (achats) and 7xx (ventes).
    /// PCG class 6 = charges (debit = cost), class 7 = produits (credit = revenue).
    fn apply_fec_material_patch(
        &self,
        by_class: &HashMap<String, AccountClassAmountStats>,
        patch: &mut ConfigPatch,
    ) {
        let sixxx: HashMap<_, _> = by_class
            .iter()
            .filter(|(k, _)| k.starts_with('6'))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if let Some(cost_mixture) = self.mixture_amount_stats(&sixxx) {
            let (mu, sigma) = self.stats_to_lognormal_params(&cost_mixture);
            patch.set(
                "master_data.materials.standard_cost_lognormal_mu",
                ConfigValue::Float(mu),
            );
            patch.set(
                "master_data.materials.standard_cost_lognormal_sigma",
                ConfigValue::Float(sigma),
            );
            if cost_mixture.min > 0.0 && cost_mixture.min.is_finite() {
                patch.set(
                    "master_data.materials.standard_cost_min",
                    ConfigValue::Float(cost_mixture.min),
                );
            }
            if cost_mixture.max > 0.0 && cost_mixture.max.is_finite() {
                patch.set(
                    "master_data.materials.standard_cost_max",
                    ConfigValue::Float(cost_mixture.max),
                );
            }
        }

        let rev_mean: f64 = by_class
            .iter()
            .filter(|(k, _)| k.starts_with('7'))
            .filter_map(|(_, s)| {
                let m = s.credit_stats.mean;
                if m > 0.0 && m.is_finite() {
                    Some(m * s.row_count as f64)
                } else {
                    None
                }
            })
            .sum();
        let rev_count: u64 = by_class
            .iter()
            .filter(|(k, _)| k.starts_with('7'))
            .map(|(_, s)| s.row_count)
            .sum();
        let cost_mean: f64 = by_class
            .iter()
            .filter(|(k, _)| k.starts_with('6'))
            .filter_map(|(_, s)| {
                let m = s.debit_stats.mean;
                if m > 0.0 && m.is_finite() {
                    Some(m * s.row_count as f64)
                } else {
                    None
                }
            })
            .sum();
        let cost_count: u64 = by_class
            .iter()
            .filter(|(k, _)| k.starts_with('6'))
            .map(|(_, s)| s.row_count)
            .sum();

        if rev_count > 0 && cost_count > 0 {
            let rev_avg = rev_mean / rev_count as f64;
            let cost_avg = cost_mean / cost_count as f64;
            if cost_avg > 0.0 {
                let margin_mean = (rev_avg / cost_avg - 1.0).clamp(0.05, 0.65);
                let rev_var: f64 = by_class
                    .iter()
                    .filter(|(k, _)| k.starts_with('7'))
                    .map(|(_, s)| {
                        let n = s.row_count as f64;
                        let m = s.credit_stats.mean;
                        let v = s.credit_stats.std_dev.powi(2);
                        (v + m * m) * n
                    })
                    .sum::<f64>()
                    / rev_count as f64
                    - rev_avg * rev_avg;
                let cost_var: f64 = by_class
                    .iter()
                    .filter(|(k, _)| k.starts_with('6'))
                    .map(|(_, s)| {
                        let n = s.row_count as f64;
                        let m = s.debit_stats.mean;
                        let v = s.debit_stats.std_dev.powi(2);
                        (v + m * m) * n
                    })
                    .sum::<f64>()
                    / cost_count as f64
                    - cost_avg * cost_avg;
                let ratio_std = (rev_var / (cost_avg * cost_avg)
                    + (rev_avg * rev_avg) * cost_var / (cost_avg.powi(4)))
                    .max(0.0)
                    .sqrt();
                let margin_std = (ratio_std * 0.5).clamp(0.05, 0.20);
                patch.set(
                    "master_data.materials.gross_margin_mean",
                    ConfigValue::Float(margin_mean),
                );
                patch.set(
                    "master_data.materials.gross_margin_std",
                    ConfigValue::Float(margin_std),
                );
                patch.set(
                    "master_data.materials.gross_margin_min",
                    ConfigValue::Float((margin_mean - 2.0 * margin_std).clamp(0.05, 0.65)),
                );
                patch.set(
                    "master_data.materials.gross_margin_max",
                    ConfigValue::Float((margin_mean + 2.0 * margin_std).min(0.65).max(0.10)),
                );
            }
        }
    }
}

impl Default for ConfigSynthesizer {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of config synthesis including optional copula generators.
#[derive(Debug)]
pub struct SynthesisResult {
    /// Configuration patch to apply.
    pub config_patch: ConfigPatch,
    /// Copula generators for preserving correlations (if enabled and correlations present).
    pub copula_generators: Vec<CopulaGeneratorSpec>,
}

/// Specification for a copula generator.
#[derive(Debug)]
pub struct CopulaGeneratorSpec {
    /// Name identifier.
    pub name: String,
    /// Table this copula applies to.
    pub table: String,
    /// Column names.
    pub columns: Vec<String>,
    /// The copula generator (ready to use).
    pub generator: CopulaGenerator,
}

impl ConfigSynthesizer {
    /// Synthesize config and copula generators from a fingerprint.
    ///
    /// This is the full synthesis method that also creates copula generators
    /// for preserving correlations.
    pub fn synthesize_full(
        &self,
        fingerprint: &Fingerprint,
        seed: u64,
    ) -> FingerprintResult<SynthesisResult> {
        let config_patch = self.synthesize(fingerprint)?;

        let mut copula_generators = Vec::new();

        if self.options.preserve_correlations {
            // Create copula generators from fingerprint
            if let Some(ref correlations) = fingerprint.correlations {
                // First, try to use pre-built copulas
                for copula in &correlations.copulas {
                    if let Some(generator) = CopulaGenerator::from_copula(copula, seed) {
                        copula_generators.push(CopulaGeneratorSpec {
                            name: copula.name.clone(),
                            table: copula.table.clone(),
                            columns: copula.columns.clone(),
                            generator,
                        });
                    }
                }

                // If no copulas, create from correlation matrices
                if copula_generators.is_empty() {
                    for (table_name, matrix) in &correlations.matrices {
                        if matrix.columns.len() >= 2 {
                            if let Some(generator) =
                                CopulaGenerator::from_correlation_matrix(matrix, seed)
                            {
                                copula_generators.push(CopulaGeneratorSpec {
                                    name: format!("{}_copula", table_name),
                                    table: table_name.clone(),
                                    columns: matrix.columns.clone(),
                                    generator,
                                });
                            }
                        }
                    }
                }
            }
        }

        Ok(SynthesisResult {
            config_patch,
            copula_generators,
        })
    }

    /// Create a copula generator from a Gaussian copula specification.
    pub fn create_copula_generator(copula: &GaussianCopula, seed: u64) -> Option<CopulaGenerator> {
        CopulaGenerator::from_copula(copula, seed)
    }

    /// Create a copula generator from a correlation matrix.
    pub fn create_copula_from_matrix(
        matrix: &CorrelationMatrix,
        seed: u64,
    ) -> Option<CopulaGenerator> {
        CopulaGenerator::from_correlation_matrix(matrix, seed)
    }
}

/// A patch of configuration values to be merged.
#[derive(Debug, Clone, Default)]
pub struct ConfigPatch {
    /// Configuration values keyed by dotted path.
    values: HashMap<String, ConfigValue>,
}

impl ConfigPatch {
    /// Create a new empty patch.
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
        }
    }

    /// Set a configuration value.
    pub fn set(&mut self, path: &str, value: ConfigValue) {
        self.values.insert(path.to_string(), value);
    }

    /// Get a configuration value.
    pub fn get(&self, path: &str) -> Option<&ConfigValue> {
        self.values.get(path)
    }

    /// Get all values.
    pub fn values(&self) -> &HashMap<String, ConfigValue> {
        &self.values
    }

    /// Merge another patch (other takes precedence).
    pub fn merge(&mut self, other: ConfigPatch) {
        self.values.extend(other.values);
    }

    /// Convert to YAML string.
    pub fn to_yaml(&self) -> FingerprintResult<String> {
        // Build nested structure from dotted paths
        let mut root = serde_yaml::Mapping::new();

        for (path, value) in &self.values {
            let parts: Vec<&str> = path.split('.').collect();
            set_nested_value(&mut root, &parts, value);
        }

        Ok(serde_yaml::to_string(&root)?)
    }
}

/// Configuration value types.
#[derive(Debug, Clone)]
pub enum ConfigValue {
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),
    Array(Vec<ConfigValue>),
}

impl ConfigValue {
    /// Convert to YAML value.
    fn to_yaml_value(&self) -> serde_yaml::Value {
        match self {
            Self::Bool(b) => serde_yaml::Value::Bool(*b),
            Self::Integer(i) => serde_yaml::Value::Number(serde_yaml::Number::from(*i)),
            Self::Float(f) => {
                if f.is_finite() {
                    serde_yaml::Value::Number(serde_yaml::Number::from(*f))
                } else {
                    serde_yaml::Value::Null
                }
            }
            Self::String(s) => serde_yaml::Value::String(s.clone()),
            Self::Array(arr) => {
                serde_yaml::Value::Sequence(arr.iter().map(|v| v.to_yaml_value()).collect())
            }
        }
    }
}

/// Set a nested value in a YAML mapping.
fn set_nested_value(root: &mut serde_yaml::Mapping, path: &[&str], value: &ConfigValue) {
    if path.is_empty() {
        return;
    }

    let key = serde_yaml::Value::String(path[0].to_string());

    if path.len() == 1 {
        root.insert(key, value.to_yaml_value());
    } else {
        let entry = root
            .entry(key)
            .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));

        if let serde_yaml::Value::Mapping(ref mut nested) = entry {
            set_nested_value(nested, &path[1..], value);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_config_patch() {
        let mut patch = ConfigPatch::new();
        patch.set("global.seed", ConfigValue::Integer(42));
        patch.set("transactions.count", ConfigValue::Integer(1000));

        assert!(patch.get("global.seed").is_some());

        let yaml = patch.to_yaml().unwrap();
        assert!(yaml.contains("global"));
        assert!(yaml.contains("seed"));
    }
}
