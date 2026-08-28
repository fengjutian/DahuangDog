use serde::Serialize;
use std::{
    collections::HashMap,
    mem::{size_of, take},
    ptr::null,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
};

#[derive(Clone, Copy, Debug, Default)]
pub struct NetworkRate {
    pub receive_bps: u64,
    pub send_bps: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectorStatus {
    pub state: String,
    pub message: String,
    pub events_lost: u64,
}

impl CollectorStatus {
    fn disabled() -> Self {
        Self { state: "disabled".into(), message: "应用网络监控尚未启用".into(), events_lost: 0 }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows_sys::{
        Win32::{
            Foundation::{ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_SUCCESS},
            System::Diagnostics::Etw::{
                CloseTrace, ControlTraceW, OpenTraceW, ProcessTrace, StartTraceW,
                TdhGetProperty, TdhGetPropertySize, CONTROLTRACE_HANDLE, EVENT_RECORD,
                EVENT_TRACE_CONTROL_STOP, EVENT_TRACE_FLAG_NETWORK_TCPIP,
                EVENT_TRACE_LOGFILEW, EVENT_TRACE_PROPERTIES, EVENT_TRACE_REAL_TIME_MODE,
                PROCESS_TRACE_MODE_EVENT_RECORD, PROCESS_TRACE_MODE_REAL_TIME,
                PROCESSTRACE_HANDLE, PROPERTY_DATA_DESCRIPTOR, SystemTraceControlGuid,
                TcpIpGuid, WNODE_FLAG_TRACED_GUID,
            },
        },
    };

    const SESSION_NAME: &str = "DahuangDog.NetworkTrace";
    const LOGGER_CAPACITY: usize = 64;

    #[derive(Default, Clone, Copy)]
    struct ByteCount { receive: u64, send: u64 }

    pub(super) struct Inner {
        counters: Mutex<HashMap<u32, ByteCount>>,
        status: Mutex<CollectorStatus>,
        stop: AtomicBool,
        session_handle: AtomicU64,
        consumer_handle: AtomicU64,
    }

    impl Inner {
        fn new() -> Self {
            Self {
                counters: Mutex::new(HashMap::new()),
                status: Mutex::new(CollectorStatus::disabled()),
                stop: AtomicBool::new(false),
                session_handle: AtomicU64::new(0),
                consumer_handle: AtomicU64::new(u64::MAX),
            }
        }

        fn set_status(&self, state: &str, message: impl Into<String>) {
            if let Ok(mut status) = self.status.lock() {
                status.state = state.into();
                status.message = message.into();
            }
        }
    }

    #[repr(C)]
    struct PropertiesBuffer {
        properties: EVENT_TRACE_PROPERTIES,
        logger_name: [u16; LOGGER_CAPACITY],
    }

    impl PropertiesBuffer {
        fn new() -> Self {
            let mut value: Self = unsafe { std::mem::zeroed() };
            let name = SESSION_NAME.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
            value.logger_name[..name.len()].copy_from_slice(&name);
            value.properties.Wnode.BufferSize = size_of::<Self>() as u32;
            value.properties.Wnode.Guid = SystemTraceControlGuid;
            value.properties.Wnode.ClientContext = 1;
            value.properties.Wnode.Flags = WNODE_FLAG_TRACED_GUID;
            value.properties.LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
            value.properties.EnableFlags = EVENT_TRACE_FLAG_NETWORK_TCPIP;
            value.properties.LoggerNameOffset = size_of::<EVENT_TRACE_PROPERTIES>() as u32;
            value.properties.LogFileNameOffset = 0;
            value
        }
    }

    fn wide(value: &str) -> Vec<u16> { value.encode_utf16().chain(Some(0)).collect() }

    unsafe fn property_bytes(event: *const EVENT_RECORD, candidates: &[&str]) -> Option<Vec<u8>> {
        for candidate in candidates {
            let name = wide(candidate);
            let descriptor = PROPERTY_DATA_DESCRIPTOR {
                PropertyName: name.as_ptr() as u64,
                ArrayIndex: u32::MAX,
                Reserved: 0,
            };
            let mut size = 0;
            if unsafe { TdhGetPropertySize(event, 0, null(), 1, &descriptor, &mut size) } != ERROR_SUCCESS
                || size == 0 || size > 16
            {
                continue;
            }
            let mut buffer = vec![0_u8; size as usize];
            if unsafe { TdhGetProperty(event, 0, null(), 1, &descriptor, size, buffer.as_mut_ptr()) }
                == ERROR_SUCCESS
            {
                return Some(buffer);
            }
        }
        None
    }

    fn little_endian_u64(bytes: &[u8]) -> Option<u64> {
        match bytes.len() {
            1 => Some(bytes[0] as u64),
            2 => Some(u16::from_le_bytes(bytes.try_into().ok()?) as u64),
            4 => Some(u32::from_le_bytes(bytes.try_into().ok()?) as u64),
            8 => Some(u64::from_le_bytes(bytes.try_into().ok()?)),
            _ => None,
        }
    }

    fn same_guid(left: &windows_sys::core::GUID, right: &windows_sys::core::GUID) -> bool {
        left.data1 == right.data1 && left.data2 == right.data2
            && left.data3 == right.data3 && left.data4 == right.data4
    }

    unsafe extern "system" fn event_callback(event: *mut EVENT_RECORD) {
        let Some(event) = event.as_ref() else { return };
        if !same_guid(&event.EventHeader.ProviderId, &TcpIpGuid) || event.UserContext.is_null() { return; }
        let opcode = event.EventHeader.EventDescriptor.Opcode;
        let direction = match opcode {
            10 | 26 => 1_u8,
            11 | 27 => 2_u8,
            _ => return,
        };
        let pid = unsafe { property_bytes(event, &["PID", "ProcessId", "ProcessID"]) }
            .and_then(|bytes| little_endian_u64(&bytes))
            .and_then(|value| u32::try_from(value).ok());
        let bytes = unsafe { property_bytes(event, &["size", "Size", "TransferSize"]) }
            .and_then(|bytes| little_endian_u64(&bytes));
        let (Some(pid), Some(bytes)) = (pid, bytes) else { return };
        if pid == 0 || bytes == 0 { return; }
        let inner = unsafe { &*(event.UserContext as *const Inner) };
        if let Ok(mut counters) = inner.counters.lock() {
            let counter = counters.entry(pid).or_default();
            if direction == 1 { counter.send = counter.send.saturating_add(bytes); }
            else { counter.receive = counter.receive.saturating_add(bytes); }
        }
    }

    unsafe fn run(inner: Arc<Inner>) {
        inner.set_status("starting", "正在启动 Windows TCP/IP ETW 会话");
        let session_name = wide(SESSION_NAME);
        let mut properties = PropertiesBuffer::new();
        let mut session = CONTROLTRACE_HANDLE::default();
        let result = unsafe { StartTraceW(&mut session, session_name.as_ptr(), &mut properties.properties) };
        if result != ERROR_SUCCESS {
            let message = match result {
                ERROR_ACCESS_DENIED => "权限不足，无法启动内核 TCP/IP ETW 会话。请以管理员身份启动采集组件",
                ERROR_ALREADY_EXISTS => "同名 ETW 会话已经存在，请关闭其他大黄狗实例后重试",
                _ => "Windows 无法启动 TCP/IP ETW 会话",
            };
            inner.set_status("error", format!("{message}（错误 {result}）"));
            return;
        }
        inner.session_handle.store(session.Value, Ordering::Release);

        let mut log: EVENT_TRACE_LOGFILEW = unsafe { std::mem::zeroed() };
        log.LoggerName = session_name.as_ptr() as *mut u16;
        log.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
        log.Anonymous2.EventRecordCallback = Some(event_callback);
        log.Context = Arc::as_ptr(&inner) as *mut _;
        let consumer = unsafe { OpenTraceW(&mut log) };
        if consumer.Value == u64::MAX {
            inner.set_status("error", "ETW 会话已经创建，但实时读取通道打开失败");
            let _ = unsafe { ControlTraceW(session, session_name.as_ptr(), &mut properties.properties, EVENT_TRACE_CONTROL_STOP) };
            inner.session_handle.store(0, Ordering::Release);
            return;
        }
        inner.consumer_handle.store(consumer.Value, Ordering::Release);
        inner.set_status("running", "正在通过 Windows ETW 采集并按应用归属网络流量");
        let result = unsafe { ProcessTrace(&consumer, 1, null(), null()) };
        if let Ok(mut status) = inner.status.lock() {
            status.events_lost = status.events_lost.saturating_add(log.EventsLost as u64);
        }
        unsafe { CloseTrace(consumer) };
        inner.consumer_handle.store(u64::MAX, Ordering::Release);
        if !inner.stop.load(Ordering::Acquire) && result != ERROR_SUCCESS {
            inner.set_status("error", format!("ETW 实时事件处理意外停止（错误 {result}）"));
        }
        let _ = unsafe { ControlTraceW(session, session_name.as_ptr(), &mut properties.properties, EVENT_TRACE_CONTROL_STOP) };
        inner.session_handle.store(0, Ordering::Release);
    }

    pub(super) struct PlatformCollector {
        inner: Arc<Inner>,
        worker: Option<JoinHandle<()>>,
    }

    impl PlatformCollector {
        pub fn new() -> Self { Self { inner: Arc::new(Inner::new()), worker: None } }

        pub fn start(&mut self) {
            if self.worker.is_some() { return; }
            self.inner.stop.store(false, Ordering::Release);
            let inner = self.inner.clone();
            self.worker = Some(thread::spawn(move || unsafe { run(inner) }));
        }

        pub fn stop(&mut self) {
            self.inner.stop.store(true, Ordering::Release);
            let session = self.inner.session_handle.load(Ordering::Acquire);
            if session != 0 {
                let mut properties = PropertiesBuffer::new();
                let name = wide(SESSION_NAME);
                unsafe {
                    ControlTraceW(CONTROLTRACE_HANDLE { Value: session }, name.as_ptr(), &mut properties.properties, EVENT_TRACE_CONTROL_STOP);
                }
            }
            let consumer = self.inner.consumer_handle.load(Ordering::Acquire);
            if consumer != u64::MAX { unsafe { CloseTrace(PROCESSTRACE_HANDLE { Value: consumer }); } }
            if let Some(worker) = self.worker.take() { let _ = worker.join(); }
            if let Ok(mut counters) = self.inner.counters.lock() { counters.clear(); }
            self.inner.set_status("disabled", "应用网络监控尚未启用");
        }

        pub fn rates(&self, interval_seconds: u64) -> HashMap<u32, NetworkRate> {
            let Ok(mut counters) = self.inner.counters.lock() else { return HashMap::new() };
            take(&mut *counters).into_iter().map(|(pid, count)| (pid, NetworkRate {
                receive_bps: count.receive / interval_seconds.max(1),
                send_bps: count.send / interval_seconds.max(1),
            })).collect()
        }

        pub fn status(&self) -> CollectorStatus {
            self.inner.status.lock().map(|value| value.clone()).unwrap_or_else(|_| CollectorStatus {
                state: "error".into(), message: "无法读取应用网络采集状态".into(), events_lost: 0,
            })
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    pub(super) struct PlatformCollector;
    impl PlatformCollector {
        pub fn new() -> Self { Self }
        pub fn start(&mut self) {}
        pub fn stop(&mut self) {}
        pub fn rates(&self, _: u64) -> HashMap<u32, NetworkRate> { HashMap::new() }
        pub fn status(&self) -> CollectorStatus { CollectorStatus { state: "unsupported".into(), message: "应用网络 ETW 仅支持 Windows".into(), events_lost: 0 } }
    }
}

pub struct NetworkCollector { platform: platform::PlatformCollector }

impl NetworkCollector {
    pub fn new(enabled: bool) -> Self {
        let mut value = Self { platform: platform::PlatformCollector::new() };
        if enabled { value.platform.start(); }
        value
    }
    pub fn set_enabled(&mut self, enabled: bool) { if enabled { self.platform.start() } else { self.platform.stop() } }
    pub fn rates(&self, interval_seconds: u64) -> HashMap<u32, NetworkRate> { self.platform.rates(interval_seconds) }
    pub fn status(&self) -> CollectorStatus { self.platform.status() }
    pub fn stop(&mut self) { self.platform.stop(); }
}

impl Drop for NetworkCollector { fn drop(&mut self) { self.stop(); } }
