//! Payroll tax diversion (negative-signal fraud).
//!
//! The perpetrator collects employee payroll deductions but does not remit them
//! to the tax authority. The company's `431000` liability grows silently.
//!
//! This is a **negative-signal** fraud — the agent must detect the *absence* of
//! expected transactions rather than the *presence* of anomalous ones.
//!
//! Stages:
//!   0 — Setup (1 month): Normal payroll + normal remittance (baseline).
//!   1 — Diversion (6–12 months): Payroll posted normally, remittance suppressed.
//!   2 — Cover-up (2 months): Fictitious remittance via suspense instead of bank.

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayrollTaxDiversionScheme {
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
    last_action_month: Option<(i32, u32)>,
}

impl PayrollTaxDiversionScheme {
    pub fn new(scheme_id: Uuid, perpetrator_id: impl Into<String>) -> Self {
        let stages = vec![
            SchemeStage::new(1, "setup", 1, (dec!(0), dec!(0)), (0, 1), AnomalyDetectionDifficulty::Expert)
                .with_description("Baseline: normal payroll + normal remittance"),
            SchemeStage::new(2, "diversion", 9, (dec!(5000), dec!(30000)), (1, 1), AnomalyDetectionDifficulty::Hard)
                .with_description("Suppress tax remittance — liability grows on 431000"),
            SchemeStage::new(3, "cover_up", 2, (dec!(5000), dec!(30000)), (1, 2), AnomalyDetectionDifficulty::Moderate)
                .with_description("Fictitious remittance: Dr 431000, Cr 471000 (suspense)")
                .with_technique(ConcealmentTechnique::AccountMisclassification),
        ];

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
            last_action_month: None,
        }
    }

    fn stage_end_date(&self) -> Option<NaiveDate> {
        self.start_date.map(|start| {
            let total_months: u32 = self.stages[..=self.current_stage_index]
                .iter()
                .map(|s| s.duration_months)
                .sum();
            start + chrono::Months::new(total_months)
        })
    }

    fn should_advance_stage(&self, current_date: NaiveDate) -> bool {
        if let Some(end) = self.stage_end_date() {
            current_date >= end && self.current_stage_index < self.stages.len() - 1
        } else {
            false
        }
    }

    fn advance_stage(&mut self) {
        if self.current_stage_index < self.stages.len() - 1 {
            self.current_stage_index += 1;
        }
    }

    fn is_last_business_day_of_month(date: NaiveDate) -> bool {
        let next_day = date + chrono::Duration::days(1);
        next_day.month() != date.month()
            || (date.month() != (date + chrono::Duration::days(2)).month()
                && date.weekday().number_from_monday() <= 5)
    }
}

impl FraudScheme for PayrollTaxDiversionScheme {
    fn scheme_type(&self) -> SchemeType { SchemeType::PayrollTaxDiversion }
    fn scheme_id(&self) -> Uuid { self.scheme_id }
    fn current_stage(&self) -> &SchemeStage { &self.stages[self.current_stage_index] }
    fn current_stage_number(&self) -> u32 { self.stages[self.current_stage_index].stage_number }
    fn stages(&self) -> &[SchemeStage] { &self.stages }
    fn status(&self) -> SchemeStatus { self.status }
    fn detection_status(&self) -> SchemeDetectionStatus { self.detection_status }

    fn advance(&mut self, context: &SchemeContext, rng: &mut dyn rand::RngCore) -> Vec<SchemeAction> {
        let mut actions = Vec::new();

        if self.status == SchemeStatus::NotStarted {
            self.start_date = Some(context.current_date);
            self.status = SchemeStatus::Active;
        }

        if self.should_advance_stage(context.current_date) {
            self.advance_stage();
        }

        if context.audit_in_progress && rng.random::<f64>() < 0.9 {
            self.status = SchemeStatus::Paused;
            return actions;
        }
        self.status = SchemeStatus::Active;

        let current_month = (context.current_date.year(), context.current_date.month());
        if self.last_action_month == Some(current_month) {
            return actions;
        }

        if !Self::is_last_business_day_of_month(context.current_date) {
            return actions;
        }

        let stage = self.stages[self.current_stage_index].clone();

        match self.current_stage_index {
            0 => {
                // Setup: no fraud action — normal remittance happens via the clean data generator.
            }
            1 => {
                // Diversion: emit a SuppressRemittance marker. The materialiser will emit a
                // no-op JE (or skip it), but the ground-truth label is recorded so the
                // evaluator knows a remittance was expected and missing.
                let accumulated = context
                    .sample_amount_from_fingerprint(rng, Some("6XXX"))
                    .unwrap_or_else(|| stage.random_amount(rng));

                let action = SchemeAction::new(
                    self.scheme_id,
                    stage.stage_number,
                    SchemeActionType::SuppressRemittance,
                    context.current_date,
                )
                .with_scheme_type(self.scheme_type())
                .with_amount(accumulated)
                .with_user(&self.perpetrator_id)
                .with_difficulty(stage.detection_difficulty)
                .with_description("Tax remittance suppressed — 431000 liability not cleared");

                self.total_impact += accumulated;
                self.last_action_month = Some(current_month);
                actions.push(action);
            }
            2 => {
                // Cover-up: fictitious remittance Dr 431000 Cr 471000 (suspense, not bank).
                let accumulated = context
                    .sample_amount_from_fingerprint(rng, Some("6XXX"))
                    .unwrap_or_else(|| stage.random_amount(rng));

                let action = SchemeAction::new(
                    self.scheme_id,
                    stage.stage_number,
                    SchemeActionType::ConcealRemittance,
                    context.current_date,
                )
                .with_scheme_type(self.scheme_type())
                .with_amount(accumulated)
                .with_user(&self.perpetrator_id)
                .with_difficulty(stage.detection_difficulty)
                .with_description("Fictitious remittance via suspense: Dr 431000, Cr 471000");

                self.last_action_month = Some(current_month);
                actions.push(action);

                if let Some(end) = self.stage_end_date() {
                    if context.current_date >= end {
                        self.status = SchemeStatus::Completed;
                    }
                }
            }
            _ => {}
        }

        self.detection_probability = (self.detection_probability + 0.02).min(0.8);
        actions
    }

    fn detection_probability(&self) -> f64 { self.detection_probability }
    fn total_impact(&self) -> Decimal { self.total_impact }
    fn should_terminate(&self, context: &SchemeContext) -> bool {
        context.detection_activity > 0.8 || self.detection_status != SchemeDetectionStatus::Undetected
    }
    fn perpetrator_id(&self) -> &str { &self.perpetrator_id }
    fn start_date(&self) -> Option<NaiveDate> { self.start_date }
    fn transaction_refs(&self) -> &[SchemeTransactionRef] { &self.transactions }
    fn record_transaction(&mut self, transaction: SchemeTransactionRef) {
        self.total_impact += transaction.amount;
        self.transactions.push(transaction);
    }
}
