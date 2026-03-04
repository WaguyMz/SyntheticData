import { useEffect, useState, useMemo, useRef } from 'react';
import {
  BarChart,
  Bar,
  XAxis,
  YAxis,
  Tooltip,
  ResponsiveContainer,
  PieChart,
  Pie,
  Cell,
  Legend,
} from 'recharts';
import { loadAnomalyLabels, loadFraudLabels, loadMultiStageLabels, loadJournalEntriesCsv } from '../api/data';
import { DataTable } from '../components/DataTable';
import type { AnomalyLabel, JournalEntryRow, SchemeInstance } from '../types';
import type { GraphData } from '../api/neo4j';
import './FraudAnomalyView.css';

/** Active fraud schemes (Single-FEC scope; Circular Funding, Phantom Warehousing, Intercompany Wash Trades removed). */
const SCHEME_TAXONOMY: Array<{ key: string; name: string; category: 'Sequential' | 'Volume' | 'Relational'; description: string }> = [
  { key: 'gradual_embezzlement', name: 'Gradual Embezzlement', category: 'Sequential', description: 'Slow-burn theft with escalating amounts' },
  { key: 'revenue_manipulation', name: 'Revenue Manipulation', category: 'Volume', description: 'Channel stuffing, period-end spikes' },
  { key: 'vendor_kickback', name: 'Vendor Kickback', category: 'Relational', description: 'Collusion with vendor, inflated invoices' },
  { key: 'triad_bypass', name: 'Triad Bypass', category: 'Relational', description: 'Payment reusing old invoice ID' },
  { key: 'shadow_payroll', name: 'Shadow Payroll', category: 'Sequential', description: 'Ghost employee, fraudulent payroll' },
  { key: 'expense_laundering', name: 'Expense Laundering', category: 'Volume', description: 'Micro-expenses to shell vendors' },
  { key: 'smurfing', name: 'Smurfing', category: 'Volume', description: 'Many small below-threshold payments' },
];

const TABLE_COLUMNS = [
  // First column: scheme instance identifier (scenario_id).
  { key: 'scenario_display', label: 'Scheme instance ID', width: '220px' },
  { key: 'anomaly_category', label: 'Category', width: '100px' },
  { key: 'type_display', label: 'Type', width: '200px' },
  { key: 'scheme_badge', label: 'Scheme', width: '64px' },
  { key: 'pathology_display', label: 'Pathology', width: '160px' },
  { key: 'perpetrator_id', label: 'Perpetrator (employee)', width: '120px' },
  { key: 'counterparty_display', label: 'Involved counterparty (vendor/customer)', width: '160px' },
  { key: 'scheme_je_count', label: '# JEs in scheme', width: '110px' },
  { key: 'stage_display', label: 'Stage', width: '64px' },
  { key: 'company_code', label: 'Company', width: '80px' },
  { key: 'anomaly_date', label: 'Date', width: '100px' },
  { key: 'severity', label: 'Severity', width: '70px' },
  { key: 'description', label: 'Description', width: '260px' },
  { key: 'monetary_impact', label: 'Impact', width: '100px', format: (v: unknown) => formatNum(v) },
];

/** All columns for the concerned-transaction subtable (full journal entry line). */
const CONCERNED_FULL_COLUMNS = [
  { key: 'line_number', label: '#', width: '44px' },
  // All JEs in this table share the same scheme instance (scenario_id).
  { key: 'scenario_id', label: 'Scheme instance ID', width: '180px' },
  { key: 'document_id', label: 'Document ID', width: '130px' },
  { key: 'company_code', label: 'Company', width: '80px' },
  { key: 'fiscal_year', label: 'FY', width: '44px' },
  { key: 'fiscal_period', label: 'Period', width: '56px' },
  { key: 'posting_date', label: 'Posting Date', width: '100px' },
  { key: 'document_date', label: 'Doc. Date', width: '100px' },
  { key: 'document_type', label: 'Type', width: '90px' },
  { key: 'gl_account', label: 'GL Account', width: '90px' },
  { key: 'auxiliary_account_number', label: 'Compte aux.', width: '90px' },
  { key: 'auxiliary_account_label', label: 'Libellé aux.', width: '130px' },
  { key: 'debit_amount', label: 'Debit', width: '100px', format: (v: unknown) => formatNum(v) },
  { key: 'credit_amount', label: 'Credit', width: '100px', format: (v: unknown) => formatNum(v) },
  { key: 'local_amount', label: 'Local', width: '90px', format: (v: unknown) => formatNum(v) },
  { key: 'currency', label: 'CCY', width: '50px' },
  { key: 'exchange_rate', label: 'Rate', width: '70px' },
  { key: 'reference', label: 'Reference', width: '110px' },
  { key: 'header_text', label: 'Header', width: '140px' },
  { key: 'cost_center', label: 'Cost Ctr', width: '80px' },
  { key: 'profit_center', label: 'Profit Ctr', width: '80px' },
  { key: 'entry_impact', label: 'Impact', width: '100px', format: (v: unknown) => formatNum(v) },
  { key: 'line_text', label: 'Line Text', width: '200px' },
  { key: 'lettrage', label: 'Lettrage', width: '70px' },
  { key: 'lettrage_date', label: 'Date lettrage', width: '100px' },
  { key: 'created_by', label: 'Created By', width: '90px' },
  { key: 'source', label: 'Source', width: '80px' },
  { key: 'business_process', label: 'Process', width: '90px' },
  { key: 'ledger', label: 'Ledger', width: '70px' },
  { key: 'is_fraud', label: 'Fraud', width: '56px' },
  { key: 'is_anomaly', label: 'Anomaly', width: '64px' },
];

const COLORS = ['#e74c3c', '#f39c12', '#3498db', '#9b59b6', '#1abc9c', '#34495e', '#e67e22', '#2ecc71', '#8e44ad', '#16a085'];

function formatNum(v: unknown): string {
  if (v == null || v === '') return '';
  const n = typeof v === 'number' ? v : parseFloat(String(v));
  if (Number.isNaN(n)) return String(v);
  return n.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 });
}

const UUID_REGEX = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** Summary of involved entities for a scheme instance (perpetrator, counterparty, documents, stage, dates). */
function InvolvedEntitiesSummary({ scenarioId, labels }: { scenarioId: string; labels: AnomalyLabel[] }) {
  const schemeLabels = labels.filter((l) => (l.scenario_id as string)?.trim() === scenarioId);
  if (schemeLabels.length === 0) return null;
  const perpetrators = [...new Set(schemeLabels.map((l) => (l.perpetrator_id as string) || '').filter(Boolean))];
  const counterparties = [...new Set(schemeLabels.map((l) => (l.counterparty as string) || '').filter(Boolean))];
  const documentIds = [...new Set(schemeLabels.map((l) => (l.document_id as string) || '').filter(Boolean))];
  const stages = [...new Set(schemeLabels.map((l) => (l.stage_display as string) || (l.stage_number != null ? `${l.stage_number}/${l.total_stages ?? '?'}` : '')).filter(Boolean))];
  const dates = schemeLabels.map((l) => (l.anomaly_date as string) || '').filter(Boolean);
  const sortedDates = [...dates].sort();
  const dateMin = sortedDates[0] ?? '';
  const dateMax = sortedDates.length ? sortedDates[sortedDates.length - 1] : '';
  const pathology = (schemeLabels[0]?.pathology_name as string) || (schemeLabels[0]?.type_display as string) || '';

  return (
    <div className="fraud-anomaly-involved-entities" aria-label="Involved entities summary">
      <dl className="fraud-anomaly-involved-dl">
        <dt>Scenario ID</dt>
        <dd><code>{scenarioId}</code></dd>
        {pathology && (
          <>
            <dt>Scheme type</dt>
            <dd>{pathology}</dd>
          </>
        )}
        {perpetrators.length > 0 && (
          <>
            <dt>Perpetrator{perpetrators.length !== 1 ? 's' : ''} (involved employee)</dt>
            <dd>{perpetrators.join(', ')}</dd>
          </>
        )}
        {counterparties.length > 0 && (
          <>
            <dt>Involved counterparty (vendor/customer)</dt>
            <dd>{counterparties.join(', ')}</dd>
          </>
        )}
        {documentIds.length > 0 && (
          <>
            <dt>Involved document{documentIds.length !== 1 ? 's' : ''}</dt>
            <dd>
              {documentIds.length <= 5
                ? documentIds.join(', ')
                : `${documentIds.slice(0, 3).join(', ')} … +${documentIds.length - 3} more`}
            </dd>
          </>
        )}
        {stages.length > 0 && (
          <>
            <dt>Stage{stages.length !== 1 ? 's' : ''}</dt>
            <dd>{[...new Set(stages)].join(', ')}</dd>
          </>
        )}
        {(dateMin || dateMax) && (
          <>
            <dt>Date range</dt>
            <dd>{dateMin && dateMax ? `${dateMin} – ${dateMax}` : dateMin || dateMax}</dd>
          </>
        )}
      </dl>
    </div>
  );
}

function parseMetadataJson(meta: string | undefined): Record<string, string> {
  if (!meta || typeof meta !== 'string') return {};
  try {
    const o = JSON.parse(meta) as Record<string, unknown>;
    const out: Record<string, string> = {};
    for (const [k, v] of Object.entries(o)) {
      if (v != null && typeof v === 'string') out[k] = v;
      else if (v != null) out[k] = String(v);
    }
    return out;
  } catch {
    return {};
  }
}

/** Friendly display name for anomaly types (including all 7 active scheme types). */
function friendlyTypeName(anomalyType: string | undefined, pathologyName?: string): string {
  if (pathologyName) {
    const t = SCHEME_TAXONOMY.find((x) => x.key === pathologyName.toLowerCase().replace(/\s+/g, '_')) || SCHEME_TAXONOMY.find((x) => x.name === pathologyName);
    if (t) return t.name;
    return pathologyName;
  }
  if (!anomalyType) return '—';
  const t = String(anomalyType);
  if (t === 'FictitiousTransaction' || t.includes('FictitiousTransaction')) return 'Multi-stage: Fictitious entry';
  if (t === 'RevenueManipulation' || t.includes('RevenueManipulation')) return 'Revenue Manipulation';
  if (t === 'VendorKickback' || t.includes('VendorKickback')) return 'Vendor Kickback';
  if (t.includes('Embezzlement')) return 'Gradual Embezzlement';
  for (const s of SCHEME_TAXONOMY) {
    if (t.toLowerCase().includes(s.key.replace(/_/g, ' '))) return s.name;
  }
  return t;
}

/** Normalize scheme identifier for comparison (PascalCase or snake_case -> no spaces, no underscores, lower). */
function schemeKeyNormalize(s: string): string {
  return s
    .toLowerCase()
    .replace(/\s+/g, '')
    .replace(/_/g, '');
}

/** Return true if a label belongs to the given scheme type (by key). */
function labelMatchesScheme(label: AnomalyLabel, schemeKey: string): boolean {
  const scheme = SCHEME_TAXONOMY.find((x) => x.key === schemeKey);
  if (!scheme) return false;
  if (label.type_display === scheme.name) return true;
  const pathology = (label.pathology_name as string) ?? '';
  const pathNorm = schemeKeyNormalize(pathology);
  const keyNorm = schemeKeyNormalize(schemeKey);
  if (pathNorm && pathNorm === keyNorm) return true;
  const typeStr = (label.anomaly_type as string)?.toLowerCase() ?? '';
  if (schemeKey === 'gradual_embezzlement' && typeStr.includes('embezzlement')) return true;
  if (schemeKey === 'revenue_manipulation' && typeStr.includes('revenue')) return true;
  if (schemeKey === 'vendor_kickback' && typeStr.includes('kickback')) return true;
  if (schemeKey === 'triad_bypass' && (typeStr.includes('triad') || pathNorm.includes('triad'))) return true;
  if (schemeKey === 'shadow_payroll' && (typeStr.includes('payroll') || pathNorm.includes('shadow'))) return true;
  if (schemeKey === 'expense_laundering' && (typeStr.includes('expense') || pathNorm.includes('laundering'))) return true;
  if (schemeKey === 'smurfing' && (typeStr.includes('smurf') || pathNorm.includes('smurf'))) return true;
  if (schemeKey === 'circular_funding' && (typeStr.includes('circular') || pathNorm.includes('circular'))) return true;
  if (schemeKey === 'phantom_warehousing' && (typeStr.includes('phantom') || pathNorm.includes('warehousing'))) return true;
  if (schemeKey === 'intercompany_wash_trades' && (typeStr.includes('intercompany') || typeStr.includes('wash') || pathNorm.includes('wash'))) return true;
  return false;
}

/** Aggregate per-document financial impact from journal entries (sum of positive local_amount per document_id). */
function computeDocImpact(rows: JournalEntryRow[]): Map<string, number> {
  const map = new Map<string, number>();
  const norm = (s: string) => s.trim().toLowerCase();
  for (const r of rows) {
    const id = norm(String(r.document_id ?? ''));
    if (!id) continue;
    const raw = r.local_amount;
    const n = typeof raw === 'number' ? raw : parseFloat(String(raw || 0));
    if (!Number.isFinite(n)) continue;
    if (Math.abs(n) < 1e-6) continue;
    if (n <= 0) continue;
    map.set(id, (map.get(id) ?? 0) + n);
  }
  return map;
}

/** Aggregate per-scheme financial impact (per scenario_id) from labels and per-document impact. */
function computeSchemeImpact(labels: AnomalyLabel[], docImpactMap: Map<string, number>): Map<string, number> {
  const byScenario = new Map<string, Set<string>>();
  const norm = (s: string) => s.trim().toLowerCase();

  for (const l of labels) {
    const sid = (l.scenario_id as string)?.trim();
    if (!sid) continue;
    const doc = (l.document_id as string) || '';
    if (!doc) continue;
    const set = byScenario.get(sid) ?? new Set<string>();
    byScenario.set(sid, set);

    const raw = doc.trim();
    if (!raw) continue;
    const candidates = [raw, raw.startsWith('scheme-') ? raw.slice(7) : ''].filter(Boolean) as string[];
    for (const c of candidates) {
      const key = norm(c);
      if (key) set.add(key);
    }
  }

  const result = new Map<string, number>();
  for (const [sid, docs] of byScenario) {
    let total = 0;
    for (const key of docs) {
      const v = docImpactMap.get(key);
      if (v != null && Number.isFinite(v)) {
        total += v;
      }
    }
    result.set(sid, total);
  }
  return result;
}

/** Build scheme instances from labels that have scenario_id. */
function buildSchemeInstances(labels: AnomalyLabel[], schemeImpactMap?: Map<string, number>): SchemeInstance[] {
  const byScheme = new Map<string, AnomalyLabel[]>();
  for (const l of labels) {
    const sid = (l.scenario_id as string)?.trim();
    if (!sid) continue;
    if (!byScheme.has(sid)) byScheme.set(sid, []);
    byScheme.get(sid)!.push(l);
  }
  const instances: SchemeInstance[] = [];
  for (const [scheme_id, schemeLabels] of byScheme) {
    const sorted = [...schemeLabels].sort((a, b) => {
      const da = (a.anomaly_date as string) || '';
      const db = (b.anomaly_date as string) || '';
      return da.localeCompare(db);
    });
    const pathology_name = (sorted[0]?.pathology_name as string) || (sorted[0]?.anomaly_type as string) || 'Scheme';
    const pathology_category = (sorted[0]?.pathology_category as string) || 'Relational';
    const dates = sorted.map((l) => (l.anomaly_date as string) || '').filter(Boolean);
    const date_min = dates.length ? dates[0] : '';
    const date_max = dates.length ? dates[dates.length - 1] : '';
    const document_ids = [...new Set(sorted.map((l) => (l.document_id as string) || '').filter(Boolean))];
    let total_impact = 0;
    if (schemeImpactMap) {
      const val = schemeImpactMap.get(scheme_id);
      if (val != null && Number.isFinite(val)) {
        total_impact = val;
      }
    } else {
      for (const l of sorted) {
        const m = l.monetary_impact;
        if (m != null && m !== '') {
          const n = typeof m === 'number' ? m : parseFloat(String(m));
          if (Number.isFinite(n)) total_impact += n;
        }
      }
    }
    instances.push({
      scheme_id,
      labels: sorted,
      pathology_name,
      pathology_category,
      date_min,
      date_max,
      document_ids,
      total_impact,
    });
  }
  return instances.sort((a, b) => a.date_min.localeCompare(b.date_min));
}

/** Normalize labels and fill scheme/pathology display fields. */
function normalizeLabels(
  rows: AnomalyLabel[],
  schemeJeCountMap?: Map<string, number>,
  schemeImpactMap?: Map<string, number>
): AnomalyLabel[] {
  return rows.map((r) => {
    const scenarioId = (r.scenario_id as string)?.trim();
    const clusterId = (r.cluster_id as string)?.trim();
    const causal = (r.causal_reason_type as string)?.trim();
    const metaFromString = parseMetadataJson(r.metadata_json as string);
    const metaFromObj =
      r.metadata && typeof r.metadata === 'object' && !Array.isArray(r.metadata)
        ? (r.metadata as Record<string, unknown>)
        : {};
    const meta = {
      ...metaFromString,
      ...Object.fromEntries(Object.entries(metaFromObj).map(([k, v]) => [k, v != null ? String(v) : ''])),
    };
    const pathology_name = (r.pathology_name as string) || meta.pathology_name || meta.scheme_type;
    const pathology_category = (r.pathology_category as string) || meta.pathology_category;
    const stage_number = r.stage_number ?? meta.stage_number;
    const stage_name = (r.stage_name as string) || meta.stage_name;
    const perpetrator_id = (r.perpetrator_id as string) || meta.perpetrator_id;
    const counterparty = (r.counterparty as string) || meta.counterparty;
    const total_stages = r.total_stages ?? meta.total_stages;
    const action_amount = (r.action_amount as string | number) ?? meta.action_amount;

    let scenario: string;
    if (scenarioId) {
      scenario = UUID_REGEX.test(scenarioId) ? `Scheme: ${scenarioId.slice(0, 8)}…` : scenarioId;
    } else {
      scenario = clusterId || causal || '—';
    }
    const typeDisplay = friendlyTypeName(r.anomaly_type as string, pathology_name);
    const isScheme = Boolean(scenarioId);
    const pathology_display = pathology_name ? `${pathology_name}${pathology_category ? ` (${pathology_category})` : ''}` : '';
    const schemeJeCount = scenarioId && schemeJeCountMap ? schemeJeCountMap.get(scenarioId) : undefined;
    const schemeImpact =
      scenarioId && schemeImpactMap ? schemeImpactMap.get(scenarioId) : undefined;
    const stageFromMeta = (meta.stage as string) || '';
    const stage_display =
      stageFromMeta ||
      (stage_number != null && total_stages != null
        ? `${stage_number}/${total_stages}`
        : stage_number != null
          ? String(stage_number)
          : '');

    return {
      ...r,
      scenario_display: scenario,
      type_display: typeDisplay,
      is_scheme: isScheme,
      scheme_badge: isScheme ? 'Scheme' : '',
      pathology_name: pathology_name || undefined,
      pathology_category: pathology_category || undefined,
      stage_number: stage_number ?? undefined,
      stage_name: stage_name || undefined,
      stage_display: stage_display || undefined,
      total_stages: total_stages ?? undefined,
      perpetrator_id: perpetrator_id || undefined,
      counterparty: counterparty || undefined,
      counterparty_display: counterparty || '—',
      action_amount: action_amount ?? undefined,
      pathology_display: pathology_display || (isScheme ? '—' : ''),
      scheme_je_count: schemeJeCount,
      monetary_impact:
        schemeImpact != null && Number.isFinite(Number(schemeImpact))
          ? (schemeImpact as unknown as number)
          : r.monetary_impact,
    };
  });
}

/** Build a small graph for one scheme instance: nodes = documents, links = chronological sequence. */
function buildInstanceGraph(instance: SchemeInstance): GraphData {
  const ids = instance.document_ids;
  const nodes = ids.map((id, i) => ({
    id,
    label: id,
    name: id,
    code: id,
    index: i,
  }));
  const links = ids.slice(0, -1).map((id, i) => ({
    source: id,
    target: ids[i + 1]!,
    type: 'sequence',
  }));
  return { nodes, links };
}

export function FraudAnomalyView() {
  const [labels, setLabels] = useState<AnomalyLabel[]>([]);
  const [journalRows, setJournalRows] = useState<JournalEntryRow[]>([]);
  const [selectedLabel, setSelectedLabel] = useState<AnomalyLabel | null>(null);
  const [selectedInstance, setSelectedInstance] = useState<SchemeInstance | null>(null);
  const [categoryFilter, setCategoryFilter] = useState<string>('');
  const [selectedSchemeKey, setSelectedSchemeKey] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const instanceGraphRef = useRef<HTMLDivElement>(null);
  const graphInstanceRef = useRef<unknown>(null);
  const concernedSectionRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    Promise.all([
      loadAnomalyLabels(),
      loadFraudLabels(),
      loadMultiStageLabels().catch(() => null),
      loadJournalEntriesCsv().catch(() => []),
    ])
      .then(([anomaly, fraud, multiStage, jeRows]) => {
        const combined: AnomalyLabel[] = [];
        if (anomaly?.length) combined.push(...anomaly);
        if (fraud?.length) {
          fraud.forEach((f) => {
            if (!combined.some((a) => (a.anomaly_id || a.document_id) === (f.anomaly_id || f.document_id)))
              combined.push(f);
          });
        }
        if (multiStage?.length) {
          multiStage.forEach((m) => {
            const key = (m.anomaly_id || m.document_id) as string;
            const idx = combined.findIndex((a) => (a.anomaly_id || a.document_id) === key);
            if (idx >= 0) {
              combined[idx] = { ...combined[idx], ...m };
            } else {
              combined.push(m);
            }
          });
        }
        setLabels(combined);
        setJournalRows(jeRows ?? []);
        setError(null);
      })
      .catch((e) => {
        setError(e instanceof Error ? e.message : 'Failed to load labels');
        setLabels([]);
        setJournalRows([]);
      })
      .finally(() => setLoading(false));
  }, []);

  const schemeJeCountMap = useMemo(() => {
    const map = new Map<string, number>();
    labels.forEach((l) => {
      const sid = (l.scenario_id as string)?.trim();
      if (!sid) return;
      map.set(sid, (map.get(sid) ?? 0) + 1);
    });
    return map;
  }, [labels]);

  const docImpactMap = useMemo(() => computeDocImpact(journalRows), [journalRows]);
  const schemeImpactMap = useMemo(
    () => computeSchemeImpact(labels, docImpactMap),
    [labels, docImpactMap]
  );

  const displayLabels = useMemo(
    () => normalizeLabels(labels, schemeJeCountMap, schemeImpactMap),
    [labels, schemeJeCountMap, schemeImpactMap]
  );
  const schemeInstances = useMemo(
    () => buildSchemeInstances(displayLabels, schemeImpactMap),
    [displayLabels, schemeImpactMap]
  );

  const filteredLabels = useMemo(() => {
    let out = displayLabels;
    if (categoryFilter) {
      out = out.filter(
        (l) =>
          (l.anomaly_category ?? l.pathology_category ?? l.anomaly_type ?? 'Other') === categoryFilter
      );
    }
    if (selectedSchemeKey) {
      out = out.filter((l) => labelMatchesScheme(l, selectedSchemeKey));
    }
    return out;
  }, [displayLabels, categoryFilter, selectedSchemeKey]);

  // Table should not display multiple rows for the same scheme instance.
  // When scenario_id is present, keep only one representative label per
  // scenario_id; enrich it with aggregated perpetrator/counterparty from all labels in that scheme.
  const tableLabels = useMemo(() => {
    const byScenario = new Map<string, AnomalyLabel>();
    const singles: AnomalyLabel[] = [];
    for (const l of filteredLabels) {
      const sid = (l.scenario_id as string)?.trim();
      if (!sid) {
        singles.push(l);
        continue;
      }
      if (!byScenario.has(sid)) {
        byScenario.set(sid, l);
      }
    }
    const scenarioRows = Array.from(byScenario.values());
    if (scenarioRows.length === 0) return [...scenarioRows, ...singles];
    const enriched = scenarioRows.map((row) => {
      const sid = (row.scenario_id as string)?.trim();
      const allInScheme = displayLabels.filter((l) => (l.scenario_id as string)?.trim() === sid);
      const perpetrators = [...new Set(allInScheme.map((l) => (l.perpetrator_id as string) || '').filter(Boolean))];
      const counterparties = [...new Set(allInScheme.map((l) => (l.counterparty as string) || '').filter(Boolean))];
      return {
        ...row,
        perpetrator_id: perpetrators.length ? perpetrators.join(', ') : row.perpetrator_id,
        counterparty_display: counterparties.length ? counterparties.join(', ') : (row.counterparty_display ?? '—'),
      };
    });
    return [...enriched, ...singles];
  }, [filteredLabels, displayLabels]);

  const categoryOptions = useMemo(() => {
    const set = new Set<string>();
    displayLabels.forEach((l) => {
      const cat = (l.anomaly_category ?? l.pathology_category ?? l.anomaly_type ?? 'Other') as string;
      if (cat) set.add(cat);
    });
    return ['', ...Array.from(set).sort()];
  }, [displayLabels]);

  // When looking at fraud schemes, counts should be per scheme instance
  // (scenario_id), not per label/action. Restrict scheme instances to those
  // that survive the current label filters, then aggregate by category/type.
  const filteredSchemeInstances = useMemo(() => {
    if (!schemeInstances.length) return [] as SchemeInstance[];
    const allowed = new Set<string>();
    filteredLabels.forEach((l) => {
      const sid = (l.scenario_id as string)?.trim();
      if (sid) allowed.add(sid);
    });
    if (allowed.size === 0) return schemeInstances;
    return schemeInstances.filter((inst) => allowed.has(inst.scheme_id));
  }, [schemeInstances, filteredLabels]);

  const byCategory = useMemo(() => {
    const map: Record<string, number> = {};
    filteredSchemeInstances.forEach((inst) => {
      const cat = (inst.pathology_category || 'Other') as string;
      map[cat] = (map[cat] ?? 0) + 1;
    });
    return Object.entries(map).map(([name, count]) => ({ name, count }));
  }, [filteredSchemeInstances]);

  const byType = useMemo(() => {
    const map: Record<string, number> = {};
    filteredSchemeInstances.forEach((inst) => {
      const t = friendlyTypeName(undefined, inst.pathology_name || 'Other');
      map[t] = (map[t] ?? 0) + 1;
    });
    return Object.entries(map)
      .map(([name, value]) => ({ name, value }))
      .sort((a, b) => b.value - a.value)
      .slice(0, 14);
  }, [filteredSchemeInstances]);

  const byPathology = useMemo(() => {
    const map: Record<string, number> = {};
    filteredSchemeInstances.forEach((inst) => {
      const p = (inst.pathology_name || inst.pathology_category || 'Other') as string;
      map[p] = (map[p] ?? 0) + 1;
    });
    return Object.entries(map)
      .map(([name, value]) => ({ name, value }))
      .sort((a, b) => b.value - a.value)
      .slice(0, 12);
  }, [filteredSchemeInstances]);

  const pieData = useMemo(
    () => byCategory.map((d, i) => ({ ...d, fill: COLORS[i % COLORS.length] })),
    [byCategory]
  );

  // When a scheme filter is active, only show concerned transaction if the selected row matches the filter.
  // Depend on anomaly_id so changing selection (different row) always recomputes even if ref is reused.
  const selectedLabelInFilter = useMemo(() => {
    if (!selectedLabel) return null;
    if (!selectedSchemeKey) return selectedLabel;
    return labelMatchesScheme(selectedLabel, selectedSchemeKey) ? selectedLabel : null;
  }, [selectedLabel, selectedSchemeKey, selectedLabel?.anomaly_id]);

  // Scroll concerned transaction section into view when selection changes so the user sees the update
  useEffect(() => {
    if (selectedLabelInFilter && concernedSectionRef.current) {
      concernedSectionRef.current.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    }
  }, [selectedLabelInFilter?.anomaly_id, selectedLabelInFilter?.document_id]);

  const concernedRows = useMemo(() => {
    const docId =
      selectedLabelInFilter?.document_id != null ? String(selectedLabelInFilter.document_id).trim() : '';
    const scenarioId =
      selectedLabelInFilter?.scenario_id != null ? String(selectedLabelInFilter.scenario_id).trim() : '';
    if (!docId && !scenarioId) return [];
    const norm = (s: string) => s.trim().toLowerCase();

    // If this is a scheme label (scenario_id present), show ALL JEs for the scheme,
    // not just the single document. We collect all document_ids for this scenario_id
    // from the normalized labels and then pull every matching journal entry, sorted
    // chronologically.
    let idsToMatch: string[] = [];
    if (scenarioId) {
      const docsForScheme = displayLabels
        .filter((l) => (l.scenario_id as string)?.trim() === scenarioId)
        .map((l) => (l.document_id as string) || '')
        .filter(Boolean);
      const idSet = new Set<string>();
      docsForScheme.forEach((d) => {
        const full = norm(d);
        if (full) idSet.add(full);
        if (d.startsWith('scheme-')) {
          const bare = norm(d.slice(7));
          if (bare) idSet.add(bare);
        }
      });

      // Fallback: if no docs discovered, fall back to the row's own document_id.
      if (idSet.size === 0 && docId) {
        const bare = docId.startsWith('scheme-') ? docId.slice(7) : docId;
        [docId, bare].map(norm).forEach((id) => {
          if (id) idSet.add(id);
        });
      }
      idsToMatch = Array.from(idSet);
    } else if (docId) {
      const bare = docId.startsWith('scheme-') ? docId.slice(7) : docId;
      idsToMatch = [docId, bare].map(norm);
    }

    const idSet = new Set(idsToMatch);
    const rows = journalRows.filter((r) => {
      const rowId = norm(String(r.document_id ?? ''));
      if (!idSet.has(rowId)) return false;
      // Drop pure-zero lines (debit=0 and credit=0) to avoid noise in scheme views.
      const d = Number.parseFloat(String(r.debit_amount ?? '0') || '0');
      const c = Number.parseFloat(String(r.credit_amount ?? '0') || '0');
      return !Number.isFinite(d) || !Number.isFinite(c) || Math.abs(d) > 1e-6 || Math.abs(c) > 1e-6;
    });

    const withImpact = rows.map((r) => {
      const key = norm(String(r.document_id ?? ''));
      const impact = docImpactMap.get(key);
      return {
        ...r,
        entry_impact: impact,
      };
    });

    // Sort chronologically (posting_date) then by line_number.
    return withImpact.sort((a, b) => {
      const da = String(a.posting_date ?? '');
      const db = String(b.posting_date ?? '');
      const cmp = da.localeCompare(db);
      if (cmp !== 0) return cmp;
      const la = Number(a.line_number ?? 0);
      const lb = Number(b.line_number ?? 0);
      return la - lb;
    });
  }, [selectedLabelInFilter, displayLabels, journalRows, docImpactMap]);

  const concernedChartData = useMemo(() => {
    return concernedRows.map((r, i) => {
      const debit = typeof r.debit_amount === 'number' ? r.debit_amount : parseFloat(String(r.debit_amount || 0));
      const credit = typeof r.credit_amount === 'number' ? r.credit_amount : parseFloat(String(r.credit_amount || 0));
      return {
        line: `L${r.line_number ?? i + 1}`,
        debit: Number.isFinite(debit) ? debit : 0,
        credit: Number.isFinite(credit) ? credit : 0,
        gl_account: r.gl_account ?? '',
      };
    });
  }, [concernedRows]);

  const instanceGraphData = useMemo(
    () => (selectedInstance ? buildInstanceGraph(selectedInstance) : null),
    [selectedInstance]
  );

  useEffect(() => {
    if (!selectedInstance || !instanceGraphRef.current || !instanceGraphData) {
      graphInstanceRef.current = null;
      return;
    }
    const container = instanceGraphRef.current;
    let graph: unknown = null;
    import('force-graph').then((module) => {
      if (!container) return;
      const ForceGraph = module.default;
      graph = new ForceGraph(container);
      const g = graph as {
        nodeLabel: (fn: (n: unknown) => string) => unknown;
        nodeAutoColorBy: (key: string) => unknown;
        linkDirectionalArrowLength: (n: number) => unknown;
        linkDirectionalArrowRelPos: (n: number) => unknown;
        linkCurvature: (n: number) => unknown;
        graphData: (d: GraphData) => void;
        destroy?: () => void;
      };
      g.nodeLabel((n: unknown) => (n as { id?: string }).id ?? '');
      g.nodeAutoColorBy('index');
      g.linkDirectionalArrowLength(3);
      g.linkDirectionalArrowRelPos(1);
      g.linkCurvature(0.2);
      g.graphData(instanceGraphData);
      graphInstanceRef.current = graph;
    });
    return () => {
      if (graph && typeof (graph as { destroy?: () => void }).destroy === 'function') {
        (graph as { destroy: () => void }).destroy();
      }
      graphInstanceRef.current = null;
    };
  }, [selectedInstance, instanceGraphData]);

  const [expandedChart, setExpandedChart] = useState<'category' | 'type' | 'pathology' | 'scheme' | null>(null);

  const ExpandIcon = () => (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M8 3H5a2 2 0 0 0-2 2v3M21 8V5a2 2 0 0 0-2-2h-3M3 16v3a2 2 0 0 0 2 2h3M16 21h3a2 2 0 0 0 2-2v-3" />
    </svg>
  );

  if (loading) return <div className="fraud-anomaly-view loading">Loading fraud and anomaly labels…</div>;
  if (error && labels.length === 0) return <div className="fraud-anomaly-view error">Error: {error}</div>;

  return (
    <div className="fraud-anomaly-view">
      <h2>Fraud, Anomalies &amp; Schemes</h2>
      <p className="fraud-anomaly-desc">
        Labels from <code>labels/anomaly_labels</code> and <code>labels/fraud_labels</code>. All{' '}
        <strong>7 fraud scheme types</strong> (RIP-GNN pathology lab) are supported: Gradual Embezzlement, Revenue
        Manipulation, Vendor Kickback, Triad Bypass, Shadow Payroll, Expense Laundering, Smurfing. Scheme instances are grouped by <code>scenario_id</code>.
        Click a row to view the concerned transaction; select a scheme instance to see its instance-wise graph.
      </p>
      {labels.length === 0 ? (
        <p className="fraud-anomaly-empty">
          No anomaly or fraud labels in output. Enable anomaly injection in config to generate labels.
        </p>
      ) : (
        <>
          <div className="fraud-anomaly-filter">
            <label htmlFor="fraud-category-filter">Filter by category:</label>
            <select
              id="fraud-category-filter"
              value={categoryFilter}
              onChange={(e) => setCategoryFilter(e.target.value)}
            >
              <option value="">All categories</option>
              {categoryOptions
                .filter((c) => c !== '')
                .map((cat) => (
                  <option key={cat} value={cat}>
                    {cat}
                  </option>
                ))}
            </select>
            <span className="fraud-anomaly-filter-count">
              {filteredLabels.length} label{filteredLabels.length !== 1 ? 's' : ''}
              {schemeInstances.length > 0 && ` · ${schemeInstances.length} scheme instance${schemeInstances.length !== 1 ? 's' : ''}`}
            </span>
          </div>

          <section className="scheme-taxonomy" aria-label="Fraud scheme taxonomy">
            <h3>Scheme taxonomy (10 types)</h3>
            <p className="scheme-taxonomy-hint">Click a scheme to show only its instances in the table below.</p>
            <div className="scheme-taxonomy-grid">
              {SCHEME_TAXONOMY.map((s) => {
                const isSelected = selectedSchemeKey === s.key;
                return (
                  <button
                    key={s.key}
                    type="button"
                    className={`scheme-taxonomy-card ${isSelected ? 'scheme-taxonomy-card--selected' : ''}`}
                    data-category={s.category}
                    onClick={() => {
                      const next = isSelected ? null : s.key;
                      setSelectedSchemeKey(next);
                      if (next) {
                        setSelectedLabel(null);
                        setSelectedInstance(null);
                      }
                    }}
                    aria-pressed={isSelected}
                    aria-label={`Filter by ${s.name}. ${isSelected ? 'Press again to clear filter.' : ''}`}
                  >
                    <span className="scheme-taxonomy-name">{s.name}</span>
                    <span className="scheme-taxonomy-cat">{s.category}</span>
                    <p className="scheme-taxonomy-desc">{s.description}</p>
                  </button>
                );
              })}
            </div>
            {selectedSchemeKey && (
              <div className="scheme-taxonomy-filter-active">
                <span>
                  Showing only: <strong>{SCHEME_TAXONOMY.find((x) => x.key === selectedSchemeKey)?.name ?? selectedSchemeKey}</strong>
                </span>
                <button type="button" className="scheme-taxonomy-clear" onClick={() => setSelectedSchemeKey(null)} aria-label="Clear scheme filter">
                  Clear filter
                </button>
              </div>
            )}
          </section>

          <div className="fraud-anomaly-charts">
            <div className="chart-box">
              <div className="chart-box-header">
                <h3>By Category</h3>
                <button type="button" className="chart-expand-btn" onClick={() => setExpandedChart('category')} title="View larger" aria-label="View By Category chart larger">
                  <ExpandIcon />
                </button>
              </div>
              <ResponsiveContainer width="100%" height={260}>
                <PieChart>
                  <Pie data={pieData} dataKey="count" nameKey="name" cx="50%" cy="50%" outerRadius={90} label={false}>
                    {pieData.map((entry) => (
                      <Cell key={entry.name} fill={entry.fill} />
                    ))}
                  </Pie>
                  <Tooltip />
                  <Legend />
                </PieChart>
              </ResponsiveContainer>
            </div>
            <div className="chart-box">
              <div className="chart-box-header">
                <h3>By Type (top 14)</h3>
                <button type="button" className="chart-expand-btn" onClick={() => setExpandedChart('type')} title="View larger" aria-label="View By Type chart larger">
                  <ExpandIcon />
                </button>
              </div>
              <ResponsiveContainer width="100%" height={300}>
                <BarChart data={byType} layout="vertical" margin={{ top: 5, right: 20, left: 140, bottom: 20 }}>
                  <XAxis type="number" tick={{ fontSize: 11 }} />
                  <YAxis dataKey="name" type="category" width={140} tick={{ fontSize: 11 }} interval={0} />
                  <Tooltip />
                  <Bar dataKey="value" fill="#4a7cff" name="Count" radius={[0, 4, 4, 0]} />
                </BarChart>
              </ResponsiveContainer>
            </div>
            <div className="chart-box">
              <div className="chart-box-header">
                <h3>By Pathology (top 12)</h3>
                <button type="button" className="chart-expand-btn" onClick={() => setExpandedChart('pathology')} title="View larger" aria-label="View By Pathology chart larger">
                  <ExpandIcon />
                </button>
              </div>
              <ResponsiveContainer width="100%" height={300}>
                <BarChart data={byPathology} layout="vertical" margin={{ top: 5, right: 20, left: 140, bottom: 20 }}>
                  <XAxis type="number" tick={{ fontSize: 11 }} />
                  <YAxis dataKey="name" type="category" width={140} tick={{ fontSize: 11 }} interval={0} />
                  <Tooltip />
                  <Bar dataKey="value" fill="#9b59b6" name="Count" radius={[0, 4, 4, 0]} />
                </BarChart>
              </ResponsiveContainer>
            </div>
          </div>

          {expandedChart && (
            <div
              className="chart-modal-overlay"
              role="dialog"
              aria-modal="true"
              aria-label={`${expandedChart} chart expanded`}
              onClick={() => setExpandedChart(null)}
            >
              <div className="chart-modal" onClick={(e) => e.stopPropagation()}>
                <div className="chart-modal-header">
                  <h3>
                    {expandedChart === 'category' && 'By Category'}
                    {expandedChart === 'type' && 'By Type (top 14)'}
                    {expandedChart === 'pathology' && 'By Pathology (top 12)'}
                    {expandedChart === 'scheme' && 'By Scenario / Scheme'}
                  </h3>
                  <button type="button" className="chart-modal-close" onClick={() => setExpandedChart(null)} aria-label="Close">
                    <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <path d="M18 6L6 18M6 6l12 12" />
                    </svg>
                  </button>
                </div>
                <div className="chart-modal-body">
                  {expandedChart === 'category' && (
                    <ResponsiveContainer width="100%" height={420}>
                      <PieChart>
                        <Pie data={pieData} dataKey="count" nameKey="name" cx="50%" cy="50%" outerRadius={140} label={false}>
                          {pieData.map((entry) => (
                            <Cell key={entry.name} fill={entry.fill} />
                          ))}
                        </Pie>
                        <Tooltip />
                        <Legend />
                      </PieChart>
                    </ResponsiveContainer>
                  )}
                  {expandedChart === 'type' && (
                    <ResponsiveContainer width="100%" height={420}>
                      <BarChart data={byType} layout="vertical" margin={{ top: 8, right: 24, left: 140, bottom: 24 }}>
                        <XAxis type="number" tick={{ fontSize: 12 }} />
                        <YAxis dataKey="name" type="category" width={140} tick={{ fontSize: 12 }} interval={0} />
                        <Tooltip />
                        <Bar dataKey="value" fill="#4a7cff" name="Count" radius={[0, 4, 4, 0]} />
                      </BarChart>
                    </ResponsiveContainer>
                  )}
                  {expandedChart === 'pathology' && (
                    <ResponsiveContainer width="100%" height={420}>
                      <BarChart data={byPathology} layout="vertical" margin={{ top: 8, right: 24, left: 140, bottom: 24 }}>
                        <XAxis type="number" tick={{ fontSize: 12 }} />
                        <YAxis dataKey="name" type="category" width={140} tick={{ fontSize: 12 }} interval={0} />
                        <Tooltip />
                        <Bar dataKey="value" fill="#9b59b6" name="Count" radius={[0, 4, 4, 0]} />
                      </BarChart>
                    </ResponsiveContainer>
                  )}
                </div>
              </div>
            </div>
          )}

          <h3>Label table (one row per scheme instance; click to view concerned scheme)</h3>
          <DataTable
            data={tableLabels as unknown as Record<string, unknown>[]}
            columns={TABLE_COLUMNS}
            keyField="anomaly_id"
            pageSize={50}
            maxHeight="40vh"
            onRowClick={(row) => {
              const r = row as AnomalyLabel;
              const id = r.anomaly_id != null ? String(r.anomaly_id) : null;
              const found = id ? tableLabels.find((l) => String(l.anomaly_id) === id) : null;
              setSelectedLabel(found ?? r);
            }}
            selectedRowKey={selectedLabel?.anomaly_id != null ? String(selectedLabel.anomaly_id) : null}
          />
          {selectedLabel && !selectedLabelInFilter && selectedSchemeKey && (
            <p className="fraud-anomaly-concerned-hint">
              The selected row is not in the current scheme filter. Click a row from the table above or clear the scheme filter to view its transaction.
            </p>
          )}
          {selectedLabelInFilter && (
            <div
              key={selectedLabelInFilter.anomaly_id ?? selectedLabelInFilter.scenario_id ?? 'concerned'}
              ref={concernedSectionRef}
              className="fraud-anomaly-concerned"
            >
              <h3>
                Concerned scheme instance:{' '}
                {selectedLabelInFilter.scenario_display ?? selectedLabelInFilter.scenario_id ?? '—'}
              </h3>
              <InvolvedEntitiesSummary
                scenarioId={selectedLabelInFilter.scenario_id as string}
                labels={displayLabels}
              />
              <p className="fraud-anomaly-concerned-desc">
                Journal entry lines for all documents in scheme instance{' '}
                <code>{selectedLabelInFilter.scenario_id}</code> (anomaly {selectedLabelInFilter.anomaly_id},{' '}
                {selectedLabelInFilter.type_display ??
                  selectedLabelInFilter.anomaly_type ??
                  selectedLabelInFilter.pathology_name ??
                  selectedLabelInFilter.anomaly_category}
                ).
              </p>
              {concernedRows.length === 0 ? (
                <p className="fraud-anomaly-concerned-empty">
                  No journal lines found for this document in <code>journal_entries.csv</code>.
                  {selectedLabelInFilter.document_id && String(selectedLabelInFilter.document_id).startsWith('scheme-') ? (
                    <> Scheme transactions are written when you generate with the current datasynth (one JE per scheme action, <code>document_id</code> = scheme UUID). Regenerate with your config, then point the viewer at that output folder (Load data) and reload.</>
                  ) : (
                    <> The document may be from another run or format.</>
                  )}
                </p>
              ) : (
                <>
                  <div className="fraud-anomaly-concerned-chart">
                    <ResponsiveContainer width="100%" height={220}>
                      <BarChart data={concernedChartData} margin={{ top: 5, right: 20, left: 5, bottom: 30 }} barCategoryGap="20%">
                        <XAxis dataKey="line" tick={{ fontSize: 11 }} />
                        <YAxis tickFormatter={(v) => (v >= 1e6 ? `${(v / 1e6).toFixed(1)}M` : v.toLocaleString())} />
                        <Tooltip formatter={(v: number | undefined) => formatNum(v)} />
                        <Bar dataKey="debit" name="Debit" fill="#2ecc71" radius={[4, 4, 0, 0]} />
                        <Bar dataKey="credit" name="Credit" fill="#e74c3c" radius={[4, 4, 0, 0]} />
                        <Legend />
                      </BarChart>
                    </ResponsiveContainer>
                  </div>
                  <DataTable
                    data={concernedRows.map((r, i) => ({
                      ...r,
                      // All rows in this table belong to the same scheme instance; attach it explicitly
                      scenario_id: selectedLabelInFilter.scenario_id,
                      _rowKey: `${r.document_id}-${r.line_number ?? i}`,
                    })) as unknown as Record<string, unknown>[]}
                    columns={CONCERNED_FULL_COLUMNS}
                    keyField="_rowKey"
                    pageSize={20}
                    maxHeight="40vh"
                  />
                </>
              )}
            </div>
          )}
        </>
      )}
    </div>
  );
}
