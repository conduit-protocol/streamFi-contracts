Component: Token Selector
Issue: The data displayed is stale and does not reflect the latest on-chain state because the Apollo cache is not invalidated.

Steps to reproduce:

Navigate to Token Selector
Trigger action
Observe bug.
Expected: Graceful handling.