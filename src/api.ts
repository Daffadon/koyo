import { invoke as tauriInvoke } from "@tauri-apps/api/core";

function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) {
    return Promise.reject(new Error("Database features need the desktop app. Run npm run tauri dev."));
  }
  return tauriInvoke<T>(command, args);
}

export type SslMode = "verify-full" | "disable";

export type ConnectionProfile = {
  id: string;
  name: string;
  host: string;
  port: number;
  database: string;
  username: string;
  sslMode: SslMode;
};

export type ConnectionInfo = {
  profile: ConnectionProfile;
  serverVersion: string;
  currentUser: string;
};

export type TableSummary = {
  schema: string;
  name: string;
  editable: boolean;
};

export type ColumnInfo = {
  name: string;
  dataType: string;
  nullable: boolean;
  primaryKey: boolean;
  generated: boolean;
  identity: boolean;
};

export type Cell = string | null;

export type GridRow = {
  version: string;
  key: Cell[];
  values: Cell[];
};

export type TablePage = {
  columns: ColumnInfo[];
  rows: GridRow[];
  offset: number;
  limit: number;
  hasMore: boolean;
  editable: boolean;
};

export type QueryResultSet = {
  columns: string[];
  rows: Cell[][];
};

export type QueryOutput = {
  resultSets: QueryResultSet[];
  affectedRows: number[];
  truncated: boolean;
};

export type GridChange =
  | { kind: "insert"; values: Record<string, Cell> }
  | { kind: "update"; key: Cell[]; version: string; values: Record<string, Cell> }
  | { kind: "delete"; key: Cell[]; version: string };

export const api = {
  listProfiles: () => invoke<ConnectionProfile[]>("list_profiles"),
  saveProfile: (profile: Omit<ConnectionProfile, "id"> & { id: string | null }, password: string | null) =>
    invoke<ConnectionProfile>("save_profile", { profile, password }),
  deleteProfile: (id: string) => invoke<void>("delete_profile", { id }),
  connect: (id: string) => invoke<ConnectionInfo>("connect_profile", { id }),
  disconnect: () => invoke<void>("disconnect"),
  listTables: () => invoke<TableSummary[]>("list_tables"),
  loadTablePage: (schema: string, table: string, offset: number, limit = 100) =>
    invoke<TablePage>("load_table_page", { schema, table, offset, limit }),
  applyGridChanges: (schema: string, table: string, changes: GridChange[]) =>
    invoke<{ applied: number }>("apply_grid_changes", { schema, table, changes }),
  runSql: (sql: string) => invoke<QueryOutput>("run_sql", { sql }),
  cancelQuery: () => invoke<void>("cancel_query"),
};
