#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, contracterror,
    Address, Env, symbol_short,
};
use soroban_sdk::token;

const DAY_IN_LEDGERS: u32 = 17280;
const INSTANCE_TTL: u32 = 7 * DAY_IN_LEDGERS;
const INSTANCE_THRESHOLD: u32 = 6 * DAY_IN_LEDGERS;
const PERSISTENT_TTL: u32 = 30 * DAY_IN_LEDGERS;
const PERSISTENT_THRESHOLD: u32 = 29 * DAY_IN_LEDGERS;

#[contracttype]
pub enum DataKey {
    Admin,
    TokenAddress,
    Session(u64),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionStatus {
    Created,
    StudentConfirmed,
    TutorConfirmed,
    Paid,
    Cancelled,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Session {
    pub id: u64,
    pub student: Address,
    pub tutor: Address,
    pub amount: i128,
    pub status: SessionStatus,
    pub student_confirmed: bool,
    pub tutor_confirmed: bool,
    pub paid: bool,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ContractError {
    NotAuthorized = 1,
    NotFound = 2,
    AlreadyExists = 3,
    InvalidInput = 4,
    TransferFailed = 5,
    AlreadyFinalized = 6,
}

#[contract]
pub struct TutoringEscrow;

#[contractimpl]
impl TutoringEscrow {
    pub fn initialize(env: Env, admin: Address, token: Address) {
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::TokenAddress, &token);
        env.storage().instance().extend_ttl(INSTANCE_THRESHOLD, INSTANCE_TTL);
    }

    /// Student creates and funds a session escrow entry. This assumes the tokens
    /// have been transferred to the contract account by the student beforehand.
    pub fn create_session(
        env: Env,
        session_id: u64,
        student: Address,
        tutor: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        student.require_auth();
        if amount <= 0 { return Err(ContractError::InvalidInput); }
        if env.storage().persistent().has(&DataKey::Session(session_id)) {
            return Err(ContractError::AlreadyExists);
        }
        let s = Session {
            id: session_id,
            student: student.clone(),
            tutor: tutor.clone(),
            amount,
            status: SessionStatus::Created,
            student_confirmed: false,
            tutor_confirmed: false,
            paid: false,
        };
        env.storage().persistent().set(&DataKey::Session(session_id), &s);
        env.storage().persistent().extend_ttl(&DataKey::Session(session_id), PERSISTENT_THRESHOLD, PERSISTENT_TTL);
        env.events().publish((symbol_short!("s_create"),), session_id);
        Ok(())
    }

    pub fn confirm_by_student(env: Env, session_id: u64, student: Address) -> Result<(), ContractError> {
        student.require_auth();
        let mut s: Session = env.storage().persistent().get(&DataKey::Session(session_id)).ok_or(ContractError::NotFound)?;
        if s.paid || matches!(s.status, SessionStatus::Cancelled | SessionStatus::Paid) {
            return Err(ContractError::AlreadyFinalized);
        }
        if s.student != student { return Err(ContractError::NotAuthorized); }
        s.student_confirmed = true;
        s.status = if s.tutor_confirmed { SessionStatus::Paid } else { SessionStatus::StudentConfirmed };
        env.storage().persistent().set(&DataKey::Session(session_id), &s);
        env.events().publish((symbol_short!("s_confirm"),), session_id);
        if s.student_confirmed && s.tutor_confirmed {
            Self::release_payment(env, session_id, s)?;
        }
        Ok(())
    }

    pub fn confirm_by_tutor(env: Env, session_id: u64, tutor: Address) -> Result<(), ContractError> {
        tutor.require_auth();
        let mut s: Session = env.storage().persistent().get(&DataKey::Session(session_id)).ok_or(ContractError::NotFound)?;
        if s.paid || matches!(s.status, SessionStatus::Cancelled | SessionStatus::Paid) {
            return Err(ContractError::AlreadyFinalized);
        }
        if s.tutor != tutor { return Err(ContractError::NotAuthorized); }
        s.tutor_confirmed = true;
        s.status = if s.student_confirmed { SessionStatus::Paid } else { SessionStatus::TutorConfirmed };
        env.storage().persistent().set(&DataKey::Session(session_id), &s);
        env.events().publish((symbol_short!("s_confirm"),), session_id);
        if s.student_confirmed && s.tutor_confirmed {
            Self::release_payment(env, session_id, s)?;
        }
        Ok(())
    }

    pub fn cancel_session(env: Env, session_id: u64, actor: Address) -> Result<(), ContractError> {
        actor.require_auth();
        let mut s: Session = env.storage().persistent().get(&DataKey::Session(session_id)).ok_or(ContractError::NotFound)?;
        if s.paid {
            return Err(ContractError::AlreadyFinalized);
        }
        if actor != s.student && actor != s.tutor && actor != env.storage().instance().get(&DataKey::Admin).ok_or(ContractError::NotFound)? {
            return Err(ContractError::NotAuthorized);
        }
        s.status = SessionStatus::Cancelled;
        env.storage().persistent().set(&DataKey::Session(session_id), &s);
        env.events().publish((symbol_short!("s_cancel"),), session_id);
        Ok(())
    }

    fn release_payment(env: Env, id: u64, mut s: Session) -> Result<(), ContractError> {
        // Transfer tokens from this contract's account to the tutor.
        let token_addr: Address = env.storage().instance().get(&DataKey::TokenAddress).ok_or(ContractError::InvalidInput)?;
        let client = token::Client::new(&env, &token_addr);
        let from = env.current_contract_address();
        // Best-effort transfer; if it fails, return an error
        client.transfer(&from, &s.tutor, &s.amount);
        s.paid = true;
        s.status = SessionStatus::Paid;
        env.storage().persistent().set(&DataKey::Session(id), &s);
        env.events().publish((symbol_short!("s_paid"),), id);
        Ok(())
    }

    pub fn get_session_status(env: Env, session_id: u64) -> SessionStatus {
        env.storage().persistent().get::<DataKey, Session>(&DataKey::Session(session_id)).unwrap().status
    }

    pub fn get_session(env: Env, id: u64) -> Session {
        env.storage().persistent().get(&DataKey::Session(id)).unwrap()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    #[contract]
    pub struct DummyToken;

    #[contractimpl]
    impl DummyToken {
        pub fn transfer(_env: Env, _from: Address, _to: Address, _amount: i128) {
            // no-op for tests
        }
    }

    #[test]
    fn test_session_lifecycle() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        // register a dummy token contract for testing
        let token_contract_id = env.register(DummyToken, ());
        let token = token_contract_id.clone();

        let contract_id = env.register(TutoringEscrow, ());
        let client = TutoringEscrowClient::new(&env, &contract_id);

        client.initialize(&admin, &token);

        let student = Address::generate(&env);
        let tutor = Address::generate(&env);
        let session_id = 1u64;

        // Student creates session (assumes tokens are pre-funded to contract in real flow)
        client.create_session(&session_id, &student, &tutor, &100);

        // Confirmations
        client.confirm_by_student(&session_id, &student);
        client.confirm_by_tutor(&session_id, &tutor);

        let s = client.get_session(&session_id);
        assert!(s.paid);
        assert_eq!(s.status, SessionStatus::Paid);
    }

    #[test]
    fn test_cancel_session() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let token_contract_id = env.register(DummyToken, ());
        let token = token_contract_id.clone();
        let contract_id = env.register(TutoringEscrow, ());
        let client = TutoringEscrowClient::new(&env, &contract_id);

        client.initialize(&admin, &token);

        let student = Address::generate(&env);
        let tutor = Address::generate(&env);
        let session_id = 2u64;

        client.create_session(&session_id, &student, &tutor, &75);
        client.cancel_session(&session_id, &student);

        let s = client.get_session(&session_id);
        assert_eq!(s.status, SessionStatus::Cancelled);
    }
}
