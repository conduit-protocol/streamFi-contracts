import React, { useState, useCallback, useRef, useEffect } from 'react';

const NETWORK_SWITCH_TIMEOUT_MS = 10_000;

interface Network {
  id: string;
  name: string;
  rpcUrl: string;
}

const NETWORKS: Network[] = [
  { id: 'testnet', name: 'Testnet', rpcUrl: 'https://soroban-testnet.stellar.org' },
  { id: 'mainnet', name: 'Mainnet', rpcUrl: 'https://soroban-mainnet.stellar.org' },
];

interface NetworkSwitcherProps {
  onNetworkChanged?: (network: Network) => void;
}

export const NetworkSwitcher: React.FC<NetworkSwitcherProps> = ({ onNetworkChanged }) => {
  const [currentNetwork, setCurrentNetwork] = useState<Network>(NETWORKS[0]);
  const [switching, setSwitching] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const abortRef = useRef<AbortController | null>(null);

  // FIX for Bug #160: The previous implementation awaited the network switch
  // with no timeout, so if the RPC provider timed out or was unreachable,
  // `switching` was never cleared and the user was stuck with a spinner
  // forever. Racing the switch against a timeout guarantees the loading state
  // always clears, either with success or a clear timeout error.
  const switchNetwork = useCallback(async (network: Network) => {
    if (network.id === currentNetwork.id) return;

    // Abort any in-flight switch
    abortRef.current?.abort();
    const controller = new AbortController();
    abortRef.current = controller;

    setSwitching(true);
    setError(null);

    const timeout = new Promise<never>((_, reject) => {
      setTimeout(
        () => reject(new Error('Network switch timed out. Please try again.')),
        NETWORK_SWITCH_TIMEOUT_MS,
      );
    });

    try {
      // Validate the new RPC endpoint is reachable
      const healthCheck = fetch(`${network.rpcUrl}/health`, {
        signal: controller.signal,
      });

      await Promise.race([healthCheck, timeout]);

      setCurrentNetwork(network);
      onNetworkChanged?.(network);
    } catch (e) {
      if (e instanceof DOMException && e.name === 'AbortError') {
        return; // Switch was superseded by another; no error to show.
      }
      setError(e instanceof Error ? e.message : 'Failed to switch network.');
    } finally {
      setSwitching(false);
    }
  }, [currentNetwork, onNetworkChanged]);

  // Cleanup: abort any in-flight request on unmount.
  useEffect(() => {
    return () => abortRef.current?.abort();
  }, []);

  return (
    <div className="network-switcher">
      <label htmlFor="network-select">Network</label>
      <select
        id="network-select"
        value={currentNetwork.id}
        onChange={(e) => {
          const network = NETWORKS.find((n) => n.id === e.target.value);
          if (network) switchNetwork(network);
        }}
        disabled={switching}
      >
        {NETWORKS.map((n) => (
          <option key={n.id} value={n.id}>
            {n.name}
          </option>
        ))}
      </select>
      {switching && <span>Switching network...</span>}
      {error && <span className="network-error">{error}</span>}
    </div>
  );
};
