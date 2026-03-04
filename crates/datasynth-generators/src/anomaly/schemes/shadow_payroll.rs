//! Shadow payroll (ghost worker) fraud scheme.
//!
//! A fictitious employee is added to HR master data; monthly payroll flows to a ghost bank account.
//!
//! v2.0 changes:
//! - Ghost employee ID uses realistic `EMP-{n:05}` format (not `GHOST-EMP-{uuid}`).
//! - Stage 0: emits `GhostHire` admin action (no financial JE).
//! - Stage 1: emits `MonthlyPayroll` on the last business day of each month.
//!   Social-charge ratio is sampled from N(0.42, 0.03) clamped to [0.36, 0.50].
//! - Stage 2: emits `GhostTermination` admin action + `FinalSettlement` payment JE.

use chrono::{Datelike, NaiveDate};
use rand::Rng;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use datasynth_core::models::{
    AnomalyDetectionDifficulty, ConcealmentTechnique, SchemeDetectionStatus, SchemeType,
};

use super::scheme::{
    FraudScheme, SchemeAction, SchemeActionType, SchemeContext, SchemeStage, SchemeStatus,
    SchemeTransactionRef,
};

/// Shadow payroll scheme: ghost employee receiving monthly payroll.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowPayrollScheme {
    pub scheme_id: Uuid,
    pub perpetrator_id: String,
    pub start_date: Option<NaiveDate>,
    current_stage_index: usize,
    stages: Vec<SchemeStage>,
    transactions: Vec<SchemeTransactionRef>,
    total_impact: Decimal,
    status: SchemeStatus,
    detection_status: SchemeDetectionStatus,
    detection_probability: f64,
    stage_transaction_count: u32,
    /// Realistic ghost employee ID (EMP-{n:05}).
    ghost_employee_id: String,
    /// (year, month) of the last MonthlyPayroll emission.
    last_payroll_month: Option<(i32, u32)>,
    /// Whether GhostHire has been emitted.
    hire_emitted: bool,
    /// Whether GhostTermination has been emitted.
    termination_emitted: bool,
}

impl ShadowPayrollScheme {
    pub fn new(scheme_id: Uuid, perpetrator_id: impl Into<String>) -> Self {
        let stages = vec![
            SchemeStage::new(
                1,
                "create_ghost",
                1,
                (dec!(0), dec!(0)),
                (0, 1),
                AnomalyDetectionDifficulty::Expert,
            )
            .with_description("Create ghost employee in HR master data")
            .with_technique(ConcealmentTechnique::FalseDocumentation),
            SchemeStage::new(
                2,
                "payroll_postings",
                12,
                (dec!(2000), dec!(8000)),
                (12, 24),
                AnomalyDetectionDifficulty::Moderate,
            )
            .with_description("Monthly payroll postings to ghost bank account")
            .with_technique(ConcealmentTechnique::ApprovalCircumvention)
            .with_technique(ConcealmentTechnique::DocumentManipulation),
            SchemeStage::new(
                3,
                "termination",
                1,
                (dec!(2000), dec!(10000)),
                (1, 2),
                AnomalyDetectionDifficulty::Hard,
            )
            .with_description("Ghost employee termination + final settlement payment")
            .with_technique(ConcealmentTechnique::DataAlteration),
        ];

        // Derive a deterministic but realistic-looking employee number from the scheme id
        let emp_number = (scheme_id.as_u128() % 90000 + 10000) as u32;
        let ghost_employee_id = format!("EMP-{:05}", emp_number);

        Self {
            scheme_id,
            perpetrator_id: perpetrator_id.into(),
            start_date: None,
            current_stage_index: 0,
            stages,
            transactions: Vec::new(),
            total_impact: Decimal::ZERO,
            status: SchemeStatus::NotStarted,
            detection_status: SchemeDetectionStatus::Undetected,
            detection_probability: 0.0,
            stage_transaction_count: 0,
            ghost_employee_id,
            last_payroll_month: None,
            hire_emitted: false,
            termination_emitted: false,
        }
    }

    /// Returns the last calendar day of the given month.
    fn last_day_of_month(year: i32, month: u32) -> u32 {
        let next_month_first = if month == 12 {
            NaiveDate::from_ymd_opt(year + 1, 1, 1)
        } else {
            NaiveDate::from_ymd_opt(year, month + 1, 1)
        };
        next_month_first
            .and_then(|d| d.pred_opt())
            .map(|d| d.day())
            .unwrap_or(30)
    }

    /// True when `date` is on or after the last business day of its month.
    /// (Last day of month, or Friday if the last day falls on Sat/Sun.)
    fn is_last_business_day_of_month(date: NaiveDate) -> bool {
        let last_day = Self::last_day_of_month(date.year(), date.month());
        let last_date = NaiveDate::from_ymd_opt(date.year(), date.month(), last_day)
            .unwrap_or(date);
        let weekday = last_date.weekday().num_days_from_monday(); // Mon=0, Fri=4, Sat=5, Sun=6
        let last_biz_day = if weekday == 5 {
            last_day.saturating_sub(1) // Friday
        } else if weekday == 6 {
            last_day.saturating_sub(2) // Friday
        } else {
            last_day
        };
        date.day() >= last_biz_day
    }

    /// Sample social charge ratio from N(0.42, 0.03) clamped to [0.36, 0.50].
    fn sample_social_charge_ratio(rng: &mut dyn rand::RngCore) -> f64 {
        // Box-Muller transform for N(0,1)
        let u1: f64 = rng.random::<f64>().max(1e-10);
        let u2: f64 = rng.random::<f64>();
        let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
        let sample = 0.42 + 0.03 * z;
        sample.clamp(0.36, 0.50)
    }

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

    fn should_advance_stage(&self, current_date: NaiveDate) -> bool {
        if let Some(end_date) = self.stage_end_date() {
            current_date >= end_date && self.current_stage_index < self.stages.len() - 1
        } else {
            false
        }
    }

    fn advance_stage(&mut self) {
        if self.current_stage_index < self.stages.len() - 1 {
            self.current_stage_index += 1;
            self.stage_transaction_count = 0;
        }
    }
}

impl FraudScheme for ShadowPayrollScheme {
    fn scheme_type(&self) -> SchemeType {
        SchemeType::ShadowPayroll
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

        if self.status == SchemeStatus::NotStarted {
            self.start_date = Some(context.current_date);
            self.status = SchemeStatus::Active;
        }

        if self.should_terminate(context) {
            self.status = SchemeStatus::Terminated;
            return actions;
        }

        if rng.random::<f64>() < self.detection_probability * context.detection_activity {
            self.detection_status = SchemeDetectionStatus::PartiallyDetected;
            self.status = SchemeStatus::Detected;
            return actions;
        }

        if self.should_advance_stage(context.current_date) {
            self.advance_stage();
        }

        if context.audit_in_progress && rng.random::<f64>() < 0.8 {
            self.status = SchemeStatus::Paused;
            return actions;
        }
        self.status = SchemeStatus::Active;

        let stage = self.stages[self.current_stage_index].clone();

        match self.current_stage_index {
            // Stage 0: ghost hire — emit once
            0 => {
                if !self.hire_emitted {
                    let action = SchemeAction::new(
                        self.scheme_id,
                        stage.stage_number,
                        SchemeActionType::GhostHire,
                        context.current_date,
                    )
                    .with_scheme_type(self.scheme_type())
                    .with_user(&self.perpetrator_id)
                    .with_difficulty(stage.detection_difficulty)
                    .with_reference(self.ghost_employee_id.clone())
                    .with_description(format!(
                        "Ghost employee hired: {} (HR master write-back, no financial JE)",
                        self.ghost_employee_id
                    ))
                    .with_technique(ConcealmentTechnique::FalseDocumentation);
                    self.hire_emitted = true;
                    self.stage_transaction_count += 1;
                    self.detection_probability = (self.detection_probability + 0.005).min(0.9);
                    actions.push(action);
                }
            }

            // Stage 1: monthly payroll — one emission per calendar month on last business day
            1 => {
                let current_month = (context.current_date.year(), context.current_date.month());
                let already_paid = self.last_payroll_month == Some(current_month);

                if !already_paid && Self::is_last_business_day_of_month(context.current_date) {
                    let gross = context
                        // Use fingerprint distribution so payroll amounts are in-distribution (not easy outliers)
                        .sample_amount_from_fingerprint(rng, Some("6XXX"))
                        .unwrap_or_else(|| stage.random_amount(rng));
                    let social_ratio = Self::sample_social_charge_ratio(rng);
                    let social_charges = gross
                        * Decimal::from_f64_retain(social_ratio).unwrap_or(dec!(0.42));

                    let action = SchemeAction::new(
                        self.scheme_id,
                        stage.stage_number,
                        SchemeActionType::MonthlyPayroll,
                        context.current_date,
                    )
                    .with_scheme_type(self.scheme_type())
                    .with_amount(gross + social_charges)
                    .with_user(&self.perpetrator_id)
                    .with_difficulty(stage.detection_difficulty)
                    .with_reference(self.ghost_employee_id.clone())
                    .with_description(format!(
                        "Ghost payroll: {} gross={} social={:.0} (ratio={:.3})",
                        self.ghost_employee_id, gross, social_charges, social_ratio
                    ))
                    .with_technique(ConcealmentTechnique::ApprovalCircumvention)
                    .with_technique(ConcealmentTechnique::DocumentManipulation);

                    self.last_payroll_month = Some(current_month);
                    self.stage_transaction_count += 1;
                    self.detection_probability = (self.detection_probability + 0.015).min(0.9);
                    actions.push(action);
                }
            }

            // Stage 2: termination + final settlement
            2 => {
                if !self.termination_emitted {
                    // GhostTermination admin record (no financial JE)
                    let term_action = SchemeAction::new(
                        self.scheme_id,
                        stage.stage_number,
                        SchemeActionType::GhostTermination,
                        context.current_date,
                    )
                    .with_scheme_type(self.scheme_type())
                    .with_user(&self.perpetrator_id)
                    .with_difficulty(stage.detection_difficulty)
                    .with_reference(self.ghost_employee_id.clone())
                    .with_description(format!(
                        "Ghost employee terminated: {} (HR master update, no financial JE)",
                        self.ghost_employee_id
                    ))
                    .with_technique(ConcealmentTechnique::DataAlteration);
                    actions.push(term_action);

                    // FinalSettlement: last payroll / severance payment (Dr 421000, Cr 512000)
                    let settlement_amount = context
                        // Use fingerprint distribution so payroll amounts are in-distribution (not easy outliers)
                        .sample_amount_from_fingerprint(rng, Some("6XXX"))
                        .unwrap_or_else(|| stage.random_amount(rng));
                    let settle_action = SchemeAction::new(
                        self.scheme_id,
                        stage.stage_number,
                        SchemeActionType::FinalSettlement,
                        context.current_date,
                    )
                    .with_scheme_type(self.scheme_type())
                    .with_amount(settlement_amount)
                    .with_user(&self.perpetrator_id)
                    .with_difficulty(stage.detection_difficulty)
                    .with_reference(self.ghost_employee_id.clone())
                    .with_description(format!(
                        "Final settlement for ghost employee {}",
                        self.ghost_employee_id
                    ))
                    .with_technique(ConcealmentTechnique::DataAlteration);
                    actions.push(settle_action);

                    self.termination_emitted = true;
                    self.stage_transaction_count += 1;
                    self.detection_probability = (self.detection_probability + 0.02).min(0.9);
                }
            }

            _ => {}
        }

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
        context.detection_activity > 0.8
            || self.detection_probability > 0.9
            || self.detection_status != SchemeDetectionStatus::Undetected
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
    fn test_ghost_employee_id_format() {
        let scheme = ShadowPayrollScheme::new(Uuid::nil(), "HR001");
        // Should be EMP-{5 digits}
        assert!(
            scheme.ghost_employee_id.starts_with("EMP-"),
            "expected EMP- prefix, got {}",
            scheme.ghost_employee_id
        );
        assert_eq!(scheme.ghost_employee_id.len(), 9); // "EMP-" + 5 digits
    }

    #[test]
    fn test_ghost_hire_emitted_once() {
        let mut scheme = ShadowPayrollScheme::new(Uuid::nil(), "HR001");
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let context =
            SchemeContext::new(NaiveDate::from_ymd_opt(2024, 1, 5).unwrap(), "1000");
        let a1 = scheme.advance(&context, &mut rng);
        let a2 = scheme.advance(&context, &mut rng);
        // GhostHire should appear exactly once
        let hire_count = a1
            .iter()
            .chain(a2.iter())
            .filter(|a| a.action_type == SchemeActionType::GhostHire)
            .count();
        assert_eq!(hire_count, 1);
    }

    #[test]
    fn test_monthly_payroll_once_per_month() {
        let mut scheme = ShadowPayrollScheme::new(Uuid::nil(), "HR001");
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        // Fast-forward to payroll stage (month 2 start)
        let start = SchemeContext::new(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(), "1000");
        let _ = scheme.advance(&start, &mut rng);
        // Simulate last business day of month 2
        let last_day = SchemeContext::new(NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(), "1000");
        let a1 = scheme.advance(&last_day, &mut rng);
        let a2 = scheme.advance(&last_day, &mut rng);
        let payroll_count = a1
            .iter()
            .chain(a2.iter())
            .filter(|a| a.action_type == SchemeActionType::MonthlyPayroll)
            .count();
        // At most one per month
        assert!(payroll_count <= 1);
    }

    #[test]
    fn test_social_charge_ratio_clamped() {
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        for _ in 0..100 {
            let r = ShadowPayrollScheme::sample_social_charge_ratio(&mut rng);
            assert!(
                (0.36..=0.50).contains(&r),
                "social charge ratio {} out of [0.36, 0.50]",
                r
            );
        }
    }
}
