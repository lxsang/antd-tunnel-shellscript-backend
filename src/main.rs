//! # //! Echo publisher example of the tunnel API
//!
//! **Author**: "Dany LE"
//!
use latpr::tunnel::{CallbackEvent, IOInterest, Msg, MsgKind, Topic};
use latpr::utils::{LogLevel, LOG};
use latpr::utils::*;
use latpr::{ERROR, EXIT, INFO, WARN};
use std::collections::HashMap;
use std::env;
use std::io::{Read, Write};
use std::os::unix::io::{AsRawFd, RawFd};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
//use std::fs;
use std::panic;
//use std::vec::Vec;

const STEP_TO_MS: u64 = 100;

struct ClientData {
    fd: RawFd,
    child: Child,
}

fn unsubscribe_client(
    opt: &mut Option<ClientData>,
    topic: &mut Topic,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(client_data) = opt {
        // un register IO
        topic.unregister_io(client_data.fd)?;
        INFO!("Killing the process associated to client");
        if let Err(error) = client_data.child.kill() {
            WARN!(
                "Unable to kill child process, probably because of it has exited: {}",
                error
            );
        }
    }
    Ok(())
}

fn step_handle(
    evt: &CallbackEvent,
    clients: &mut HashMap<u16, Option<ClientData>>,
    topic: &mut Topic,
) -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if let Some(msg) = evt.msg {
        match msg.kind {
            MsgKind::ChannelSubscribe => {
                clients.insert(msg.client_id, None);
                INFO!("Client {} subscribe to channel {}", msg.client_id, &args[2]);
            }
            MsgKind::ChannelUnsubscribe => {
                WARN!(
                    "Client {} unsubscribe to channel {}",
                    msg.client_id,
                    &args[2]
                );
                match clients.remove(&msg.client_id) {
                    None => WARN!("Client {} is not in the client list", msg.client_id),
                    Some(mut opt) => {
                        unsubscribe_client(&mut opt, topic)?;
                    }
                }
            }
            MsgKind::ChannelUnsubscribeAll => {
                INFO!("Unsubcribed all clients from channel {}", args[2]);
                for (key, value) in clients.iter_mut() {
                    let msg = Msg::create(MsgKind::ChannelUnsubscribe, 0, *key, Vec::new());
                    topic.write(&msg)?;
                    unsubscribe_client(value, topic)?;
                }
                clients.clear();
            }
            MsgKind::ChannelData => {
                // create the process if necessary then write data to the handle
                let option = clients
                    .get_mut(&msg.client_id)
                    .ok_or(format!("Client {} is not in the list", msg.client_id))?;
                let client_data = match option {
                    None => {
                        // init the process and register an IO event
                        let process = Command::new(&args[3])
                            .stdin(Stdio::piped())
                            .stdout(Stdio::piped())
                            .spawn()?;
                        let fd = process
                            .stdout
                            .as_ref()
                            .ok_or("Unable to get child process STDOUT")?
                            .as_raw_fd();
                        topic.register_io(fd, IOInterest::READABLE)?;

                        clients.insert(msg.client_id, Some(ClientData { fd, child: process }));
                        clients
                            .get_mut(&msg.client_id)
                            .ok_or(format!("Client {} is not in the list", msg.client_id))?
                            .as_ref()
                            .ok_or(format!("No data found for client {}", msg.client_id))?
                    }
                    Some(c) => c,
                };
                // write data to child
                if let Some(mut stdin) = client_data.child.stdin.as_ref() {
                    stdin.write_all(&msg.data)?;
                }
            }
            _ => {
                WARN!(
                    "Recive mesage kind {} from client {}",
                    msg.kind,
                    msg.client_id
                );
            }
        };
    }
    monitor_clients(clients, topic)?;
    let event = match evt.event {
        None => return Ok(()),
        Some(e) => e,
    };
    let fd = match evt.fd {
        None => return Ok(()),
        Some(d) => d,
    };
    if event.is_readable() {
        // got data send it to client
        let mut buf = [0; 2048];
        let result = clients.iter_mut().filter(|(_k, v)| match v {
            None => false,
            Some(c) => c.fd == fd,
        });
        for (k, v) in result {
            if let Some(client_data) = v {
                if let Some(stdout) = client_data.child.stdout.as_mut() {
                    let n = stdout.read(&mut buf[..])?;
                    INFO!("Sending {} bytes of raw data to client {}", n, k);
                    let msg = Msg::create(MsgKind::ChannelData, 0, *k, (&buf[0..n]).to_vec());
                    topic.write(&msg)?;
                }
            }
        }
    }
    Ok(())
}

fn monitor_clients(
    clients: &mut HashMap<u16, Option<ClientData>>,
    topic: &mut Topic,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut list = Vec::new();
    for (key, value) in clients.iter_mut() {
        if let Some(client_data) = value {
            // check if the child is exited
            match client_data.child.try_wait()? {
                Some(status) => {
                    // unregister IO
                    WARN!(
                        "Process attached to client {} has exited with status {}",
                        key,
                        status
                    );
                    topic.unregister_io(client_data.fd)?;
                    list.push(*key);
                }
                None => {}
            }
        }
    }
    for key in list.iter() {
        clients.insert(*key, None);
    }
    Ok(())
}

fn clean_up(n: i32) {
    if n != 0 {
        panic!("{}", format!("Service is terminated by system signal: {}", n));
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // init the system log
    // Create an empty log object and keep it alive in the scope
    // of `main`. When this object is dropped, the syslog will
    // be closed automatically
    let _log = LOG::init_log();
    on_exit(clean_up);
    // read all the arguments
    let args: Vec<String> = env::args().collect();
    // there must be minimum 4 arguments:
    // - the program
    // - the socket file
    // - the topic name
    // - the command to run
    if args.len() != 4 {
        EXIT!("Invalid arguments: {}", format!("{:?}", args));
    }
    let mut clients = HashMap::<u16, Option<ClientData>>::new();
    let mut msg_handle =
        |evt: &CallbackEvent, topic: &mut Topic| step_handle(evt, &mut clients, topic);
    {
        let mut topic = Topic::create(&args[2], &args[1]);
        let mut running = true;
        topic.on_message(&mut msg_handle);
        topic.set_step_to(Duration::from_millis(STEP_TO_MS));
        topic.open()?;
        while running {
            if let Err(error) = topic.step() {
                ERROR!("Error step: {}", error);
                running = false;
            }
        }
    }
    Ok(())
}
