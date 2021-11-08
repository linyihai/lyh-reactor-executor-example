use std::io::prelude::*;
use rand::prelude::*;
use std::os::unix::io::AsRawFd;

use std::{
    collections::{HashMap}, 
    io, 
    net::{TcpListener, TcpStream}, 
    sync::{Mutex, mpsc::channel
}};

use lazy_static::lazy_static;

mod executor;
mod reactor;
mod poll;

// 用来识别所有注册的事件
type EventId = usize;


lazy_static! {
    static ref EXECUTOR: Mutex<executor::Executor> = Mutex::new(executor::Executor::new());
    static ref REACTOR: Mutex<reactor::Reactor> = Mutex::new(reactor::Reactor::new());
    static ref CONTEXTS: Mutex<HashMap<EventId, RequestContext>> = Mutex::new(HashMap::new());
}


#[derive(Debug)]
pub struct RequestContext {
    pub stream: TcpStream,
    pub content_length: usize,
    pub buf: Vec<u8>,
}

const HTTP_RESP: &[u8] = b"HTTP/1.1 200 OK
content-type: text/html
content-length: 5

Hello";

// 处理业务函数。读取TCPStream中的数据然后返回
impl RequestContext {
    fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            buf: Vec::new(),
            content_length: 0,
        }
    }

    fn read_cb(&mut self, event_id: EventId, exec: &mut executor::Executor) -> io::Result<()> {
        let mut buf = [0u8; 4096];
        match self.stream.read(&mut buf) {
            Ok(_) => {
                if let Ok(data) = std::str::from_utf8(&buf) {
                    self.parse_and_set_content_length(data);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => {
                return Err(e);
            }
        };
        self.buf.extend_from_slice(&buf);
        if self.buf.len() >= self.content_length {
            REACTOR
                .lock()
                .expect("can get reactor lock")
                .write_interest(self.stream.as_raw_fd(), event_id)
                .expect("can set write interest");

            write_cb(exec, event_id);
        } else {
            REACTOR
                .lock()
                .expect("can get reactor lock")
                .read_interest(self.stream.as_raw_fd(), event_id)
                .expect("can set write interest");
            read_cb(exec, event_id);
        }
        Ok(())
    }

    fn parse_and_set_content_length(&mut self, data: &str) {
        if data.contains("HTTP") {
            if let Some(content_length) = data
                .lines()
                .find(|l| l.to_lowercase().starts_with("content-length: "))
            {
                if let Some(len) = content_length
                    .to_lowercase()
                    .strip_prefix("content-length: ")
                {
                    self.content_length = len.parse::<usize>().expect("content-length is valid");
                    println!("set content length: {} bytes", self.content_length);
                }
            }
        }
    }

    fn write_cb(&mut self, event_id: EventId) -> io::Result<()> {
        println!("in write event of stream with event id: {}", event_id);
        match self.stream.write(HTTP_RESP) {
            Ok(_) => println!("answered from request {}", event_id),
            Err(e) => eprintln!("could not answer to request {}, {}", event_id, e),
        };
        self.stream
            .shutdown(std::net::Shutdown::Both)
            .expect("can close a stream");

        REACTOR
            .lock()
            .expect("can get reactor lock")
            .close(self.stream.as_raw_fd())
            .expect("can close fd and clean up reactor");

        Ok(())
    }
}


// 1. reactor.run() epoll_wait 轮询就绪事件
// 2. read_interest 注册一个可读事件，当客户端连接过来后触发。
// 3. 1中的轮询到客户端连接事件，通过channel通知executor
// 4. executor通过channel接受后，开始处理
fn main() -> io::Result<()> {
    let listener_event_id = 100;
    let listener = TcpListener::bind("127.0.0.1:8000")?;
    listener.set_nonblocking(true)?;
    let listener_fd = listener.as_raw_fd();
    println!("start server");
    let (sender, receiver) = channel();

    // reactor 中使用epoll_wait 系统调用轮询事件是否就绪。就绪通过channel 通知 executor
    match REACTOR.lock() {
        Ok(mut re) => re.run(sender),
        Err(e) => panic!("error running reactor, {}", e),
    };

    // 注册事件。当listener_fd 有可读事件后，可以在listener.accept 里处理
    REACTOR
        .lock()
        .expect("can get reactor lock")
        .read_interest(listener_fd, listener_event_id)?;


    listener_cb(listener, listener_event_id);

    // 轮询 channel 是否有就绪事件
    while let Ok(event_id) = receiver.recv() {
        EXECUTOR
            .lock()
            .expect("can get an executor lock")
            .run(event_id);
    }

    Ok(())
}

fn listener_cb(listener: TcpListener, event_id: EventId) {
    let mut exec_lock = EXECUTOR.lock().expect("can get executor lock");
    exec_lock.await_keep(event_id, move |exec| {
        match listener.accept() {
            Ok((stream, addr)) => {
                let event_id: EventId = random();
                stream.set_nonblocking(true).expect("nonblocking works");
                println!(
                    "new client: {}, event_id: {}, raw fd: {}",
                    addr,
                    event_id,
                    stream.as_raw_fd()
                );
                // 注册客户端可读事件
                REACTOR
                    .lock()
                    .expect("can get reactor lock")
                    .read_interest(stream.as_raw_fd(), event_id)
                    .expect("can set read interest");
                // context保存客户端连接，保存到另一个随机的event_id key 
                CONTEXTS
                    .lock()
                    .expect("can lock request contests")
                    .insert(event_id, RequestContext::new(stream));
                // 注册到excutor中
                read_cb(exec, event_id);
            }
            Err(e) => eprintln!("couldn't accept: {}", e),
        };
        REACTOR
            .lock()
            .expect("can get reactor lock")
            .read_interest(listener.as_raw_fd(), event_id)
            .expect("re-register works");
    });
    drop(exec_lock);
}

fn read_cb(exec: &mut executor::Executor, event_id: EventId) {
    exec.await_once(event_id, move |write_exec| {
        if let Some(ctx) = CONTEXTS
            .lock()
            .expect("can lock request_contexts")
            .get_mut(&event_id)
        {
            ctx.read_cb(event_id, write_exec)
                .expect("read callback works");
        }
    });
}

fn write_cb(exec: &mut executor::Executor, event_id: EventId) {
    exec.await_once(event_id, move |_| {
        if let Some(ctx) = CONTEXTS
            .lock()
            .expect("can lock request_contexts")
            .get_mut(&event_id)
        {
            ctx.write_cb(event_id).expect("write callback works");
        }
        CONTEXTS
            .lock()
            .expect("can lock request contexts")
            .remove(&event_id);
    });
}



