import React, { useState } from 'react';
import { useMutation, gql } from '@apollo/client';
import { validateStreamPayload } from './lib/validateStreamPayload';

const SUBMIT_STREAM_REQUEST_MOBILE = gql`
  mutation SubmitStreamRequestMobile($recipient: String!, $amount: Float!, $ratePerSecond: Float!) {
    submitStreamRequest(recipient: $recipient, amount: $amount, ratePerSecond: $ratePerSecond) {
      id
      status
    }
  }
`;

export const MobileView: React.FC = () => {
  const [recipient, setRecipient] = useState('');
  const [amount, setAmount] = useState('');
  const [ratePerSecond, setRatePerSecond] = useState('');
  const [validationErrors, setValidationErrors] = useState<string[]>([]);

  const [submitStreamRequest, { loading }] = useMutation(SUBMIT_STREAM_REQUEST_MOBILE);

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault();

    const payload = {
      recipient,
      amount: Number(amount),
      ratePerSecond: Number(ratePerSecond),
    };

    // FIX for Bug #159: same underlying issue as the desktop Wallet
    // Connection flow (#149) — validation only ran on each input's
    // onChange, never re-checked at submit time, so an invalid payload
    // could reach the contract via the mobile form's Enter-to-submit or a
    // programmatically dispatched submit event. Reuses the shared
    // validateStreamPayload() helper and re-runs it as the last step
    // before the mutation fires.
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
    <form className="mobile-view" onSubmit={handleSubmit}>
      <h2>Create Stream</h2>

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
