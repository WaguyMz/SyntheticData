//! Multi-stage fraud scheme framework.
//!
//! This module provides realistic multi-stage fraud schemes that evolve over time,
//! including embezzlement, revenue manipulation, kickback, and RIP-GNN pathology lab schemes.

mod circular_cash_flow;
mod embezzlement;
mod expense_laundering;
mod inventory_manipulation;
mod kickback;
mod payroll_tax_diversion;
mod related_party;
mod revenue_manipulation;
mod scheme;
mod shadow_payroll;
mod triad_bypass;

pub use circular_cash_flow::CircularCashFlowScheme;
pub use embezzlement::GradualEmbezzlementScheme;
pub use expense_laundering::ExpenseLaunderingScheme;
pub use inventory_manipulation::InventoryManipulationScheme;
pub use kickback::VendorKickbackScheme;
pub use payroll_tax_diversion::PayrollTaxDiversionScheme;
pub use related_party::RelatedPartyScheme;
pub use revenue_manipulation::RevenueManipulationScheme;
pub use scheme::{
    FraudScheme, GhostEmployeeRecord, SchemeAction, SchemeActionType, SchemeContext, SchemeStage,
    SchemeStatus, SchemeTransactionRef,
};
pub use shadow_payroll::ShadowPayrollScheme;
pub use triad_bypass::TriadBypassScheme;
