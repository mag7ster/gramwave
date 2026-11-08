use super::*;
#[cfg(test)]
mod regression {
    use super::*;
    #[test]
    fn credential_validation() {
        assert!(credentials("123", "0123456789abcdef0123456789abcdef"));
        assert!(!credentials("0", "0123456789abcdef0123456789abcdef"));
        assert!(!credentials("123", "wrong"));
    }
    #[test]
    fn qr_can_require_password() {
        assert_eq!(
            phase(true, &json!({"@type":"authorizationStateWaitPassword"})),
            Phase::Password
        );
    }
}
