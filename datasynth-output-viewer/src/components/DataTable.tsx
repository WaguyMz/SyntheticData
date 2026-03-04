import { useId, useMemo, useState, useEffect } from 'react';
import './DataTable.css';

const DEFAULT_PAGE_SIZE_OPTIONS = [50, 100, 200, 500];

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
}: DataTableProps<T>) {
  const paginationId = useId();
  const [page, setPage] = useState(0);
  const [sortKey, setSortKey] = useState<string | null>(null);
  const [sortDir, setSortDir] = useState<'asc' | 'desc'>('asc');
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
      if (a == null) return -1;
      if (b == null) return 1;

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

      const as = String(a);
      const bs = String(b);
      return as.localeCompare(bs);
    };

    copy.sort((ra, rb) => {
      const av = getVal(ra, key);
      const bv = getVal(rb, key);
      const base = cmp(av, bv);
      return sortDir === 'asc' ? base : -base;
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
          <tbody>
            {slice.map((row, i) => {
              const key = keyField ? String(getVal(row, keyField)) || `row-${page * currentPageSize + i}` : `row-${page * currentPageSize + i}`;
              const isSelected = selectedRowKey != null && key === selectedRowKey;
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
