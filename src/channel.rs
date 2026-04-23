use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::value::{FlatValue, Value};

#[derive(Clone)]
pub enum PinChannel {
    Power(Arc<AtomicI32>),
    XBus(Arc<(Mutex<Option<FlatValue>>, Condvar)>),
}

impl PinChannel {
    pub fn send(&self, value: Value) {
        match self {
            PinChannel::Power(a) => a.store(value.to_int(), Ordering::SeqCst),
            PinChannel::XBus(cv) => {
                let (lock, cvar) = &**cv;
                let mut slot = lock.lock().unwrap();
                while slot.is_some() {
                    let (s, _) = cvar
                        .wait_timeout(slot, Duration::from_millis(100))
                        .unwrap();
                    slot = s;
                }
                *slot = Some(FlatValue::from_value(value));
                cvar.notify_all();
            }
        }
    }

    pub fn receive(&self) -> Value {
        match self {
            PinChannel::Power(a) => Value::I(a.load(Ordering::SeqCst)),
            PinChannel::XBus(cv) => {
                let (lock, cvar) = &**cv;
                let mut slot = lock.lock().unwrap();
                while slot.is_none() {
                    let (s, _) = cvar
                        .wait_timeout(slot, Duration::from_millis(100))
                        .unwrap();
                    slot = s;
                }
                let v = slot.take().unwrap();
                cvar.notify_all();
                v.into_value()
            }
        }
    }

    pub fn sleep_until_ready(&self) {
        if let PinChannel::XBus(cv) = self {
            let (lock, cvar) = &**cv;
            let mut slot = lock.lock().unwrap();
            while slot.is_none() {
                let (s, _) = cvar
                    .wait_timeout(slot, Duration::from_millis(100))
                    .unwrap();
                slot = s;
            }
        }
    }

    pub fn is_xbus(&self) -> bool {
        matches!(self, PinChannel::XBus(_))
    }

    pub fn new_power() -> Self {
        PinChannel::Power(Arc::new(AtomicI32::new(0)))
    }

    pub fn new_xbus() -> Self {
        PinChannel::XBus(Arc::new((Mutex::new(None), Condvar::new())))
    }
}
