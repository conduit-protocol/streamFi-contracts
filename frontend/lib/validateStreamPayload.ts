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

  return { valid: errors.length === 0, errors };
}
