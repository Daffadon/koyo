import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import { Pencil, Plug, Plus, ShieldAlert, ShieldCheck, Trash2, X } from "lucide-react";
import { api } from "../api";
import type { ConnectionInfo, ConnectionProfile, SslMode } from "../api";

type Props = {
  profiles: ConnectionProfile[];
  activeProfileId: string | null;
  canConnect: () => boolean;
  onClose: () => void;
  onConnected: (info: ConnectionInfo) => Promise<void>;
  onProfilesChanged: () => Promise<void>;
};

const blankProfile = {
  name: "",
  host: "localhost",
  port: 5432,
  database: "postgres",
  username: "postgres",
  sslMode: "verify-full" as SslMode,
};

export function ConnectionDialog({ profiles, activeProfileId, canConnect, onClose, onConnected, onProfilesChanged }: Props) {
  const [profileId, setProfileId] = useState<string | null>(null);
  const [form, setForm] = useState(blankProfile);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const dialogRef = useRef<HTMLElement>(null);
  const nameRef = useRef<HTMLInputElement>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  const selectedProfile = profiles.find((profile) => profile.id === profileId);
  const targetChanged = selectedProfile !== undefined && (
    form.host !== selectedProfile.host ||
    form.port !== selectedProfile.port ||
    form.database !== selectedProfile.database ||
    form.username !== selectedProfile.username
  );

  useEffect(() => {
    const previousFocus = document.activeElement as HTMLElement | null;
    nameRef.current?.focus();

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        closeRef.current();
        return;
      }
      if (event.key !== "Tab" || !dialogRef.current) return;
      const focusable = [...dialogRef.current.querySelectorAll<HTMLElement>(
        'button:not(:disabled), input:not(:disabled), select:not(:disabled), [href], [tabindex]:not([tabindex="-1"])',
      )];
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }

    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      previousFocus?.focus();
    };
  }, []);

  async function connect(id: string) {
    if (!canConnect()) return;
    setBusy(true);
    setError("");
    try {
      const info = await api.connect(id);
      await onConnected(info);
      onClose();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  function editProfile(profile: ConnectionProfile) {
    setProfileId(profile.id);
    setForm({
      name: profile.name,
      host: profile.host,
      port: profile.port,
      database: profile.database,
      username: profile.username,
      sslMode: profile.sslMode,
    });
    setPassword("");
    setError("");
  }

  function updateForm<K extends keyof typeof form>(key: K, value: (typeof form)[K]) {
    setForm((current) => ({ ...current, [key]: value }));
  }

  async function saveAndConnect(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!canConnect()) return;
    setBusy(true);
    setError("");
    try {
      const saved = await api.saveProfile(
        { ...form, id: profileId },
        password === "" ? null : password,
      );
      setProfileId(saved.id);
      await onProfilesChanged();
      const info = await api.connect(saved.id);
      setPassword("");
      await onConnected(info);
      onClose();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function remove(profile: ConnectionProfile) {
    if (!window.confirm(`Remove connection “${profile.name}” and its saved password?`)) return;
    if (profile.id === activeProfileId && !canConnect()) return;
    setBusy(true);
    setError("");
    try {
      await api.deleteProfile(profile.id);
      await onProfilesChanged();
      if (profileId === profile.id) {
        setProfileId(null);
        setForm(blankProfile);
        setPassword("");
      }
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }


  return (
    <div className="dialog-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section ref={dialogRef} className="connection-dialog" role="dialog" aria-modal="true" aria-labelledby="connection-title">
        <header className="dialog-header">
          <div>
            <h2 id="connection-title">Connections</h2>
            <p>Choose a saved profile or add a database.</p>
          </div>
          <button className="icon-button" aria-label="Close connections" onClick={onClose}>
            <X size={18} />
          </button>
        </header>

        <div className="connection-dialog-body">
          <section className="saved-connections" aria-label="Saved connections">
            <div className="section-heading">
              <span>Saved</span>
              <span className="count-badge">{profiles.length}</span>
            </div>
            {profiles.length === 0 ? (
              <p className="quiet-copy">No saved connections yet.</p>
            ) : (
              <ul className="profile-list">
                {profiles.map((profile) => (
                  <li className="profile-item" key={profile.id}>
                    <button className="profile-connect" onClick={() => void connect(profile.id)} disabled={busy}>
                      <span className="profile-mark"><Plug size={16} /></span>
                      <span className="profile-copy">
                        <strong>{profile.name}</strong>
                        <small>{profile.username}@{profile.host}:{profile.port}/{profile.database}</small>
                      </span>
                    </button>
                    <button className="icon-button profile-edit" aria-label={`Edit ${profile.name}`} title="Edit connection" onClick={() => editProfile(profile)} disabled={busy}>
                      <Pencil size={14} />
                    </button>
                    <button
                      className="icon-button profile-remove"
                      aria-label={`Remove ${profile.name}`}
                      onClick={() => void remove(profile)}
                      disabled={busy}
                    >
                      <Trash2 size={15} />
                    </button>
                  </li>
                ))}
              </ul>
            )}

            <div className="secure-note">
              <ShieldCheck size={16} />
              <span>Passwords stay in your operating-system credential store.</span>
            </div>
          </section>

          <form className="connection-form" onSubmit={saveAndConnect}>
            <div className="section-heading">
              <span>{profileId ? "Edit connection" : "New connection"}</span>
              <span className="form-mark">{profileId ? <Pencil size={14} /> : <Plus size={15} />}</span>
            </div>
            <label>
              Connection name
              <input ref={nameRef} required value={form.name} onChange={(event) => updateForm("name", event.currentTarget.value)} placeholder="Local development" />
            </label>
            <div className="field-row">
              <label>
                Host
                <input required value={form.host} onChange={(event) => updateForm("host", event.currentTarget.value)} autoComplete="off" />
              </label>
              <label className="port-field">
                Port
                <input required type="number" min="1" max="65535" value={form.port} onChange={(event) => updateForm("port", Number(event.currentTarget.value))} />
              </label>
            </div>
            <div className="field-row">
              <label>
                Database
                <input required value={form.database} onChange={(event) => updateForm("database", event.currentTarget.value)} />
              </label>
              <label>
                Username
                <input required value={form.username} onChange={(event) => updateForm("username", event.currentTarget.value)} autoComplete="username" />
              </label>
            </div>
            <label>
              Password
              <input
                type="password"
                value={password}
                onChange={(event) => setPassword(event.currentTarget.value)}
                autoComplete="new-password"
                placeholder={profileId ? targetChanged ? "Enter password for the new target" : "Leave blank to keep saved password" : "Can be blank for passwordless auth"}
              />
            </label>
            {targetChanged && <p className="quiet-copy">Changing host, port, database, or username clears the saved password.</p>}
            <label>
              TLS mode
              <select value={form.sslMode} onChange={(event) => updateForm("sslMode", event.currentTarget.value as SslMode)}>
                <option value="verify-full">Verify server identity</option>
                <option value="disable">Disable TLS (local only)</option>
              </select>
            </label>
            {form.sslMode === "disable" && (
              <p className="security-warning"><ShieldAlert size={15} /> Traffic will not be encrypted. Use only on trusted local networks.</p>
            )}
            {error && <p className="inline-error" role="alert">{error}</p>}
            <div className="form-actions">
              <button className="button-secondary" type="button" onClick={onClose} disabled={busy}>Cancel</button>
              <button className="button-primary" type="submit" disabled={busy}>
                {busy ? "Connecting…" : "Save and connect"}
              </button>
            </div>
          </form>
        </div>
      </section>
    </div>
  );
}
