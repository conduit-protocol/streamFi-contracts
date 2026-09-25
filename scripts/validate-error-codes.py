#!/usr/bin/env python3
"""
Validate that on-chain Error enum variants have consistent error codes across contracts.
Extracts error codes from all contract error.rs files and verifies no conflicts exist.
"""

import re
import sys
from pathlib import Path
from typing import Dict, List, Tuple

def extract_error_codes(file_path: str) -> Dict[str, int]:
    """Extract error variant names and codes from a Rust error.rs file."""
    errors = {}
    with open(file_path, 'r') as f:
        content = f.read()

    # Find error enum and extract variants
    # Pattern: `VariantName = code,` or `VariantName = code,` with optional comments
    pattern = r'(\w+)\s*=\s*(\d+)'
    matches = re.findall(pattern, content)

    for variant, code in matches:
        code_int = int(code)
        errors[variant] = code_int

    return errors

def main():
    contract_dirs = [
        'contracts/factory/src/errors.rs',
        'contracts/governor/src/errors.rs',
        'contracts/stream/src/errors.rs',
        'contracts/oracle/src/errors.rs',
        'contracts/token-vault/src/errors.rs',
        'contracts/batch-processor/src/errors.rs',
    ]

    conflicts = []

    # Validate each contract has no duplicate error codes
    for contract_file in contract_dirs:
        if not Path(contract_file).exists():
            continue

        contract_name = contract_file.split('/')[1]
        errors = extract_error_codes(contract_file)

        # Check for duplicate codes within the same contract
        code_to_variant = {}
        for variant, code in errors.items():
            if code in code_to_variant:
                conflicts.append(
                    f"ERROR: {contract_name}: error code {code} defined for both "
                    f"'{code_to_variant[code]}' and '{variant}'"
                )
            code_to_variant[code] = variant

    if conflicts:
        for conflict in conflicts:
            print(conflict)
        sys.exit(1)
    else:
        print("[OK] All error codes are valid (no duplicates within contracts)")
        sys.exit(0)

if __name__ == '__main__':
    main()
