import React, { useState } from 'react';
import { useQuery, useMutation, gql, useApolloClient } from '@apollo/client';

const MUTATION_TIMEOUT_MS = 10_000;

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

interface TransactionHistoryProps {
  onRefreshNeeded?: () => void;
}

export const TransactionHistory: React.FC<TransactionHistoryProps> = ({ onRefreshNeeded }) => {
  const { data, loading, error } = useQuery(GET_TRANSACTIONS);
  const [actionError, setActionError] = useState<string | null>(null);
  const client = useApolloClient();

  const [triggerAction, { loading: mutationLoading }] = useMutation(TRIGGER_ACTION, {
    onCompleted: () => {
      client.cache.reset();
      onRefreshNeeded?.();
    },
  });

  if (loading) return <p>Loading transaction history...</p>;
  if (error) return <p>Error loading transactions: {error.message}</p>;

  const handleAction = async () => {
    setActionError(null);

    const timeout = new Promise<never>((_, reject) => {
      setTimeout(() => reject(new Error('Action timed out. Please try again.')), MUTATION_TIMEOUT_MS);
    });

    try {
      await Promise.race([triggerAction({ variables: { amount: 100 } }), timeout]);
    } catch (e) {
      setActionError(e instanceof Error ? e.message : 'Action failed. Please try again.');
    }
  };

  return (
    <div className="transaction-history">
      <h2>Transaction History</h2>
      <button onClick={handleAction} disabled={mutationLoading}>
        {mutationLoading ? 'Processing...' : 'Trigger Action'}
      </button>

      {actionError && <p className="action-error">{actionError}</p>}
      
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
