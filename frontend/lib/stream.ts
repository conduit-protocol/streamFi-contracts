import { StreamSDK, StreamInfo } from '../src/streams';
import { estimateFee, StreamOperation } from './estimateFee';

export interface StreamActionParams {
  streamId: string;
  callerAddress: string;
  sourceAccount: string;
  rpcUrl: string;
  networkPassphrase?: string;
}

export interface SetOperatorParams extends StreamActionParams {
  operatorAddress: string;
}

export interface ExtendDurationParams extends StreamActionParams {
  extraTimeSeconds: string;
}

export interface TopUpAndExtendParams extends StreamActionParams {
  amount: string;
  extraTimeSeconds: string;
}

export class StreamAppSDK {
  private sdk: StreamSDK;
  private rpcUrl: string;

  constructor(rpcUrl: string, networkPassphrase?: string) {
    this.sdk = new StreamSDK(rpcUrl, networkPassphrase);
    this.rpcUrl = rpcUrl;
  }

  /**
   * Set operator for a stream
   */
  async setOperator(params: SetOperatorParams): Promise<{ transactionXDR: string; feeEstimate?: any }> {
    const transactionXDR = await this.sdk.setOperator(
      params.streamId,
      params.callerAddress,
      params.operatorAddress,
      params.sourceAccount
    );

    const feeEstimate = await estimateFee(
      this.rpcUrl,
      params.streamId,
      params.callerAddress,
      'SetOperator'
    ).catch(() => undefined);

    return { transactionXDR, feeEstimate };
  }

  /**
   * Revoke operator for a stream
   */
  async revokeOperator(params: StreamActionParams): Promise<{ transactionXDR: string; feeEstimate?: any }> {
    const transactionXDR = await this.sdk.revokeOperator(
      params.streamId,
      params.callerAddress,
      params.sourceAccount
    );

    const feeEstimate = await estimateFee(
      this.rpcUrl,
      params.streamId,
      params.callerAddress,
      'RevokeOperator'
    ).catch(() => undefined);

    return { transactionXDR, feeEstimate };
  }

  /**
   * Extend stream duration
   */
  async extendDuration(params: ExtendDurationParams): Promise<{ transactionXDR: string; feeEstimate?: any }> {
    const transactionXDR = await this.sdk.extendDuration(
      params.streamId,
      params.callerAddress,
      params.extraTimeSeconds,
      params.sourceAccount
    );

    const feeEstimate = await estimateFee(
      this.rpcUrl,
      params.streamId,
      params.callerAddress,
      'ExtendDuration'
    ).catch(() => undefined);

    return { transactionXDR, feeEstimate };
  }

  /**
   * Top up and extend duration
   */
  async topUpAndExtend(params: TopUpAndExtendParams): Promise<{ transactionXDR: string; feeEstimate?: any }> {
    const transactionXDR = await this.sdk.topUpAndExtend(
      params.streamId,
      params.callerAddress,
      params.amount,
      params.extraTimeSeconds,
      params.sourceAccount
    );

    const feeEstimate = await estimateFee(
      this.rpcUrl,
      params.streamId,
      params.callerAddress,
      'TopUpAndExtend'
    ).catch(() => undefined);

    return { transactionXDR, feeEstimate };
  }

  /**
   * Get stream information
   */
  async getStreamInfo(streamId: string): Promise<StreamInfo> {
    return await this.sdk.getStreamInfo(streamId);
  }

  /**
   * Get current operator
   */
  async getOperator(streamId: string): Promise<string | null> {
    return await this.sdk.getOperator(streamId);
  }

  /**
   * Get withdrawable balance
   */
  async getWithdrawable(streamId: string): Promise<string> {
    return await this.sdk.getWithdrawable(streamId);
  }

  /**
   * Check if clawback is enabled
   */
  async isClawbackEnabled(streamId: string): Promise<boolean> {
    return await this.sdk.isClawbackEnabled(streamId);
  }

  /**
   * Get total streamed amount
   */
  async getStreamedTotal(streamId: string): Promise<string> {
    return await this.sdk.getStreamedTotal(streamId);
  }

  /**
   * Get fee estimate for any stream operation
   */
  async getFeeEstimate(
    streamId: string,
    callerAddress: string,
    operation: StreamOperation
  ): Promise<any> {
    return await estimateFee(this.rpcUrl, streamId, callerAddress, operation);
  }
}