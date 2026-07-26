import React, { useState } from 'react';
import { useMutation, gql } from '@apollo/client';
import { validateStreamPayload } from './lib/validateStreamPayload';

const SUBMIT_STREAM_REQUEST = gql`
  mutation SubmitStreamRequest($recipient: String!, $amount: Float!, $ratePerSecond: Float!) {
    submitStreamRequest(recipient: $recipient, amount: $amount, ratePerSecond: $ratePerSecond) {
      id
      status
    }
  }
`;

export const WalletConnection: React.FC = () => {
  const [recipient, setRecipient] = useState('');
  const [amount, setAmount] = useState('');
  const [ratePerSecond, setRatePerSecond] = useState('');
  const [validationErrors, setValidationErrors] = useState<string[]>([]);

  const [submitStreamRequest, { loading }] = useMutation(SUBMIT_STREAM_REQUEST);

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault();

    const payload = {
      recipient,
      amount: Number(amount),
      ratePerSecond: Number(ratePerSecond),
    };

    // FIX for Bug #149: validation previously only ran on each input's
    // onChange (disabling the submit button via UI state), but the submit
    // handler itself never re-checked the values before sending them
    // on-chain. That let invalid payloads through via the Enter key,
    // devtools, or a programmatically dispatched submit event. Re-validating
    // here, as the last step before the mutation fires, ensures a malformed
    // payload can never reach the contract regardless of how submit was
    // triggered.
    const result = validateStreamPayload(payload);
    setValidationErrors(result.errors);
    if (!result.valid) {
      return;
    }

    try {
      await submitStreamRequest({ variables: payload });
    } catch (e) {
      setValidationErrors([e instanceof Error ? e.message : 'Failed to submit stream request.']);
    }
  };

  return (
    <form className="wallet-connection" onSubmit={handleSubmit}>
      <h2>Connect Wallet &amp; Create Stream</h2>

      <label>
        Recipient address
        <input value={recipient} onChange={(e) => setRecipient(e.target.value)} />
      </label>

      <label>
        Amount
        <input value={amount} onChange={(e) => setAmount(e.target.value)} />
      </label>

      <label>
        Rate per second
        <input value={ratePerSecond} onChange={(e) => setRatePerSecond(e.target.value)} />
      </label>

      {validationErrors.length > 0 && (
        <ul className="validation-errors">
          {validationErrors.map((err) => (
            <li key={err}>{err}</li>
          ))}
        </ul>
      )}

      <button type="submit" disabled={loading}>
        {loading ? 'Submitting...' : 'Submit'}
      </button>
    </form>
  );
};
