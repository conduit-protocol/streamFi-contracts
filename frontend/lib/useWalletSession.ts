import { useCallback, useEffect, useRef, useState } from 'react';

/**
 * Issue #589: WalletConnection.tsx previously had no connect/disconnect or
 * session-expiry handling — only a stream-creation form. This hook owns
 * that half of the flow: it tracks whether a wallet is connected, exposes
 * connect()/disconnect(), and clears the session automatically once it
 * expires so the UI can re-prompt instead of surfacing a stale-session
 * error from a downstream mutation.
 */

const DEFAULT_SESSION_TTL_MS = 15 * 60 * 1000; // 15 minutes

export interface WalletSession {
  address: string;
  expiresAt: number;
}

export interface UseWalletSessionResult {
  session: WalletSession | null;
  isConnected: boolean;
  /** True for one render after the session clock ran out, before the UI re-prompts. */
  didExpire: boolean;
  connect: (address: string, ttlMs?: number) => void;
  disconnect: () => void;
}

export function useWalletSession(): UseWalletSessionResult {
  const [session, setSession] = useState<WalletSession | null>(null);
  const [didExpire, setDidExpire] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const clearTimer = () => {
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  };

  const disconnect = useCallback(() => {
    clearTimer();
    setSession(null);
  }, []);

  const connect = useCallback((address: string, ttlMs: number = DEFAULT_SESSION_TTL_MS) => {
    clearTimer();
    setDidExpire(false);
    const expiresAt = Date.now() + ttlMs;
    setSession({ address, expiresAt });

    timerRef.current = setTimeout(() => {
      setSession(null);
      setDidExpire(true);
    }, ttlMs);
  }, []);

  useEffect(() => clearTimer, []);

  return {
    session,
    isConnected: session !== null,
    didExpire,
    connect,
    disconnect,
  };
}
