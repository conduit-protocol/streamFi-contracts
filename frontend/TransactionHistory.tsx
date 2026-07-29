import React, { useState, useEffect } from 'react';
import { useQuery, useMutation, gql } from '@apollo/client';

export const GET_TRANSACTIONS = gql`
  query GetTransactions {
    transactions {
      id
      amount
      date
      status
    }
  }
`;

const TRIGGER_ACTION = gql`
  mutation TriggerAction($amount: Float!) {
    triggerAction(amount: $amount) {
      id
      amount
      date
      status
    }
  }
`;

const TRANSACTION_TIMEOUT_MS = 10_000;

export const TransactionHistory: React.FC = () => {
  const { data, loading, error } = useQuery(GET_TRANSACTIONS);
  const [timedOut, setTimedOut] = useState(false);

  useEffect(() => {
    if (!loading) {
      setTimedOut(false);
      return;
    }
    const timer = setTimeout(() => setTimedOut(true), TRANSACTION_TIMEOUT_MS);
    return () => clearTimeout(timer);
  }, [loading]);
  
  const [triggerAction, { loading: mutationLoading }] = useMutation(TRIGGER_ACTION, {
    // FIX for Bug #140: Invalidate Apollo cache to ensure the data displayed 
    // is not stale and reflects the latest on-chain state.
    // Using refetchQueries guarantees graceful handling and fresh data on re-render.
    refetchQueries: [{ query: GET_TRANSACTIONS }],
    awaitRefetchQueries: true,
  });

  if (timedOut) return <p>Error loading transactions: Request timed out.</p>;
  if (loading) return <p>Loading transaction history...</p>;
  if (error) return <p>Error loading transactions: {error.message}</p>;

  const handleAction = async () => {
    try {
      await triggerAction({ variables: { amount: 100 } });
    } catch (e) {
      console.error('Action failed gracefully', e);
    }
  };

  return (
    <div className="transaction-history">
      <h2>Transaction History</h2>
      <button onClick={handleAction} disabled={mutationLoading}>
        {mutationLoading ? 'Processing...' : 'Trigger Action'}
      </button>
      
      <ul className="transaction-list">
        {data?.transactions?.map((tx: any) => (
          <li key={tx.id} className="transaction-item">
            <span>{tx.date}</span>
            <span>${tx.amount}</span>
            <span>Status: {tx.status}</span>
          </li>
        ))}
        {(!data?.transactions || data.transactions.length === 0) && (
          <p>No transactions found.</p>
        )}
      </ul>
    </div>
  );
};
