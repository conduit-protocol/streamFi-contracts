export interface StreamPayload {
  recipient: string;
  amount: number;
  ratePerSecond: number;
}

const STELLAR_ADDRESS_RE = /^G[A-Z2-7]{55}$/;

export interface ValidationResult {
  valid: boolean;
  errors: string[];
}

export function validateStreamPayload(payload: Partial<StreamPayload>): ValidationResult {
  const errors: string[] = [];

  if (!payload.recipient || !STELLAR_ADDRESS_RE.test(payload.recipient)) {
    errors.push('Recipient must be a valid Stellar public address (starts with G, 56 characters).');
  }

  if (payload.amount === undefined || !Number.isFinite(payload.amount) || payload.amount <= 0) {
    errors.push('Amount must be a positive number.');
  }

  if (
    payload.ratePerSecond === undefined ||
    !Number.isFinite(payload.ratePerSecond) ||
    payload.ratePerSecond <= 0
  ) {
    errors.push('Rate per second must be a positive number.');
  }

  // `create_stream` requires `deposit >= rate_per_sec`, so the two independent
  // positivity checks above are not enough: a payload can satisfy both and
  // still revert on-chain with `InsufficientDeposit`. Cross-check the
  // relationship the factory enforces so pre-submission validation actually
  // predicts the on-chain outcome.
  //
  // This only runs when both fields are individually valid — otherwise the
  // caller already gets the specific "must be a positive number" error and a
  // second derived complaint would just be noise. If `endTime` is ever added
  // to `StreamPayload`, the full-duration check (`amount >= ratePerSecond *
  // (endTime - startTime)`) belongs here as well.
  if (
    payload.amount !== undefined &&
    Number.isFinite(payload.amount) &&
    payload.amount > 0 &&
    payload.ratePerSecond !== undefined &&
    Number.isFinite(payload.ratePerSecond) &&
    payload.ratePerSecond > 0 &&
    payload.amount < payload.ratePerSecond
  ) {
    errors.push(
      'Amount must be greater than or equal to the rate per second: the deposit has to cover at least one second of streaming.',
    );
  }

  return { valid: errors.length === 0, errors };
}
