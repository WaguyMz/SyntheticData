import { useEffect, useState } from 'react';
import { loadMasterData } from '../api/data';
import { DataTable } from '../components/DataTable';
import type { MasterRecord } from '../types';
import './MasterDataView.css';

type MasterTab = 'vendors' | 'customers' | 'materials' | 'fixed_assets' | 'employees';

interface MasterState {
  vendors: MasterRecord[];
  customers: MasterRecord[];
  materials: MasterRecord[];
  fixed_assets: MasterRecord[];
  employees: MasterRecord[];
}

const TABS: { id: MasterTab; label: string }[] = [
  { id: 'vendors', label: 'Vendors' },
  { id: 'customers', label: 'Customers' },
  { id: 'materials', label: 'Materials' },
  { id: 'fixed_assets', label: 'Fixed Assets' },
  { id: 'employees', label: 'Employees' },
];

const PRIORITY_COLUMNS: Partial<Record<MasterTab, string[]>> = {
  vendors: ['vendor_id', 'name', 'country', 'account_number', 'primary_bank_account', 'primary_bank_name', 'bank_account_count'],
  customers: ['customer_id', 'name', 'country', 'account_number', 'primary_bank_account', 'primary_bank_name', 'bank_account_count'],
  employees: ['employee_id', 'display_name', 'company_code', 'hire_date', 'creation_date', 'payroll_iban', 'payroll_bank_name', 'payroll_bank_country'],
};

function columnsFromSample(
  rows: MasterRecord[],
  maxCols = 16,
  priorityKeys: string[] = [],
): { key: string; label: string }[] {
  if (rows.length === 0) return [];
  const sample = rows[0];
  const keys = Object.keys(sample).filter((k) => typeof sample[k] !== 'object' || sample[k] === null);

  const orderedKeys: string[] = [];
  for (const key of priorityKeys) {
    if (keys.includes(key) && !orderedKeys.includes(key)) {
      orderedKeys.push(key);
    }
  }
  for (const key of keys) {
    if (!orderedKeys.includes(key)) {
      orderedKeys.push(key);
    }
  }

  return orderedKeys.slice(0, maxCols).map((k) => ({
    key: k,
    label: k.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase()),
  }));
}

/** For vendors/customers under French GAAP: show auxiliary_gl_account (401xxxx/411xxxx) in the Account number column. */
function normalizeVendorCustomerRows(rows: MasterRecord[], tab: MasterTab): MasterRecord[] {
  if (rows.length === 0 || (tab !== 'vendors' && tab !== 'customers')) return rows;
  return rows.map((row) => {
    const r = { ...row } as Record<string, unknown>;
    const aux = r.auxiliary_gl_account;
    if (aux != null && aux !== '') {
      r.account_number = aux;
    }
    // Flatten bank account information: show count and primary IBAN (account_number)
    const bankAccounts = Array.isArray(r.bank_accounts) ? (r.bank_accounts as unknown[]) : [];
    if (bankAccounts.length > 0) {
      r.bank_account_count = bankAccounts.length;
      const first = bankAccounts[0] as Record<string, unknown>;
      if (typeof first.account_number === 'string') {
        r.primary_bank_account = first.account_number;
      }
      if (typeof first.bank_name === 'string') {
        r.primary_bank_name = first.bank_name;
      }
       if (typeof first.bank_country === 'string') {
         r.primary_bank_country = first.bank_country;
       }
    }
    return r as MasterRecord;
  });
}

/** For employees: expose payroll bank account (IBAN) fields in a flattened way. */
function normalizeEmployeeRows(rows: MasterRecord[]): MasterRecord[] {
  return rows.map((row) => {
    const r = { ...row } as Record<string, unknown>;
    const ba = r.bank_account as Record<string, unknown> | undefined;
    if (ba && typeof ba === 'object') {
      if (typeof ba.account_number === 'string') {
        r.payroll_iban = ba.account_number;
      }
      if (typeof ba.bank_name === 'string') {
        r.payroll_bank_name = ba.bank_name;
      }
      if (typeof ba.bank_country === 'string') {
        r.payroll_bank_country = ba.bank_country;
      }
    }
    return r as MasterRecord;
  });
}

export function MasterDataView() {
  const [data, setData] = useState<MasterState | null>(null);
  const [activeTab, setActiveTab] = useState<MasterTab>('vendors');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    loadMasterData()
      .then((d) => {
        setData((d ?? { vendors: [], customers: [], materials: [], fixed_assets: [], employees: [] }) as MasterState);
        setError(null);
      })
      .catch((e) => {
        setError(e instanceof Error ? e.message : 'Failed to load master data');
        setData({ vendors: [], customers: [], materials: [], fixed_assets: [], employees: [] });
      })
      .finally(() => setLoading(false));
  }, []);

  if (loading) return <div className="master-data-view loading">Loading master data…</div>;
  if (error && !data) return <div className="master-data-view error">Error: {error}</div>;

  const state = data!;
  const rawRows = state[activeTab] as MasterRecord[];
  const vendorCustomerNormalized = normalizeVendorCustomerRows(rawRows, activeTab);
  const rows =
    activeTab === 'employees' ? normalizeEmployeeRows(vendorCustomerNormalized) : vendorCustomerNormalized;
  const priorityKeys = PRIORITY_COLUMNS[activeTab] ?? [];
  const cols = columnsFromSample(rows, 16, priorityKeys);

  return (
    <div className="master-data-view">
      <h2>Master Data</h2>
      <p className="master-data-desc">Detailed view of vendors, customers, materials, fixed assets, and employees from <code>master_data/</code>.</p>
      <div className="master-data-tabs">
        {TABS.map((tab) => (
          <button
            key={tab.id}
            type="button"
            className={activeTab === tab.id ? 'active' : ''}
            onClick={() => setActiveTab(tab.id)}
          >
            {tab.label} ({(state[tab.id] as MasterRecord[])?.length ?? 0})
          </button>
        ))}
      </div>
      {rows.length === 0 ? (
        <p className="master-data-empty">No {activeTab.replace('_', ' ')} data in output.</p>
      ) : (
        <DataTable
          data={rows}
          columns={cols.map((c) => ({ ...c, format: (v) => (v != null && typeof v === 'object' ? JSON.stringify(v) : String(v ?? '')) }))}
          keyField={cols[0]?.key ?? 'id'}
          pageSize={50}
          maxHeight="65vh"
        />
      )}
    </div>
  );
}
