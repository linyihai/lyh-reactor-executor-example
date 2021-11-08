use crate::{
    poll::{Poll, Registry},
    EventId,
};
use std::io;
use std::os::unix::prelude::RawFd;
use std::sync::mpsc::Sender;


pub struct Reactor {
    // registry 提供操作fd读写实际的api，设置成Option，可以初始为None，后续通过lazy_static懒加载
    pub registry: Option<Registry>,
}

// 注册事件调度器
impl Reactor {
    pub fn new() -> Self {
        Self { registry: None}
    }
    // 轮询就绪实际，一有实际就绪，就通过channel发送出去
    pub fn run(&mut self, sender: Sender<EventId>) {
        let poller = Poll::new();
        let registry = poller.get_registry();
        self.registry = Some(registry);
        std::thread::spawn(move || {
            let mut events: Vec<libc::epoll_event> = Vec::with_capacity(1024);
            loop {
                poller.poll(&mut events);

                for e in &events {
                    sender.send(e.u64 as EventId).expect("channel works");
                }
            }
        });
    }
    // 提供注册fd的可读事件
    pub fn read_interest(&mut self, fd:RawFd, event_id: EventId) -> io::Result<()> {
        self.registry.as_mut().expect("registry read is set").register_read(fd, event_id)
    }
    // 注册fd的可写事件
    pub fn write_interest(&mut self, fd:RawFd, event_id: EventId) -> io::Result<()> {
        self.registry.as_mut().expect("registry write is set").register_write(fd, event_id)
    }
    // 关闭fd
    pub fn close(&mut self, fd:RawFd) -> io::Result<()> {
        self.registry.as_mut().expect("registry close is set").remove_interests(fd)
    }

}
