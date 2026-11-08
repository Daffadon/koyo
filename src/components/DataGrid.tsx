import { useMemo, useState } from "react";
import { ArrowDown, ArrowUp, ArrowUpDown, Plus, Square, Trash2, Undo2 } from "lucide-react";
import type {
  Cell,
  ColumnInfo,
  FilterKind,
  FilterOperator,
  GridChange,
  GridRow,
  QueryResultSet,
  TableFilter,
  TablePage,
  TableSort,
} from "../api";

export type StagedGridChange = GridChange & { localId: string };

type EditableProps = {
  page: TablePage;
  changes: StagedGridChange[];
  filters: TableFilter[];
  sort: TableSort | null;
  busy: boolean;
  cancelling: boolean;
  onChanges: (changes: StagedGridChange[]) => void;
  onPage: (offset: number) => void;
  onApplyFilters: (filters: TableFilter[]) => void;
  onSort: (column: string) => void;
  onCancel: () => void;
};

type FilterDraft = {
  operator: FilterOperator | "none";
  value: string;
};

function needsValue(operator: FilterOperator | "none") {
  return [
    "contains",
    "equals",
    "notEquals",
    "lessThan",
    "lessThanOrEqual",
    "greaterThan",
    "greaterThanOrEqual",
  ].includes(operator);
}

function operatorOptions(kind: FilterKind): Array<{ value: FilterOperator; label: string }> {
  switch (kind) {
    case "text":
      return [
        { value: "contains", label: "contains" },
        { value: "equals", label: "equals" },
        { value: "notEquals", label: "not equals" },
        { value: "isNull", label: "is null" },
        { value: "isNotNull", label: "is not null" },
      ];
    case "number":
    case "temporal":
      return [
        { value: "equals", label: "equals" },
        { value: "notEquals", label: "not equals" },
        { value: "lessThan", label: "less than" },
        { value: "lessThanOrEqual", label: "at most" },
        { value: "greaterThan", label: "greater than" },
        { value: "greaterThanOrEqual", label: "at least" },
        { value: "isNull", label: "is null" },
        { value: "isNotNull", label: "is not null" },
      ];
    case "boolean":
      return [
        { value: "isTrue", label: "is true" },
        { value: "isFalse", label: "is false" },
        { value: "isNull", label: "is null" },
        { value: "isNotNull", label: "is not null" },
      ];
    case "other":
      return [
        { value: "isNull", label: "is null" },
        { value: "isNotNull", label: "is not null" },
      ];
  }
}


export function DataGrid({
  page,
  changes,
  filters,
  sort,
  busy,
  cancelling,
  onChanges,
  onPage,
  onApplyFilters,
  onSort,
  onCancel,
}: EditableProps) {
  const [drafts, setDrafts] = useState<Record<string, FilterDraft>>(() =>
    Object.fromEntries(
      page.columns.map((column) => {
        const filter = filters.find((item) => item.column === column.name);
        return [
          column.name,
          {
            operator: filter?.operator ?? "none",
            value: filter?.value ?? "",
          },
        ];
      }),
    ),
  );
  const pendingByKey = useMemo(() => {
    const map = new Map<string, StagedGridChange>();
    for (const change of changes) {
      if (change.kind !== "insert") map.set(JSON.stringify(change.key), change);
    }
    return map;
  }, [changes]);
  const inserts = changes.filter((change) => change.kind === "insert");
  const pageNumber = Math.floor(page.offset / page.limit) + 1;
  const filterLocked = changes.length > 0 || busy;
  const hasDrafts = Object.values(drafts).some((draft) => draft.operator !== "none");
  const hasAppliedFilters = filters.length > 0;
  const hasIncompleteFilter = Object.values(drafts).some(
    (draft) => draft.operator !== "none" && needsValue(draft.operator) && draft.value === "",
  );

  function applyFilters() {
    if (filterLocked || hasIncompleteFilter) return;
    const next = page.columns.flatMap((column) => {
      const draft = drafts[column.name];
      if (!draft || draft.operator === "none") return [];
      if (needsValue(draft.operator) && draft.value === "") return [];
      return [{
        column: column.name,
        operator: draft.operator,
        value: needsValue(draft.operator) ? draft.value : null,
      }];
    });
    onApplyFilters(next);
  }

  function clearFilters() {
    if (filterLocked) return;
    setDrafts(Object.fromEntries(
      page.columns.map((column) => [column.name, { operator: "none" as const, value: "" }]),
    ));
    onApplyFilters([]);
  }

  function updateFilter(column: string, patch: Partial<FilterDraft>) {
    setDrafts((current) => ({
      ...current,
      [column]: { ...current[column], ...patch },
    }));
  }

  function updateCell(row: GridRow, column: ColumnInfo, index: number, value: Cell) {
    const key = JSON.stringify(row.key);
    const existing = pendingByKey.get(key);
    if (existing?.kind === "delete") return;
    const values: Record<string, Cell> = existing?.kind === "update" ? { ...existing.values } : {};
    if (row.values[index] === value) delete values[column.name];
    else values[column.name] = value;

    const next = changes.filter((change) => !(change.kind !== "insert" && JSON.stringify(change.key) === key));
    if (Object.keys(values).length > 0) {
      next.push({
        localId: key,
        kind: "update",
        key: row.key,
        version: row.version,
        values,
        original: row.values,
      });
    }
    onChanges(next);
  }

  function toggleDelete(row: GridRow) {
    const key = JSON.stringify(row.key);
    const existing = pendingByKey.get(key);
    if (existing?.kind === "update" && !window.confirm("Discard staged edits to this row and stage its deletion?")) return;
    const next = changes.filter((change) => !(change.kind !== "insert" && JSON.stringify(change.key) === key));
    if (existing?.kind !== "delete") {
      next.push({ localId: key, kind: "delete", key: row.key, version: row.version, original: row.values });
    }
    onChanges(next);
  }

  function updateInsert(localId: string, column: ColumnInfo, mode: "default" | "value" | "null", value = "") {
    onChanges(changes.map((change) => {
      if (change.kind !== "insert" || change.localId !== localId) return change;
      const values = { ...change.values };
      if (mode === "default") delete values[column.name];
      else values[column.name] = mode === "null" ? null : value;
      return { ...change, values };
    }));
  }

  return (
    <div className="data-panel">
      <div className="data-toolbar">
        <div className="table-context">
          <span className="table-context-label">Table data</span>
          {page.editable ? (
            <span className="editable-indicator">Primary key · edits staged</span>
          ) : (
            <span className="readonly-indicator">Read only · no primary key</span>
          )}
        </div>
        <div className="table-tools">
          {busy && (
            <button className="button-small" onClick={onCancel} disabled={cancelling}>
              <Square size={10} fill="currentColor" />
              {cancelling ? "Stopping" : "Cancel"}
            </button>
          )}
          {filterLocked && changes.length > 0 && (
            <span className="filter-lock">Apply or discard edits to filter or sort.</span>
          )}
          <button className="button-small" onClick={clearFilters} disabled={filterLocked || (!hasDrafts && !hasAppliedFilters)}>Clear</button>
          <button
            className="button-small"
            onClick={applyFilters}
            disabled={filterLocked || hasIncompleteFilter || (!hasDrafts && !hasAppliedFilters)}
            title={hasIncompleteFilter ? "Enter a value for every active filter." : "Apply filters"}
          >
            Apply filters
          </button>
          {page.editable && (
            <button className="button-small button-add" disabled={busy} onClick={() => onChanges([
              ...changes,
              { localId: crypto.randomUUID(), kind: "insert", values: {} },
            ])}>
              <Plus size={14} /> Add row
            </button>
          )}
          <span className="page-label">Page {pageNumber}</span>
          <button className="page-button" aria-label="Previous page" disabled={busy || page.offset === 0} onClick={() => onPage(Math.max(0, page.offset - page.limit))}>‹</button>
          <button className="page-button" aria-label="Next page" disabled={busy || !page.hasMore} onClick={() => onPage(page.offset + page.limit)}>›</button>
        </div>
      </div>

      <div className="grid-scroll">
        <table className="data-grid">
          <thead>
            <tr>
              {page.editable && <th className="row-action-head" aria-label="Row actions" />}
              {page.columns.map((column) => {
                const activeSort = column.sortable && sort?.column === column.name;
                return (
                  <th
                    key={column.name}
                    aria-sort={activeSort ? (sort?.descending ? "descending" : "ascending") : "none"}
                  >
                    <button
                      type="button"
                      className={`sort-button ${activeSort ? "is-sorted" : ""}`}
                      disabled={filterLocked || !column.sortable}
                      title={!column.sortable
                        ? "This type has no default sort order."
                        : changes.length > 0
                          ? "Apply or discard staged edits before sorting."
                          : `Sort by ${column.name}`}
                      onClick={() => onSort(column.name)}
                    >
                      <span>{column.name}</span>
                      <small>{column.dataType}</small>
                      {activeSort && sort
                        ? sort.descending ? <ArrowDown size={12} /> : <ArrowUp size={12} />
                        : <ArrowUpDown size={12} />}
                    </button>
                  </th>
                );
              })}
            </tr>
            <tr className="filter-row">
              {page.editable && <th className="row-action-head" aria-label="Filter actions" />}
              {page.columns.map((column) => {
                const draft = drafts[column.name] ?? { operator: "none" as const, value: "" };
                return (
                  <th key={column.name}>
                    <div className="filter-cell">
                      <select
                        aria-label={`${column.name} filter operator`}
                        value={draft.operator}
                        disabled={filterLocked}
                        onChange={(event) => {
                          const operator = event.currentTarget.value as FilterOperator | "none";
                          updateFilter(column.name, {
                            operator,
                            value: needsValue(operator) ? draft.value : "",
                          });
                        }}
                      >
                        <option value="none">No filter</option>
                        {operatorOptions(column.filterKind).map((option) => (
                          <option key={option.value} value={option.value}>{option.label}</option>
                        ))}
                      </select>
                      {needsValue(draft.operator) && (
                        <input
                          aria-label={`${column.name} filter value`}
                          value={draft.value}
                          disabled={filterLocked}
                          placeholder="Value"
                          onChange={(event) => updateFilter(column.name, { value: event.currentTarget.value })}
                          onKeyDown={(event) => {
                            if (event.key === "Enter") {
                              event.preventDefault();
                              applyFilters();
                            }
                          }}
                        />
                      )}
                    </div>
                  </th>
                );
              })}
            </tr>
          </thead>
          <tbody>
            {page.rows.map((row) => {
              const pending = pendingByKey.get(JSON.stringify(row.key));
              const values = pending?.kind === "update"
                ? row.values.map((value, index) => Object.prototype.hasOwnProperty.call(pending.values, page.columns[index].name)
                  ? pending.values[page.columns[index].name]
                  : value)
                : row.values;
              return (
                <tr className={pending?.kind === "delete" ? "row-deleted" : pending ? "row-changed" : ""} key={JSON.stringify(row.key)}>
                  {page.editable && (
                    <td className="row-action-cell">
                      <button
                        className={`row-action ${pending?.kind === "delete" ? "undo-action" : "delete-action"}`}
                        aria-label={pending?.kind === "delete" ? "Undo delete" : "Stage delete"}
                        title={pending?.kind === "delete" ? "Undo delete" : "Stage delete"}
                        onClick={() => toggleDelete(row)}
                      >
                        {pending?.kind === "delete" ? <Undo2 size={14} /> : <Trash2 size={14} />}
                      </button>
                    </td>
                  )}
                  {page.columns.map((column, index) => (
                    <td key={column.name}>
                      {page.editable && !column.generated && !column.identity && pending?.kind !== "delete" ? (
                        <GridCell label={column.name} value={values[index]} allowNull={column.nullable} onChange={(value) => updateCell(row, column, index, value)} />
                      ) : (
                        <CellValue value={values[index]} />
                      )}
                    </td>
                  ))}
                </tr>
              );
            })}
            {inserts.map((change) => change.kind === "insert" && (
              <tr className="row-inserted" key={change.localId}>
                <td className="row-action-cell">
                  <button className="row-action delete-action" aria-label="Discard new row" title="Discard new row" onClick={() => onChanges(changes.filter((item) => item.localId !== change.localId))}>
                    <Trash2 size={14} />
                  </button>
                </td>
                {page.columns.map((column) => {
                  const hasValue = Object.prototype.hasOwnProperty.call(change.values, column.name);
                  const value = change.values[column.name];
                  const mode = !hasValue ? "default" : value === null ? "null" : "value";
                  const isGenerated = column.generated || column.identity;
                  return (
                    <td key={column.name}>
                      {isGenerated ? (
                        <span className="default-value">Generated</span>
                      ) : (
                        <div className="insert-cell">
                          <select
                            aria-label={`${column.name} insert mode`}
                            value={mode}
                            onChange={(event) => updateInsert(change.localId, column, event.currentTarget.value as "default" | "value" | "null", typeof value === "string" ? value : "")}
                          >
                            <option value="default">Default</option>
                            <option value="value">Value</option>
                            <option value="null" disabled={!column.nullable}>NULL</option>
                          </select>
                          {mode === "value" && (
                            <input
                              aria-label={`${column.name} value`}
                              value={typeof value === "string" ? value : ""}
                              onChange={(event) => updateInsert(change.localId, column, "value", event.currentTarget.value)}
                            />
                          )}
                        </div>
                      )}
                    </td>
                  );
                })}
              </tr>
            ))}
            {page.rows.length === 0 && inserts.length === 0 && (
              <tr><td className="empty-grid" colSpan={page.columns.length + Number(page.editable)}>No rows in this table.</td></tr>
            )}
          </tbody>
        </table>
      </div>
      <div className="grid-footer">
        <span>{page.rows.length} rows on this page</span>
        <span>{page.columns.length} columns</span>
      </div>
    </div>
  );
}

export function ResultGrid({ result }: { result: QueryResultSet }) {
  return (
    <div className="result-table-wrap">
      <table className="data-grid result-grid">
        <thead><tr>{result.columns.map((column, index) => <th key={`${column}-${index}`}>{column}</th>)}</tr></thead>
        <tbody>
          {result.rows.map((row, rowIndex) => (
            <tr key={rowIndex}>{row.map((value, index) => <td key={index}><CellValue value={value} /></td>)}</tr>
          ))}
          {result.rows.length === 0 && (
            <tr><td className="empty-grid" colSpan={Math.max(1, result.columns.length)}>No rows returned.</td></tr>
          )}
        </tbody>
      </table>
    </div>
  );
}

function GridCell({ label, value, allowNull, onChange }: { label: string; value: Cell; allowNull: boolean; onChange: (value: Cell) => void }) {
  const isNull = value === null;
  return (
    <div className="grid-cell-editor">
      <input
        aria-label={`${label} value`}
        value={isNull ? "" : value}
        disabled={isNull}
        onChange={(event) => onChange(event.currentTarget.value)}
      />
      <button type="button" className={`null-toggle ${isNull ? "is-null" : ""}`} onClick={() => onChange(isNull ? "" : null)} aria-label={isNull ? `Set ${label} value` : `Set ${label} to NULL`} title={isNull ? "Set value" : "Set NULL"} disabled={!allowNull && !isNull}>
        {isNull ? "NULL" : "null"}
      </button>
    </div>
  );
}

function CellValue({ value }: { value: Cell }) {
  return value === null
    ? <span className="null-value">NULL</span>
    : <span className="cell-text" title={value}>{value === "" ? <span className="empty-string">empty</span> : value}</span>;
}
