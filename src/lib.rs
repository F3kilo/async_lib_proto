use std::ffi::c_void;

use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{Receiver, Sender};

#[repr(u8)]
pub enum ErrorCode {
    Success = 0,
    Failure = 1,
    TryLater = 2,
}

impl ErrorCode {
    pub fn unwrap(&self) {
        match self {
            Self::Success => {}
            Self::Failure => panic!("failure"),
            Self::TryLater => panic!("try later"),
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn run(handle_out: *mut LinkHandle) -> ErrorCode {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread().build() else {
        return ErrorCode::Failure;
    };

    let (client_tx, runtime_rx) = tokio::sync::mpsc::channel(1024);
    let (runtime_tx, client_rx) = tokio::sync::mpsc::channel(1024);

    let link = Link {
        tx: client_tx,
        rx: client_rx,
    };

    *handle_out = LinkHandle(Box::into_raw(Box::new(link)) as *mut c_void);

    rt.block_on(process_commands(runtime_tx, runtime_rx));

    ErrorCode::Success
}

#[no_mangle]
pub unsafe extern "C" fn send_command(link: LinkHandle, command: Command) -> ErrorCode {
    if link.0.is_null() {
        return ErrorCode::Failure;
    }

    let link = link.to_ref_mut();
    if link.tx.blocking_send(command).is_err() {
        return ErrorCode::Failure;
    }

    ErrorCode::Success
}

#[no_mangle]
pub unsafe extern "C" fn receive_response(
    link: LinkHandle,
    response_out: *mut Response,
) -> ErrorCode {
    if link.0.is_null() {
        return ErrorCode::Failure;
    }

    let link = link.to_ref_mut();
    *response_out = match link.rx.try_recv() {
        Ok(response) => response,
        Err(TryRecvError::Empty) => return ErrorCode::TryLater,
        Err(TryRecvError::Disconnected) => return ErrorCode::Failure,
    };

    ErrorCode::Success
}

#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct LinkHandle(*mut c_void);

impl Default for LinkHandle {
    fn default() -> Self {
        Self(std::ptr::null_mut())
    }
}

impl LinkHandle {
    unsafe fn to_ref_mut(&self) -> &mut Link {
        &mut *(self.0 as *mut Link)
    }
}

struct Link {
    tx: Sender<Command>,
    rx: Receiver<Response>,
}

#[repr(u8)]
#[derive(Debug)]
pub enum Command {
    Exit,
    ComA,
    ComB,
}

#[repr(u8)]
#[derive(Debug, PartialEq, Eq)]
pub enum Response {
    ResA,
    ResB,
}

async fn process_commands(tx: Sender<Response>, mut rx: Receiver<Command>) {
    while let Some(command) = rx.recv().await {
        let send_result = match command {
            Command::Exit => break,
            Command::ComA => tx.send(Response::ResA).await,
            Command::ComB => tx.send(Response::ResB).await,
        };

        if send_result.is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{LinkHandle, run, send_command, Response, receive_response};

    struct LinkHandleSetter(*mut LinkHandle);

    unsafe impl Send for LinkHandleSetter {}
    unsafe impl Sync for LinkHandleSetter {}

    #[test]
    fn runtime_works() {
        let link_handle = LinkHandle::default();
        let setter = LinkHandleSetter(&link_handle as *const _ as *mut _);

        std::thread::spawn(move || unsafe {
            let setter = setter;
            run(setter.0).unwrap();
        });
        
        while link_handle.0.is_null() {
            std::thread::yield_now();
        }

        unsafe {
            send_command(link_handle, crate::Command::ComA).unwrap();
            send_command(link_handle, crate::Command::ComB).unwrap();
            send_command(link_handle, crate::Command::ComA).unwrap();
            send_command(link_handle, crate::Command::ComB).unwrap();

            std::thread::sleep(Duration::from_secs(1));

            let mut response_out = Response::ResB;
            receive_response(link_handle, &mut response_out as _).unwrap();
            assert_eq!(response_out, Response::ResA);
            receive_response(link_handle, &mut response_out as _).unwrap();
            assert_eq!(response_out, Response::ResB);
            receive_response(link_handle, &mut response_out as _).unwrap();
            assert_eq!(response_out, Response::ResA);
            receive_response(link_handle, &mut response_out as _).unwrap();
            assert_eq!(response_out, Response::ResB);
        }
    }
}