import { useEffect, useState } from "react";
import { api, type ClientView } from "../api";
import { Btn } from "../components";
import { useApp } from "../store";

export function ClientsView() {
  const { toast, accounts, nameOf } = useApp();
  const [rows, setRows] = useState<ClientView[]>([]);
  useEffect(() => {
    let alive = true;
    const load = async () => { try { const r = await api.clientsSnapshot(); if (alive) setRows(r); } catch { /* ignore */ } };
    void load();
    const t = window.setInterval(load, 3000);
    return () => { alive = false; window.clearInterval(t); };
  }, []);
  const close = async (pid: number) => {
    try { await api.closeClient(pid); toast("Asked the client to close."); } catch (e) { toast(String(e), "bad"); }
  };
  return (
    <div className="clients">
      {rows.length === 0 ? (
        <div className="empty"><div><b>No Roblox clients running.</b><br />Launch an account from Accounts; its client appears here once Windows shows it.</div></div>
      ) : (
        <>
        <div className="row" style={{ marginBottom: 10, gap: 8 }}>
          <Btn sm onClick={async () => { try { toast(`Minimized ${await api.clientsMinimizeAll(true)} client(s).`); } catch (e) { toast(String(e), "bad"); } }}>Minimize all</Btn>
          <Btn sm onClick={async () => { try { toast(`Restored ${await api.clientsMinimizeAll(false)} client(s).`); } catch (e) { toast(String(e), "bad"); } }}>Restore all</Btn>
          <span className="hint">Minimizing never steals focus. Main and Full quality accounts are included here; the automatic limits skip them.</span>
        </div>
        <table className="table">
          <thead><tr><th>Account</th><th>PID</th><th style={{ textAlign: "right" }}>Working set</th><th style={{ textAlign: "right" }}>Private</th><th></th></tr></thead>
          <tbody>
            {rows.map((c) => (
              <tr key={c.pid}>
                <td>{c.account != null ? nameOf(accounts.find((a) => a.id === c.account)?.username ?? c.account_name ?? "") : <span className="muted">Unassigned</span>}</td>
                <td className="mono muted">{c.pid}</td>
                <td style={{ textAlign: "right" }}>{c.working_set_mb} MB</td>
                <td style={{ textAlign: "right" }}>{c.private_mb} MB</td>
                <td style={{ textAlign: "right" }}><Btn sm onClick={() => close(c.pid)}>Close</Btn></td>
              </tr>
            ))}
          </tbody>
        </table>
        </>
      )}
    </div>
  );
}
