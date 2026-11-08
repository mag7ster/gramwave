use serde_json::{json, Value};
#[cfg(feature = "telegram")]
use std::{
    collections::HashMap,
    ffi::{CStr, CString},
    time::Instant,
};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};
#[derive(Clone)]
pub struct Telegram {
    send: mpsc::Sender<(Value, mpsc::Sender<Value>)>,
    serial: Arc<AtomicU64>,
}
impl Telegram {
    #[cfg(feature = "telegram")]
    pub fn start() -> Result<(Self, mpsc::Receiver<Value>), String> {
        unsafe extern "C" {
            fn td_send(client_id: i32, request: *const std::ffi::c_char);
            fn td_receive(timeout: f64) -> *const std::ffi::c_char;
            fn td_execute(request: *const std::ffi::c_char) -> *const std::ffi::c_char;
        }
        let (tx, rx) = mpsc::channel::<(Value, mpsc::Sender<Value>)>();
        let (updates, out) = mpsc::channel();
        std::thread::spawn(move || {
            unsafe {
                td_execute(
                    c"{\"@type\":\"setLogVerbosityLevel\",\"new_verbosity_level\":0}".as_ptr(),
                );
            }
            let id = tdlib_rs::create_client();
            let mut pending = HashMap::new();
            loop {
                while let Ok((v, reply)) = rx.try_recv() {
                    let key = v["@extra"].as_u64().unwrap();
                    let s = CString::new(v.to_string()).unwrap();
                    unsafe { td_send(id, s.as_ptr()) };
                    pending.insert(key, (Instant::now(), reply));
                }
                let ptr = unsafe { td_receive(0.05) };
                if !ptr.is_null() {
                    let s = unsafe { CStr::from_ptr(ptr) }.to_string_lossy();
                    if let Ok(v) = serde_json::from_str::<Value>(&s) {
                        if let Some(key) = v["@extra"].as_u64() {
                            if let Some((_, tx)) = pending.remove(&key) {
                                let _ = tx.send(v);
                            }
                        } else {
                            let closed =
                                v["authorization_state"]["@type"] == "authorizationStateClosed";
                            let _ = updates.send(v);
                            if closed {
                                break;
                            }
                        }
                    }
                }
                pending.retain(|_, (t, _)| t.elapsed() < Duration::from_secs(60));
            }
        });
        Ok((
            Self {
                send: tx,
                serial: Arc::new(AtomicU64::new(1)),
            },
            out,
        ))
    }
    #[cfg(not(feature = "telegram"))]
    pub fn start() -> Result<(Self, mpsc::Receiver<Value>), String> {
        Err("Build with the default telegram feature to connect".into())
    }
    pub fn request(&self, mut v: Value) -> mpsc::Receiver<Value> {
        if !privacy_safe(&v) {
            let (tx, rx) = mpsc::channel();
            let _=tx.send(json!({"@type":"error","message":"This action is disabled in the read-only player"}));
            return rx;
        }
        let (tx, rx) = mpsc::channel();
        v["@extra"] = json!(self.serial.fetch_add(1, Ordering::Relaxed));
        let _ = self.send.send((v, tx));
        rx
    }
    pub fn call(&self, v: Value) -> Result<Value, String> {
        let v = self
            .request(v)
            .recv_timeout(Duration::from_secs(60))
            .map_err(|e| e.to_string())?;
        if v["@type"] == "error" {
            Err(v["message"].as_str().unwrap_or("Telegram error").into())
        } else {
            Ok(v)
        }
    }
    pub fn offline(&self) {
        self.request(json!({"@type":"setOption","name":"online","value":{"@type":"optionValueBoolean","value":false}}));
    }
}

fn privacy_safe(v: &Value) -> bool {
    let method = v["@type"].as_str().unwrap_or("");
    !matches!(
        method,
        "viewMessages"
            | "openMessageContent"
            | "sendChatAction"
            | "sendMessage"
            | "deleteMessages"
            | "editMessageText"
    ) && !(method == "setOption" && v["name"] == "online" && v["value"]["value"] != false)
}
#[cfg(test)]
impl Telegram {
    pub fn mock_file(path: std::path::PathBuf) -> Self {
        let (send, rx) = mpsc::channel::<(Value, mpsc::Sender<Value>)>();
        std::thread::spawn(move || {
            while let Ok((v, reply)) = rx.recv() {
                let size = std::fs::metadata(&path).unwrap().len();
                let response = match v["@type"].as_str().unwrap_or("") {
                    "getFileDownloadedPrefixSize" => {
                        json!({"size":size.saturating_sub(v["offset"].as_u64().unwrap_or(0))})
                    }
                    _ => json!({"size":size,"local":{"path":path,"is_downloading_completed":true}}),
                };
                let _ = reply.send(response);
            }
        });
        Self {
            send,
            serial: Arc::new(AtomicU64::new(1)),
        }
    }
}

#[cfg(test)]
mod tests {
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
    }
}

#[cfg(all(test, feature = "telegram"))]
mod native_tests {
    #[test]
    fn native_tdlib_version() {
        let (tg, _) = super::Telegram::start().unwrap();
        let v = tg
            .call(serde_json::json!({"@type":"getOption","name":"version"}))
            .unwrap();
        assert_eq!(v["value"], "1.8.61");
        tg.call(serde_json::json!({"@type":"close"})).unwrap();
    }
}
