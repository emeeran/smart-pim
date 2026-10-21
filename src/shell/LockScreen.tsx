/**
 * Database lock screen (S7): passphrase prompt on launch and after auto-lock.
 * A wrong passphrase re-prompts — it is a retry, not an error state.
 */
import { signal } from "@preact/signals";
import { JSX } from "preact";
import { useState } from "preact/hooks";
import { api, type UnlockOutcome } from "../api";
import { logo } from "../design/logo";

const busy = signal(false);

export function LockScreen({ onUnlocked }: { onUnlocked: () => void }) {
  const [pass, setPass] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [created, setCreated] = useState(false);

  const submit = async (e: Event) => {
    e.preventDefault();
    if (busy.value || pass.length === 0) return;
    busy.value = true;
    setError(null);
    try {
      const out: UnlockOutcome = await api.unlock(pass);
      if (out.ok) {
        if (out.created) setCreated(true);
        setPass("");
        onUnlocked();
      } else {
        setError(out.error ?? "wrong passphrase");
      }
    } catch (err) {
      setError(String(err));
    } finally {
      busy.value = false;
    }
  };

  const onInput = (e: JSX.TargetedEvent<HTMLInputElement>) => setPass(e.currentTarget.value);

  return (
    <div class="lock-screen">
      <form class="lock-card" onSubmit={submit}>
        <img src={logo(64)} alt="Smart PIM" width={64} height={64} />
        <h1>Smart PIM</h1>
        <p class="lock-hint">
          {created
            ? "New database created. This passphrase unlocks it — keep it safe; it cannot be recovered."
            : "Enter the database passphrase to unlock."}
        </p>
        <input
          type="password"
          placeholder="Passphrase"
          value={pass}
          onInput={onInput}
          autoFocus
          aria-label="Database passphrase"
        />
        {error && <div class="lock-error" role="alert">{error}</div>}
        <button type="submit" disabled={busy.value || pass.length === 0}>
          {busy.value ? "Unlocking…" : "Unlock"}
        </button>
      </form>
    </div>
  );
}
