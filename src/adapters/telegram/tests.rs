use super::*;
#[cfg(test)]
mod regression {
    use super::*;
    #[test]
    fn read_only_presence_policy() {
        assert!(!privacy_safe(
            &json!({"@type":"setOption","name":"online","value":{"@type":"optionValueEmpty"}})
        ));
        assert!(!privacy_safe(
            &json!({"@type":"setOption","name":"online","value":{"value":true}})
        ));
        assert!(privacy_safe(
            &json!({"@type":"setOption","name":"online","value":{"value":false}})
        ));
        for method in [
            "viewMessages",
            "openMessageContent",
            "sendChatAction",
            "sendMessage",
        ] {
            assert!(!privacy_safe(&json!({"@type":method})));
        }
        assert!(privacy_safe(&json!({"@type":"downloadFile"})));
        assert!(!privacy_safe(&json!({"@type":"unknownFutureMethod"})));
        assert!(!privacy_safe(
            &json!({"@type":"setOption","name":"unknown","value":{"value":false}})
        ));
    }
}

#[cfg(all(test, feature = "telegram", feature = "native-tests"))]
mod native_tests {
    #[test]
    fn native_tdlib_version() {
        let (tg, _) = super::Telegram::start().unwrap();
        let v = tg
            .call(serde_json::json!({"@type":"getOption","name":"version"}))
            .unwrap();
        assert_eq!(v["value"], "1.8.61");
        tg.shutdown(std::time::Duration::from_secs(5)).unwrap();
    }
}
