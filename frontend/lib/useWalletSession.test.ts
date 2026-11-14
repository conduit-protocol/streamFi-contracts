import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { renderHook, act } from '@testing-library/react';
import { useWalletSession } from './useWalletSession';

describe('useWalletSession', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('starts disconnected', () => {
    const { result } = renderHook(() => useWalletSession());
    expect(result.current.isConnected).toBe(false);
    expect(result.current.session).toBeNull();
  });

  it('connects with the given address', () => {
    const { result } = renderHook(() => useWalletSession());
    act(() => result.current.connect('GABC...'));
    expect(result.current.isConnected).toBe(true);
    expect(result.current.session?.address).toBe('GABC...');
  });

  it('disconnects explicitly', () => {
    const { result } = renderHook(() => useWalletSession());
    act(() => result.current.connect('GABC...'));
    act(() => result.current.disconnect());
    expect(result.current.isConnected).toBe(false);
    expect(result.current.session).toBeNull();
  });

  it('expires the session automatically after the TTL and flags didExpire', () => {
    const { result } = renderHook(() => useWalletSession());
    act(() => result.current.connect('GABC...', 1000));
    expect(result.current.isConnected).toBe(true);

    act(() => vi.advanceTimersByTime(1000));

    expect(result.current.isConnected).toBe(false);
    expect(result.current.session).toBeNull();
    expect(result.current.didExpire).toBe(true);
  });

  it('clears the expiry flag on a fresh connect', () => {
    const { result } = renderHook(() => useWalletSession());
    act(() => result.current.connect('GABC...', 1000));
    act(() => vi.advanceTimersByTime(1000));
    expect(result.current.didExpire).toBe(true);

    act(() => result.current.connect('GDEF...'));
    expect(result.current.didExpire).toBe(false);
  });
});
