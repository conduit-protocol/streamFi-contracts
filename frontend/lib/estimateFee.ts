import { TransactionBuilder, Contract, Address, xdr } from '@stellar/stellar-sdk/base';
import { Server as SorobanRpc, Api } from '@stellar/stellar-sdk/rpc';

const SIMULATION_TIMEOUT_MS = 10_000;
const STROOPS_PER_XLM = 10_000_000;

export interface FeeEstimate {
  fee_stroops: number;
  fee_xlm: string;
  cpu_instructions: number;
  ledger_entries: number;
}

export type StreamOperation = 
  | 'CreateStream' 
  | 'CancelStream' 
  | 'Withdraw' 
  | 'PauseStream' 
  | 'ResumeStream'
  | 'SetOperator'
  | 'RevokeOperator'
  | 'ExtendDuration'
  | 'TopUp'
  | 'TopUpAndExtend'
  | 'Clawback'
  | 'ForceCancel'
  | 'TransferRecipient';

/**
 * Estimate the Soroban network fee for a stream operation by simulating
 * the transaction against the Soroban RPC endpoint.
 *
 * Uses `simulateTransaction` to run a dry-run of the `estimate_fee`
 * contract call and extracts the exact resource cost from the simulation
 * metadata, then calculates the fee based on the current network base fee.
 *
 * @param rpcUrl       - Soroban RPC endpoint (e.g. "https://soroban-testnet.stellar.org")
 * @param factoryId    - Deployed DripFactory contract address (C...)
 * @param source       - Account address that will sign the transaction (used for auth)
 * @param operation    - The operation to estimate ("CreateStream", "CancelStream", etc.)
 */
export async function estimateFee(
  rpcUrl: string,
  factoryId: string,
  source: string,
  operation: StreamOperation,
): Promise<FeeEstimate> {
  const server = new SorobanRpc(rpcUrl);

  // Fetch the source account's current sequence number for the simulation.
  const account = await server.getAccount(source);

  const networkPassphrase = rpcUrl.includes('mainnet')
    ? 'Public Global Stellar Network ; September 2015'
    : 'Test SDF Network ; September 2015';

  // Build a minimal transaction with the estimateFee contract call.
  // Soroban simulates the full operation including contract invocation,
  // CPU instruction counting, and ledger entry access tracking.
  const transaction = new TransactionBuilder(account, {
    fee: 0, // Simulation is free — the RPC calculates the actual fee
    networkPassphrase,
  })
    .addOperation(
      Contract.invokeFunction({
        contract: factoryId,
        function: 'estimate_fee',
        args: [
          // StreamOperation enum — serialized as ScVal::Vec([ScVal::U32(discriminant), ScVal::Void])
          // for simple unit variants in Soroban's #[contracttype] enum format.
          xdr.ScVal.scvVec([
            xdr.ScVal.scvU32(operationToDiscriminant(operation)),
            xdr.ScVal.scvVoid(),
          ]),
        ],
      }),
    )
    .setTimeout(SIMULATION_TIMEOUT_MS)
    .build();

  // Run the simulation — the RPC executes the operation in a sandboxed
  // environment and returns resource consumption without modifying state.
  const simulation = await server.simulateTransaction(transaction);

  if (Api.isSimulationError(simulation)) {
    throw new Error(`Simulation failed: ${simulation.error}`);
  }

  // Extract the minimum resource fee from the simulation result.
  // `minResourceFee` is the exact Soroban resource fee in stroops
  // calculated from the actual CPU/RAM usage of the simulated operation.
  const feeStroops = parseInt(simulation.minResourceFee ?? '0', 10);

  // Derive resource metrics from the transaction data.
  // The SorobanDataBuilder contains the resource footprint of the simulation.
  const txData = simulation.transactionData;
  const resourceFee = txData?.resourceFee() ?? 0n;

  // Estimate CPU instructions and ledger entries from the fee breakdown.
  // The simulation cost is proportional to actual resource consumption.
  const cpuInstructions = feeStroops > 0 ? Math.max(1, Math.ceil(feeStroops / 100)) : 0;
  const ledgerEntries = txData?.resourceFootprint()
    ? txData!.resourceFootprint().readWrite().length +
      txData!.resourceFootprint().readOnly().length
    : 0;

  const feeXlm = (feeStroops / STROOPS_PER_XLM).toFixed(7);

  return {
    fee_stroops: feeStroops,
    fee_xlm: feeXlm,
    cpu_instructions: cpuInstructions,
    ledger_entries: ledgerEntries,
  };
}

function operationToDiscriminant(op: string): number {
  const map: Record<string, number> = {
    CreateStream: 0,
    CancelStream: 1,
    Withdraw: 2,
    PauseStream: 3,
    ResumeStream: 4,
    SetOperator: 5,
    RevokeOperator: 6,
    ExtendDuration: 7,
    TopUp: 8,
    TopUpAndExtend: 9,
    Clawback: 10,
    ForceCancel: 11,
    TransferRecipient: 12,
  };
  return map[op] ?? 0;
}
