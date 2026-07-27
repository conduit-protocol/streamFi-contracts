/// <reference types="react" />
import React, { useState } from 'react';
// @ts-ignore: Missing Apollo Client module/type declarations
import { useQuery, useMutation, gql } from '@apollo/client';

export const GET_NOTIFICATIONS = gql`
  query GetNotifications {
    notifications {
      id
      message
      date
      status
    }
  }
`;

const TRIGGER_ACTION = gql`
  mutation TriggerAction($message: String!) {
    triggerAction(message: $message) {
      id
      message
      date
      status
    }
  }
`;

// Validation helper defined outside component to avoid recreation on every render.
function validateNotificationMessage(message: string): string[] {
  const errors: string[] = [];

  if (!message || message.trim().length === 0) {
    errors.push('Message cannot be empty.');
  }

  if (message.length > 500) {
    errors.push('Message must be 500 characters or less.');
  }

  return errors;
}

export const Notifications: React.FC = () => {
  const { data, loading, error } = useQuery(GET_NOTIFICATIONS);
  const [validationErrors, setValidationErrors] = useState<string[]>([]);

  const [triggerAction, { loading: mutationLoading }] = useMutation(
    TRIGGER_ACTION,
    {
      // Refetch notifications after the action completes so the UI
      // reflects the latest on-chain state instead of stale Apollo cache data.
      refetchQueries: [{ query: GET_NOTIFICATIONS }],
      awaitRefetchQueries: true,
    }
  );

  if (loading) {
    return <div>Loading notifications...</div>;
  }

  if (error) {
    return <div>Error loading notifications: {error.message}</div>;
  }

  const handleAction = async () => {
    // Clear any previous errors before re-validating
    setValidationErrors([]);

    const message = 'Action completed successfully';

    // FIX for Bug: Notifications bypasses validation
    // Validate the payload before sending to the smart contract.
    // This ensures invalid payloads (empty messages, oversized content)
    // are caught client-side and never reach the contract.
    const errors = validateNotificationMessage(message);
    if (errors.length > 0) {
      setValidationErrors(errors);
      return;
    }

    try {
      await triggerAction({
        variables: {
          message,
        },
      });
      // Clear validation errors on successful submission
      setValidationErrors([]);
    } catch (e) {
      // Set error message for runtime failures
      setValidationErrors([
        e instanceof Error ? e.message : 'Action failed. Please try again.',
      ]);
    }
  };

  return (
    <div className="notifications">
      <h2>Notifications</h2>

      {validationErrors.length > 0 && (
        <ul className="validation-errors">
          {validationErrors.map((err) => (
            <li key={err}>{err}</li>
          ))}
        </ul>
      )}

      <button
        type="button"
        onClick={handleAction}
        disabled={mutationLoading}
      >
        {mutationLoading ? 'Processing...' : 'Trigger Action'}
      </button>

      <ul className="notification-list">
        {data?.notifications?.map((notification: any) => (
          <li key={notification.id} className="notification-item">
            <span>{notification.message}</span>
            <span>{notification.date}</span>
            <span>Status: {notification.status}</span>
          </li>
        ))}

        {(!data?.notifications || data.notifications.length === 0) && (
          <p>No notifications found.</p>
        )}
      </ul>
    </div>
  );
};
