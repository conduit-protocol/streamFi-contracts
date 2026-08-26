import { useState, useEffect, useRef } from 'react';
import { estimateFee, FeeEstimate } from './estimateFee';

const FEE_DEBOUNCE_MS = 500;

const DEFAULT_RPC_URL = 'https://soroban-testnet.stellar.org';

export interface UseFeeEstimateOptions {
  rpcUrl?: string;
  factoryAddress: string;
  senderAddress: string;
  enabled: boolean;
}

export function useFeeEstimate({
  rpcUrl = DEFAULT_RPC_URL,
  factoryAddress,
  senderAddress,
  enabled,
}: UseFeeEstimateOptions) {
  const [estimate, setEstimate] = useState<FeeEstimate | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const abortRef = useRef<AbortController | null>(null);

  useEffect(() => {
    if (!enabled || !factoryAddress || !senderAddress) {
      setEstimate(null);
      return;
    }

    abortRef.current?.abort();
    const controller = new AbortController();
    abortRef.current = controller;

    const timeout = setTimeout(async () => {
      setLoading(true);
      setError(null);
      try {
        const result = await estimateFee(
          rpcUrl,
          factoryAddress,
          senderAddress,
          'CreateStream',
        );
        if (!controller.signal.aborted) {
          setEstimate(result);
        }
      } catch (e) {
        if (!controller.signal.aborted) {
          setError(e instanceof Error ? e.message : 'Failed to estimate fee');
        }
      } finally {
        if (!controller.signal.aborted) {
          setLoading(false);
        }
      }
    }, FEE_DEBOUNCE_MS);

    return () => {
      clearTimeout(timeout);
      controller.abort();
    };
  }, [rpcUrl, factoryAddress, senderAddress, enabled]);

  return { estimate, loading, error };
}
