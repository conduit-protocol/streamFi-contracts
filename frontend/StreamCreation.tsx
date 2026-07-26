import React, { useState } from 'react';

const RPC_TIMEOUT_MS = 10_000;

interface CreateStreamPayload {
  recipient: string;
  amount: number;
}

async function createStreamOnChain(payload: CreateStreamPayload): Promise<{ streamId: string }> {
  const response = await fetch('/api/streams', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  });

  if (!response.ok) {
    throw new Error(`Stream creation failed with status ${response.status}`);
  }

  return response.json();
}

export const StreamCreation: React.FC = () => {
  const [recipient, setRecipient] = useState('');
  const [amount, setAmount] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    setError(null);
    setLoading(true);

    // FIX for Bug #150: the previous implementation awaited the RPC call
    // with no timeout, so if the RPC provider never responded, `loading`
    // was never reset and the spinner spun forever. Racing the request
    // against a timeout guarantees the loading state always clears, either
    // with a result or a clear timeout error.
    const timeout = new Promise<never>((_, reject) => {
      setTimeout(() => reject(new Error('RPC provider timed out. Please try again.')), RPC_TIMEOUT_MS);
    });

    try {
      await Promise.race([createStreamOnChain({ recipient, amount: Number(amount) }), timeout]);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to create stream.');
    } finally {
      setLoading(false);
    }
  };

  return (
    <form className="stream-creation" onSubmit={handleSubmit}>
      <h2>Create Stream</h2>

      <label>
        Recipient address
        <input value={recipient} onChange={(e) => setRecipient(e.target.value)} />
      </label>

      <label>
        Amount
        <input value={amount} onChange={(e) => setAmount(e.target.value)} />
      </label>

      {error && <p className="error">{error}</p>}

      <button type="submit" disabled={loading}>
        {loading ? 'Creating stream...' : 'Create stream'}
      </button>
    </form>
  );
};
