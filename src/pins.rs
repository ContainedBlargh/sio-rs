use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::channel::PinChannel;

struct PinMaps {
    xbus: HashMap<i32, PinChannel>,
    power: HashMap<i32, PinChannel>,
}

static PINS: OnceLock<Mutex<PinMaps>> = OnceLock::new();

fn pins() -> &'static Mutex<PinMaps> {
    PINS.get_or_init(|| {
        Mutex::new(PinMaps {
            xbus: HashMap::new(),
            power: HashMap::new(),
        })
    })
}

pub fn get_pin_channel(pin_id: i32, is_xbus: bool) -> PinChannel {
    let mut p = pins().lock().unwrap();
    if is_xbus {
        p.xbus
            .entry(pin_id)
            .or_insert_with(PinChannel::new_xbus)
            .clone()
    } else {
        p.power
            .entry(pin_id)
            .or_insert_with(PinChannel::new_power)
            .clone()
    }
}
