/// <reference types="react" />
import React from 'react';
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

export const Notifications: React.FC = () => {
  const { data, loading, error } = useQuery(GET_NOTIFICATIONS);

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
    try {
      await triggerAction({
        variables: {
          message: 'Action completed successfully',
        },
      });
    } catch (e) {
      console.error('Action failed gracefully', e);
    }
  };

  return (
    <div className="notifications">
      <h2>Notifications</h2>

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