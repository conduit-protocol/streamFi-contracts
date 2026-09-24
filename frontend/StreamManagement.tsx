import React, { useState, useEffect } from 'react';
import { StreamAppSDK } from './lib/stream';
import { estimateFee } from './lib/estimateFee';
import styles from './StreamManagement.module.css';

const RPC_URL = process.env.REACT_APP_RPC_URL ?? 'https://soroban-testnet.stellar.org';

interface StreamManagementProps {
  streamId: string;
  currentUserAddress: string;
  sourceAccount: string;
}

export const StreamManagement: React.FC<StreamManagementProps> = ({
  streamId,
  currentUserAddress,
  sourceAccount,
}) => {
  const [operatorAddress, setOperatorAddress] = useState('');
  const [extraTimeSeconds, setExtraTimeSeconds] = useState('');
  const [topUpAmount, setTopUpAmount] = useState('');
  const [loading, setLoading] = useState(false);
  const [currentOperator, setCurrentOperator] = useState<string | null>(null);
  const [streamInfo, setStreamInfo] = useState<any>(null);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [feeEstimate, setFeeEstimate] = useState<any>(null);

  const sdk = new StreamAppSDK(RPC_URL);

  useEffect(() => {
    loadStreamInfo();
  }, [streamId]);

  const loadStreamInfo = async () => {
    try {
      setLoading(true);
      const [operator, info] = await Promise.all([
        sdk.getOperator(streamId),
        sdk.getStreamInfo(streamId),
      ]);
      setCurrentOperator(operator);
      setStreamInfo(info);
    } catch (err) {
      setError(`Failed to load stream info: ${err instanceof Error ? err.message : 'Unknown error'}`);
    } finally {
      setLoading(false);
    }
  };

  const handleSetOperator = async () => {
    if (!operatorAddress.trim()) {
      setError('Operator address is required');
      return;
    }

    try {
      setLoading(true);
      setError(null);
      setSuccess(null);

      const result = await sdk.setOperator({
        streamId,
        callerAddress: currentUserAddress,
        operatorAddress,
        sourceAccount,
        rpcUrl: RPC_URL,
      });

      setSuccess(`Operator set successfully! Transaction XDR: ${result.transactionXDR.slice(0, 50)}...`);
      setOperatorAddress('');
      await loadStreamInfo();
    } catch (err) {
      setError(`Failed to set operator: ${err instanceof Error ? err.message : 'Unknown error'}`);
    } finally {
      setLoading(false);
    }
  };

  const handleRevokeOperator = async () => {
    try {
      setLoading(true);
      setError(null);
      setSuccess(null);

      const result = await sdk.revokeOperator({
        streamId,
        callerAddress: currentUserAddress,
        sourceAccount,
        rpcUrl: RPC_URL,
      });

      setSuccess(`Operator revoked successfully! Transaction XDR: ${result.transactionXDR.slice(0, 50)}...`);
      await loadStreamInfo();
    } catch (err) {
      setError(`Failed to revoke operator: ${err instanceof Error ? err.message : 'Unknown error'}`);
    } finally {
      setLoading(false);
    }
  };

  const handleExtendDuration = async () => {
    if (!extraTimeSeconds.trim() || isNaN(Number(extraTimeSeconds)) || Number(extraTimeSeconds) <= 0) {
      setError('Valid extra time in seconds is required');
      return;
    }

    try {
      setLoading(true);
      setError(null);
      setSuccess(null);

      const result = await sdk.extendDuration({
        streamId,
        callerAddress: currentUserAddress,
        extraTimeSeconds,
        sourceAccount,
        rpcUrl: RPC_URL,
      });

      setSuccess(`Duration extended successfully! Transaction XDR: ${result.transactionXDR.slice(0, 50)}...`);
      setExtraTimeSeconds('');
      await loadStreamInfo();
    } catch (err) {
      setError(`Failed to extend duration: ${err instanceof Error ? err.message : 'Unknown error'}`);
    } finally {
      setLoading(false);
    }
  };

  const handleTopUpAndExtend = async () => {
    if (!topUpAmount.trim() || isNaN(Number(topUpAmount)) || Number(topUpAmount) <= 0) {
      setError('Valid top-up amount is required');
      return;
    }
    if (!extraTimeSeconds.trim() || isNaN(Number(extraTimeSeconds)) || Number(extraTimeSeconds) <= 0) {
      setError('Valid extra time in seconds is required');
      return;
    }

    try {
      setLoading(true);
      setError(null);
      setSuccess(null);

      const result = await sdk.topUpAndExtend({
        streamId,
        callerAddress: currentUserAddress,
        amount: topUpAmount,
        extraTimeSeconds,
        sourceAccount,
        rpcUrl: RPC_URL,
      });

      setSuccess(`Top-up and extend successful! Transaction XDR: ${result.transactionXDR.slice(0, 50)}...`);
      setTopUpAmount('');
      setExtraTimeSeconds('');
      await loadStreamInfo();
    } catch (err) {
      setError(`Failed to top-up and extend: ${err instanceof Error ? err.message : 'Unknown error'}`);
    } finally {
      setLoading(false);
    }
  };

  const estimateOperatorFee = async () => {
    try {
      const fee = await estimateFee(RPC_URL, streamId, currentUserAddress, 'SetOperator');
      setFeeEstimate(fee);
    } catch (err) {
      setError(`Failed to estimate fee: ${err instanceof Error ? err.message : 'Unknown error'}`);
    }
  };

  const estimateExtendFee = async () => {
    try {
      const fee = await estimateFee(RPC_URL, streamId, currentUserAddress, 'ExtendDuration');
      setFeeEstimate(fee);
    } catch (err) {
      setError(`Failed to estimate fee: ${err instanceof Error ? err.message : 'Unknown error'}`);
    }
  };

  if (loading && !streamInfo) {
    return <div className={styles.loading}>Loading stream information...</div>;
  }

  return (
    <div className={styles.streamManagement}>
      <h2>Stream Management</h2>
      <p>Stream ID: <code>{streamId}</code></p>

      {streamInfo && (
        <div className={styles.streamInfo}>
          <h3>Stream Information</h3>
          <p>Sender: {streamInfo.sender}</p>
          <p>Recipient: {streamInfo.recipient}</p>
          <p>Current Operator: {currentOperator || 'None'}</p>
          <p>End Time: {streamInfo.end_time}</p>
          <p>Withdrawable: {streamInfo.withdrawn} / {streamInfo.rate_per_second} per second</p>
        </div>
      )}

      <div className={styles.operatorManagement}>
        <h3>Operator Management</h3>
        
        <div className={styles.formGroup}>
          <label>
            Operator Address
            <input
              type="text"
              value={operatorAddress}
              onChange={(e) => setOperatorAddress(e.target.value)}
              placeholder="Enter operator address"
              disabled={loading}
            />
          </label>
          <button
            onClick={handleSetOperator}
            disabled={loading || !operatorAddress.trim()}
          >
            {loading ? 'Setting Operator...' : 'Set Operator'}
          </button>
          <button
            onClick={estimateOperatorFee}
            disabled={loading}
            className={styles.secondary}
          >
            Estimate Fee
          </button>
        </div>

        {currentOperator && (
          <div className={styles.formGroup}>
            <p>Current Operator: {currentOperator}</p>
            <button
              onClick={handleRevokeOperator}
              disabled={loading}
              className={styles.danger}
            >
              {loading ? 'Revoking...' : 'Revoke Operator'}
            </button>
          </div>
        )}
      </div>

      <div className={styles.durationManagement}>
        <h3>Duration Management</h3>
        
        <div className={styles.formGroup}>
          <label>
            Extra Time (seconds)
            <input
              type="number"
              value={extraTimeSeconds}
              onChange={(e) => setExtraTimeSeconds(e.target.value)}
              placeholder="Enter extra time in seconds"
              disabled={loading}
              min="1"
            />
          </label>
          <button
            onClick={handleExtendDuration}
            disabled={loading || !extraTimeSeconds.trim()}
          >
            {loading ? 'Extending...' : 'Extend Duration'}
          </button>
          <button
            onClick={estimateExtendFee}
            disabled={loading}
            className={styles.secondary}
          >
            Estimate Fee
          </button>
        </div>

        <div className={styles.formGroup}>
          <label>
            Top-up Amount
            <input
              type="number"
              value={topUpAmount}
              onChange={(e) => setTopUpAmount(e.target.value)}
              placeholder="Enter amount to top up"
              disabled={loading}
              min="1"
            />
          </label>
          <button
            onClick={handleTopUpAndExtend}
            disabled={loading || !topUpAmount.trim() || !extraTimeSeconds.trim()}
          >
            {loading ? 'Processing...' : 'Top-up and Extend'}
          </button>
        </div>
      </div>

      {feeEstimate && (
        <div className={styles.feeEstimate}>
          <h4>Fee Estimate</h4>
          <p>Network Fee: {feeEstimate.fee_xlm} XLM</p>
          <p>CPU Instructions: {feeEstimate.cpu_instructions}</p>
          <p>Ledger Entries: {feeEstimate.ledger_entries}</p>
        </div>
      )}

      {error && (
        <div className={styles.error}>
          <p>{error}</p>
        </div>
      )}

      {success && (
        <div className={styles.success}>
          <p>{success}</p>
        </div>
      )}
    </div>
  );
};