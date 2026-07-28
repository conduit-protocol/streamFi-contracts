use soroban_sdk::{Address, BytesN, Env, Symbol, Val, Vec};

/// Deploy a new DripStream instance and call its `initialize` function.
///
/// `deploy_v2` in soroban-sdk is specifically for contracts that use a
/// `__constructor` built-in. Since DripStream uses a named `initialize`
/// function, we use the two-step pattern: deploy the WASM first, then
/// invoke `initialize` via `env.invoke_contract`.
///
/// The `wasm_hash` must be valid and represent the currently-deployed DripStream
/// WASM code. If `wasm_hash` is invalid, stale, or has been replaced by
/// `upgrade_stream_wasm()` between the time a client read it and the time this
/// deployment executes, the `deploy()` call will fail. Off-chain clients should
/// read `stream_wasm_hash()` immediately before submitting the `create_stream`
/// transaction to minimize the window for concurrent upgrades.
pub fn deploy_stream(
    env: &Env,
    wasm_hash: &BytesN<32>,
    stream_id: u64,
    init_args: Vec<Val>,
) -> Address {
    // Derive a deterministic salt from the stream ID so each stream gets a
    // unique, reproducible contract address.
    let salt: BytesN<32> = env
        .crypto()
        .sha256(&soroban_sdk::Bytes::from_array(
            env,
            &stream_id.to_be_bytes(),
        ))
        .into();

    // Step 1: deploy the WASM — no constructor called yet.
    let addr = env
        .deployer()
        .with_current_contract(salt)
        .deploy(wasm_hash.clone());

    // Step 2: call `initialize` on the freshly deployed contract.
    let _: () = env.invoke_contract(&addr, &Symbol::new(env, "initialize"), init_args);

    addr
}
