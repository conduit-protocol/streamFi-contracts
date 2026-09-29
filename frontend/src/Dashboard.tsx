import React, { useState, useEffect } from "react";
import { useQuery, gql } from "@apollo/client";
import styles from "./Dashboard.module.css";

export const GET_DASHBOARD_SUMMARY = gql`
  query GetDashboardSummary {
    dashboardSummary {
      activeStreams
      totalStreamed
      totalWithdrawn
    }
  }
`;

type MetricKey = "activeStreams" | "totalStreamed" | "totalWithdrawn";

const DASHBOARD_TIMEOUT_MS = 10_000;

export const Dashboard: React.FC = () => {
  const { data, loading, error } = useQuery(GET_DASHBOARD_SUMMARY);
  const [timedOut, setTimedOut] = useState(false);

  useEffect(() => {
    if (!loading) {
      setTimedOut(false);
      return;
    }
    const timer = setTimeout(() => setTimedOut(true), DASHBOARD_TIMEOUT_MS);
    return () => clearTimeout(timer);
  }, [loading]);

  // FIX for Bug #151: the previous version read
  // `data.dashboardSummary.activeStreams` directly inside useState's
  // initializer to seed the selected metric. That initializer runs on the
  // very first render, before the query has resolved, so `data` was still
  // `undefined` and the component crashed immediately on mount. Seeding
  // with `null` here defers touching the query result until it has
  // actually loaded (see `summary` below), so the initial render can never
  // dereference an undefined value.
  const [selectedMetric, setSelectedMetric] = useState<MetricKey | null>(null);

  if (timedOut) return <p>Error loading dashboard: Request timed out.</p>;
  if (loading) return <p>Loading dashboard...</p>;
  if (error) return <p>Error loading dashboard: {error.message}</p>;

  const summary = data?.dashboardSummary ?? {
    activeStreams: 0,
    totalStreamed: 0,
    totalWithdrawn: 0,
  };

  return (
    <div className={styles.dashboard}>
      <h2 className={styles.title}>Dashboard</h2>
      <ul className={styles.metricList}>
        <li className={styles.metricItem}>
          <button
            className={styles.metricButton}
            onClick={() => setSelectedMetric("activeStreams")}
          >
            Active streams: {summary.activeStreams}
          </button>
        </li>
        <li className={styles.metricItem}>
          <button
            className={styles.metricButton}
            onClick={() => setSelectedMetric("totalStreamed")}
          >
            Total streamed: {summary.totalStreamed}
          </button>
        </li>
        <li className={styles.metricItem}>
          <button
            className={styles.metricButton}
            onClick={() => setSelectedMetric("totalWithdrawn")}
          >
            Total withdrawn: {summary.totalWithdrawn}
          </button>
        </li>
      </ul>
      {selectedMetric && (
        <p className={styles.selectedLabel}>
          Selected metric: {selectedMetric}
        </p>
      )}
    </div>
  );
};
