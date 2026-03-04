//! Config synthesizer - converts fingerprints to generator configs.

use std::collections::HashMap;

use datasynth_core::AmountDistributionConfig;

use super::distribution_fitter::{estimate_lognormal_params, fit_account_class_distributions, FittedDistribution};
use super::CopulaGenerator;
use crate::error::FingerprintResult;
use crate::models::{
    CorrelationMatrix, DistributionType, Fingerprint, GaussianCopula, NumericStats,
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

        // Map numeric distributions to amount config
        for (key, stats) in &fingerprint.statistics.numeric_columns {
            if key.contains("amount") || key.contains("value") || key.contains("price") {
                let amount_config = self.map_numeric_distribution(stats);
                for (k, v) in amount_config {
                    patch.set(&format!("transactions.amounts.{}", k), v);
                }
                break; // Use first matching column
            }
        }

        // Map anomaly rates if present and enabled
        if self.options.inject_anomalies {
            if let Some(ref anomalies) = fingerprint.anomalies {
                let rate = anomalies.overall.anomaly_rate;
                patch.set("anomaly_injection.overall_rate", ConfigValue::Float(rate));
                patch.set("anomaly_injection.enabled", ConfigValue::Bool(rate > 0.0));
            }
        }

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
    /// Account class proportions (e.g. "1XXX" -> 0.2) from fingerprint for biasing JE account selection.
    pub account_class_proportions: Option<HashMap<String, f64>>,
    /// Per-account-class amount distribution configs for sampling amounts by class (from fingerprint numeric stats).
    pub account_class_amount_configs: Option<HashMap<String, AmountDistributionConfig>>,
}

/// Convert fitted per-class distribution to AmountDistributionConfig for sampling.
fn fitted_distribution_to_amount_config(
    dist: &FittedDistribution,
) -> Option<AmountDistributionConfig> {
    use crate::models::DistributionType;
    let (mu, sigma) = match dist.distribution_type {
        DistributionType::LogNormal => {
            let mu = dist.params.param1?;
            let sigma = dist.params.param2?;
            if sigma <= 0.0 {
                return None;
            }
            (mu, sigma)
        }
        _ => {
            if dist.mean <= 0.0 {
                return None;
            }
            let variance = dist.std_dev * dist.std_dev;
            estimate_lognormal_params(dist.mean, variance)
        }
    };
    let min_amount = dist.min.max(0.01);
    let max_amount = dist.max.max(min_amount * 1.01);
    Some(AmountDistributionConfig {
        min_amount,
        max_amount,
        lognormal_mu: mu,
        lognormal_sigma: sigma,
        decimal_places: 2,
        round_number_probability: 0.2,
        nice_number_probability: 0.15,
    })
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

        // Account class proportions (first 3 digits / class pattern) for JE account selection
        let account_class_proportions = Self::account_class_proportions_from_fingerprint(fingerprint);

        // Per-class amount distributions from fingerprint (numeric stats per account class)
        let account_class_amount_configs =
            Self::account_class_amount_configs_from_fingerprint(fingerprint);

        Ok(SynthesisResult {
            config_patch,
            copula_generators,
            account_class_proportions,
            account_class_amount_configs,
        })
    }

    /// Build per-account-class amount configs from fingerprint for JE amount sampling.
    /// Aggregates 3-digit patterns to 1XXX: for each first digit, uses the 3-digit class with
    /// the largest row_count as representative so the generator can look up by 6XXX, 7XXX.
    fn account_class_amount_configs_from_fingerprint(
        fingerprint: &Fingerprint,
    ) -> Option<HashMap<String, AmountDistributionConfig>> {
        let stats = &fingerprint.statistics.account_class_stats;
        if stats.is_empty() {
            return None;
        }
        let fitted = fit_account_class_distributions(stats).ok()?;
        if fitted.is_empty() {
            return None;
        }
        let mut by_1xxx: HashMap<String, (u64, AmountDistributionConfig)> = HashMap::new();
        for (pattern, dist) in fitted {
            let key_1xxx = match Self::pattern_to_1xxx(&pattern) {
                Some(k) => k,
                None => continue,
            };
            let cfg = fitted_distribution_to_amount_config(&dist)?;
            let row_count = stats.iter().find(|s| s.class_pattern == pattern)?.row_count;
            let entry = by_1xxx.entry(key_1xxx).or_insert((0, cfg));
            if row_count > entry.0 {
                entry.0 = row_count;
                entry.1 = fitted_distribution_to_amount_config(&dist).unwrap();
            }
        }
        let map: HashMap<String, AmountDistributionConfig> = by_1xxx
            .into_iter()
            .map(|(k, (_, cfg))| (k, cfg))
            .collect();
        if map.is_empty() {
            return None;
        }
        Some(map)
    }

    /// Normalize class pattern to 1XXX for generator (first digit + "XXX").
    fn pattern_to_1xxx(pattern: &str) -> Option<String> {
        pattern.trim().chars().next().filter(|c| c.is_ascii_digit()).map(|d| format!("{}XXX", d))
    }

    /// Build account-class proportions (e.g. "1XXX" -> 0.2) from fingerprint for JE account selection.
    /// Aggregates 3-digit patterns (411, 512) to 1XXX so the generator can look up by 6XXX, 7XXX.
    fn account_class_proportions_from_fingerprint(
        fingerprint: &Fingerprint,
    ) -> Option<HashMap<String, f64>> {
        let stats = &fingerprint.statistics.account_class_stats;
        if stats.is_empty() {
            return None;
        }
        let total: u64 = stats.iter().map(|s| s.row_count).sum();
        if total == 0 {
            return None;
        }
        let total_f = total as f64;
        let mut by_1xxx: HashMap<String, u64> = HashMap::new();
        for s in stats {
            if let Some(key) = Self::pattern_to_1xxx(&s.class_pattern) {
                *by_1xxx.entry(key).or_insert(0) += s.row_count;
            }
        }
        let map = by_1xxx
            .into_iter()
            .map(|(k, count)| (k, count as f64 / total_f))
            .collect();
        Some(map)
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
