//! Scheme advancer for managing multiple fraud schemes.
//!
//! The SchemeAdvancer coordinates the lifecycle of multiple fraud schemes,
//! handling scheme creation, advancement, and completion.

use chrono::NaiveDate;
use datasynth_core::utils::seeded_rng;
use datasynth_core::uuid_factory::{DeterministicUuidFactory, GeneratorType};
use rand::Rng;
use rand::seq::SliceRandom;
use rand_chacha::ChaCha8Rng;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use std::collections::HashMap;

use datasynth_core::models::{SchemeDetectionStatus, SchemeType};

/// Returns (pathology_name, pathology_category) for RIP-GNN taxonomy.
fn pathology_taxonomy(scheme_type: SchemeType) -> (&'static str, &'static str) {
    use SchemeType::*;
    match scheme_type {
        GradualEmbezzlement => ("GradualEmbezzlement", "Sequential"),
        RevenueManipulation => ("RevenueManipulation", "Volume"),
        VendorKickback => ("VendorKickback", "Relational"),
        TriadBypass => ("TriadBypass", "Relational"),
        ShadowPayroll => ("ShadowPayroll", "Sequential"),
        ExpenseLaundering => ("ExpenseLaundering", "Volume"),
        Smurfing => ("Smurfing", "Volume"),
        CircularFunding => ("CircularFunding", "Relational"),
        PhantomWarehousing => ("PhantomWarehousing", "Relational"),
        IntercompanyWashTrades => ("IntercompanyWashTrades", "Relational"),
        RoundTripping | GhostEmployee | ExpenseReimbursement | InventoryTheft | Custom => {
            (scheme_type.name(), "Relational")
        }
    }
}

use super::schemes::{
    ExpenseLaunderingScheme, FraudScheme, GradualEmbezzlementScheme, RevenueManipulationScheme,
    SchemeAction, SchemeContext, ShadowPayrollScheme, SchemeStatus, SmurfingScheme,
    TriadBypassScheme, VendorKickbackScheme,
};

/// Configuration for scheme generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemeAdvancerConfig {
    /// Probability of starting an embezzlement scheme per period.
    pub embezzlement_probability: f64,
    /// Probability of starting a revenue manipulation scheme per period.
    pub revenue_manipulation_probability: f64,
    /// Probability of starting a kickback scheme per period.
    pub kickback_probability: f64,
    /// Kickback commission as fraction of vendor usual amount (suggests fraud).
    pub kickback_amount_shift_percent: f64,
    /// Vendor typical order amount range when no prior inflated base is recorded.
    pub kickback_vendor_typical_amount_min: f64,
    pub kickback_vendor_typical_amount_max: f64,
    /// Probability of starting a triad bypass scheme per period.
    pub triad_bypass_probability: f64,
    /// Probability of starting a shadow payroll scheme per period.
    pub shadow_payroll_probability: f64,
    /// Probability of starting an expense laundering scheme per period.
    pub expense_laundering_probability: f64,
    /// Probability of starting a smurfing scheme per period.
    pub smurfing_probability: f64,
    /// Probability of starting a circular funding scheme per period.
    pub circular_funding_probability: f64,
    /// Probability of starting a phantom warehousing scheme per period.
    pub phantom_warehousing_probability: f64,
    /// Probability of starting an intercompany wash trade scheme per period.
    pub intercompany_wash_trade_probability: f64,
    /// Maximum number of concurrent schemes **per scheme type**.
    pub max_concurrent_schemes: usize,
    /// Whether to allow the same perpetrator in multiple schemes.
    pub allow_repeat_perpetrators: bool,
    /// Random seed for reproducibility.
    pub seed: u64,
}

impl Default for SchemeAdvancerConfig {
    fn default() -> Self {
        Self {
            embezzlement_probability: 0.02,
            revenue_manipulation_probability: 0.01,
            kickback_probability: 0.01,
            kickback_amount_shift_percent: 0.15,
            kickback_vendor_typical_amount_min: 5_000.0,
            kickback_vendor_typical_amount_max: 50_000.0,
            triad_bypass_probability: 0.005,
            shadow_payroll_probability: 0.005,
            expense_laundering_probability: 0.005,
            smurfing_probability: 0.005,
            circular_funding_probability: 0.005,
            phantom_warehousing_probability: 0.005,
            intercompany_wash_trade_probability: 0.005,
            max_concurrent_schemes: 10      ,
            allow_repeat_perpetrators: false,
            seed: 42,
        }
    }
}

/// Summary of a completed scheme.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedScheme {
    /// Scheme ID.
    pub scheme_id: Uuid,
    /// Scheme type.
    pub scheme_type: SchemeType,
    /// Perpetrator ID.
    pub perpetrator_id: String,
    /// Start date.
    pub start_date: Option<NaiveDate>,
    /// End date.
    pub end_date: NaiveDate,
    /// Final status.
    pub final_status: SchemeStatus,
    /// Detection status.
    pub detection_status: SchemeDetectionStatus,
    /// Total financial impact.
    pub total_impact: Decimal,
    /// Number of stages completed.
    pub stages_completed: u32,
    /// Total transactions.
    pub transaction_count: usize,
}

/// Label for an anomaly that's part of a multi-stage scheme.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiStageAnomalyLabel {
    /// Anomaly ID.
    pub anomaly_id: String,
    /// Scheme ID (scenario id).
    pub scheme_id: Uuid,
    /// Action ID (one per SchemeAction).
    pub action_id: Uuid,
    /// Scheme type.
    pub scheme_type: SchemeType,
    /// Stage number within scheme.
    pub stage_number: u32,
    /// Stage name.
    pub stage_name: String,
    /// Total stages in scheme.
    pub total_stages: u32,
    /// Perpetrator ID.
    pub perpetrator_id: String,
    /// Whether scheme was ultimately detected.
    pub scheme_detected: bool,
    /// Pathology name for RIP-GNN taxonomy (e.g. "Smurfing", "CircularFunding").
    #[serde(default)]
    pub pathology_name: String,
    /// Pathology category: "Sequential", "Volume", or "Relational".
    #[serde(default)]
    pub pathology_category: String,
    /// Date of the action (from SchemeAction.target_date).
    #[serde(default)]
    pub anomaly_date: Option<NaiveDate>,
    /// Involved counterparty (vendor/customer ID) if applicable.
    #[serde(default)]
    pub counterparty: Option<String>,
    /// Action amount if applicable (for display in viewer).
    #[serde(default)]
    pub action_amount: Option<rust_decimal::Decimal>,
}

/// Manages the lifecycle of multiple fraud schemes.
pub struct SchemeAdvancer {
    config: SchemeAdvancerConfig,
    rng: ChaCha8Rng,
    /// Deterministic UUID factory for unique scheme ids.
    scheme_uuid_factory: DeterministicUuidFactory,
    /// Per-scheme RNG state. Keeping state (instead of re-seeding every day)
    /// ensures schemes have realistic variation over time while remaining deterministic.
    scheme_rngs: HashMap<Uuid, ChaCha8Rng>,
    /// Active schemes.
    active_schemes: Vec<Box<dyn FraudScheme>>,
    /// Completed schemes.
    completed_schemes: Vec<CompletedScheme>,
    /// Indices of schemes that finished in advance_all but are not removed until
    /// after the caller records labels (so record_label can still find them).
    pending_removal: Vec<usize>,
    /// Users who are currently perpetrators.
    active_perpetrators: Vec<String>,
    /// Vendors involved in active schemes.
    active_vendors: Vec<String>,
    /// Multi-stage labels generated.
    labels: Vec<MultiStageAnomalyLabel>,
}

impl SchemeAdvancer {
    /// Creates a new scheme advancer.
    pub fn new(config: SchemeAdvancerConfig) -> Self {
        let rng = seeded_rng(config.seed, 0);
        let scheme_uuid_factory = DeterministicUuidFactory::new(config.seed, GeneratorType::Anomaly);
        Self {
            config,
            rng,
            scheme_uuid_factory,
            scheme_rngs: HashMap::new(),
            active_schemes: Vec::new(),
            completed_schemes: Vec::new(),
            pending_removal: Vec::new(),
            active_perpetrators: Vec::new(),
            active_vendors: Vec::new(),
            labels: Vec::new(),
        }
    }

    /// Potentially starts new scheme(s) based on **independent** per-scheme probabilities.
    ///
    /// Each scheme type is evaluated with its own Bernoulli draw (so probabilities do NOT need
    /// to sum to 1.0). If multiple scheme types fire in the same period, we start as many as
    /// allowed by the per-type `max_concurrent_schemes` limit and available perpetrators/vendors.
    pub fn maybe_start_schemes(&mut self, context: &SchemeContext) -> Vec<Uuid> {
        let mut available_users: Vec<String> = if self.config.allow_repeat_perpetrators {
            context.available_users.clone()
        } else {
            context
                .available_users
                .iter()
                .filter(|u| !self.active_perpetrators.contains(u))
                .cloned()
                .collect()
        };

        if available_users.is_empty() {
            return Vec::new();
        }

        // Independent Bernoulli draws per scheme type.
        // Clamp to [0,1] to keep "probability" semantics even if configs exceed 1.0.
        let mut fired: Vec<SchemeType> = Vec::new();
        let mut draw = |t: SchemeType, p: f64| {
            if p > 0.0 && self.rng.random::<f64>() < p.clamp(0.0, 1.0) {
                fired.push(t);
            }
        };
        draw(SchemeType::GradualEmbezzlement, self.config.embezzlement_probability);
        draw(
            SchemeType::RevenueManipulation,
            self.config.revenue_manipulation_probability,
        );
        draw(SchemeType::VendorKickback, self.config.kickback_probability);
        draw(SchemeType::TriadBypass, self.config.triad_bypass_probability);
        draw(SchemeType::ShadowPayroll, self.config.shadow_payroll_probability);
        draw(
            SchemeType::ExpenseLaundering,
            self.config.expense_laundering_probability,
        );
        draw(SchemeType::Smurfing, self.config.smurfing_probability);
        // Circular Funding, Phantom Warehousing, Intercompany Wash Trades are disabled
        // (Single-FEC scope: no multi-entity cyclical patterns).

        if fired.is_empty() {
            return Vec::new();
        }

        // Avoid deterministic bias if many schemes fire in the same period.
        fired.shuffle(&mut self.rng);

        let mut started: Vec<Uuid> = Vec::new();
        for scheme_type in fired {
            // Enforce a per-type **concurrency** cap only: at most
            // `max_concurrent_schemes` active instances of a given
            // scheme type at the same time. Completed schemes do not
            // count toward this limit, so a type can appear again in
            // a later period once earlier instances have finished.
            let active_of_type = self
                .active_schemes
                .iter()
                .filter(|s| s.scheme_type() == scheme_type)
                .count();
            if active_of_type >= self.config.max_concurrent_schemes {
                continue;
            }
            if available_users.is_empty() {
                break;
            }

            // Vendor kickback requires a vendor not already "active" in another kickback scheme.
            let vendor_for_kickback: Option<String> = if scheme_type == SchemeType::VendorKickback {
                let available_vendors: Vec<_> = context
                    .available_counterparties
                    .iter()
                    .filter(|v| !self.active_vendors.contains(v))
                    .cloned()
                    .collect();
                if available_vendors.is_empty() {
                    continue;
                }
                let vendor_idx = self.rng.random_range(0..available_vendors.len());
                Some(available_vendors[vendor_idx].clone())
            } else {
                None
            };

            let user_idx = self.rng.random_range(0..available_users.len());
            let perpetrator = if self.config.allow_repeat_perpetrators {
                available_users[user_idx].clone()
            } else {
                available_users.swap_remove(user_idx)
            };

            let scheme_id = self.scheme_uuid_factory.next();
            let scheme: Box<dyn FraudScheme> = match scheme_type {
                SchemeType::GradualEmbezzlement => {
                    let s = GradualEmbezzlementScheme::new(scheme_id, &perpetrator)
                        .with_accounts(context.available_accounts.clone());
                    Box::new(s)
                }
                SchemeType::RevenueManipulation => Box::new(RevenueManipulationScheme::new(scheme_id, &perpetrator)),
                SchemeType::VendorKickback => {
                    let vendor = vendor_for_kickback.expect("vendor should be set for VendorKickback");
                    let inflation = 0.10 + self.rng.random::<f64>() * 0.15;
                    let s = VendorKickbackScheme::new(scheme_id, &perpetrator, &vendor)
                        .with_inflation_percent(inflation)
                        .with_amount_shift_percent(self.config.kickback_amount_shift_percent)
                        .with_vendor_typical_range(
                            self.config.kickback_vendor_typical_amount_min,
                            self.config.kickback_vendor_typical_amount_max,
                        );
                    self.active_vendors.push(vendor);
                    Box::new(s)
                }
                SchemeType::TriadBypass => Box::new(TriadBypassScheme::new(scheme_id, &perpetrator)),
                SchemeType::ShadowPayroll => Box::new(ShadowPayrollScheme::new(scheme_id, &perpetrator)),
                SchemeType::ExpenseLaundering => Box::new(ExpenseLaunderingScheme::new(scheme_id, &perpetrator)),
                SchemeType::Smurfing => Box::new(SmurfingScheme::new(scheme_id, &perpetrator)),
                // Disabled: Circular Funding, Phantom Warehousing, Intercompany Wash Trades.
                SchemeType::CircularFunding
                | SchemeType::PhantomWarehousing
                | SchemeType::IntercompanyWashTrades => continue,
                // These are currently not wired in the config-driven advancer list.
                _ => continue,
            };

            self.scheme_rngs
                .entry(scheme_id)
                .or_insert_with(|| seeded_rng(self.config.seed, scheme_id.as_u128() as u64));
            self.active_perpetrators.push(perpetrator);
            self.active_schemes.push(scheme);
            started.push(scheme_id);
        }

        started
    }

    /// Backwards-compatible wrapper: starts independent schemes and returns the first started id.
    pub fn maybe_start_scheme(&mut self, context: &SchemeContext) -> Option<Uuid> {
        self.maybe_start_schemes(context).into_iter().next()
    }

    /// Advances all active schemes and returns actions to execute.
    pub fn advance_all(&mut self, context: &SchemeContext) -> Vec<SchemeAction> {
        let mut all_actions = Vec::new();
        let mut schemes_to_complete = Vec::new();

        for (idx, scheme) in self.active_schemes.iter_mut().enumerate() {
            let scheme_id = scheme.scheme_id();
            let scheme_rng = self
                .scheme_rngs
                .entry(scheme_id)
                .or_insert_with(|| seeded_rng(self.config.seed, scheme_id.as_u128() as u64));
            let actions = scheme.advance(context, scheme_rng);
            all_actions.extend(actions);

            // Check if scheme is done
            if matches!(
                scheme.status(),
                SchemeStatus::Completed | SchemeStatus::Terminated | SchemeStatus::Detected
            ) {
                schemes_to_complete.push(idx);
            }
        }

        // Defer removal so the caller can record_label for these actions while
        // the scheme is still in active_schemes. Call flush_completed_schemes after recording.
        self.pending_removal = schemes_to_complete;

        all_actions
    }

    /// Removes schemes that completed in the last advance_all. Call this after
    /// recording labels for the returned actions so record_label can find the scheme.
    pub fn flush_completed_schemes(&mut self, context: &SchemeContext) {
        for idx in self.pending_removal.drain(..).rev() {
            let scheme = self.active_schemes.remove(idx);
            let completed = CompletedScheme {
                scheme_id: scheme.scheme_id(),
                scheme_type: scheme.scheme_type(),
                perpetrator_id: scheme.perpetrator_id().to_string(),
                start_date: scheme.start_date(),
                end_date: context.current_date,
                final_status: scheme.status(),
                detection_status: scheme.detection_status(),
                total_impact: scheme.total_impact(),
                stages_completed: scheme.current_stage_number(),
                transaction_count: scheme.transaction_refs().len(),
            };
            self.active_perpetrators
                .retain(|p| p != scheme.perpetrator_id());
            self.completed_schemes.push(completed);
        }
    }

    /// Records a label for a scheme action.
    pub fn record_label(&mut self, anomaly_id: impl Into<String>, action: &SchemeAction) {
        if let Some(scheme) = self
            .active_schemes
            .iter()
            .find(|s| s.scheme_id() == action.scheme_id)
        {
            let (pathology_name, pathology_category) =
                pathology_taxonomy(scheme.scheme_type());
            let label = MultiStageAnomalyLabel {
                anomaly_id: anomaly_id.into(),
                scheme_id: scheme.scheme_id(),
                action_id: action.action_id,
                scheme_type: scheme.scheme_type(),
                stage_number: action.stage,
                stage_name: scheme.current_stage().name.clone(),
                total_stages: scheme.stages().len() as u32,
                perpetrator_id: scheme.perpetrator_id().to_string(),
                scheme_detected: scheme.detection_status() != SchemeDetectionStatus::Undetected,
                pathology_name: pathology_name.to_string(),
                pathology_category: pathology_category.to_string(),
                anomaly_date: Some(action.target_date),
                counterparty: action.counterparty.clone(),
                action_amount: action.amount,
            };
            self.labels.push(label);
        }
    }

    /// Returns all generated labels.
    pub fn get_labels(&self) -> &[MultiStageAnomalyLabel] {
        &self.labels
    }

    /// Returns completed schemes.
    pub fn get_completed_schemes(&self) -> &[CompletedScheme] {
        &self.completed_schemes
    }

    /// Returns the number of active schemes.
    pub fn active_scheme_count(&self) -> usize {
        self.active_schemes.len()
    }

    /// Returns the number of completed schemes.
    pub fn completed_scheme_count(&self) -> usize {
        self.completed_schemes.len()
    }

    /// Returns active schemes summary.
    pub fn active_schemes_summary(&self) -> Vec<(Uuid, SchemeType, SchemeStatus)> {
        self.active_schemes
            .iter()
            .map(|s| (s.scheme_id(), s.scheme_type(), s.status()))
            .collect()
    }

    /// Gets a specific scheme by ID.
    pub fn get_scheme(&self, scheme_id: Uuid) -> Option<&dyn FraudScheme> {
        self.active_schemes
            .iter()
            .find(|s| s.scheme_id() == scheme_id)
            .map(|s| s.as_ref())
    }

    /// Resets the advancer state.
    pub fn reset(&mut self) {
        self.active_schemes.clear();
        self.completed_schemes.clear();
        self.active_perpetrators.clear();
        self.active_vendors.clear();
        self.labels.clear();
        self.scheme_rngs.clear();
        self.scheme_uuid_factory = DeterministicUuidFactory::new(self.config.seed, GeneratorType::Anomaly);
        self.rng = seeded_rng(self.config.seed, 0);
    }

    /// Returns statistics about schemes.
    pub fn get_statistics(&self) -> SchemeStatistics {
        let total_impact: Decimal = self
            .completed_schemes
            .iter()
            .map(|s| s.total_impact)
            .sum::<Decimal>()
            + self
                .active_schemes
                .iter()
                .map(|s| s.total_impact())
                .sum::<Decimal>();

        let detected_count = self
            .completed_schemes
            .iter()
            .filter(|s| s.detection_status != SchemeDetectionStatus::Undetected)
            .count();

        let by_type = |t: SchemeType| {
            self.completed_schemes
                .iter()
                .filter(|s| s.scheme_type == t)
                .count()
                + self
                    .active_schemes
                    .iter()
                    .filter(|s| s.scheme_type() == t)
                    .count()
        };

        SchemeStatistics {
            total_schemes: self.active_schemes.len() + self.completed_schemes.len(),
            active_schemes: self.active_schemes.len(),
            completed_schemes: self.completed_schemes.len(),
            detected_schemes: detected_count,
            total_impact,
            embezzlement_count: by_type(SchemeType::GradualEmbezzlement),
            revenue_manipulation_count: by_type(SchemeType::RevenueManipulation),
            kickback_count: by_type(SchemeType::VendorKickback),
            triad_bypass_count: by_type(SchemeType::TriadBypass),
            shadow_payroll_count: by_type(SchemeType::ShadowPayroll),
            expense_laundering_count: by_type(SchemeType::ExpenseLaundering),
            smurfing_count: by_type(SchemeType::Smurfing),
            circular_funding_count: by_type(SchemeType::CircularFunding),
            phantom_warehousing_count: by_type(SchemeType::PhantomWarehousing),
            intercompany_wash_trade_count: by_type(SchemeType::IntercompanyWashTrades),
        }
    }
}

/// Statistics about fraud schemes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemeStatistics {
    /// Total number of schemes (active + completed).
    pub total_schemes: usize,
    /// Number of active schemes.
    pub active_schemes: usize,
    /// Number of completed schemes.
    pub completed_schemes: usize,
    /// Number of detected schemes.
    pub detected_schemes: usize,
    /// Total financial impact.
    pub total_impact: Decimal,
    /// Number of embezzlement schemes.
    pub embezzlement_count: usize,
    /// Number of revenue manipulation schemes.
    pub revenue_manipulation_count: usize,
    /// Number of kickback schemes.
    pub kickback_count: usize,
    /// Number of triad bypass schemes.
    pub triad_bypass_count: usize,
    /// Number of shadow payroll schemes.
    pub shadow_payroll_count: usize,
    /// Number of expense laundering schemes.
    pub expense_laundering_count: usize,
    /// Number of smurfing schemes.
    pub smurfing_count: usize,
    /// Number of circular funding schemes.
    pub circular_funding_count: usize,
    /// Number of phantom warehousing schemes.
    pub phantom_warehousing_count: usize,
    /// Number of intercompany wash trade schemes.
    pub intercompany_wash_trade_count: usize,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_scheme_advancer_creation() {
        let advancer = SchemeAdvancer::new(SchemeAdvancerConfig::default());
        assert_eq!(advancer.active_scheme_count(), 0);
        assert_eq!(advancer.completed_scheme_count(), 0);
    }

    #[test]
    fn test_scheme_advancer_start_scheme() {
        let mut advancer = SchemeAdvancer::new(SchemeAdvancerConfig {
            embezzlement_probability: 1.0, // Always start
            ..Default::default()
        });

        let context = SchemeContext::new(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(), "1000")
            .with_users(vec!["USER001".to_string(), "USER002".to_string()])
            .with_accounts(vec!["5000".to_string()]);

        let scheme_id = advancer.maybe_start_scheme(&context);
        assert!(scheme_id.is_some());
        assert_eq!(advancer.active_scheme_count(), 1);
    }

    #[test]
    fn test_scheme_advancer_max_concurrent() {
        let mut advancer = SchemeAdvancer::new(SchemeAdvancerConfig {
            embezzlement_probability: 1.0,
            max_concurrent_schemes: 2,
            ..Default::default()
        });

        let context = SchemeContext::new(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(), "1000")
            .with_users(vec![
                "USER001".to_string(),
                "USER002".to_string(),
                "USER003".to_string(),
            ])
            .with_accounts(vec!["5000".to_string()]);

        // Start schemes up to max
        advancer.maybe_start_scheme(&context);
        advancer.maybe_start_scheme(&context);
        let third = advancer.maybe_start_scheme(&context);

        assert_eq!(advancer.active_scheme_count(), 2);
        assert!(third.is_none()); // Should not start third due to max
    }

    #[test]
    fn test_scheme_advancer_advance_all() {
        let mut advancer = SchemeAdvancer::new(SchemeAdvancerConfig {
            embezzlement_probability: 1.0,
            ..Default::default()
        });

        let context = SchemeContext::new(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(), "1000")
            .with_users(vec!["USER001".to_string()])
            .with_accounts(vec!["5000".to_string()]);

        advancer.maybe_start_scheme(&context);

        // Advance for several days
        for day in 0..30 {
            let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap() + chrono::Duration::days(day);
            let mut ctx = context.clone();
            ctx.current_date = date;

            let _actions = advancer.advance_all(&ctx);
        }

        assert_eq!(advancer.active_scheme_count(), 1);
    }

    #[test]
    fn test_scheme_advancer_statistics() {
        let mut advancer = SchemeAdvancer::new(SchemeAdvancerConfig {
            embezzlement_probability: 1.0,
            ..Default::default()
        });

        let context = SchemeContext::new(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(), "1000")
            .with_users(vec!["USER001".to_string()])
            .with_accounts(vec!["5000".to_string()]);

        advancer.maybe_start_scheme(&context);

        let stats = advancer.get_statistics();
        assert_eq!(stats.total_schemes, 1);
        assert_eq!(stats.active_schemes, 1);
        assert_eq!(stats.embezzlement_count, 1);
    }
}
