/** Journal entry line (flat CSV row from journal_entries.csv) */
export interface JournalEntryRow {
  document_id: string;
  company_code: string;
  fiscal_year: number;
  fiscal_period: number;
  posting_date: string;
  document_date: string;
  document_type: string;
  currency: string;
  exchange_rate: string;
  reference: string;
  header_text: string;
  created_by: string;
  source: string;
  business_process: string;
  ledger: string;
  is_fraud: string | boolean;
  is_anomaly: string | boolean;
  line_number: number;
  gl_account: string;
  debit_amount: string | number;
  credit_amount: string | number;
  local_amount: string | number;
  cost_center: string;
  profit_center: string;
  line_text: string;
  /** French GAAP: compte auxiliaire (e.g. 4010001, 4110001) */
  auxiliary_account_number?: string;
  /** French GAAP: libellé compte auxiliaire */
  auxiliary_account_label?: string;
  /** FEC lettrage (document flow matching code) */
  lettrage?: string;
  /** FEC date de lettrage */
  lettrage_date?: string;
  [key: string]: unknown;
}

/** FEC row (semicolon-separated, 18 columns) */
export interface FECRow {
  'Code journal': string;
  "Libellé journal": string;
  "Numéro de l'écriture": string;
  'Date de comptabilisation': string;
  'Numéro de compte': string;
  'Libellé de compte': string;
  "Numéro de compte auxiliaire": string;
  "Libellé de compte auxiliaire": string;
  "Référence de la pièce justificative": string;
  "Date d'émission de la pièce justificative": string;
  "Libellé de l'écriture comptable": string;
  'Montant au débit': string;
  'Montant au crédit': string;
  Lettrage: string;
  'Date de lettrage': string;
  'Date de validation de l\'écriture': string;
  'Montant en devise': string;
  "Identifiant de la devise": string;
  [key: string]: unknown;
}

/** Trial balance entry (from period_close/trial_balances.json) */
export interface TrialBalanceEntry {
  account_code: string;
  account_name: string;
  category: string;
  debit_balance: number | string;
  credit_balance: number | string;
}

/** Period trial balance */
export interface PeriodTrialBalance {
  fiscal_year: number;
  fiscal_period: number;
  period_start: string;
  period_end: string;
  entries: TrialBalanceEntry[];
}

/** Anomaly / fraud label (from labels/anomaly_labels.csv or .json) */
export interface AnomalyLabel {
  anomaly_id?: string;
  anomaly_category?: string;
  anomaly_type?: string;
  document_id?: string;
  document_type?: string;
  company_code?: string;
  anomaly_date?: string;
  severity?: number | string;
  description?: string;
  is_injected?: string | boolean;
  monetary_impact?: string;
  scenario_id?: string;
  cluster_id?: string;
  causal_reason_type?: string;
  /** Display: scenario_id || cluster_id || causal_reason_type (filled in viewer so scenario is never empty) */
  scenario_display?: string;
  /** Friendly type for display (e.g. multi-stage scheme types); set in viewer from anomaly_type */
  type_display?: string;
  /** True when label is from a multi-stage fraud scheme (scenario_id set by runtime) */
  is_scheme?: boolean;
  /** Optional: from metadata or multi_stage export — RIP-GNN pathology name */
  pathology_name?: string;
  /** Optional: Sequential | Volume | Relational */
  pathology_category?: string;
  /** Optional: stage number within scheme (1-based) */
  stage_number?: number | string;
  /** Optional: stage name (e.g. "testing", "escalation") */
  stage_name?: string;
  /** Optional: total stages in scheme */
  total_stages?: number | string;
  /** Optional: perpetrator user ID (involved employee) */
  perpetrator_id?: string;
  /** Optional: involved counterparty (vendor/customer ID) */
  counterparty?: string;
  /** Optional: action amount from scheme action (for display) */
  action_amount?: string | number;
  /** Optional: raw metadata JSON string from export */
  metadata_json?: string;
  /** Optional: number of JEs in the same fraud scheme (per scenario_id) */
  scheme_je_count?: number;
  [key: string]: unknown;
}

/** A single fraud scheme instance: all labels sharing the same scenario_id (scheme_id). */
export interface SchemeInstance {
  scheme_id: string;
  labels: AnomalyLabel[];
  /** Inferred or from first label: pathology name / scheme type display */
  pathology_name: string;
  pathology_category: string;
  /** First and last anomaly date in this instance */
  date_min: string;
  date_max: string;
  document_ids: string[];
  /** Sum of monetary_impact where parseable */
  total_impact: number;
}

/** Master data entity (generic) */
export interface MasterRecord {
  [key: string]: unknown;
}
