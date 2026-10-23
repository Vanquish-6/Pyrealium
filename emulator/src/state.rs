//! Deterministic lifecycle: I/O and wall clocks stay outside the state machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Connecting,
    CharacterSelection,
    Entering { character: u64 },
    InWorld { character: u64 },
    Closed,
}

#[derive(Clone, Debug)]
pub struct Session {
    pub account: String,
    pub phase: Phase,
    cookie: u64,
    deadline_ms: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum StateError {
    InvalidTransition,
    WrongCookie,
    NotOwner,
    BootstrapUnavailable,
    Expired,
}

impl Session {
    pub fn new(account: String, cookie: u64, now_ms: u64) -> Self {
        Self {
            account,
            phase: Phase::Connecting,
            cookie,
            deadline_ms: now_ms.saturating_add(30_000),
        }
    }
    pub fn connect_ack(&mut self, cookie: u64, now_ms: u64) -> Result<(), StateError> {
        if now_ms >= self.deadline_ms {
            return Err(StateError::Expired);
        }
        if cookie != self.cookie {
            return Err(StateError::WrongCookie);
        }
        match self.phase {
            Phase::Connecting => {
                self.phase = Phase::CharacterSelection;
                self.touch(now_ms);
                Ok(())
            }
            Phase::CharacterSelection => Ok(()), // UDP retry is idempotent.
            _ => Err(StateError::InvalidTransition),
        }
    }
    pub fn request_enter(
        &mut self,
        character: u64,
        owner: &str,
        bootstrap_available: bool,
    ) -> Result<(), StateError> {
        if owner != self.account {
            return Err(StateError::NotOwner);
        }
        if self.phase != Phase::CharacterSelection {
            return Err(StateError::InvalidTransition);
        }
        if !bootstrap_available {
            return Err(StateError::BootstrapUnavailable);
        }
        self.phase = Phase::Entering { character };
        Ok(())
    }
    pub fn scene_complete(&mut self) -> Result<(), StateError> {
        match self.phase {
            Phase::Entering { character } => {
                self.phase = Phase::InWorld { character };
                Ok(())
            }
            Phase::InWorld { .. } => Ok(()),
            _ => Err(StateError::InvalidTransition),
        }
    }
    pub fn touch(&mut self, now_ms: u64) {
        self.deadline_ms = now_ms.saturating_add(120_000);
    }
    pub fn leave_world(&mut self, character_id: u64) -> Result<(), StateError> {
        match self.phase {
            Phase::Entering { character } | Phase::InWorld { character }
                if character == character_id =>
            {
                self.phase = Phase::CharacterSelection;
                Ok(())
            }
            _ => Err(StateError::InvalidTransition),
        }
    }
    pub fn expired(&self, now_ms: u64) -> bool {
        now_ms >= self.deadline_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifecycle_requires_owner_and_real_bootstrap() {
        let mut s = Session::new("local".into(), 42, 0);
        assert_eq!(s.scene_complete(), Err(StateError::InvalidTransition));
        assert_eq!(s.connect_ack(7, 1), Err(StateError::WrongCookie));
        s.connect_ack(42, 1).unwrap();
        s.connect_ack(42, 2).unwrap();
        assert_eq!(s.request_enter(1, "other", true), Err(StateError::NotOwner));
        assert_eq!(
            s.request_enter(1, "local", false),
            Err(StateError::BootstrapUnavailable)
        );
        assert_eq!(s.phase, Phase::CharacterSelection);
        s.request_enter(1, "local", true).unwrap();
        assert_eq!(s.phase, Phase::Entering { character: 1 });
        s.scene_complete().unwrap();
        assert_eq!(s.phase, Phase::InWorld { character: 1 });
    }
    #[test]
    fn handshake_expires() {
        let mut s = Session::new("local".into(), 42, 0);
        assert_eq!(s.connect_ack(42, 30_000), Err(StateError::Expired));
    }
    #[test]
    fn logout_requires_active_identity_and_allows_reentry() {
        let mut s = Session::new("local".into(), 42, 0);
        s.connect_ack(42, 1).unwrap();
        assert!(s.leave_world(1).is_err());
        s.request_enter(1, "local", true).unwrap();
        assert!(s.leave_world(2).is_err());
        s.scene_complete().unwrap();
        s.leave_world(1).unwrap();
        assert_eq!(s.phase, Phase::CharacterSelection);
        s.request_enter(1, "local", true).unwrap();
    }
}
