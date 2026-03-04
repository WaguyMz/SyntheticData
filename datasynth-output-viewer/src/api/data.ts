import Papa from 'papaparse';
import { dataUrl } from '../config';
import type { JournalEntryRow, FECRow, AnomalyLabel } from '../types';

async function fetchText(path: string): Promise<string> {
  const res = await fetch(dataUrl(path));
  if (!res.ok) throw new Error(`Failed to load ${path}: ${res.status}`);
  return res.text();
}

async function fetchJson<T>(path: string): Promise<T> {
  const res = await fetch(dataUrl(path));
  if (!res.ok) throw new Error(`Failed to load ${path}: ${res.status}`);
  return res.json();
}

/** Load journal entries CSV (flat, one row per line). */
export async function loadJournalEntriesCsv(): Promise<JournalEntryRow[]> {
  const text = await fetchText('journal_entries.csv');
  const parsed = Papa.parse<JournalEntryRow>(text, { header: true, skipEmptyLines: true });
  return parsed.data || [];
}

/** Load FEC CSV (semicolon-separated). Returns rows or null if file missing. */
export async function loadFEC(): Promise<FECRow[] | null> {
  try {
    const text = await fetchText('fec.csv');
    const parsed = Papa.parse<FECRow>(text, {
      delimiter: ';',
      header: true,
      skipEmptyLines: true,
    });
    return parsed.data || [];
  } catch {
    return null;
  }
}

/** Load trial balances from period_close/trial_balances.json (optional fallback; trial balance is computed on the fly from JEs). */
export async function loadTrialBalances(): Promise<{ fiscal_year: number; fiscal_period: number; period_start: string; period_end: string; entries: Array<{ account_code: string; account_name: string; category: string; debit_balance: number | string; credit_balance: number | string }> }[] | null> {
  try {
    return await fetchJson('period_close/trial_balances.json');
  } catch {
    return null;
  }
}

/** Chart of accounts: map account_number -> { short_description, account_type }. */
export interface CoaAccount {
  account_number?: string;
  short_description?: string;
  account_type?: string;
}

export interface ChartOfAccountsData {
  accounts?: CoaAccount[];
}

/**
 * Resolve account display name by first ancestor in the COA: given an account number,
 * retract digits from right to left until a COA entry is found, then use that entry's name.
 * If none matches, returns the code itself.
 */
export function getAccountNameByAncestor(
  code: string | number,
  coaMap: Map<string, { name: string; category?: string }>
): string {
  const trimmed = String(code ?? '').trim();
  if (!trimmed) return trimmed;
  for (let len = trimmed.length; len >= 1; len--) {
    const prefix = trimmed.slice(0, len);
    const info = coaMap.get(prefix);
    if (info?.name) return info.name;
  }
  return trimmed;
}

/**
 * Resolve account name and category by first ancestor in the COA (retract digits right to left).
 * Returns { name, category }; if no ancestor is found, name is the code and category is '—'.
 */
export function getAccountInfoByAncestor(
  code: string | number,
  coaMap: Map<string, { name: string; category: string }>
): { name: string; category: string } {
  const trimmed = String(code ?? '').trim();
  for (let len = trimmed.length; len >= 1; len--) {
    const prefix = trimmed.slice(0, len);
    const info = coaMap.get(prefix);
    if (info) return { name: info.name, category: info.category ?? '—' };
  }
  return { name: trimmed || '—', category: '—' };
}

/** Load chart of accounts for account labels (optional). */
export async function loadChartOfAccounts(): Promise<Map<string, { name: string; category: string }>> {
  const map = new Map<string, { name: string; category: string }>();
  try {
    const data = await fetchJson<ChartOfAccountsData>('chart_of_accounts.json');
    // Support both { accounts: [...] } and root array
    const accounts = Array.isArray(data) ? data : (data?.accounts ?? []);
    for (const a of accounts) {
      const raw = a.account_number ?? (a as Record<string, unknown>).account_number ?? '';
      const code = String(raw).trim();
      if (!code) continue;
      const name = a.short_description ?? (a as Record<string, unknown>).short_description ?? code;
      const category = a.account_type ?? (a as Record<string, unknown>).account_type ?? '—';
      map.set(code, {
        name: String(name),
        category: String(category),
      });
    }
  } catch {
    // no coa file or wrong path
  }
  return map;
}

/** Load anomaly labels from labels/anomaly_labels.csv or .json */
export async function loadAnomalyLabels(): Promise<AnomalyLabel[] | null> {
  try {
    const text = await fetchText('labels/anomaly_labels.csv');
    const parsed = Papa.parse<AnomalyLabel>(text, { header: true, skipEmptyLines: true });
    return parsed.data?.length ? parsed.data : null;
  } catch {
    try {
      return await fetchJson<AnomalyLabel[]>('labels/anomaly_labels.json');
    } catch {
      return null;
    }
  }
}

/** Load fraud labels if present */
export async function loadFraudLabels(): Promise<AnomalyLabel[] | null> {
  try {
    const text = await fetchText('labels/fraud_labels.csv');
    const parsed = Papa.parse<AnomalyLabel>(text, { header: true, skipEmptyLines: true });
    return parsed.data?.length ? parsed.data : null;
  } catch {
    try {
      return await fetchJson<AnomalyLabel[]>('labels/fraud_labels.json');
    } catch {
      return null;
    }
  }
}

/** Load multi-stage scheme labels if present (optional; may include pathology_name, stage_number, perpetrator_id). */
export async function loadMultiStageLabels(): Promise<AnomalyLabel[] | null> {
  try {
    const text = await fetchText('labels/multi_stage_scheme_labels.csv');
    const parsed = Papa.parse<AnomalyLabel>(text, { header: true, skipEmptyLines: true });
    return parsed.data?.length ? parsed.data : null;
  } catch {
    try {
      return await fetchJson<AnomalyLabel[]>('labels/multi_stage_scheme_labels.json');
    } catch {
      return null;
    }
  }
}

/** Load master data JSON */
export async function loadMasterData(): Promise<{
  vendors: unknown[];
  customers: unknown[];
  materials: unknown[];
  fixed_assets: unknown[];
  employees: unknown[];
} | null> {
  const base = 'master_data';
  try {
    const [vendors, customers, materials, fixed_assets, employees] = await Promise.all([
      fetchJson<unknown[]>(`${base}/vendors.json`).catch(() => []),
      fetchJson<unknown[]>(`${base}/customers.json`).catch(() => []),
      fetchJson<unknown[]>(`${base}/materials.json`).catch(() => []),
      fetchJson<unknown[]>(`${base}/fixed_assets.json`).catch(() => []),
      fetchJson<unknown[]>(`${base}/employees.json`).catch(() => []),
    ]);
    return { vendors, customers, materials, fixed_assets, employees };
  } catch {
    return null;
  }
}

/** Check if FEC file exists */
export async function hasFEC(): Promise<boolean> {
  try {
    const res = await fetch(dataUrl('fec.csv'), { method: 'HEAD' });
    return res.ok;
  } catch {
    return false;
  }
}

const SUBLEDGER_BASE = 'subledger';

/** Load AR subledger (customer invoices, receipts, aging). */
export async function loadSubledgerAr(): Promise<Record<string, unknown>[]> {
  try {
    return await fetchJson<Record<string, unknown>[]>(`${SUBLEDGER_BASE}/ar_invoices.json`);
  } catch {
    return [];
  }
}

/** Load AP subledger (vendor invoices, payments, aging). */
export async function loadSubledgerAp(): Promise<Record<string, unknown>[]> {
  try {
    return await fetchJson<Record<string, unknown>[]>(`${SUBLEDGER_BASE}/ap_invoices.json`);
  } catch {
    return [];
  }
}

/** Load FA subledger (asset capitalization, depreciation, disposals). */
export async function loadSubledgerFa(): Promise<Record<string, unknown>[]> {
  try {
    return await fetchJson<Record<string, unknown>[]>(`${SUBLEDGER_BASE}/fa_records.json`);
  } catch {
    return [];
  }
}

/** Load Inventory subledger (positions, valuation FIFO/LIFO/weighted average). */
export async function loadSubledgerInventory(): Promise<Record<string, unknown>[]> {
  try {
    return await fetchJson<Record<string, unknown>[]>(`${SUBLEDGER_BASE}/inventory_positions.json`);
  } catch {
    return [];
  }
}

/** Load subledger reconciliation (balance/subledger_reconciliation.json). */
export async function loadSubledgerReconciliation(): Promise<Record<string, unknown>[]> {
  try {
    const data = await fetchJson<Record<string, unknown> | Record<string, unknown>[]>('balance/subledger_reconciliation.json');
    return Array.isArray(data) ? data : [];
  } catch {
    return [];
  }
}
