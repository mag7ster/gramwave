use super::*;
#[cfg(test)]
mod regression {
    use super::*;
    #[test]
    fn qr_can_require_password() {
        assert_eq!(
            phase(true, &AuthorizationState::Password(String::new())),
            Phase::Password
        );
    }
}
