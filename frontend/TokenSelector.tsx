import React, { useState, useImperativeHandle, forwardRef } from 'react';

const STELLAR_CONTRACT_ID_RE = /^C[A-Z2-7]{55}$/;

interface PresetToken {
  label: string;
  address: string;
}

const PRESET_TOKENS: PresetToken[] = [
  { label: 'USDC', address: 'CDLZFC3SYJYDZT7K3VJXUSJIIE5B6THFDLRLCKOS2I4CJ7D6N4J6P7' },
  { label: 'XLM', address: 'CAS3FHW5CJ7RKKKJ6W6C5H4K5G6H7J8K9L0M1N2B3V4C5X6Y7Z8A9B0C' },
];

export interface TokenSelectorHandle {
  validate: () => boolean;
}

interface TokenSelectorProps {
  onTokenSelected: (token: string) => void;
  initialToken?: string;
}

export const TokenSelector = forwardRef<TokenSelectorHandle, TokenSelectorProps>(
  ({ onTokenSelected, initialToken = '' }, ref) => {
    const [mode, setMode] = useState<'preset' | 'custom'>(
      initialToken && !PRESET_TOKENS.some((t) => t.address === initialToken)
        ? 'custom'
        : 'preset',
    );
    const [selectedPreset, setSelectedPreset] = useState<string>(
      initialToken && PRESET_TOKENS.some((t) => t.address === initialToken)
        ? initialToken
        : '',
    );
    const [customAddress, setCustomAddress] = useState(
      initialToken && !PRESET_TOKENS.some((t) => t.address === initialToken)
        ? initialToken
        : '',
    );
    const [validationErrors, setValidationErrors] = useState<string[]>([]);

    const validateToken = (address: string): boolean => {
      const errors: string[] = [];
      if (!address) {
        errors.push('Please select or enter a token address.');
      } else if (!STELLAR_CONTRACT_ID_RE.test(address)) {
        errors.push(
          'Token address must be a valid Soroban contract ID (starts with C, 56 characters).',
        );
      }
      setValidationErrors(errors);
      return errors.length === 0;
    };

    useImperativeHandle(ref, () => ({
      validate: () => {
        const address = mode === 'preset' ? selectedPreset : customAddress;
        return validateToken(address);
      },
    }));

    const handlePresetChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
      const value = e.target.value;
      setSelectedPreset(value);
      const isOtherOption = value === '__other__';
      setMode(isOtherOption ? 'custom' : 'preset');
      if (!isOtherOption) {
        setValidationErrors([]);
        onTokenSelected(value);
      }
    };

    const handleCustomChange = (e: React.ChangeEvent<HTMLInputElement>) => {
      const value = e.target.value;
      setCustomAddress(value);
      validateToken(value);
      if (STELLAR_CONTRACT_ID_RE.test(value)) {
        onTokenSelected(value);
      }
    };

    return (
      <div className="token-selector">
        <label>Token</label>

        {mode === 'preset' && (
          <select value={selectedPreset} onChange={handlePresetChange}>
            <option value="" disabled>
              Select a token...
            </option>
            {PRESET_TOKENS.map((token) => (
              <option key={token.address} value={token.address}>
                {token.label}
              </option>
            ))}
            <option value="__other__">Other (custom address)...</option>
          </select>
        )}

        {mode === 'custom' && (
          <div className="token-selector-custom">
            <input
              value={customAddress}
              onChange={handleCustomChange}
              placeholder="Enter Soroban contract ID (starts with C)"
            />
            <button
              type="button"
              onClick={() => {
                setMode('preset');
                setCustomAddress('');
                setSelectedPreset('');
                setValidationErrors([]);
              }}
            >
              Back to presets
            </button>
          </div>
        )}

        {validationErrors.length > 0 && (
          <ul className="validation-errors">
            {validationErrors.map((err) => (
              <li key={err}>{err}</li>
            ))}
          </ul>
        )}
      </div>
    );
  },
);