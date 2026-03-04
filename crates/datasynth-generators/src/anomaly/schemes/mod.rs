//! Multi-stage fraud scheme framework.
//!
//! This module provides realistic multi-stage fraud schemes that evolve over time,
//! including embezzlement, revenue manipulation, kickback, and RIP-GNN pathology lab schemes.

mod embezzlement;
mod expense_laundering;
mod kickback;
mod revenue_manipulation;
mod scheme;
mod shadow_payroll;
mod triad_bypass;

pub use embezzlement::GradualEmbezzlementScheme;
pub use expense_laundering::ExpenseLaunderingScheme;
pub use kickback::VendorKickbackScheme;
pub use revenue_manipulation::RevenueManipulationScheme;
pub use scheme::{
    FraudScheme, SchemeAction, SchemeActionType, SchemeContext, SchemeStage, SchemeStatus,
    SchemeTransactionRef,
};
pub use shadow_payroll::ShadowPayrollScheme;
pub use triad_bypass::TriadBypassScheme;
