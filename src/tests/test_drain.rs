#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address,
};

use crate::{
    contract::{VestingDrips, VestingDripsClient, DRAIN_DELAY_LEDGERS},
    error::VestingError,
    types::RATE_DECIMALS,
};

use super::super::tests::token_helper::{create_token, mint_to};

/// Creates a fixture whose entries stay live throughout the drain delay.
pub(super) fn setup_env() -> soroban_sdk::Env {
    let env = soroban_sdk::Env::default();
    env.mock_all_auths();
    // Model restored/maintained entries before the year-long ledger jump.
    // Raising only max_entry_ttl after creation does not extend existing keys.
    env.ledger().with_mut(|info| {
        info.sequence_number = 100;
        info.min_persistent_entry_ttl = DRAIN_DELAY_LEDGERS + 10_000;
        info.max_entry_ttl = DRAIN_DELAY_LEDGERS + 20_000;
    });
    env
}

fn advance_ledger_high_ttl(env: &soroban_sdk::Env, n: u32) {
    let current = env.ledger().sequence();
    env.ledger().set(LedgerInfo {
        timestamp: 0,
        protocol_version: 22,
        sequence_number: current + n,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 100,
        min_persistent_entry_ttl: 1000,
        max_entry_ttl: u32::MAX,
    });
}

/// Creates a basic stream for drain testing.
/// rate=10, cliff_duration=50, total_duration=200 → deposit=2000
fn setup_drain_stream(
    env: &soroban_sdk::Env,
) -> (
    soroban_sdk::Address, // contract_id
    VestingDripsClient,
    soroban_sdk::Address, // sponsor
    soroban_sdk::Address, // recipient
    soroban_sdk::Address, // token_id
) {
    let contract_id = env.register(VestingDrips, ());
    let client = VestingDripsClient::new(env, &contract_id);
    client.initialize(&Address::generate(env), &0, &Address::generate(env));

    let sponsor = Address::generate(env);
    let recipient = Address::generate(env);
    let (token_id, _) = create_token(env, &sponsor);
    mint_to(env, &token_id, &sponsor, 2_000);

    client.create_vesting_stream(
        &sponsor, &recipient, &token_id, &(10 * RATE_DECIMALS), &50, &200, &None,
    );

    (contract_id, client, sponsor, recipient, token_id)
}

#[test]
fn test_drain_before_end_ledger_fails() {
    let env = setup_env();
    let (_, client, _, recipient, _) = setup_drain_stream(&env);

    let caller = Address::generate(&env);
    // Stream end_ledger = 300; we're at ledger 100+50=150, still before end.
    advance_ledger_high_ttl(&env, 50);

    let err = client.try_drain_expired_stream(&caller, &recipient).unwrap_err();
    assert_eq!(err, Ok(VestingError::StreamNotExpired));
}

#[test]
fn test_drain_after_end_but_before_delay_fails() {
    let env = setup_env();
    let (_, client, _, recipient, _) = setup_drain_stream(&env);

    let caller = Address::generate(&env);
    // Advance past end_ledger (100 + 200 = 300) by 10 ledgers → ledger 310
    // But not yet past the drain delay (300 + 6_307_200).
    advance_ledger_high_ttl(&env, 210);

    let err = client.try_drain_expired_stream(&caller, &recipient).unwrap_err();
    assert_eq!(err, Ok(VestingError::DrainDelayNotExpired));
}

#[test]
fn test_drain_after_full_delay_succeeds() {
    let env = setup_env();
    let (_, client, sponsor, recipient, token_id) = setup_drain_stream(&env);

    let caller = Address::generate(&env);
    let token_client = soroban_sdk::token::TokenClient::new(&env, &token_id);

    // end_ledger = 300.  drain_available_at = 300 + 6_307_200 = 6_307_500.
    // current = 100 → advance 6_307_401 to reach ledger 6_307_501.
    advance_ledger_high_ttl(&env, DRAIN_DELAY_LEDGERS + 201);

    client.drain_expired_stream(&caller, &recipient);

    // All unclaimed tokens should go to sponsor.
    assert_eq!(token_client.balance(&sponsor), 2_000);
    assert_eq!(token_client.balance(&recipient), 0);
    assert!(client.get_schedule(&recipient).is_none());
}

#[test]
fn test_drain_partial_claim_then_drain() {
    let env = setup_env();
    let (_, client, sponsor, recipient, token_id) = setup_drain_stream(&env);

    let caller = Address::generate(&env);
    let token_client = soroban_sdk::token::TokenClient::new(&env, &token_id);

    // Recipient claims at cliff (ledger 150 → claimed 500).
    advance_ledger_high_ttl(&env, 50);
    client.claim_vested(&recipient, &None);

    // Advance past drain threshold.
    advance_ledger_high_ttl(&env, DRAIN_DELAY_LEDGERS + 151);

    client.drain_expired_stream(&caller, &recipient);

    // last_claimed_ledger after first claim = 150
    // remaining = (300 - 150) × 10 = 1500
    assert_eq!(token_client.balance(&recipient), 500);
    assert_eq!(token_client.balance(&sponsor), 1_500);
    assert!(client.get_schedule(&recipient).is_none());
}

#[test]
fn test_drain_nonexistent_stream_fails() {
    let env = setup_env();
    let contract_id = env.register(VestingDrips, ());
    let client = VestingDripsClient::new(&env, &contract_id);
    let _adm_client = soroban_sdk::Address::generate(&env);
    let _trs_client = soroban_sdk::Address::generate(&env);
    client.initialize(&_adm_client, &0u32, &_trs_client);

    let caller = Address::generate(&env);
    let recipient = Address::generate(&env);

    let err = client.try_drain_expired_stream(&caller, &recipient).unwrap_err();
    assert_eq!(err, Ok(VestingError::ScheduleNotFound));
}

#[test]
fn test_drain_callable_by_anyone() {
    let env = setup_env();
    let (_, client, sponsor, recipient, token_id) = setup_drain_stream(&env);

    // A random third party (not sponsor, not recipient) can call drain.
    let random_caller = Address::generate(&env);
    let token_client = soroban_sdk::token::TokenClient::new(&env, &token_id);

    advance_ledger_high_ttl(&env, DRAIN_DELAY_LEDGERS + 201);
    client.drain_expired_stream(&random_caller, &recipient);

    // Tokens go to original sponsor, not the caller.
    assert_eq!(token_client.balance(&sponsor), 2_000);
    assert_eq!(token_client.balance(&random_caller), 0);
}

#[test]
fn test_drain_exact_boundary_still_fails() {
    let env = setup_env();
    let (_, client, _, recipient, _) = setup_drain_stream(&env);

    let caller = Address::generate(&env);
    // Advance to exactly drain_available_at - 1.
    // end_ledger = 300. drain_available_at = 300 + 6_307_200 = 6_307_500.
    // current = 100 → advance 6_307_399 to reach ledger 6_307_499 (one before).
    advance_ledger_high_ttl(&env, DRAIN_DELAY_LEDGERS + 199);

    let err = client.try_drain_expired_stream(&caller, &recipient).unwrap_err();
    assert_eq!(err, Ok(VestingError::DrainDelayNotExpired));
}

// ── Boundary tests for drain_expired_stream delay enforcement (Issue #734) ──
//
// The 1-year drain delay is: end_ledger + DRAIN_DELAY_LEDGERS.
// For the test stream: end_ledger = 300, so drain_available_at = 300 + DRAIN_DELAY_LEDGERS.
//
// Three critical boundary points:
//   1. At exactly end_ledger (ledger 300) → StreamNotExpired
//   2. At drain_available_at - 1 (one ledger before threshold) → DrainDelayNotExpired
//   3. At exactly drain_available_at → success

/// At exactly `end_ledger`, the stream is expired but the drain delay has not
/// started — `drain_expired_stream` should fail with `StreamNotExpired`.
#[test]
fn test_drain_at_exactly_end_ledger_fails_delay_not_expired() {
    let env = setup_env();
    let (_, client, _, recipient, _) = setup_drain_stream(&env);
    let caller = Address::generate(&env);

    // Stream: start_ledger=100, total_duration=200 → end_ledger=300.
    // Advance from 100 to exactly 300 (advance 200 ledgers).
    advance_ledger_high_ttl(&env, 200);

    let err = client.try_drain_expired_stream(&caller, &recipient).unwrap_err();
    assert_eq!(
        err,
        Ok(VestingError::DrainDelayNotExpired),
        "at exactly end_ledger, expected DrainDelayNotExpired"
    );
}

/// One ledger before the drain threshold (`drain_available_at - 1`) must fail
/// with `DrainDelayNotExpired`, not succeed.
///
/// drain_available_at = end_ledger + DRAIN_DELAY_LEDGERS = 300 + DRAIN_DELAY_LEDGERS
/// Target ledger: drain_available_at - 1 = 299 + DRAIN_DELAY_LEDGERS
/// Advance from 100: 299 + DRAIN_DELAY_LEDGERS - 100 = 199 + DRAIN_DELAY_LEDGERS
#[test]
fn test_drain_one_ledger_before_threshold_fails() {
    let env = setup_env();
    let (_, client, _, recipient, _) = setup_drain_stream(&env);
    let caller = Address::generate(&env);

    // Advance to drain_available_at - 1.
    // end_ledger = 300; drain_available_at = 300 + DRAIN_DELAY_LEDGERS.
    // current = 100 → advance (300 + DRAIN_DELAY_LEDGERS - 1 - 100) = 199 + DRAIN_DELAY_LEDGERS.
    advance_ledger_high_ttl(&env, DRAIN_DELAY_LEDGERS + 199);

    let err = client.try_drain_expired_stream(&caller, &recipient).unwrap_err();
    assert_eq!(
        err,
        Ok(VestingError::DrainDelayNotExpired),
        "one ledger before threshold, expected DrainDelayNotExpired"
    );
}

/// At exactly `drain_available_at` (`end_ledger + DRAIN_DELAY_LEDGERS`),
/// `drain_expired_stream` must succeed and send all tokens to the sponsor.
///
/// Target ledger: 300 + DRAIN_DELAY_LEDGERS
/// Advance from 100: 300 + DRAIN_DELAY_LEDGERS - 100 = 200 + DRAIN_DELAY_LEDGERS
#[test]
fn test_drain_at_exact_threshold_succeeds() {
    let env = setup_env();
    let (_, client, sponsor, recipient, token_id) = setup_drain_stream(&env);
    let caller = Address::generate(&env);
    let token_client = soroban_sdk::token::TokenClient::new(&env, &token_id);

    // Advance to exactly drain_available_at = 300 + DRAIN_DELAY_LEDGERS.
    // current = 100 → advance 200 + DRAIN_DELAY_LEDGERS.
    advance_ledger_high_ttl(&env, DRAIN_DELAY_LEDGERS + 200);

    client
        .drain_expired_stream(&caller, &recipient);

    // All unclaimed tokens go to original sponsor.
    assert_eq!(token_client.balance(&sponsor), 2_000);
    assert_eq!(token_client.balance(&recipient), 0);
    // Schedule is cleaned up.
    assert!(client.get_schedule(&recipient).is_none());
}

/// `drain_expired_stream` does not require authentication — any address can
/// trigger it. The caller receives nothing; tokens always go to the original
/// sponsor.
#[test]
fn test_drain_permissionless_no_auth_required() {
    let env = setup_env();
    let (_, client, sponsor, recipient, token_id) = setup_drain_stream(&env);
    let token_client = soroban_sdk::token::TokenClient::new(&env, &token_id);

    // Use a completely unrelated address (not sponsor, not recipient).
    let unrelated_caller = Address::generate(&env);

    advance_ledger_high_ttl(&env, DRAIN_DELAY_LEDGERS + 201);

    // Clear setup authorization: drain must genuinely be permissionless.
    env.mock_auths(&[]);
    client
        .drain_expired_stream(&unrelated_caller, &recipient);

    // Caller receives nothing; sponsor gets all unclaimed tokens.
    assert_eq!(token_client.balance(&unrelated_caller), 0);
    assert_eq!(token_client.balance(&sponsor), 2_000);
}

/// Fuzz-style test: 10 different random caller addresses all succeed and the
/// economic outcome is always the same (tokens go to sponsor, not caller).
#[test]
fn test_drain_fuzz_random_callers_all_succeed() {
    for _i in 0..10 {
        let env = setup_env();
        let (_, client, sponsor, recipient, token_id) = setup_drain_stream(&env);
        let token_client = soroban_sdk::token::TokenClient::new(&env, &token_id);

        // Generate a fresh random caller each iteration.
        let random_caller = Address::generate(&env);

        advance_ledger_high_ttl(&env, DRAIN_DELAY_LEDGERS + 201);

        client
            .drain_expired_stream(&random_caller, &recipient);

        // Invariant: tokens go to sponsor, never to the caller.
        assert_eq!(
            token_client.balance(&sponsor),
            2_000,
            "iteration {_i}: sponsor should receive all tokens"
        );
        assert_eq!(
            token_client.balance(&random_caller),
            0,
            "iteration {_i}: caller should receive nothing"
        );
        // Schedule removed after drain.
        assert!(client.get_schedule(&recipient).is_none());
    }
}

// Direct fixtures exercise drain independently of creation/claim entry points.
pub(super) fn seed_cleanup_stream(env: &soroban_sdk::Env, claimed: i128) -> (Address, Address, Address, Address) {
    use crate::types::{DataKey, VestingSchedule, CURRENT_SCHEMA_VERSION};
    let contract = env.register(VestingDrips, ());
    let sponsor = Address::generate(env);
    let recipient = Address::generate(env);
    let (token, _) = create_token(env, &sponsor);
    mint_to(env, &token, &contract, 2_000 - claimed);
    let schedule = VestingSchedule {
        token: token.clone(), sponsor: sponsor.clone(), rate_per_ledger: 10 * RATE_DECIMALS,
        start_ledger: 100, cliff_ledger: 150, end_ledger: 300, last_claimed_ledger: 100,
        total_claimed: claimed, claimed_amount: claimed, metadata: None,
        paused_at_ledger: None, accumulated_pause_ledgers: 0, version: 1,
        schema_version: CURRENT_SCHEMA_VERSION,
    };
    env.as_contract(&contract, || {
        env.storage().persistent().set(&DataKey::Schedule(recipient.clone()), &schedule);
        env.storage().persistent().set(&DataKey::ScheduleById(recipient.clone(), 0), &schedule);
        env.storage().persistent().set(&DataKey::SponsorStreams(sponsor.clone()), &soroban_sdk::vec![env, recipient.clone()]);
    });
    (contract, sponsor, recipient, token)
}

#[test]
fn test_drain_cleanup_exact_year_event_and_no_auth() {
    use soroban_sdk::{testutils::Events, IntoVal};
    use crate::types::DataKey;
    let env = setup_env();
    let (contract, sponsor, recipient, token) = seed_cleanup_stream(&env, 500);
    let caller = Address::generate(&env);
    let client = VestingDripsClient::new(&env, &contract);
    assert_eq!(DRAIN_DELAY_LEDGERS, 6_307_200);
    env.mock_auths(&[]);
    env.ledger().set_sequence_number(300 + 6_307_200 - 1);
    assert_eq!(client.try_drain_expired_stream(&caller, &recipient), Err(Ok(VestingError::DrainDelayNotExpired)));
    env.ledger().set_sequence_number(300 + 6_307_200);
    client.drain_expired_stream(&caller, &recipient);
    assert!(env.auths().is_empty());
    let token_client = soroban_sdk::token::Client::new(&env, &token);
    assert_eq!(token_client.balance(&sponsor), 1_500);
    assert_eq!(token_client.balance(&contract), 0);
    assert_eq!(token_client.balance(&caller), 0);
    env.as_contract(&contract, || {
        assert!(!env.storage().persistent().has(&DataKey::Schedule(recipient.clone())));
        assert!(!env.storage().persistent().has(&DataKey::ScheduleById(recipient.clone(), 0)));
        assert!(!env.storage().persistent().has(&DataKey::SponsorStreams(sponsor.clone())));
    });
    let event = env.events().all().iter().find(|(id, topics, _)|
        *id == contract && topics.first() == Some(soroban_sdk::symbol_short!("vc_drain").into_val(&env))
    ).unwrap();
    assert_eq!(event.1, (soroban_sdk::symbol_short!("vc_drain"), recipient.clone()).into_val(&env));
    assert_eq!(event.2, (caller.clone(), sponsor, token, 1_500_i128, 300_u32 + 6_307_200).into_val(&env));
    assert_eq!(client.try_drain_expired_stream(&caller, &recipient), Err(Ok(VestingError::ScheduleNotFound)));
}

#[test]
fn test_drain_cleanup_zero_emits_without_token_transfer() {
    use soroban_sdk::{testutils::Events, IntoVal};
    let env = setup_env();
    let (contract, sponsor, recipient, token) = seed_cleanup_stream(&env, 2_000);
    let caller = Address::generate(&env);
    env.ledger().set_sequence_number(301 + 6_307_200);
    env.mock_auths(&[]);
    let client = VestingDripsClient::new(&env, &contract);
    client.drain_expired_stream(&caller, &recipient);
    let events = env.events().all();
    assert_eq!(events.len(), 1);
    assert_eq!(events.get(0).unwrap().2, (caller, sponsor, token, 0_i128, 301_u32 + 6_307_200).into_val(&env));
    env.as_contract(&contract, || assert!(!env.storage().persistent().has(&crate::types::DataKey::Schedule(recipient.clone()))));
}

#[test]
fn test_drain_cleanup_transfer_failure_keeps_schedule() {
    let env = setup_env();
    let (contract, sponsor, recipient, token) = seed_cleanup_stream(&env, 0);
    // Simulate an underfunded vault and check that the failed transaction rolls back removal.
    soroban_sdk::token::Client::new(&env, &token).transfer(&contract, &sponsor, &2_000);
    env.ledger().set_sequence_number(300 + 6_307_200);
    env.mock_auths(&[]);
    let client = VestingDripsClient::new(&env, &contract);
    assert_eq!(client.try_drain_expired_stream(&Address::generate(&env), &recipient), Err(Ok(VestingError::TransferFailed)));
    env.as_contract(&contract, || {
        assert!(env.storage().persistent().has(&crate::types::DataKey::Schedule(recipient.clone())));
        assert!(crate::storage::get_sponsor_streams(&env, &sponsor).contains(&recipient));
    });
}
