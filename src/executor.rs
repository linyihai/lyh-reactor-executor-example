use crate::EventId;
use std::collections::HashMap;


// 用来识别所有注册的事件
pub struct Executor {
    event_map: HashMap<EventId, Box<dyn FnMut(&mut Self) + Sync + Send + 'static>>,
    event_map_once: HashMap<EventId, Box<dyn FnOnce(&mut Self) + Sync + Send + 'static>>,
}

// 接受就绪事件通知，调用回调函数处理就绪事件
impl Executor {
    pub fn new() -> Self {
        Self {
            event_map: HashMap::new(),
            event_map_once: HashMap::new()
        }
    }

    pub fn await_once(
        &mut self,
        event_id: EventId,
        f: impl FnMut(&mut Self) + Sync + Send + 'static,
    ) {
        self.event_map_once.insert(event_id, Box::new(f));
    }

    pub fn await_keep(
        &mut self,
        event_id: EventId,
        f: impl FnMut(&mut Self) + Sync + Send + 'static,
    ) {
        self.event_map.insert(event_id, Box::new(f));
    }

    pub fn run(&mut self, event_id: EventId) {
        if let Some(mut fun) = self.event_map.remove(&event_id) {
            fun(self);
            self.event_map.insert(event_id, fun);
        } else {
            if let Some(fun) = self.event_map_once.remove(&event_id) {
                fun(self);
            }
        }
    }
}