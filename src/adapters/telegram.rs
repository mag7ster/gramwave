use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tg_player_application::{
    telegram::{AuthRequest, GatewayFuture, MediaPage, TelegramGateway, TelegramUpdate},
    AppError,
};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

struct Pending {
    deadline: Instant,
    reply: oneshot::Sender<Value>,
}
type Registry = Arc<Mutex<HashMap<u64, Pending>>>;
struct Registration {
    deadline: Instant,
    id: u64,
    pending: Registry,
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.pending.lock().unwrap().remove(&self.id);
    }
}
type Resolutions = Arc<
    Mutex<
        HashMap<
            crate::domain::TrackId,
            Arc<tokio::sync::OnceCell<tg_player_application::media::MediaTrack>>,
        >,
    >,
>;
#[derive(Clone)]
pub struct Telegram {
    send: mpsc::Sender<Value>,
    pending: Registry,
    serial: Arc<AtomicU64>,
    stopping: Arc<AtomicBool>,
    worker: Arc<Mutex<Option<std::thread::JoinHandle<()>>>>,
    downloads: Arc<Mutex<HashMap<i64, DownloadEntry>>>,
    resolved: Resolutions,
    pub connection: u64,
}
impl Telegram {
    #[cfg(feature = "telegram")]
    pub fn start() -> Result<(Self, mpsc::Receiver<TelegramUpdate>), String> {
        use std::ffi::{CStr, CString};
        unsafe extern "C" {
            fn td_send(client_id: i32, request: *const std::ffi::c_char);
            fn td_receive(timeout: f64) -> *const std::ffi::c_char;
            fn td_execute(request: *const std::ffi::c_char) -> *const std::ffi::c_char;
        }
        static CONNECTION: AtomicU64 = AtomicU64::new(1);
        let connection = CONNECTION.fetch_add(1, Ordering::Relaxed);
        let (send, rx) = mpsc::channel::<Value>();
        let (updates, out) = mpsc::channel();
        let pending = Registry::default();
        let registry = pending.clone();
        let resolved: Resolutions = Default::default();
        let resolved_updates = resolved.clone();
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = stopping.clone();
        let worker = std::thread::spawn(move || {
            unsafe {
                td_execute(
                    c"{\"@type\":\"setLogVerbosityLevel\",\"new_verbosity_level\":0}".as_ptr(),
                );
            }
            let id = tdlib_rs::create_client();
            let mut closing = None;
            loop {
                loop {
                    match rx.try_recv() {
                        Ok(v) => {
                            // Cancelled requests can be dropped before touching TDLib.
                            if v["@extra"]
                                .as_u64()
                                .is_some_and(|key| !registry.lock().unwrap().contains_key(&key))
                            {
                                continue;
                            }
                            if let Ok(s) = CString::new(v.to_string()) {
                                unsafe {
                                    td_send(id, s.as_ptr());
                                }
                            }
                        }
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            stop.store(true, Ordering::Release);
                            break;
                        }
                    }
                }
                if stop.load(Ordering::Acquire) && closing.is_none() {
                    unsafe {
                        td_send(id, c"{\"@type\":\"close\"}".as_ptr());
                    }
                    closing = Some(Instant::now());
                }
                let ptr = unsafe { td_receive(0.05) };
                if !ptr.is_null() {
                    let s = unsafe { CStr::from_ptr(ptr) }.to_string_lossy();
                    if let Ok(v) = serde_json::from_str::<Value>(&s) {
                        if v["@client_id"]
                            .as_i64()
                            .is_some_and(|client| client != i64::from(id))
                        {
                            continue;
                        }
                        if let Some(key) = v["@extra"].as_u64() {
                            if let Some(pending) = registry.lock().unwrap().remove(&key) {
                                let _ = pending.reply.send(v);
                            }
                        } else {
                            let closed =
                                v["authorization_state"]["@type"] == "authorizationStateClosed";
                            if let Some(update) = super::telegram_mapping::update(v) {
                                match &update {
                                    TelegramUpdate::Message { id, .. } => {
                                        resolved_updates.lock().unwrap().remove(id);
                                    }
                                    TelegramUpdate::Deleted {
                                        chat,
                                        messages,
                                        from_cache: false,
                                    } => {
                                        for message in messages {
                                            resolved_updates.lock().unwrap().remove(
                                                &crate::domain::TrackId {
                                                    chat: *chat,
                                                    message: *message,
                                                },
                                            );
                                        }
                                    }
                                    _ => {}
                                }
                                let _ = updates.send(update);
                            }
                            if closed {
                                break;
                            }
                        }
                    }
                }
                registry.lock().unwrap().retain(|_, pending| {
                    !pending.reply.is_closed() && Instant::now() < pending.deadline
                });
                if closing.is_some_and(|t| t.elapsed() >= Duration::from_secs(2)) {
                    break;
                }
            }
            registry.lock().unwrap().clear();
        });
        Ok((
            Self {
                send,
                pending,
                serial: Arc::new(AtomicU64::new(1)),
                stopping,
                worker: Arc::new(Mutex::new(Some(worker))),
                downloads: Default::default(),
                resolved,
                connection,
            },
            out,
        ))
    }
    #[cfg(not(feature = "telegram"))]
    pub fn start() -> Result<(Self, mpsc::Receiver<TelegramUpdate>), String> {
        Err("Build with the default telegram feature to connect".into())
    }
    fn request(&self, v: Value) {
        if privacy_safe(&v) && !self.stopping.load(Ordering::Acquire) {
            let _ = self.send.send(v);
        }
    }
    fn register(
        &self,
        mut v: Value,
        timeout: Duration,
    ) -> Result<(Registration, oneshot::Receiver<Value>), AppError> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(AppError::Cancelled);
        }
        if !privacy_safe(&v) {
            return Err(AppError::Telegram {
                code: 403,
                message: "This action is disabled in the read-only player".into(),
            });
        }
        let id = self.serial.fetch_add(1, Ordering::Relaxed);
        v["@extra"] = json!(id);
        let (reply, rx) = oneshot::channel();
        let deadline = Instant::now() + timeout;
        let guard = Registration {
            deadline,
            id,
            pending: self.pending.clone(),
        };
        self.pending
            .lock()
            .unwrap()
            .insert(id, Pending { deadline, reply });
        self.send
            .send(v)
            .map_err(|_| AppError::Unavailable("Telegram"))?;
        Ok((guard, rx))
    }
    fn response(v: Value) -> Result<Value, AppError> {
        if v["@type"] == "error" {
            Err(AppError::Telegram {
                code: v["code"].as_i64().unwrap_or(500) as i32,
                message: v["message"].as_str().unwrap_or("Telegram error").into(),
            })
        } else {
            Ok(v)
        }
    }
    pub(super) async fn call_async(
        &self,
        v: Value,
        cancel: &CancellationToken,
    ) -> Result<Value, AppError> {
        self.call_with_timeout(v, cancel, Duration::from_secs(60))
            .await
    }
    async fn call_with_timeout(
        &self,
        v: Value,
        cancel: &CancellationToken,
        timeout: Duration,
    ) -> Result<Value, AppError> {
        let (guard, rx) = self.register(v, timeout)?;
        tokio::select! {
            biased;
            _ = cancel.cancelled()=>Err(AppError::Cancelled),
            result=tokio::time::timeout_at(tokio::time::Instant::from_std(guard.deadline),rx)=> {
                let value=result.map_err(|_|AppError::Timeout("Telegram RPC"))?.map_err(|_|if Instant::now()>=guard.deadline {AppError::Timeout("Telegram RPC")}else{AppError::Unavailable("Telegram")})?;
                Self::response(value)
            }
        }
    }
    pub fn shutdown(&self, timeout: Duration) -> Result<(), String> {
        self.stopping.store(true, Ordering::Release);
        for (_, pending) in self.pending.lock().unwrap().drain() {
            let _ = pending.deadline;
            drop(pending.reply);
        }
        let deadline = Instant::now() + timeout;
        let mut worker = self.worker.lock().map_err(|e| e.to_string())?;
        if let Some(handle) = worker.as_ref() {
            while !handle.is_finished() {
                if Instant::now() >= deadline {
                    return Err("Telegram shutdown timed out".into());
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        if let Some(handle) = worker.take() {
            handle
                .join()
                .map_err(|_| "Telegram worker panicked".to_owned())?;
        }
        Ok(())
    }
    pub(crate) async fn chat_url(
        &self,
        chat: i64,
        cancel: &CancellationToken,
    ) -> Result<String, AppError> {
        let chat = self
            .call_async(json!({"@type":"getChat","chat_id":chat}), cancel)
            .await?;
        let ty = &chat["type"];
        let peer = match ty["@type"].as_str() {
            Some("chatTypeSupergroup") => {
                self.call_async(
                    json!({"@type":"getSupergroup","supergroup_id":ty["supergroup_id"]}),
                    cancel,
                )
                .await?
            }
            Some("chatTypePrivate") => {
                self.call_async(json!({"@type":"getUser","user_id":ty["user_id"]}), cancel)
                    .await?
            }
            _ => return Ok(String::new()),
        };
        Ok(peer["usernames"]["active_usernames"][0]
            .as_str()
            .map(|s| format!("https://t.me/{s}"))
            .unwrap_or_default())
    }
    pub(crate) async fn artwork(
        &self,
        file: i64,
        cancel: &CancellationToken,
    ) -> Result<String, AppError> {
        let value=self.call_async(json!({"@type":"downloadFile","file_id":file,"priority":1,"offset":0,"limit":0,"synchronous":true}),cancel).await?;
        let file = super::telegram_mapping::file_state(&value);
        if file.complete {
            file.path.ok_or(AppError::Unavailable("Artwork"))
        } else {
            Err(AppError::Unavailable("Artwork"))
        }
    }
    pub(crate) async fn begin_download(
        &self,
        file: i64,
        offset: u64,
        full: bool,
        cancel: &CancellationToken,
    ) -> Result<DownloadLease, AppError> {
        let gate = {
            let mut downloads = self.downloads.lock().unwrap();
            let entry = downloads.entry(file).or_default();
            entry.users += 1;
            if full {
                entry.full += 1;
            } else {
                *entry.streams.entry(offset).or_default() += 1;
            }
            entry.gate.clone()
        };
        let lease = DownloadLease {
            tg: self.clone(),
            file,
            full,
            offset,
        };
        let _gate = tokio::select! {_=cancel.cancelled()=>return Err(AppError::Cancelled),lock=gate.lock()=>lock};
        let request = {
            let downloads = self.downloads.lock().unwrap();
            let entry = &downloads[&file];
            let desired = if !full {
                offset
            } else {
                entry.streams.keys().next_back().copied().unwrap_or(0)
            };
            (!entry.started || desired != entry.offset).then_some(desired)
        };
        if let Some(offset) = request {
            self.call_async(json!({"@type":"downloadFile","file_id":file,"priority":if full {8}else{32},"offset":offset,"limit":0,"synchronous":false}),cancel).await?;
            if let Some(entry) = self.downloads.lock().unwrap().get_mut(&file) {
                entry.started = true;
                entry.offset = offset;
            }
        }
        Ok(lease)
    }
    pub(crate) fn configure_cache(&self) {
        self.request(json!({"@type":"setLogVerbosityLevel","new_verbosity_level":0}));
        self.request(json!({"@type":"setOption","name":"use_storage_optimizer","value":{"@type":"optionValueBoolean","value":true}}));
        self.request(json!({"@type":"setOption","name":"storage_max_files_size","value":{"@type":"optionValueInteger","value":2147483648i64}}));
    }
    pub(crate) fn prefetch(&self, file: i64) {
        self.request(json!({"@type":"downloadFile","file_id":file,"priority":1,"offset":0,"limit":1048576,"synchronous":false}));
    }
    pub(crate) async fn message_link(
        &self,
        chat: i64,
        message: i64,
        cancel: &CancellationToken,
    ) -> Result<String, AppError> {
        let value=self.call_async(json!({"@type":"getMessageLink","chat_id":chat,"message_id":message,"media_timestamp":0,"for_album":false,"in_message_thread":false}),cancel).await?;
        value["link"]
            .as_str()
            .map(str::to_owned)
            .ok_or(AppError::Unavailable("Message link"))
    }
    pub(crate) async fn clear_cache(&self, cancel: &CancellationToken) -> Result<(), AppError> {
        self.call_async(json!({"@type":"optimizeStorage","size":0,"ttl":0,"count":0,"immunity_delay":0,"file_types":[],"chat_ids":[],"exclude_chat_ids":[],"return_deleted_file_statistics":true,"chat_limit":0}),cancel).await.map(|_|())
    }
    pub fn offline(&self) {
        self.request(json!({"@type":"setOption","name":"online","value":{"@type":"optionValueBoolean","value":false}}));
    }
}
#[derive(Default)]
struct DownloadEntry {
    streams: std::collections::BTreeMap<u64, usize>,
    users: usize,
    full: usize,
    started: bool,
    offset: u64,
    gate: Arc<tokio::sync::Mutex<()>>,
}
pub(crate) struct DownloadLease {
    offset: u64,
    tg: Telegram,
    file: i64,
    full: bool,
}
impl Drop for DownloadLease {
    fn drop(&mut self) {
        let action = {
            let mut downloads = self.tg.downloads.lock().unwrap();
            if let Some(entry) = downloads.get_mut(&self.file) {
                entry.users -= 1;
                if self.full {
                    entry.full -= 1;
                } else if let Some(count) = entry.streams.get_mut(&self.offset) {
                    *count -= 1;
                    if *count == 0 {
                        entry.streams.remove(&self.offset);
                    }
                }
                if entry.users == 0 {
                    downloads.remove(&self.file);
                    Some(None)
                } else {
                    let desired = if entry.streams.contains_key(&entry.offset) {
                        entry.offset
                    } else {
                        entry.streams.keys().next_back().copied().unwrap_or(0)
                    };
                    if entry.started && desired != entry.offset {
                        entry.offset = desired;
                        Some(Some((
                            desired,
                            if entry.streams.is_empty() { 8 } else { 32 },
                        )))
                    } else {
                        None
                    }
                }
            } else {
                None
            }
        };
        match action {
            Some(None)=>self.tg.request(json!({"@type":"cancelDownloadFile","file_id":self.file,"only_if_pending":false})),
            Some(Some((offset,priority)))=>self.tg.request(json!({"@type":"downloadFile","file_id":self.file,"priority":priority,"offset":offset,"limit":0,"synchronous":false})),
            None=>{}
        }
    }
}

fn privacy_safe(v: &Value) -> bool {
    let method = v["@type"].as_str().unwrap_or("");
    matches!(
        method,
        "getChat"
            | "getChats"
            | "loadChats"
            | "getMessage"
            | "getMessages"
            | "searchChatMessages"
            | "searchChats"
            | "searchPublicChats"
            | "searchChatsOnServer"
            | "getFile"
            | "getFileDownloadedPrefixSize"
            | "downloadFile"
            | "cancelDownloadFile"
            | "getMessageLink"
            | "getSupergroup"
            | "getUser"
            | "getOption"
            | "optimizeStorage"
            | "close"
            | "setLogVerbosityLevel"
            | "setTdlibParameters"
            | "requestQrCodeAuthentication"
            | "checkAuthenticationCode"
            | "checkAuthenticationPassword"
            | "checkAuthenticationEmailCode"
            | "setAuthenticationEmailAddress"
            | "setAuthenticationPhoneNumber"
            | "resendAuthenticationCode"
    ) || (method == "setOption"
        && matches!(
            v["name"].as_str(),
            Some("use_storage_optimizer" | "storage_max_files_size")
        ))
        || (method == "setOption" && v["name"] == "online" && v["value"]["value"] == false)
}

#[cfg(test)]
impl Telegram {
    pub fn mock_rpc(mut handler: impl FnMut(Value) -> Value + Send + 'static) -> Self {
        let (send, rx) = mpsc::channel::<Value>();
        let pending = Registry::default();
        let registry = pending.clone();
        std::thread::spawn(move || {
            while let Ok(v) = rx.recv() {
                let id = v["@extra"].as_u64();
                let response = handler(v);
                if let Some(pending) = id.and_then(|id| registry.lock().unwrap().remove(&id)) {
                    let _ = pending.reply.send(response);
                }
            }
        });
        Self {
            send,
            pending,
            serial: Arc::new(AtomicU64::new(1)),
            stopping: Default::default(),
            worker: Default::default(),
            connection: 1,
            downloads: Default::default(),
            resolved: Default::default(),
        }
    }
    #[cfg(feature = "native-tests")]
    pub fn mock_file(path: std::path::PathBuf) -> Self {
        Self::mock_rpc(move |v| {
            let size = std::fs::metadata(&path).unwrap().len();
            match v["@type"].as_str().unwrap_or("") {
                "getFileDownloadedPrefixSize" => {
                    json!({"size":size.saturating_sub(v["offset"].as_u64().unwrap_or(0))})
                }
                _ => json!({"size":size,"local":{"path":path,"is_downloading_completed":true}}),
            }
        })
    }
}
#[cfg(test)]
#[path = "telegram/tests.rs"]
mod tests;

impl TelegramGateway for Telegram {
    fn search_media<'a>(
        &'a self,
        chat: i64,
        kind: crate::domain::MediaKind,
        cursor: i64,
        cancel: &'a CancellationToken,
    ) -> GatewayFuture<'a, MediaPage> {
        Box::pin(async move {
            #[cfg(feature = "telegram")]
            let filter = serde_json::to_value(match kind {
                crate::domain::MediaKind::Audio => tdlib_rs::enums::SearchMessagesFilter::Audio,
                crate::domain::MediaKind::Document => {
                    tdlib_rs::enums::SearchMessagesFilter::Document
                }
                crate::domain::MediaKind::Voice => tdlib_rs::enums::SearchMessagesFilter::VoiceNote,
            })
            .map_err(|e| AppError::Telegram {
                code: 500,
                message: e.to_string(),
            })?;
            #[cfg(not(feature = "telegram"))]
            let filter = json!({"@type":(["searchMessagesFilterAudio","searchMessagesFilterDocument","searchMessagesFilterVoiceNote"][usize::from(kind)])});
            let value=self.call_async(json!({"@type":"searchChatMessages","chat_id":chat,"topic_id":null,"query":"","sender_id":null,"from_message_id":cursor,"offset":0,"limit":100,"filter":filter}),cancel).await?;
            let next = value["next_from_message_id"]
                .as_i64()
                .ok_or_else(|| AppError::Protocol("Invalid search cursor".into()))?;
            if next != 0 && next == cursor {
                return Err(AppError::Protocol("Search cursor did not advance".into()));
            }
            let messages = value["messages"]
                .as_array()
                .ok_or_else(|| AppError::Protocol("Invalid search response".into()))?;
            let newest = messages.iter().try_fold(0, |newest, message| {
                message["id"]
                    .as_i64()
                    .map(|id| newest.max(id))
                    .ok_or_else(|| AppError::Protocol("Invalid search message identity".into()))
            })?;
            Ok(MediaPage {
                newest,
                tracks: messages
                    .iter()
                    .filter_map(|m| super::telegram_mapping::track(chat, m))
                    .collect(),
                next,
            })
        })
    }
    fn message<'a>(
        &'a self,
        id: crate::domain::TrackId,
        cancel: &'a CancellationToken,
    ) -> GatewayFuture<'a, tg_player_application::media::MediaTrack> {
        Box::pin(async move {
            let cell = self.resolved.lock().unwrap().entry(id).or_default().clone();
            cell.get_or_try_init(|| async {
                let value = self
                    .call_async(
                        json!({"@type":"getMessage","chat_id":id.chat,"message_id":id.message}),
                        cancel,
                    )
                    .await?;
                super::telegram_mapping::track(id.chat, &value)
                    .filter(|t| t.id() == id && t.file > 0)
                    .ok_or(AppError::Telegram {
                        code: 404,
                        message: crate::tr("This message no longer contains playable audio"),
                    })
            })
            .await
            .cloned()
        })
    }

    fn file<'a>(
        &'a self,
        id: i64,
        cancel: &'a CancellationToken,
    ) -> GatewayFuture<'a, tg_player_application::telegram::TelegramFile> {
        Box::pin(async move {
            let file = self
                .call_async(json!({"@type":"getFile","file_id":id}), cancel)
                .await?;
            Ok(super::telegram_mapping::file_state(&file))
        })
    }
    fn authenticate<'a>(
        &'a self,
        request: AuthRequest,
        cancel: &'a CancellationToken,
    ) -> GatewayFuture<'a, ()> {
        Box::pin(async move {
            let value = match request {
                AuthRequest::Qr => {
                    json!({"@type":"requestQrCodeAuthentication","other_user_ids":[]})
                }
                AuthRequest::Phone(phone) => {
                    json!({"@type":"setAuthenticationPhoneNumber","phone_number":phone,"settings":null})
                }
                AuthRequest::Code(code) => json!({"@type":"checkAuthenticationCode","code":code}),
                AuthRequest::EmailCode(code) => {
                    json!({"@type":"checkAuthenticationEmailCode","code":{"@type":"emailAddressAuthenticationCode","code":code}})
                }
                AuthRequest::Password(password) => {
                    json!({"@type":"checkAuthenticationPassword","password":password})
                }
                AuthRequest::Email(email) => {
                    json!({"@type":"setAuthenticationEmailAddress","email_address":email})
                }
                AuthRequest::Resend => json!({"@type":"resendAuthenticationCode","reason":null}),
                AuthRequest::Close => json!({"@type":"close"}),
                AuthRequest::Parameters(p) => {
                    json!({"@type":"setTdlibParameters","database_directory":p.database,"files_directory":p.files,"use_message_database":true,"use_file_database":true,"use_chat_info_database":true,"use_secret_chats":false,"api_id":p.api_id,"api_hash":p.api_hash,"system_language_code":"en","device_model":"Gramwave","system_version":"Linux","application_version":"0.1.0"})
                }
            };
            self.call_async(value, cancel).await.map(|_| ())
        })
    }
}
