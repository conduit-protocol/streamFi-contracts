import React, { useState } from 'react';
import { useQuery, gql } from '@apollo/client';

const GET_DASHBOARD_SUMMARY = gql`
  query GetDashboardSummary {
    dashboardSummary {
      activeStreams
      totalStreamed
      totalWithdrawn
    }
  }
`;

type MetricKey = 'activeStreams' | 'totalStreamed' | 'totalWithdrawn';

export const Dashboard: React.FC = () => {
  const { data, loading, error } = useQuery(GET_DASHBOARD_SUMMARY);

  // FIX for Bug #151: the previous version read
  // `data.dashboardSummary.activeStreams` directly inside useState's
  // initializer to seed the selected metric. That initializer runs on the
  // very first render, before the query has resolved, so `data` was still
  // `undefined` and the component crashed immediately on mount. Seeding
  // with `null` here defers touching the query result until it has
  // actually loaded (see `summary` below), so the initial render can never
  // dereference an undefined value.
  const [selectedMetric, setSelectedMetric] = useState<MetricKey | null>(null);

  if (loading) return <p>Loading dashboard...</p>;
  if (error) return <p>Error loading dashboard: {error.message}</p>;

  const summary = data?.dashboardSummary ?? {
    activeStreams: 0,
    totalStreamed: 0,
    totalWithdrawn: 0,
  };

  return (
    <div className="dashboard">
      <h2>Dashboard</h2>
      <ul>
        <li>
          <button onClick={() => setSelectedMetric('activeStreams')}>
            Active streams: {summary.activeStreams}
          </button>
        </li>
        <li>
          <button onClick={() => setSelectedMetric('totalStreamed')}>
            Total streamed: {summary.totalStreamed}
          </button>
        </li>
        <li>
          <button onClick={() => setSelectedMetric('totalWithdrawn')}>
            Total withdrawn: {summary.totalWithdrawn}
          </button>
        </li>
      </ul>
      {selectedMetric && <p>Selected metric: {selectedMetric}</p>}
    </div>
  );
};
