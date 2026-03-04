import { useId, useMemo, useState, useEffect } from 'react';
import './DataTable.css';

const DEFAULT_PAGE_SIZE_OPTIONS = [50, 100, 200, 500];

/** True if string looks like ISO date (YYYY-MM-DD or with time). */
function isIsoDateString(s: string): boolean {
  return /^\d{4}-\d{2}-\d{2}/.test(s.trim());
}

/** True if string is YYYYMMDD (e.g. FEC date). */
function isYyyyMmDd(s: string): boolean {
  return /^\d{8}$/.test(s.trim());
}

/** Normalize to YYYY-MM-DD for comparable date sort (handles datetime strings). */
function datePart(s: string): string {
  return s.trim().slice(0, 10);
}

/** Parse YYYYMMDD to a string comparable with datePart (YYYY-MM-DD). */
function yyyyMmDdToIso(s: string): string {
  const t = s.trim();
  if (t.length >= 8) return `${t.slice(0, 4)}-${t.slice(4, 6)}-${t.slice(6, 8)}`;
  return t;
}

interface DataTableProps<T extends Record<string, unknown>> {
  data: T[];
  columns: {
    key: keyof T | string;
    label: string;
    width?: string;
    format?: (v: unknown) => string;
    /** Whether this column is sortable (default true). */
    sortable?: boolean;
  }[];
  keyField?: keyof T | string;
  pageSize?: number;
  /** Options for rows-per-page selector; default [50, 100, 200, 500]. Pass [] to hide. */
  pageSizeOptions?: number[];
  maxHeight?: string;
  /** Optional row click handler (e.g. to select row for detail view) */
  onRowClick?: (row: T) => void;
  /** Optional class name for the selected row (when onRowClick is used and row is selected) */
  selectedRowKey?: string | null;
  /** Initial sort column key (e.g. "posting_date", "anomaly_date"). */
  defaultSortKey?: string | null;
  /** Initial sort direction. */
  defaultSortDir?: 'asc' | 'desc';
}

export function DataTable<T extends Record<string, unknown>>({
  data,
  columns,
  keyField,
  pageSize = 50,
  pageSizeOptions = DEFAULT_PAGE_SIZE_OPTIONS,
  maxHeight = '60vh',
  onRowClick,
  selectedRowKey = null,
  defaultSortKey = null,
  defaultSortDir = 'asc',
}: DataTableProps<T>) {
  const paginationId = useId();
  const goToPageId = useId();
  const [page, setPage] = useState(0);
  const [sortKey, setSortKey] = useState<string | null>(defaultSortKey ?? null);
  const [sortDir, setSortDir] = useState<'asc' | 'desc'>(defaultSortDir);
  const [goToPageInput, setGoToPageInput] = useState('');
  const [currentPageSize, setCurrentPageSize] = useState(() =>
    pageSizeOptions.length && pageSizeOptions.includes(pageSize)
      ? pageSize
      : pageSizeOptions[0] ?? pageSize
  );
  const rowCount = data.length;
  const totalPages =
    rowCount === 0 ? 1 : Math.max(1, Math.ceil(rowCount / currentPageSize));
  useEffect(() => {
    if (pageSizeOptions.length && !pageSizeOptions.includes(currentPageSize)) {
      setCurrentPageSize(pageSizeOptions[0] ?? 50);
    }
  }, [pageSizeOptions, currentPageSize]);
  useEffect(() => {
    if (page >= totalPages && totalPages > 0) setPage(totalPages - 1);
  }, [page, totalPages]);
  useEffect(() => {
    setPage(0);
  }, [data]);

  const getVal = (row: T, key: keyof T | string): unknown => {
    const k = key as string;
    return k in row ? row[k] : '';
  };

  const sorted = useMemo(() => {
    if (!sortKey) return data;
    const copy = [...data];
    const key = sortKey;

    const cmp = (a: unknown, b: unknown): number => {
      if (a == null && b == null) return 0;
      if (a == null || a === '') return -1;
      if (b == null || b === '') return 1;

      const as = String(a).trim();
      const bs = String(b).trim();

      // Chronological sort for ISO date strings (YYYY-MM-DD or YYYY-MM-DDTHH:mm:ss)
      if (isIsoDateString(as) && isIsoDateString(bs)) {
        const da = datePart(as);
        const db = datePart(bs);
        return da.localeCompare(db);
      }
      // FEC-style YYYYMMDD (no hyphen)
      if (isYyyyMmDd(as) && isYyyyMmDd(bs)) {
        return yyyyMmDdToIso(as).localeCompare(yyyyMmDdToIso(bs));
      }
      // One ISO, one YYYYMMDD: normalize both to YYYY-MM-DD for comparison
      const normA = isIsoDateString(as) ? datePart(as) : isYyyyMmDd(as) ? yyyyMmDdToIso(as) : as;
      const normB = isIsoDateString(bs) ? datePart(bs) : isYyyyMmDd(bs) ? yyyyMmDdToIso(bs) : bs;
      if (isIsoDateString(normA) && isIsoDateString(normB)) {
        return normA.localeCompare(normB);
      }

      const an =
        typeof a === 'number'
          ? a
          : Number(String(a).replace(/,/g, ''));
      const bn =
        typeof b === 'number'
          ? b
          : Number(String(b).replace(/,/g, ''));
      if (!Number.isNaN(an) && !Number.isNaN(bn)) {
        return an === bn ? 0 : an < bn ? -1 : 1;
      }

      return as.localeCompare(bs);
    };

    copy.sort((ra, rb) => {
      const av = getVal(ra, key);
      const bv = getVal(rb, key);
      const base = cmp(av, bv);
      if (base !== 0) return sortDir === 'asc' ? base : -base;
      // Tie-break by line/écriture number when sorting by date (same FEC/document)
      const dateSortKeys = ['posting_date', 'anomaly_date', "Date de comptabilisation"];
      if (dateSortKeys.includes(key)) {
        const lineKey = key === "Date de comptabilisation" ? "Numéro de l'écriture" : 'line_number';
        const la = getVal(ra, lineKey);
        const lb = getVal(rb, lineKey);
        if (la != null && lb != null) {
          const na = typeof la === 'number' ? la : Number(la);
          const nb = typeof lb === 'number' ? lb : Number(lb);
          if (!Number.isNaN(na) && !Number.isNaN(nb)) {
            return sortDir === 'asc' ? na - nb : nb - na;
          }
        }
      }
      return 0;
    });
    return copy;
  }, [data, sortKey, sortDir]);

  const slice = useMemo(
    () =>
      sorted.slice(
        page * currentPageSize,
        (page + 1) * currentPageSize
      ),
    [sorted, page, currentPageSize]
  );

  const handleSort = (key: string, enabled: boolean) => {
    if (!enabled) return;
    setSortKey((prev) => {
      if (prev === key) {
        setSortDir((d) => (d === 'asc' ? 'desc' : 'asc'));
        return prev;
      }
      setSortDir('asc');
      return key;
    });
    setPage(0);
  };

  return (
    <div className="data-table-wrap">
      <div className="data-table-scroll" style={{ maxHeight }}>
        <table className="data-table">
          <thead>
            <tr>
              {columns.map((col) => {
                const keyStr = String(col.key);
                const sortable = col.sortable !== false;
                const isActive = sortKey === keyStr;
                const indicator = isActive
                  ? sortDir === 'asc'
                    ? ' ▲'
                    : ' ▼'
                  : '';
                return (
                  <th
                    key={keyStr}
                    style={col.width ? { width: col.width } : undefined}
                    className={
                      sortable ? 'data-table-header-sortable' : undefined
                    }
                    onClick={() => handleSort(keyStr, sortable)}
                  >
                    {col.label}
                    {indicator}
                  </th>
                );
              })}
            </tr>
          </thead>
          <tbody key={`page-${page}`}>
            {slice.map((row, i) => {
              // Unique key per row: keyField + secondary when present; always add slice index so keys are unique even when data has duplicate (document_id, line_number)
              const baseKey = keyField ? String(getVal(row, keyField)) : '';
              const secondary =
                keyField && getVal(row, 'line_number') != null
                  ? String(getVal(row, 'line_number'))
                  : keyField && getVal(row, "Numéro de compte") != null
                    ? String(getVal(row, "Numéro de compte"))
                    : null;
              const sliceIndex = page * currentPageSize + i;
              const key =
                (baseKey && secondary !== null
                  ? `${baseKey}-${secondary}-${sliceIndex}`
                  : baseKey
                    ? `${baseKey}-${sliceIndex}`
                    : `row-${sliceIndex}`) ||
                `row-${sliceIndex}`;
              const isSelected = selectedRowKey != null && baseKey === selectedRowKey;
              return (
              <tr
                key={key}
                className={onRowClick ? (isSelected ? 'data-table-row-selectable data-table-row-selected' : 'data-table-row-selectable') : ''}
                onClick={onRowClick ? () => onRowClick(row) : undefined}
                role={onRowClick ? 'button' : undefined}
              >
                {columns.map((col) => {
                  const v = getVal(row, col.key);
                  return (
                    <td key={String(col.key)}>
                      {col.format ? col.format(v) : v != null ? String(v) : ''}
                    </td>
                  );
                })}
              </tr>
            );
            })}
          </tbody>
        </table>
      </div>
      {(totalPages > 1 || pageSizeOptions.length > 0) && (
        <div className="data-table-pagination">
          {pageSizeOptions.length > 0 && (
            <div className="data-table-page-size">
              <label htmlFor={paginationId}>Rows per page:</label>
              <select
                id={paginationId}
                value={currentPageSize}
                onChange={(e) => {
                  const v = Number(e.target.value);
                  if (Number.isFinite(v) && v > 0) {
                    setCurrentPageSize(v);
                    setPage(0);
                  }
                }}
              >
                {pageSizeOptions.map((n) => (
                  <option key={n} value={n}>
                    {n}
                  </option>
                ))}
              </select>
            </div>
          )}
          {totalPages > 1 && (
            <>
              <button
                type="button"
                disabled={page <= 0}
                onClick={() => setPage((p) => Math.max(0, p - 1))}
              >
                Previous
              </button>
              <span>
                Page {page + 1} of {totalPages} ({rowCount} rows)
              </span>
              <span className="data-table-go-to-page">
                <label htmlFor={goToPageId}>Go to page</label>
                <input
                  id={goToPageId}
                  type="number"
                  min={1}
                  max={totalPages}
                  value={goToPageInput}
                  onChange={(e) => setGoToPageInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      e.preventDefault();
                      const n = Number(goToPageInput);
                      if (Number.isFinite(n) && n >= 1 && n <= totalPages) {
                        setPage(n - 1);
                        setGoToPageInput('');
                      }
                    }
                  }}
                  placeholder={`1–${totalPages}`}
                  aria-label="Page number"
                />
                <button
                  type="button"
                  onClick={() => {
                    const n = Number(goToPageInput);
                    if (Number.isFinite(n) && n >= 1 && n <= totalPages) {
                      setPage(n - 1);
                      setGoToPageInput('');
                    }
                  }}
                >
                  Go
                </button>
              </span>
              <button
                type="button"
                disabled={page + 1 >= totalPages || rowCount === 0}
                onClick={() =>
                  setPage((p) => Math.min(totalPages - 1, p + 1))
                }
              >
                Next
              </button>
            </>
          )}
        </div>
      )}
    </div>
  );
}
