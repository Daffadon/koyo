import { useEffect, useMemo, useState } from "react";
import CodeMirror from "@uiw/react-codemirror";
import { sql } from "@codemirror/lang-sql";
import {
  Braces,
  ChevronDown,
  ChevronRight,
  CirclePlus,
  Database,
  Play,
  PlugZap,
  RefreshCw,
  Save,
  Square,
  Table2,
  Terminal,
  X,
} from "lucide-react";
import { api, type ConnectionInfo, type ConnectionProfile, type GridChange, type QueryOutput, type TablePage, type TableSummary } from "./api";
import { ConnectionDialog } from "./components/ConnectionDialog";
import { DataGrid, ResultGrid, type StagedGridChange } from "./components/DataGrid";
import "./App.css";

const STARTER_SQL = "SELECT current_database(), current_user;";

function App() {
  const [profiles, setProfiles] = useState<ConnectionProfile[]>([]);
  const [connection, setConnection] = useState<ConnectionInfo | null>(null);
  const [tables, setTables] = useState<TableSummary[]>([]);
  const [showConnections, setShowConnections] = useState(false);
  const [expandedSchemas, setExpandedSchemas] = useState<Record<string, boolean>>({});
  const [activeView, setActiveView] = useState<"query" | "table">("query");
  const [selectedTable, setSelectedTable] = useState<TableSummary | null>(null);
  const [tablePage, setTablePage] = useState<TablePage | null>(null);
  const [pageLoading, setPageLoading] = useState(false);
  const [queryText, setQueryText] = useState(STARTER_SQL);
  const [queryOutput, setQueryOutput] = useState<QueryOutput | null>(null);
  const [activeResult, setActiveResult] = useState(0);
  const [queryError, setQueryError] = useState("");
  const [queryRunning, setQueryRunning] = useState(false);
  const [queryCancelling, setQueryCancelling] = useState(false);
  const [queryDuration, setQueryDuration] = useState<number | null>(null);
  const [stagedChanges, setStagedChanges] = useState<StagedGridChange[]>([]);
  const [notice, setNotice] = useState("");

  const schemaNames = useMemo(
    () => [...new Set(tables.map((table) => table.schema))].sort((left, right) => left.localeCompare(right)),
    [tables],
  );

  async function refreshProfiles() {
    try {
      const saved = await api.listProfiles();
      setProfiles(saved);
      if (connection && !saved.some((profile) => profile.id === connection.profile.id)) {
        await api.disconnect();
        setConnection(null);
        setTables([]);
        setTablePage(null);
        setSelectedTable(null);
        setStagedChanges([]);
        setActiveView("query");
      }
    } catch (reason) {
      setNotice(String(reason));
    }
  }

  function confirmConnectionChange() {
    if (queryRunning) {
      setNotice("Cancel or wait for the running query before switching connections.");
      return false;
    }
    if (stagedChanges.length === 0) return true;
    return window.confirm("Switch connections? Pending edits will be discarded if the connection succeeds.");
  }

  async function refreshCurrentTable() {
    if (!selectedTable) return;
    if (stagedChanges.length > 0 && !window.confirm("Discard staged table changes and reload this page?")) return;
    await loadTable(selectedTable.schema, selectedTable.name, tablePage?.offset ?? 0);
  }

  async function refreshTables() {
    try {
      setTables(await api.listTables());
    } catch (reason) {
      setNotice(String(reason));
    }
  }

  useEffect(() => {
    void refreshProfiles();
  }, []);

  async function handleConnected(info: ConnectionInfo) {
    setConnection(info);
    setShowConnections(false);
    setSelectedTable(null);
    setTablePage(null);
    setStagedChanges([]);
    setQueryOutput(null);
    setQueryError("");
    setActiveView("query");
    try {
      const listed = await api.listTables();
      setTables(listed);
      const schemas = [...new Set(listed.map((table) => table.schema))];
      setExpandedSchemas(Object.fromEntries(schemas.map((schema) => [schema, true])));
    } catch (reason) {
      setTables([]);
      setNotice(String(reason));
    }
    await refreshProfiles();
  }

  async function loadTable(schema: string, table: string, offset: number) {
    setPageLoading(true);
    try {
      const page = await api.loadTablePage(schema, table, offset);
      setTablePage(page);
      setSelectedTable(tables.find((item) => item.schema === schema && item.name === table) ?? { schema, name: table, editable: page.editable });
      setActiveView("table");
      setQueryError("");
      setStagedChanges([]);
    } catch (reason) {
      setNotice(String(reason));
    } finally {
      setPageLoading(false);
    }
  }

  async function openTable(table: TableSummary) {
    if (stagedChanges.length > 0 && !window.confirm("Discard staged table changes and open another table?")) return;
    await loadTable(table.schema, table.name, 0);
  }

  async function changePage(offset: number) {
    if (!selectedTable) return;
    if (stagedChanges.length > 0 && !window.confirm("Discard staged table changes and change page?")) return;
    await loadTable(selectedTable.schema, selectedTable.name, offset);
  }

  function closeTableTab() {
    if (stagedChanges.length > 0 && !window.confirm("Discard staged changes and close table?")) return;
    setSelectedTable(null);
    setTablePage(null);
    setStagedChanges([]);
    setActiveView("query");
  }

  async function runQuery() {
    if (!connection || queryRunning) return;
    setQueryRunning(true);
    setQueryCancelling(false);
    setQueryError("");
    setNotice("");
    setQueryOutput(null);
    const startedAt = performance.now();
    try {
      const output = await api.runSql(queryText);
      setQueryOutput(output);
      setActiveResult(0);
    } catch (reason) {
      setQueryError(String(reason));
    } finally {
      setQueryDuration(Math.round(performance.now() - startedAt));
      setQueryRunning(false);
      setQueryCancelling(false);
    }
  }

  async function cancelQuery() {
    if (!queryRunning || queryCancelling) return;
    setQueryCancelling(true);
    try {
      await api.cancelQuery();
      setNotice("Cancellation requested.");
    } catch (reason) {
      setNotice(String(reason));
      setQueryCancelling(false);
    }
  }

  useEffect(() => {
    function handleShortcut(event: KeyboardEvent) {
      if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
        event.preventDefault();
        void runQuery();
      }
    }
    window.addEventListener("keydown", handleShortcut);
    return () => window.removeEventListener("keydown", handleShortcut);
  });

  async function applyChanges() {
    if (!selectedTable || stagedChanges.length === 0) return;
    setPageLoading(true);
    setNotice("");
    try {
      const changes: GridChange[] = stagedChanges.map((change) => {
        switch (change.kind) {
          case "insert":
            return { kind: change.kind, values: change.values };
          case "update":
            return { kind: change.kind, key: change.key, version: change.version, values: change.values };
          case "delete":
            return { kind: change.kind, key: change.key, version: change.version };
        }
      });
      const result = await api.applyGridChanges(selectedTable.schema, selectedTable.name, changes);
      setStagedChanges([]);
      setNotice(`${result.applied} staged ${result.applied === 1 ? "change" : "changes"} applied.`);
      const page = await api.loadTablePage(selectedTable.schema, selectedTable.name, tablePage?.offset ?? 0);
      setTablePage(page);
    } catch (reason) {
      const message = String(reason);
      if (message.startsWith("COMMIT_OUTCOME_UNKNOWN:")) {
        setStagedChanges([]);
        try {
          const page = await api.loadTablePage(selectedTable.schema, selectedTable.name, tablePage?.offset ?? 0);
          setTablePage(page);
          setNotice("Commit result could not be confirmed. Page reloaded; verify data before retrying.");
        } catch {
          setNotice("Commit result is unknown. Reconnect and reload this table before retrying.");
        }
      } else {
        setNotice(message);
      }
    } finally {
      setPageLoading(false);
    }
  }

  function discardChanges() {
    if (stagedChanges.length === 0 || window.confirm("Discard all staged table changes?")) {
      setStagedChanges([]);
    }
  }

  async function disconnect() {
    if (stagedChanges.length > 0 && !window.confirm("Discard staged table changes before disconnecting?")) return;
    try {
      await api.disconnect();
      setConnection(null);
      setTables([]);
      setSelectedTable(null);
      setTablePage(null);
      setQueryOutput(null);
      setStagedChanges([]);
      setActiveView("query");
      setNotice("Disconnected.");
    } catch (reason) {
      setNotice(String(reason));
    }
  }


  return (
    <main className="app-shell">
      <aside className="sidebar">
        <div className="brand-row">
          <div className="brand-mark" aria-hidden="true">K</div>
          <div className="brand-copy"><strong>Koyo</strong><span>PostgreSQL workspace</span></div>
          <button className="sidebar-icon" aria-label="Manage connections" title="Manage connections" onClick={() => setShowConnections(true)}>
            <CirclePlus size={17} />
          </button>
        </div>

        <section className="connection-summary">
          <div className={`connection-led ${connection ? "is-live" : ""}`} />
          <div className="connection-summary-copy">
            <span className="small-label">Connection</span>
            <strong>{connection?.profile.name ?? "Not connected"}</strong>
            <small>{connection ? `${connection.profile.username}@${connection.profile.host}` : "Choose a saved profile"}</small>
          </div>
          {connection && <button className="sidebar-icon disconnect-button" aria-label="Disconnect" title="Disconnect" onClick={() => void disconnect()}><PlugZap size={16} /></button>}
        </section>

        <div className="sidebar-section-title">
          <span>Explorer</span>
          {connection && <button className="sidebar-icon" aria-label="Refresh tables" title="Refresh tables" onClick={() => void refreshTables()}><RefreshCw size={14} /></button>}
        </div>

        {connection ? (
          <nav className="schema-tree" aria-label="Database schemas">
            {schemaNames.map((schema) => {
              const isExpanded = expandedSchemas[schema] ?? true;
              return (
                <section className="schema-group" key={schema}>
                  <button className="schema-heading" aria-expanded={isExpanded} onClick={() => {
                    setExpandedSchemas((current) => {
                      const next = { ...current };
                      next[schema] = !next[schema];
                      return next;
                    });
                  }}>
                    {isExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                    <Database size={14} />
                    <span>{schema}</span>
                    <small>{tables.filter((table) => table.schema === schema).length}</small>
                  </button>
                  {isExpanded && tables.filter((table) => table.schema === schema).map((table) => (
                    <button
                      className={`table-link ${selectedTable?.schema === schema && selectedTable.name === table.name && activeView === "table" ? "is-selected" : ""}`}
                      key={`${schema}.${table.name}`}
                      onClick={() => void openTable(table)}
                      title={table.editable ? "Open editable table data" : "Open read-only table data"}
                    >
                      <Table2 size={14} />
                      <span>{table.name}</span>
                      {!table.editable && <span className="readonly-dot" title="No primary key" />}
                    </button>
                  ))}
                </section>
              );
            })}
            {tables.length === 0 && <p className="sidebar-empty">No tables found.</p>}
          </nav>
        ) : (
          <div className="sidebar-empty-state">
            <Database size={22} />
            <p>Connect to browse schemas and table data.</p>
          </div>
        )}

        <div className="sidebar-footer">
          <span className="footer-dot" />
          {connection ? (
            <>
              <span>{connection.currentUser}</span>
              <span>PostgreSQL {connection.serverVersion.split(" ")[0]}</span>
            </>
          ) : (
            <span>Local profiles only</span>
          )}
        </div>
      </aside>

      <section className="workspace">
        <header className="topbar">
          <div className="workspace-title">
            <strong>{connection ? connection.profile.database : "PostgreSQL client"}</strong>
            <span>{connection ? `${connection.profile.name} on ${connection.profile.host}` : "Connect to begin"}</span>
          </div>
          <div className="topbar-actions">
            {connection ? (
              <>
                <span className="connected-chip"><span /> Connected</span>
                <button className="button-secondary" onClick={() => setShowConnections(true)} disabled={queryRunning}>Switch connection</button>
              </>
            ) : (
              <button className="button-primary" onClick={() => setShowConnections(true)}><PlugZap size={15} /> Connect</button>
            )}
          </div>
        </header>

        <div className="workspace-tabs" role="tablist" aria-label="Workspace views">
          <button
            role="tab"
            id="query-tab"
            aria-controls="query-panel"
            aria-selected={activeView === "query"}
            onClick={() => setActiveView("query")}
          >
            <Braces size={15} /> SQL workbench
          </button>
          {selectedTable && (
            <div className="table-tab">
              <button
                role="tab"
                id="table-tab"
                aria-controls="table-panel"
                aria-selected={activeView === "table"}
                onClick={() => setActiveView("table")}
              >
                <Table2 size={15} /> {selectedTable.schema}.{selectedTable.name}
              </button>
              <button className="tab-close" aria-label="Close table tab" title="Close table" onClick={closeTableTab}>
                <X size={12} />
              </button>
            </div>
          )}
          <div className="tab-spacer" />
        </div>

        <div className="workspace-content">
          {activeView === "query" ? (
            <section className="query-workspace" id="query-panel" role="tabpanel" aria-labelledby="query-tab">
              <div className="query-toolbar">
                <div className="query-name"><span className="query-status-dot" /> SQL editor</div>
                <div className="query-actions">
                  <span className="keyboard-hint">Ctrl / ⌘ + Enter to run</span>
                  {queryRunning && (
                    <button className="button-secondary cancel-run" onClick={() => void cancelQuery()} disabled={queryCancelling}>
                      <Square size={12} fill="currentColor" />
                      {queryCancelling ? "Stopping" : "Cancel"}
                    </button>
                  )}
                  <button className="button-run" onClick={() => void runQuery()} disabled={!connection || queryRunning}>
                    {queryRunning ? <RefreshCw className="spin" size={15} /> : <Play size={14} fill="currentColor" />}
                    {queryRunning ? "Running" : "Run query"}
                  </button>
                </div>
              </div>
              {!connection && (
                <div className="connect-strip">
                  <PlugZap size={14} />
                  <span>Connect a database to run SQL.</span>
                  <button className="text-action" onClick={() => setShowConnections(true)}>Connect</button>
                </div>
              )}
              <div className="editor-shell">
                <CodeMirror
                  value={queryText}
                  height="100%"
                  extensions={[sql()]}
                  onChange={setQueryText}
                  theme="light"
                  basicSetup={{
                    lineNumbers: true,
                    foldGutter: false,
                    highlightActiveLineGutter: false,
                    highlightActiveLine: true,
                    dropCursor: false,
                    tabSize: 2,
                  }}
                />
              </div>
              <section className="results-panel" aria-label="Query results">
                <div className="results-header">
                  <div className="results-title"><span className="results-mark" /> Results</div>
                  <div className="results-meta">
                    {queryError ? <span className="error-summary">Query failed</span> : null}
                    {queryDuration !== null && !queryError && <span>{queryDuration} ms</span>}
                    {queryOutput?.truncated && <span className="truncated-note">Each result capped at 1,000 rows</span>}
                  </div>
                </div>
                {queryError ? (
                  <pre className="query-error" role="alert">{queryError}</pre>
                ) : queryOutput ? (
                  <>
                    {queryOutput.resultSets.length > 1 && (
                      <div className="result-set-tabs" role="tablist" aria-label="Result sets">
                        {queryOutput.resultSets.map((result, index) => (
                          <button key={index} role="tab" aria-selected={activeResult === index} className={activeResult === index ? "selected-result" : ""} onClick={() => setActiveResult(index)}>
                            Result {index + 1} <span>{result.rows.length}</span>
                          </button>
                        ))}
                      </div>
                    )}
                    {queryOutput.resultSets[activeResult] ? (
                      <ResultGrid result={queryOutput.resultSets[activeResult]} />
                    ) : (
                      <div className="command-result">
                        <span className="command-check">✓</span>
                        <div><strong>Statement complete</strong><small>{queryOutput.affectedRows.reduce((sum, count) => sum + count, 0)} rows affected</small></div>
                      </div>
                    )}
                  </>
                ) : (
                  <div className="results-empty">
                    <Terminal size={22} strokeWidth={1.5} />
                    <p>{connection ? "Results from your next statement appear here." : "Connect to run a statement and see results."}</p>
                  </div>
                )}
              </section>
            </section>
          ) : (
            <section className="table-workspace" id="table-panel" role="tabpanel" aria-labelledby="table-tab">
              <div className="table-topline">
                <div>
                  <h1>{selectedTable?.schema}.{selectedTable?.name}</h1>
                  <p>{tablePage?.editable ? "Edits are staged until you apply them." : "Read-only table without a primary key."}</p>
                </div>
                <div className="table-top-actions">
                  {stagedChanges.length > 0 && <span className="staged-count">{stagedChanges.length} pending</span>}
                  <button className="button-secondary" onClick={discardChanges} disabled={stagedChanges.length === 0}><X size={14} /> Discard</button>
                  <button className="button-primary" onClick={() => void applyChanges()} disabled={stagedChanges.length === 0 || pageLoading}><Save size={14} /> Apply changes</button>
                  <button className="icon-button refresh-table" aria-label="Refresh table" onClick={() => void refreshCurrentTable()} disabled={pageLoading}>
                    <RefreshCw className={pageLoading ? "spin" : ""} size={15} />
                  </button>
                </div>
              </div>
              {notice && <div className="notice-bar" role="status">{notice}</div>}
              {tablePage ? (
                <DataGrid page={tablePage} changes={stagedChanges} onChanges={setStagedChanges} onPage={(offset) => void changePage(offset)} />
              ) : (
                <div className="table-loading">{pageLoading ? "Loading table…" : "Choose a table from the explorer."}</div>
              )}
            </section>
          )}
        </div>

        {notice && activeView === "query" && <div className="toast-notice" role="status">{notice}</div>}
        <footer className="statusbar">
          <span><span className={`status-dot ${connection ? "online" : ""}`} />{connection ? `Connected to ${connection.profile.host}:${connection.profile.port}` : "Disconnected"}</span>
          <span>{activeView === "table" && tablePage ? `${tablePage.columns.length} columns` : "PostgreSQL"}</span>
          <span className="status-spacer" />
          {queryOutput && <span>{queryOutput.affectedRows.reduce((sum, count) => sum + count, 0)} rows</span>}
          <span>UTF-8</span>
        </footer>
      </section>

      {showConnections && (
        <ConnectionDialog
          profiles={profiles}
          activeProfileId={connection?.profile.id ?? null}
          canConnect={confirmConnectionChange}
          onClose={() => setShowConnections(false)}
          onConnected={handleConnected}
          onProfilesChanged={refreshProfiles}
        />
      )}
    </main>
  );
}

export default App;
