//! Vendor kickback fraud scheme.
//!
//! Models a kickback scheme where an employee colludes with a vendor
//! to inflate invoices and receive a portion of the excess payment.

use chrono::NaiveDate;
use rand::Rng;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// scheme_id is provided by the advancer to avoid collisions across schemes.

use datasynth_core::models::{
    AnomalyDetectionDifficulty, ConcealmentTechnique, SchemeDetectionStatus, SchemeType,
};

use super::scheme::{
    FraudScheme, SchemeAction, SchemeActionType, SchemeContext, SchemeStage, SchemeStatus,
    SchemeTransactionRef,
};

/// A vendor kickback scheme involving price inflation.
///
/// Stages:
/// 1. Setup (3 months): Establish vendor relationship
/// 2. Price inflation (12 months): Inflate invoices 10-25%
/// 3. Kickback payments (ongoing): Receive portion of inflated amounts
/// 4. Concealment (ongoing): Hide the relationship
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VendorKickbackScheme {
    /// Unique scheme ID.
    pub scheme_id: Uuid,
    /// Employee perpetrator ID.
    pub perpetrator_id: String,
    /// Colluding vendor ID.
    pub vendor_id: String,
    /// Start date.
    pub start_date: Option<NaiveDate>,
    /// Current stage index.
    current_stage_index: usize,
    /// All stages.
    stages: Vec<SchemeStage>,
    /// Transaction references.
    transactions: Vec<SchemeTransactionRef>,
    /// Total impact (inflated amounts).
    total_impact: Decimal,
    /// Total kickback payments emitted so far.
    total_kickbacks: Decimal,
    /// Whether at least one kickback payment (stage 3/4) has been emitted.
    has_emitted_kickback: bool,
    /// Number of explicit kickback JEs emitted in stage 3/4 (cap with stage.transaction_count_max).
    kickback_payment_count: u32,
    /// Status.
    status: SchemeStatus,
    /// Detection status.
    detection_status: SchemeDetectionStatus,
    /// Detection probability.
    detection_probability: f64,
    /// Price inflation percentage (10-25%).
    inflation_percent: f64,
    /// Kickback percentage (typically 50% of inflation).
    kickback_percent: f64,
    /// Commission / amount shift as fraction of vendor's usual (base) amount — suggests the fraud.
    amount_shift_percent: f64,
    /// Typical base amount for this vendor (from first or recent inflated invoice); used to compute kickback amount.
    vendor_typical_base_amount: Option<Decimal>,
    /// Fallback range for "usual" amount when no inflated invoice has been recorded yet.
    vendor_typical_amount_min: Decimal,
    vendor_typical_amount_max: Decimal,
    /// Legitimate vendor transactions to blend in.
    legitimate_transaction_count: u32,
    /// Inflated transaction count.
    inflated_transaction_count: u32,
    /// Probability per advance to emit one inflated-invoice action in the operation stage (configurable for shorter schemes).
    inflation_action_probability: f64,
}

impl VendorKickbackScheme {
    /// Creates a new vendor kickback scheme.
    pub fn new(
        scheme_id: Uuid,
        perpetrator_id: impl Into<String>,
        vendor_id: impl Into<String>,
    ) -> Self {
        let stages = vec![
            // Stage 1: Setup (3 months)
            SchemeStage::new(
                1,
                "setup",
                3,
                (dec!(0), dec!(0)), // No fraudulent amounts during setup
                (0, 1),
                AnomalyDetectionDifficulty::Expert, // Setup is very hard to detect
            )
            .with_description("Establish vendor relationship and trust")
            .with_technique(ConcealmentTechnique::FalseDocumentation),
            // Stage 2: Price inflation (12 months)
            SchemeStage::new(
                2,
                "price_inflation",
                12,
                (dec!(5000), dec!(100000)),
                // Fewer inflated invoices per scheme so JE counts stay closer to ground truth per vendor.
                (1, 2),
                AnomalyDetectionDifficulty::Hard,
            )
            .with_description("Inflate invoice amounts 10-25%")
            .with_technique(ConcealmentTechnique::DocumentManipulation)
            .with_technique(ConcealmentTechnique::Collusion),
            // Stage 3: Kickback payments (6 months)
            SchemeStage::new(
                3,
                "kickback_payments",
                6,
                (dec!(500), dec!(25000)),
                // Exactly one explicit kickback payment JE per scheme by default.
                (1, 1),
                AnomalyDetectionDifficulty::Moderate,
            )
            .with_description("Receive kickback payments from vendor")
            .with_technique(ConcealmentTechnique::Collusion)
            .with_technique(ConcealmentTechnique::TimingExploitation),
            // Stage 4: Concealment (3 months)
            SchemeStage::new(
                4,
                "concealment",
                3,
                (dec!(0), dec!(0)),
                (0, 1),
                AnomalyDetectionDifficulty::Hard,
            )
            .with_description("Cover tracks and maintain relationship")
            .with_technique(ConcealmentTechnique::DataAlteration)
            .with_technique(ConcealmentTechnique::FalseDocumentation),
        ];

        Self {
            scheme_id,
            perpetrator_id: perpetrator_id.into(),
            vendor_id: vendor_id.into(),
            start_date: None,
            current_stage_index: 0,
            stages,
            transactions: Vec::new(),
            total_impact: Decimal::ZERO,
            total_kickbacks: Decimal::ZERO,
            has_emitted_kickback: false,
            kickback_payment_count: 0,
            status: SchemeStatus::NotStarted,
            detection_status: SchemeDetectionStatus::Undetected,
            detection_probability: 0.0,
            inflation_percent: 0.15, // 15% default
            kickback_percent: 0.50,  // 50% of inflation
            amount_shift_percent: 0.15,
            vendor_typical_base_amount: None,
            vendor_typical_amount_min: dec!(5000),
            vendor_typical_amount_max: dec!(50000),
            legitimate_transaction_count: 0,
            inflated_transaction_count: 0,
            inflation_action_probability: 0.2,
        }
    }

    /// Sets stage durations from config (shorter values = fewer JEs per scheme).
    pub fn with_stage_durations(
        mut self,
        setup_months: u32,
        operation_months: u32,
        kickback_payments_months: u32,
        concealment_months: u32,
    ) -> Self {
        if self.stages.len() >= 4 {
            self.stages[0].duration_months = setup_months;
            self.stages[1].duration_months = operation_months;
            self.stages[2].duration_months = kickback_payments_months;
            self.stages[3].duration_months = concealment_months;
        }
        self
    }

    /// Sets the probability per advance to emit one inflated invoice in the operation stage (lower = fewer JEs).
    pub fn with_inflation_action_probability(mut self, p: f64) -> Self {
        self.inflation_action_probability = p.clamp(0.01, 1.0);
        self
    }

    /// Sets the inflation percentage (0.10 to 0.25).
    pub fn with_inflation_percent(mut self, percent: f64) -> Self {
        self.inflation_percent = percent.clamp(0.10, 0.25);
        self
    }

    /// Sets the kickback percentage (0.30 to 0.70).
    pub fn with_kickback_percent(mut self, percent: f64) -> Self {
        self.kickback_percent = percent.clamp(0.30, 0.70);
        self
    }

    /// Sets the commission / amount shift as fraction of vendor's usual amount (e.g. 0.10–0.25).
    pub fn with_amount_shift_percent(mut self, percent: f64) -> Self {
        self.amount_shift_percent = percent.clamp(0.05, 0.50);
        self
    }

    /// Sets the fallback range for vendor "usual" amount when no prior inflated invoice exists.
    pub fn with_vendor_typical_range(mut self, min_amount: f64, max_amount: f64) -> Self {
        self.vendor_typical_amount_min = Decimal::from_f64_retain(min_amount.min(max_amount))
            .unwrap_or(dec!(5000));
        self.vendor_typical_amount_max = Decimal::from_f64_retain(min_amount.max(max_amount))
            .unwrap_or(dec!(50000));
        self
    }

    /// Starts the scheme.
    pub fn start(&mut self, date: NaiveDate) {
        self.start_date = Some(date);
        self.status = SchemeStatus::Active;
    }

    /// Gets the stage end date.
    fn stage_end_date(&self) -> Option<NaiveDate> {
        self.start_date.map(|start| {
            let months_elapsed: u32 = self.stages[..self.current_stage_index]
                .iter()
                .map(|s| s.duration_months)
                .sum();
            let stage_months = self.stages[self.current_stage_index].duration_months;
            start + chrono::Months::new(months_elapsed + stage_months)
        })
    }

    /// Checks if stage should advance.
    fn should_advance_stage(&self, current_date: NaiveDate) -> bool {
        if let Some(end_date) = self.stage_end_date() {
            current_date >= end_date && self.current_stage_index < self.stages.len() - 1
        } else {
            false
        }
    }

    /// Advances to next stage.
    fn advance_stage(&mut self) {
        if self.current_stage_index < self.stages.len() - 1 {
            self.current_stage_index += 1;
        }
    }

    /// Calculates the inflation amount for a base invoice amount.
    pub fn calculate_inflation(&self, base_amount: Decimal) -> Decimal {
        let inflation_factor =
            Decimal::from_f64_retain(self.inflation_percent).unwrap_or(dec!(0.15));
        base_amount * inflation_factor
    }

    /// Calculates the kickback amount from an inflated amount.
    pub fn calculate_kickback(&self, inflated_amount: Decimal) -> Decimal {
        let kickback_factor = Decimal::from_f64_retain(self.kickback_percent).unwrap_or(dec!(0.50));
        inflated_amount * kickback_factor
    }

    /// Updates detection probability.
    fn update_detection_probability(&mut self) {
        let base_prob = 0.1;

        // Vendor concentration increases detection risk
        let concentration_factor = if self.inflated_transaction_count > 20 {
            0.15
        } else if self.inflated_transaction_count > 10 {
            0.10
        } else {
            0.0
        };

        // Inflation level affects detection
        let inflation_factor = if self.inflation_percent > 0.20 {
            0.15
        } else if self.inflation_percent > 0.15 {
            0.10
        } else {
            0.05
        };

        // Total amount affects detection
        let amount_factor = if self.total_impact > dec!(500000) {
            0.20
        } else if self.total_impact > dec!(100000) {
            0.10
        } else {
            0.0
        };

        // Kickback stage is riskier
        let stage_factor = if self.current_stage_index == 2 {
            0.15
        } else {
            0.0
        };

        let prob: f64 =
            base_prob + concentration_factor + inflation_factor + amount_factor + stage_factor;
        self.detection_probability = prob.min(0.85);
    }
}

impl FraudScheme for VendorKickbackScheme {
    fn scheme_type(&self) -> SchemeType {
        SchemeType::VendorKickback
    }

    fn scheme_id(&self) -> Uuid {
        self.scheme_id
    }

    fn current_stage(&self) -> &SchemeStage {
        &self.stages[self.current_stage_index]
    }

    fn current_stage_number(&self) -> u32 {
        self.stages[self.current_stage_index].stage_number
    }

    fn stages(&self) -> &[SchemeStage] {
        &self.stages
    }

    fn status(&self) -> SchemeStatus {
        self.status
    }

    fn detection_status(&self) -> SchemeDetectionStatus {
        self.detection_status
    }

    fn advance(
        &mut self,
        context: &SchemeContext,
        rng: &mut dyn rand::RngCore,
    ) -> Vec<SchemeAction> {
        let mut actions = Vec::new();

        // Start if not started
        if self.status == SchemeStatus::NotStarted {
            self.start(context.current_date);
        }

        // Check termination
        if self.should_terminate(context) {
            self.status = SchemeStatus::Terminated;
            return actions;
        }

        // Check detection
        if rng.random::<f64>() < self.detection_probability * context.detection_activity {
            self.detection_status = SchemeDetectionStatus::UnderInvestigation;
            if rng.random::<f64>() < 0.3 {
                self.status = SchemeStatus::Detected;
                return actions;
            }
        }

        // Pause during audit
        if context.audit_in_progress && rng.random::<f64>() < 0.6 {
            self.status = SchemeStatus::Paused;
            return actions;
        }
        self.status = SchemeStatus::Active;

        // Check stage advancement
        if self.should_advance_stage(context.current_date) {
            self.advance_stage();
        }

        // Clone current stage so we can freely mutate self later (record_transaction)
        // without running into Rust's borrow checker for &self.stages[..].
        let stage = self.stages[self.current_stage_index].clone();

        // Generate actions based on stage
        match self.current_stage_index {
            0 => {
                // Setup stage - create fictitious vendor if needed
                if rng.random::<f64>() < 0.1 {
                    let action = SchemeAction::new(
                        self.scheme_id,
                        stage.stage_number,
                        SchemeActionType::CreateFictitiousVendor,
                        context.current_date,
                    )
                    .with_scheme_type(self.scheme_type())
                    .with_counterparty(&self.vendor_id)
                    .with_user(&self.perpetrator_id)
                    .with_difficulty(stage.detection_difficulty)
                    .with_description("Establish vendor relationship for kickback scheme");

                    actions.push(action);
                }
            }
            1 => {
                // Price inflation stage — full P2P cycle: inflated invoice (billing) + payment with VAT.
                // Base amount = vendor's "usual" order; total_amount = inflated gross (what company pays).
                // Respect stage.transaction_count_max so we don't flood schemes with hundreds of invoices.
                if self.inflated_transaction_count < stage.transaction_count_max
                    && rng.random::<f64>() < self.inflation_action_probability
                {
                    // Use fingerprint distribution so inflated amounts stay in-distribution (not easy outliers)
                    let base_amount = context
                        .sample_amount_from_fingerprint(rng, Some("6XXX"))
                        .unwrap_or_else(|| stage.random_amount(rng));
                    let inflation = self.calculate_inflation(base_amount);
                    let total_amount = base_amount + inflation;

                    // Record vendor's usual (base) amount so kickback payments are calculated from it (amount_shift_percent).
                    self.vendor_typical_base_amount = Some(base_amount);

                    // Shared reference to link invoice JE and payment JE (end-to-end P2P).
                    let invoice_ref = format!(
                        "KB-INV-{:08x}-{}",
                        self.scheme_id.as_u128() as u32,
                        self.inflated_transaction_count
                    );

                    // 1) InflateInvoice → materialized as invoice posting JE (Dr Expense, Dr VAT, Cr AP).
                    let mut inv_action = SchemeAction::new(
                        self.scheme_id,
                        stage.stage_number,
                        SchemeActionType::InflateInvoice,
                        context.current_date,
                    )
                    .with_scheme_type(self.scheme_type())
                    .with_amount(total_amount)
                    .with_counterparty(&self.vendor_id)
                    .with_user(&self.perpetrator_id)
                    .with_difficulty(stage.detection_difficulty)
                    .with_reference(&invoice_ref)
                    .with_description(format!(
                        "Inflated invoice (P2P) - base: {}, inflation: {}, gross: {}",
                        base_amount, inflation, total_amount
                    ));

                    // Treat the inflation portion as the scheme's financial impact so that
                    // later kickback stages (3/4) have a non-zero total_impact to work from.
                    let tx_ref = SchemeTransactionRef::new(
                        invoice_ref.clone(),
                        context.current_date,
                        inflation,
                        stage.stage_number,
                    )
                    .with_action(inv_action.action_id);
                    self.record_transaction(tx_ref);

                    for technique in &stage.concealment_techniques {
                        inv_action = inv_action.with_technique(*technique);
                    }

                    self.inflated_transaction_count += 1;
                    actions.push(inv_action);

                    // 2) Split payment into 2-3 partial payments at T+10, T+15, T+20
                    let n_parts = rng.random_range(2u32..=3);
                    let splits: Vec<f64> = if n_parts == 2 {
                        vec![0.60, 0.40]
                    } else {
                        vec![0.50, 0.30, 0.20]
                    };
                    let offsets: Vec<i64> = vec![10, 15, 20];
                    for (i, (frac, days_offset)) in splits.iter().zip(offsets.iter()).enumerate() {
                        let part_amount = Decimal::from_f64_retain(
                            f64::try_from(total_amount).unwrap_or(1000.0) * frac
                        ).unwrap_or(total_amount / Decimal::from(n_parts));
                        if part_amount <= Decimal::ZERO {
                            continue;
                        }
                        let payment_date =
                            context.current_date + chrono::Duration::days(*days_offset);
                        let mut pay_action = SchemeAction::new(
                            self.scheme_id,
                            stage.stage_number,
                            SchemeActionType::PayInflatedInvoicePartial,
                            payment_date,
                        )
                        .with_scheme_type(self.scheme_type())
                        .with_amount(part_amount)
                        .with_counterparty(&self.vendor_id)
                        .with_user(&self.perpetrator_id)
                        .with_difficulty(stage.detection_difficulty)
                        .with_reference(&invoice_ref)
                        .with_description(format!(
                            "Partial payment {}/{} of inflated invoice {} (P2P)",
                            i + 1, n_parts, invoice_ref
                        ));

                        for technique in &stage.concealment_techniques {
                            pay_action = pay_action.with_technique(*technique);
                        }

                        actions.push(pay_action);
                    }
                }
            }
            2 => {
                // Kickback payment stage — amount = vendor's usual (base) amount × configurable amount_shift_percent (commission).
                // Guarantee at least one kickback payment per scheme whenever total_impact > 0,
                // then fall back to probabilistic emissions (15%) for any additional payments,
                // but never exceed stage.transaction_count_max payments.
                if self.total_impact > Decimal::ZERO
                    && self.kickback_payment_count < stage.transaction_count_max
                {
                    let must_emit_once = !self.has_emitted_kickback;
                    let should_emit = must_emit_once || rng.random::<f64>() < 0.15;
                    if should_emit {
                    let usual_amount = self.vendor_typical_base_amount.unwrap_or_else(|| {
                        let min_f = self.vendor_typical_amount_min.try_into().unwrap_or(5000.0);
                        let max_f = self.vendor_typical_amount_max.try_into().unwrap_or(50000.0);
                        let t = rng.random::<f64>();
                        Decimal::from_f64_retain(min_f + t * (max_f - min_f)).unwrap_or(self.vendor_typical_amount_min)
                    });
                    let shift = Decimal::from_f64_retain(self.amount_shift_percent).unwrap_or(dec!(0.15));
                    let raw_kickback = usual_amount * shift;
                    let max_kickback = self.total_impact
                        * Decimal::from_f64_retain(self.kickback_percent * self.inflation_percent)
                            .unwrap_or(dec!(0.075));
                    let remaining = (max_kickback - self.total_kickbacks).max(Decimal::ZERO);
                    let kickback_amount = raw_kickback.min(remaining).max(dec!(500));

                    if kickback_amount > Decimal::ZERO {
                        let mut action = SchemeAction::new(
                            self.scheme_id,
                            stage.stage_number,
                            SchemeActionType::MakeKickbackPayment,
                            context.current_date,
                        )
                        .with_scheme_type(self.scheme_type())
                        .with_amount(kickback_amount)
                        .with_counterparty(&self.vendor_id)
                        .with_user(&self.perpetrator_id)
                        .with_difficulty(stage.detection_difficulty)
                        .with_description(format!(
                            "Kickback payment from vendor {}",
                            self.vendor_id
                        ));

                        for technique in &stage.concealment_techniques {
                            action = action.with_technique(*technique);
                        }

                        self.total_kickbacks += kickback_amount;
                        self.has_emitted_kickback = true;
                        self.kickback_payment_count += 1;
                        actions.push(action);
                    }
                }
            }
            }
            3 => {
                // Concealment stage
                if rng.random::<f64>() < 0.05 {
                    let action = SchemeAction::new(
                        self.scheme_id,
                        stage.stage_number,
                        SchemeActionType::CoverUp,
                        context.current_date,
                    )
                    .with_scheme_type(self.scheme_type())
                    .with_user(&self.perpetrator_id)
                    .with_difficulty(stage.detection_difficulty)
                    .with_description("Cover up kickback scheme evidence")
                    .with_technique(ConcealmentTechnique::DataAlteration);

                    actions.push(action);
                }
            }
            _ => {}
        }

        // Update detection probability
        self.update_detection_probability();

        // Check completion
        if self.current_stage_index == self.stages.len() - 1 {
            if let Some(end_date) = self.stage_end_date() {
                if context.current_date >= end_date {
                    self.status = SchemeStatus::Completed;
                }
            }
        }

        actions
    }

    fn detection_probability(&self) -> f64 {
        self.detection_probability
    }

    fn total_impact(&self) -> Decimal {
        self.total_impact
    }

    fn should_terminate(&self, context: &SchemeContext) -> bool {
        // Kickback schemes are more sensitive to detection
        if context.detection_activity > 0.7 {
            return true;
        }

        if self.detection_status != SchemeDetectionStatus::Undetected
            && self.detection_status == SchemeDetectionStatus::FullyDetected
        {
            return true;
        }
        // Under investigation - might continue if they think they can beat it

        false
    }

    fn perpetrator_id(&self) -> &str {
        &self.perpetrator_id
    }

    fn start_date(&self) -> Option<NaiveDate> {
        self.start_date
    }

    fn transaction_refs(&self) -> &[SchemeTransactionRef] {
        &self.transactions
    }

    fn record_transaction(&mut self, transaction: SchemeTransactionRef) {
        self.total_impact += transaction.amount;
        self.transactions.push(transaction);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_kickback_scheme_creation() {
        let scheme = VendorKickbackScheme::new(Uuid::nil(), "EMP001", "VENDOR001")
            .with_inflation_percent(0.20)
            .with_kickback_percent(0.50);

        assert_eq!(scheme.perpetrator_id, "EMP001");
        assert_eq!(scheme.vendor_id, "VENDOR001");
        assert!((scheme.inflation_percent - 0.20).abs() < 0.01);
        assert_eq!(scheme.stages.len(), 4);
    }

    #[test]
    fn test_kickback_scheme_stages() {
        let scheme = VendorKickbackScheme::new(Uuid::nil(), "EMP001", "VENDOR001");

        assert_eq!(scheme.stages[0].name, "setup");
        assert_eq!(scheme.stages[1].name, "price_inflation");
        assert_eq!(scheme.stages[2].name, "kickback_payments");
        assert_eq!(scheme.stages[3].name, "concealment");
    }

    #[test]
    fn test_inflation_calculation() {
        let scheme = VendorKickbackScheme::new(Uuid::nil(), "EMP001", "VENDOR001")
            .with_inflation_percent(0.20);

        let base = dec!(10000);
        let inflation = scheme.calculate_inflation(base);

        // Use approximate comparison due to floating point conversion
        let expected = dec!(2000);
        let diff = (inflation - expected).abs();
        assert!(
            diff < dec!(0.01),
            "Expected ~{}, got {}",
            expected,
            inflation
        );
    }

    #[test]
    fn test_kickback_calculation() {
        let scheme = VendorKickbackScheme::new(Uuid::nil(), "EMP001", "VENDOR001")
            .with_kickback_percent(0.50);

        let inflated = dec!(2000);
        let kickback = scheme.calculate_kickback(inflated);

        assert_eq!(kickback, dec!(1000));
    }

    #[test]
    fn test_kickback_scheme_advance() {
        let mut scheme = VendorKickbackScheme::new(Uuid::nil(), "EMP001", "VENDOR001");
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let context = SchemeContext::new(NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(), "1000");

        // Advance multiple times
        let mut total_actions = 0;
        for day in 0..100 {
            let date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap() + chrono::Duration::days(day);
            let mut ctx = context.clone();
            ctx.current_date = date;

            let actions = scheme.advance(&ctx, &mut rng);
            total_actions += actions.len();
        }

        // total_actions is usize, inherently non-negative
        let _ = total_actions; // May or may not have actions depending on RNG
        assert_eq!(scheme.status, SchemeStatus::Active);
    }
}
